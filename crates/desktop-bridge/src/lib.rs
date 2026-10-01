//! Native-free desktop boundary. All requests use endpoint-bound proof and the
//! very same TCP connection; no arbitrary request, process, or token command.
mod chat;
pub mod dto;
mod error;
mod settings;
mod sse;
pub use dto::*;
pub use error::{BridgeError, Result};
use hyper::Method;
use runtime_api::{
    Config,
    token::{init_private_token, write_private_new},
};
use runtime_cli::{
    client::VerifiedConnection,
    instance::{Discovery, InstanceLock, wait_stopped},
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::{
    fs, io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::Mutex as AsyncMutex;

pub struct DesktopBridge {
    root: PathBuf,
    executable: PathBuf,
    work: AsyncMutex<()>,
    snapshot_gate: AsyncMutex<()>,
    close_gate: AsyncMutex<()>,
    closing: AtomicBool,
    close_signal: tokio::sync::Notify,
    import_disconnected: AtomicBool,
    chat: Mutex<chat::ChatSlot>,
}
impl DesktopBridge {
    /// Paths come from the native shell's verified package layout, never invoke.
    pub fn new(data_dir: PathBuf, runtime_executable: PathBuf) -> Result<Self> {
        if !data_dir.is_absolute()
            || !runtime_executable.is_absolute()
            || runtime_executable.file_name()
                != Some(std::ffi::OsStr::new(if cfg!(windows) {
                    "ai-runtime.exe"
                } else {
                    "ai-runtime"
                }))
        {
            return Err(BridgeError::new("packaged_runtime_missing"));
        }
        Ok(Self {
            root: data_dir,
            executable: runtime_executable,
            work: AsyncMutex::new(()),
            snapshot_gate: AsyncMutex::new(()),
            close_gate: AsyncMutex::new(()),
            closing: AtomicBool::new(false),
            close_signal: tokio::sync::Notify::new(),
            import_disconnected: AtomicBool::new(false),
            chat: Mutex::new(chat::ChatSlot::default()),
        })
    }
    fn open(&self) -> Result<()> {
        if self.closing.load(Ordering::Acquire) {
            Err(BridgeError::new("desktop_closing"))
        } else {
            Ok(())
        }
    }
    async fn connect(&self) -> Result<VerifiedConnection> {
        runtime_cli::client::connect_data_dir(&self.root)
            .await
            .map_err(|_| BridgeError::new("connection_failed"))
    }
    async fn json<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<T> {
        let mut connection = self.connect().await?;
        let value = connection.json(method, path, body).await?;
        serde_json::from_value(value).map_err(|_| BridgeError::new("response_invalid"))
    }
    pub async fn snapshot(&self) -> Result<DesktopSnapshot> {
        let _gate = self
            .snapshot_gate
            .try_lock()
            .map_err(|_| BridgeError::new("desktop_busy"))?;
        self.snapshot_inner().await
    }
    async fn snapshot_inner(&self) -> Result<DesktopSnapshot> {
        if !self.root.exists()
            || (!self.root.join("config.toml").exists()
                && !self.root.join("secrets/api-token").exists())
        {
            return Ok(DesktopSnapshot {
                initialized: false,
                connection: ConnectionState::Stopped,
                api_address: None,
                runtime: None,
                settings: DesktopSettings::default(),
            });
        }
        let config = settings::require_initialized(&self.root)?;
        let preferences = settings::preferences(&self.root)?;
        let mut snapshot = DesktopSnapshot {
            initialized: true,
            connection: ConnectionState::Stopped,
            api_address: Some(format!("http://{}", config.api.listen)),
            runtime: None,
            settings: preferences.with_idle(config.runtime.idle_unload_seconds),
        };
        if let Some(lock) = InstanceLock::try_acquire(&self.root)
            .map_err(|_| BridgeError::new("instance_unavailable"))?
        {
            if lock.has_discovery() {
                // A free OS lock alone does not establish successful cleanup.
                snapshot.connection = ConnectionState::Error;
            }
            return Ok(snapshot);
        }
        match Discovery::read(&self.root) {
            Ok(discovery) => {
                snapshot.api_address = Some(format!("http://{}", discovery.listen));
                match self.status().await {
                    Ok(status) => {
                        snapshot.runtime = Some(status);
                        snapshot.connection = ConnectionState::Connected;
                    }
                    Err(_) => snapshot.connection = ConnectionState::Error,
                }
            }
            Err(_) => snapshot.connection = ConnectionState::Error,
        }
        Ok(snapshot)
    }
    async fn status(&self) -> Result<RuntimeStatus> {
        let mut status: RuntimeStatus = self.json(Method::GET, "/runtime/status", None).await?;
        if let Some(error) = status.last_error.as_mut() {
            *error = BridgeError::api(Some(&error.code));
        }
        Ok(status)
    }
    pub async fn start(&self, initialize_if_missing: bool) -> Result<DesktopSnapshot> {
        self.open()?;
        let _work = self
            .work
            .try_lock()
            .map_err(|_| BridgeError::new("desktop_busy"))?;
        self.open()?;
        if settings::require_initialized(&self.root).is_err() {
            if !initialize_if_missing {
                return Err(BridgeError::new("not_initialized"));
            }
            let _lock = InstanceLock::try_acquire(&self.root)
                .map_err(|_| BridgeError::new("instance_unavailable"))?
                .ok_or_else(|| BridgeError::new("runtime_running"))?;
            init_private_token(&self.root)
                .map_err(|_| BridgeError::new("credentials_unavailable"))?;
            match write_private_new(
                &self.root.join("config.toml"),
                Config::default()
                    .to_toml()
                    .map_err(|_| BridgeError::new("configuration_invalid"))?
                    .as_bytes(),
            ) {
                Ok(()) => (),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
                Err(_) => return Err(BridgeError::new("configuration_unavailable")),
            }
            settings::require_initialized(&self.root)?;
        }
        let lock = InstanceLock::try_acquire(&self.root)
            .map_err(|_| BridgeError::new("instance_unavailable"))?;
        if lock.is_none() {
            let snapshot = self.snapshot_inner().await?;
            return if matches!(snapshot.connection, ConnectionState::Connected) {
                Ok(snapshot)
            } else {
                Err(BridgeError::new("connection_failed"))
            };
        }
        self.check_executable()?;
        drop(lock); // serve owns acquisition/bind arbitration; we never remove discovery.
        let mut command = Command::new(&self.executable);
        command
            .arg("--data-dir")
            .arg(&self.root)
            .arg("serve")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command.current_dir(
            self.executable
                .parent()
                .ok_or_else(|| BridgeError::new("packaged_runtime_missing"))?,
        );
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            use windows_sys::Win32::System::Threading::{
                CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS,
            };
            command.creation_flags(
                CREATE_BREAKAWAY_FROM_JOB | CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS,
            );
        }
        let mut child = command
            .spawn()
            .map_err(|error| BridgeError::spawn(&error))?;
        let started = tokio::time::Instant::now();
        let result = loop {
            if let Ok(snapshot) = self.snapshot_inner().await
                && matches!(snapshot.connection, ConnectionState::Connected)
            {
                break Ok(snapshot);
            }
            if child
                .try_wait()
                .map_err(|_| BridgeError::new("runtime_start_failed"))?
                .is_some()
            {
                // A competing start may have won. Discover/prove once, never replay.
                break match self.snapshot_inner().await {
                    Ok(s) if matches!(s.connection, ConnectionState::Connected) => Ok(s),
                    _ => Err(BridgeError::new("runtime_start_failed")),
                };
            }
            if started.elapsed() >= Duration::from_secs(30) {
                break Err(BridgeError::new("runtime_start_failed"));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        // Reap a child started by us without owning/killing its lifetime. No UI
        // pipe, async-runtime shutdown wait, or drop-triggered termination.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        result
    }
    fn check_executable(&self) -> Result<()> {
        for p in [
            &self.executable,
            &self.executable.with_file_name(if cfg!(windows) {
                "ai-runtime-worker.exe"
            } else {
                "ai-runtime-worker"
            }),
        ] {
            if !fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_file()) {
                return Err(BridgeError::new("packaged_runtime_missing"));
            }
        }
        Ok(())
    }
    pub async fn models_page(&self, after: Option<String>) -> Result<ModelsPage> {
        self.open()?;
        let path = match after.as_deref() {
            Some(id) => format!(
                "/runtime/models?limit=64&after={}",
                runtime_types::ModelId::new(id).map_err(|_| BridgeError::new("invalid_request"))?
            ),
            None => "/runtime/models?limit=64".into(),
        };
        let page: ModelsPage = self.json(Method::GET, &path, None).await?;
        if page.data.len() > 64
            || page.data.windows(2).any(|w| w[0].id >= w[1].id)
            || page
                .data
                .first()
                .is_some_and(|m| after.as_deref().is_some_and(|a| m.id.as_str() <= a))
            || page.next_after.as_ref().is_some_and(|next| {
                runtime_types::ModelId::new(next).is_err()
                    || page.data.last().is_none_or(|m| m.id.as_str() != next)
            })
        {
            return Err(BridgeError::new("response_invalid"));
        }
        Ok(page)
    }
    pub async fn import_model(&self, path: PathBuf, model_id: String) -> Result<ModelSummary> {
        self.open()?;
        let _work = self
            .work
            .try_lock()
            .map_err(|_| BridgeError::new("desktop_busy"))?;
        self.open()?;
        let id = runtime_types::ModelId::new(model_id)
            .map_err(|_| BridgeError::new("invalid_request"))?;
        let path = local_source(&path)?;
        #[derive(serde::Deserialize)]
        struct ImportResponse {
            model: ModelSummary,
            id: runtime_types::ModelId,
            size_bytes: u64,
            sha256: String,
        }
        let body = json!({"id":id,"file":path});
        let response: ImportResponse = tokio::select! {
            biased;
            _=self.closing_requested()=>{
                // Dropping this future drops only its proved TCP stream. The
                // API ImportGuard cancels only this import; no global shutdown.
                self.import_disconnected.store(true,Ordering::Release);
                return Err(BridgeError::new("import_interrupted"));
            }
            result=self.json(Method::POST,"/runtime/models/import",Some(&body))=>result?,
        };
        if response.id != id
            || response.model.id != id
            || response.model.size_bytes != response.size_bytes
            || response.model.sha256 != response.sha256
        {
            return Err(BridgeError::new("response_invalid"));
        }
        Ok(response.model)
    }
    pub async fn load_model(&self, request: LoadModelRequest) -> Result<RuntimeStatus> {
        self.open()?;
        let _work = self
            .work
            .try_lock()
            .map_err(|_| BridgeError::new("desktop_busy"))?;
        self.open()?;
        let id = runtime_types::ModelId::new(request.model_id)
            .map_err(|_| BridgeError::new("invalid_request"))?;
        runtime_types::LoadOptions {
            context_size: request.context_size,
            threads: request.threads,
            batch_size: request.batch_size,
        }
        .validate()
        .map_err(|_| BridgeError::new("settings_invalid"))?;
        self.json(Method::POST,"/runtime/load",Some(&json!({"model":id,"backend":"cpu","gpu_layers":0,"context_size":request.context_size,"threads":request.threads,"batch_size":request.batch_size}))).await
    }
    pub async fn unload_model(&self) -> Result<RuntimeStatus> {
        self.open()?;
        let _work = self
            .work
            .try_lock()
            .map_err(|_| BridgeError::new("desktop_busy"))?;
        self.open()?;
        self.json(Method::POST, "/runtime/unload", Some(&json!({})))
            .await
    }
    pub async fn settings_save(&self, preferences: DesktopPreferences) -> Result<DesktopSnapshot> {
        self.open()?;
        let _work = self
            .work
            .try_lock()
            .map_err(|_| BridgeError::new("desktop_busy"))?;
        self.open()?;
        settings::require_initialized(&self.root)?;
        settings::save_preferences(&self.root, &preferences)?;
        self.snapshot_inner().await
    }
    pub async fn save_idle(&self, idle_unload_seconds: u64) -> Result<DesktopSnapshot> {
        self.open()?;
        let _work = self
            .work
            .try_lock()
            .map_err(|_| BridgeError::new("desktop_busy"))?;
        self.open()?;
        let lock = InstanceLock::try_acquire(&self.root)
            .map_err(|_| BridgeError::new("instance_unavailable"))?
            .ok_or_else(|| BridgeError::new("runtime_running"))?;
        let mut config = settings::require_initialized(&self.root)?;
        if !(1..=86400).contains(&idle_unload_seconds) {
            return Err(BridgeError::new("settings_invalid"));
        }
        config.runtime.idle_unload_seconds = idle_unload_seconds;
        config
            .validate()
            .map_err(|_| BridgeError::new("settings_invalid"))?;
        settings::atomic_replace(
            &self.root.join("config.toml"),
            config
                .to_toml()
                .map_err(|_| BridgeError::new("configuration_invalid"))?
                .as_bytes(),
        )?;
        drop(lock);
        self.snapshot_inner().await
    }
    pub async fn stop(&self) -> Result<Stopped> {
        self.open()?;
        let _work = self
            .work
            .try_lock()
            .map_err(|_| BridgeError::new("desktop_busy"))?;
        self.open()?;
        self.stop_inner().await
    }
    async fn stop_inner(&self) -> Result<Stopped> {
        if !self.root.exists() {
            return Ok(Stopped { stopped: true });
        }
        settings::require_initialized(&self.root)?;
        if let Some(lock) = InstanceLock::try_acquire(&self.root)
            .map_err(|_| BridgeError::new("instance_unavailable"))?
        {
            return if lock.has_discovery() {
                Err(BridgeError::new("runtime_stop_unconfirmed"))
            } else {
                Ok(Stopped { stopped: true })
            };
        }
        let discovery =
            Discovery::read(&self.root).map_err(|_| BridgeError::new("connection_failed"))?;
        let mut connection = VerifiedConnection::connect(
            discovery.listen,
            discovery.instance_id,
            runtime_api::token::load_private_token(&self.root)
                .map_err(|_| BridgeError::new("credentials_unavailable"))?,
        )
        .await?;
        let _: Value = connection
            .json(Method::POST, "/runtime/shutdown", Some(&json!({})))
            .await?;
        drop(connection);
        wait_stopped(&self.root, discovery.instance_id, Duration::from_secs(30))
            .await
            .map_err(|_| BridgeError::new("runtime_stop_unconfirmed"))?;
        Ok(Stopped { stopped: true })
    }
    async fn closing_requested(&self) {
        loop {
            let notified = self.close_signal.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.closing.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }
    pub async fn close(&self) -> Result<()> {
        self.close_with(false).await
    }
    pub async fn close_ui_only(&self) -> Result<()> {
        self.close_with(true).await
    }
    async fn close_with(&self, ui_only: bool) -> Result<()> {
        let _close = self.close_gate.lock().await;
        self.closing.store(true, Ordering::Release);
        self.close_signal.notify_waiters();
        let result = async {
            self.close_chat().await?;
            let _work = tokio::time::timeout(Duration::from_secs(10), self.work.lock())
                .await
                .map_err(|_| BridgeError::new("desktop_busy"))?;
            if self.import_disconnected.load(Ordering::Acquire) {
                tokio::time::timeout(Duration::from_secs(10), async {
                    loop {
                        let status = self.status().await?;
                        if !status.registry_busy {
                            return Ok::<_, BridgeError>(());
                        }
                        // Registry ownership is not exposed. A competing
                        // client's busy registry is never cancelled by us.
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                })
                .await
                .map_err(|_| BridgeError::new("import_cleanup_unconfirmed"))??;
                self.import_disconnected.store(false, Ordering::Release);
            }
            let stop = !ui_only
                && self.root.join("config.toml").exists()
                && settings::preferences(&self.root)?.close_runtime_on_exit;
            if stop {
                self.stop_inner().await?;
            }
            Ok(())
        }
        .await;
        if result.is_err() {
            self.closing.store(false, Ordering::Release);
        }
        result
    }
}
pub fn default_data_dir() -> Result<PathBuf> {
    runtime_cli::command::parse(vec!["status".into()])
        .map(|o| o.data_dir)
        .map_err(|_| BridgeError::new("data_directory_unavailable"))
}
fn local_source(path: &Path) -> Result<PathBuf> {
    let text = path
        .to_str()
        .ok_or_else(|| BridgeError::new("invalid_model_source"))?;
    if !path.is_absolute()
        || text.contains("://")
        || text.starts_with("\\\\")
        || text.starts_with("//")
        || text.contains('\0')
        || !fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_file())
    {
        return Err(BridgeError::new("invalid_model_source"));
    }
    let canonical = fs::canonicalize(path).map_err(|_| BridgeError::new("invalid_model_source"))?;
    #[cfg(windows)]
    if let Some(normal) = canonical
        .to_str()
        .and_then(|s| s.strip_prefix(r"\\?\"))
        .filter(|s| s.as_bytes().get(1) == Some(&b':') && s.as_bytes().get(2) == Some(&b'\\'))
    {
        return Ok(PathBuf::from(normal));
    }
    if canonical.to_str().is_none() {
        return Err(BridgeError::new("invalid_model_source"));
    }
    Ok(canonical)
}
