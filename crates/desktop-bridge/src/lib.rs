//! Native-free desktop boundary. All requests use endpoint-bound proof and the
//! very same TCP connection; no arbitrary request, process, or token command.
mod chat;
mod configuration;
mod download;
pub mod dto;
mod error;
mod lan_addresses;
mod library;
mod loading;
mod onboarding;
mod settings;
mod sse;
mod unregister;
pub use dto::*;
pub use error::{BridgeError, Result};
use hyper::Method;
pub use model_store::library::selected::{SelectedFile, validate_selection};
use runtime_api::Config;
pub use runtime_api::configuration::{
    ConfigurationMigrateRequest, ConfigurationSaveRequest, ConfigurationSnapshot,
    ModelConfiguration, UiPreferencesSaveRequest, UiPreferencesSnapshot,
};
use runtime_cli::{
    client::VerifiedConnection,
    instance::{Discovery, InstanceLock, wait_stopped},
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::Mutex as AsyncMutex;

/// Native-only numeric observations for the last accepted start attempt.
/// These are not part of any invoke command or frontend DTO.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StartupDiagnostics {
    pub os_error: Option<i32>,
    pub process_exit_code: Option<i32>,
}

pub struct DesktopBridge {
    startup_diagnostics: Mutex<StartupDiagnostics>,
    root: PathBuf,
    executable: PathBuf,
    work: std::sync::Arc<AsyncMutex<()>>,
    snapshot_gate: AsyncMutex<()>,
    close_gate: AsyncMutex<()>,
    closing: AtomicBool,
    close_signal: tokio::sync::Notify,
    import_disconnected: AtomicBool,
    load_disconnected: AtomicBool,
    directory_validator: Option<std::sync::Arc<library::DirectoryValidator>>,
    chat: Mutex<chat::ChatSlot>,
    library: Mutex<library::LibrarySlot>,
    loads: Mutex<loading::LoadSlot>,
    library_poll: AsyncMutex<()>,
    default_model_directory: Option<PathBuf>,
    downloads: Mutex<download::DownloadSlot>,
    download_poll: AsyncMutex<()>,
    validation_build: Mutex<Option<(Vec<model_store::library::FileIdentity>, String)>>,
    list_generation: Mutex<Option<(uuid::Uuid, uuid::Uuid)>>,
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
            startup_diagnostics: Mutex::new(StartupDiagnostics::default()),
            root: data_dir,
            executable: runtime_executable,
            work: std::sync::Arc::new(AsyncMutex::new(())),
            snapshot_gate: AsyncMutex::new(()),
            close_gate: AsyncMutex::new(()),
            closing: AtomicBool::new(false),
            close_signal: tokio::sync::Notify::new(),
            import_disconnected: AtomicBool::new(false),
            load_disconnected: AtomicBool::new(false),
            directory_validator: None,
            chat: Mutex::new(chat::ChatSlot::default()),
            library: Mutex::new(library::LibrarySlot::default()),
            loads: Mutex::new(loading::LoadSlot::default()),
            library_poll: AsyncMutex::new(()),
            default_model_directory: None,
            downloads: Mutex::new(download::DownloadSlot::default()),
            download_poll: AsyncMutex::new(()),
            validation_build: Mutex::new(None),
            list_generation: Mutex::new(None),
        })
    }
    pub fn startup_diagnostics(&self) -> StartupDiagnostics {
        *self.startup_diagnostics.lock().unwrap()
    }
    fn open(&self) -> Result<()> {
        if self.closing.load(Ordering::Acquire) {
            Err(BridgeError::new("desktop_closing"))
        } else if self.download_active() {
            Err(BridgeError::new("model_download_active"))
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
        let verification = if runtime_cli::client::may_verify_model(&method, path) {
            body.and_then(|body| body.get("model"))
                .and_then(Value::as_str)
                .map(|model| settings::external_verification_budget(&self.root, model))
                .transpose()?
                .flatten()
        } else {
            None
        };
        let value = if let Some(budget) = verification {
            connection
                .json_with_verification(method, path, body, budget)
                .await?
        } else {
            connection.json(method, path, body).await?
        };
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
                configuration: Some(runtime_api::configuration::preview()),
                configuration_error: None,
                ui_preferences: Some(self.ui_preferences_get()?),
                initialized: false,
                connection: ConnectionState::Stopped,
                api_address: None,
                runtime: None,
                settings: settings::preferences(&self.root)?.with_idle(300),
                lan_api: Default::default(),
                model_directory: library::directory_snapshot(&self.root, None)?,
            });
        }
        let config = settings::require_initialized(&self.root)?;
        let preferences = settings::preferences(&self.root)?;
        let mut snapshot = DesktopSnapshot {
            configuration: None,
            configuration_error: None,
            ui_preferences: Some(self.ui_preferences_get()?),
            initialized: true,
            connection: ConnectionState::Stopped,
            api_address: Some(format!("http://{}", config.api.listen)),
            runtime: None,
            settings: preferences.with_runtime(&config),
            lan_api: config.lan_api.clone(),
            model_directory: library::directory_snapshot(&self.root, None)?,
        };
        if self.library.lock().unwrap().owns_instance() || self.download_holds_instance() {
            snapshot.configuration = Some(self.configuration_offline()?);
            return Ok(snapshot);
        }
        if let Some(lock) = InstanceLock::try_acquire(&self.root)
            .map_err(|_| BridgeError::new("instance_unavailable"))?
        {
            snapshot.configuration = Some(self.configuration_offline()?);
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
                        snapshot.configuration =
                            match self.json(Method::GET, "/runtime/configuration", None).await {
                                Ok(configuration) => Some(configuration),
                                Err(error) => {
                                    snapshot.configuration_error =
                                        Some(if error.code == "not_found" {
                                            BridgeError::new("configuration_unavailable")
                                        } else {
                                            error
                                        });
                                    None
                                }
                            };
                        if config.schema_version == 2
                            && let Some(configuration) = snapshot.configuration.as_ref()
                        {
                            let values = &configuration
                                .runtime_effective
                                .as_ref()
                                .ok_or_else(|| BridgeError::new("response_invalid"))?
                                .values;
                            snapshot.settings.context_size = values.global_defaults.context_size;
                            snapshot.settings.threads = values
                                .global_defaults
                                .threads
                                .unwrap_or_else(|| Config::available_parallelism().min(4));
                            snapshot.settings.batch_size = values.global_defaults.batch_size;
                            snapshot.settings.max_output_tokens =
                                values.request_defaults.max_output_tokens;
                            snapshot.settings.idle_unload_enabled =
                                values.runtime.idle_unload_enabled;
                            snapshot.settings.idle_unload_seconds =
                                values.runtime.idle_unload_seconds;
                            snapshot.settings.model_verification_timeout_seconds =
                                values.runtime.model_verification_timeout_seconds;
                            snapshot.lan_api = values.lan_api.clone();
                        }
                        snapshot.model_directory =
                            library::directory_snapshot(&self.root, Some(&status))?;
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
        self.start_inner(initialize_if_missing, None).await
    }
    async fn start_inner(
        &self,
        initialize_if_missing: bool,
        onboarding: Option<&AtomicBool>,
    ) -> Result<DesktopSnapshot> {
        if onboarding.is_none() {
            self.open()?;
        } else if self.closing.load(Ordering::Acquire) {
            return Err(BridgeError::new("desktop_closing"));
        }
        let _work = self
            .work
            .try_lock()
            .map_err(|_| BridgeError::new("desktop_busy"))?;
        if onboarding.is_none() {
            self.open()?;
        }
        self.check_start_cancelled(onboarding)?;
        *self.startup_diagnostics.lock().unwrap() = StartupDiagnostics::default();
        if !self.root.join("config.toml").exists() && !self.root.join("secrets/api-token").exists()
        {
            if !initialize_if_missing {
                return Err(BridgeError::new("not_initialized"));
            }
            let lock = InstanceLock::try_acquire(&self.root)
                .map_err(|_| BridgeError::new("instance_unavailable"))?
                .ok_or_else(|| BridgeError::new("runtime_running"))?;
            if lock.has_discovery() {
                return Err(BridgeError::new("runtime_stop_unconfirmed"));
            }
            runtime_api::configuration::initialize(&self.root)
                .map_err(|e| BridgeError::new(e.code))?;
        }
        settings::require_initialized(&self.root)?;
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
            command.creation_flags(runtime_creation_flags());
        }
        self.check_start_cancelled(onboarding)?;
        let mut child = command.spawn().map_err(|error| {
            self.startup_diagnostics.lock().unwrap().os_error = error.raw_os_error();
            BridgeError::spawn(&error)
        })?;
        let started = tokio::time::Instant::now();
        let result = loop {
            if let Err(error) = self.check_start_cancelled(onboarding) {
                break Err(error);
            }
            if let Ok(snapshot) = self.snapshot_inner().await
                && matches!(snapshot.connection, ConnectionState::Connected)
            {
                break Ok(snapshot);
            }
            if let Some(status) = child.try_wait().map_err(|error| {
                self.startup_diagnostics.lock().unwrap().os_error = error.raw_os_error();
                BridgeError::new("runtime_start_failed")
            })? {
                self.startup_diagnostics.lock().unwrap().process_exit_code = status.code();
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
    fn check_start_cancelled(&self, onboarding: Option<&AtomicBool>) -> Result<()> {
        if self.closing.load(Ordering::Acquire)
            || onboarding.is_some_and(|cancel| cancel.load(Ordering::Acquire))
        {
            Err(BridgeError::new("request_cancelled"))
        } else {
            Ok(())
        }
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
    pub async fn models_page(
        &self,
        after: Option<String>,
        generation: Option<uuid::Uuid>,
    ) -> Result<ModelsPage> {
        if self.closing.load(Ordering::Acquire) {
            return Err(BridgeError::new("desktop_closing"));
        }
        // These are this bridge's offline owners, not a proved API service.
        let local_owner =
            self.library.lock().unwrap().owns_instance() || self.download_holds_instance();
        if after.is_some() && generation.is_none() {
            return Err(BridgeError::new("model_list_changed"));
        }
        if let Some(after) = &after {
            runtime_types::ModelId::new(after).map_err(|_| BridgeError::new("invalid_request"))?;
        }
        if self.root.exists() {
            model_store::inventory::validate_data_directory(&self.root)
                .map_err(|_| BridgeError::new("data_directory_unavailable"))?;
        }
        if local_owner {
            return self.local_models_page(after.as_deref(), generation);
        }
        match InstanceLock::observe(&self.root)
            .map_err(|_| BridgeError::new("instance_unavailable"))?
        {
            runtime_cli::instance::InstanceObservation::Stopped(_guard) => {
                return self.local_models_page(after.as_deref(), generation);
            }
            runtime_cli::instance::InstanceObservation::Running => (),
        }
        let upstream_generation = if let Some(generation) = generation {
            Some(
                self.list_generation
                    .lock()
                    .unwrap()
                    .filter(|(public, _)| *public == generation)
                    .ok_or_else(|| BridgeError::new("model_list_changed"))?
                    .1,
            )
        } else {
            None
        };
        let mut path = match after.as_deref() {
            Some(id) => format!(
                "/runtime/models?limit=64&after={}",
                runtime_types::ModelId::new(id).map_err(|_| BridgeError::new("invalid_request"))?
            ),
            None => "/runtime/models?limit=64".into(),
        };
        if after.is_some() && generation.is_none() {
            return Err(BridgeError::new("model_list_changed"));
        }
        if let Some(generation) = upstream_generation {
            path.push_str(&format!("&generation={generation}"));
        }
        let value: Value = self.json(Method::GET, &path, None).await?;
        if value.get("generation").is_none() {
            return Err(BridgeError::new("model_library_unsupported"));
        }
        let mut page: ModelsPage =
            serde_json::from_value(value).map_err(|_| BridgeError::new("response_invalid"))?;
        if page.generation.is_nil()
            || upstream_generation.is_some_and(|expected| expected != page.generation)
        {
            return Err(BridgeError::new("model_list_changed"));
        }
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
        let upstream = page.generation;
        self.runtime_observations(&mut page).await?;
        if generation.is_some_and(|expected| expected != page.generation) {
            return Err(BridgeError::new("model_list_changed"));
        }
        *self.list_generation.lock().unwrap() = Some((page.generation, upstream));
        Ok(page)
    }
    pub async fn import_model(&self, path: PathBuf, model_id: String) -> Result<ModelSummary> {
        self.import_files(path, None, model_id).await
    }
    pub async fn import_model_pair(
        &self,
        path: PathBuf,
        projector: PathBuf,
        model_id: String,
    ) -> Result<ModelSummary> {
        self.import_files(path, Some(projector), model_id).await
    }
    async fn import_files(
        &self,
        path: PathBuf,
        projector: Option<PathBuf>,
        model_id: String,
    ) -> Result<ModelSummary> {
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
        let mut body = json!({"id":id,"file":path});
        if let Some(projector) = projector {
            body["projector"] = json!({"file":local_source(&projector)?});
        }
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
        let body = json!({"model":id,"backend":"cpu","gpu_layers":0,"context_size":request.context_size,"threads":request.threads,"batch_size":request.batch_size});
        tokio::select! {
            biased;
            _ = self.closing_requested() => {
                self.load_disconnected.store(true, Ordering::Release);
                Err(BridgeError::new("model_load_interrupted"))
            }
            result = self.json::<Value>(Method::POST,"/runtime/load-and-test",Some(&body)) => {
                let value=result?;
                onboarding::check_load_observation(&value)?;
                serde_json::from_value(value).map_err(|_|BridgeError::new("response_invalid"))
            },
        }
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
        settings::save_preferences(&self.root, &preferences)?;
        self.snapshot_inner().await
    }
    /// Legacy callers change the positive idle budget without toggling its policy.
    pub async fn save_idle(&self, idle_unload_seconds: u64) -> Result<DesktopSnapshot> {
        self.save_idle_policy(idle_unload_seconds, None).await
    }
    pub async fn save_idle_policy(
        &self,
        idle_unload_seconds: u64,
        idle_unload_enabled: Option<bool>,
    ) -> Result<DesktopSnapshot> {
        if !(1..=86400).contains(&idle_unload_seconds) {
            return Err(BridgeError::new("settings_invalid"));
        }
        self.save_runtime_config(|config| {
            config.runtime.idle_unload_seconds = idle_unload_seconds;
            if let Some(enabled) = idle_unload_enabled {
                config.runtime.idle_unload_enabled = enabled;
            }
        })
        .await
    }
    pub async fn save_verification_timeout(&self, seconds: u64) -> Result<DesktopSnapshot> {
        self.save_runtime_config(|config| {
            config.runtime.model_verification_timeout_seconds = seconds;
        })
        .await
    }
    async fn save_runtime_config(
        &self,
        update: impl FnOnce(&mut runtime_api::Config),
    ) -> Result<DesktopSnapshot> {
        self.open()?;
        let _work = self
            .work
            .try_lock()
            .map_err(|_| BridgeError::new("desktop_busy"))?;
        self.open()?;
        if !self.root.join("config.toml").exists() {
            return Err(BridgeError::new("not_initialized"));
        }
        let lock = InstanceLock::try_acquire(&self.root)
            .map_err(|_| BridgeError::new("instance_unavailable"))?
            .ok_or_else(|| BridgeError::new("runtime_running"))?;
        if lock.has_discovery() {
            return Err(BridgeError::new("runtime_stop_unconfirmed"));
        }
        settings::require_initialized(&self.root)?;
        runtime_api::configuration::legacy_update(&self.root, update).map_err(|e| {
            BridgeError::new(if e.code == "configuration_invalid" {
                "settings_invalid"
            } else {
                e.code
            })
        })?;
        drop(lock);
        self.snapshot_inner().await
    }
    /// This configuration publication never starts/stops a service or creates a
    /// LAN credential. The instance lock arbitrates against other windows/CLI.
    pub async fn save_lan(&self, lan_api: runtime_api::LanApiConfig) -> Result<DesktopSnapshot> {
        self.open()?;
        let _work = self
            .work
            .try_lock()
            .map_err(|_| BridgeError::new("desktop_busy"))?;
        self.open()?;
        if !self.root.join("config.toml").exists() {
            return Err(BridgeError::new("not_initialized"));
        }
        let lock = InstanceLock::try_acquire(&self.root)
            .map_err(|_| BridgeError::new("instance_unavailable"))?
            .ok_or_else(|| BridgeError::new("runtime_running"))?;
        if lock.has_discovery() {
            return Err(BridgeError::new("runtime_stop_unconfirmed"));
        }
        settings::require_initialized(&self.root)?;
        lan_api
            .validate()
            .map_err(|_| BridgeError::new("lan_settings_invalid"))?;
        runtime_api::configuration::legacy_update(&self.root, |config| config.lan_api = lan_api)
            .map_err(|e| BridgeError::new(e.code))?;
        drop(lock);
        self.snapshot_inner().await
    }
    /// Native-shell-only explicit copy operation. SecretToken is intentionally
    /// not serializable; never use this from a snapshot, status or normal DTO.
    pub async fn lan_token_for_copy(&self) -> Result<runtime_api::token::SecretToken> {
        self.open()?;
        let _work = self
            .work
            .try_lock()
            .map_err(|_| BridgeError::new("desktop_busy"))?;
        self.open()?;
        let config = settings::require_initialized(&self.root)
            .map_err(|_| BridgeError::new("lan_token_unavailable"))?;
        if !config.lan_api.enabled {
            return Err(BridgeError::new("lan_token_unavailable"));
        }
        let lan_token = runtime_api::token::load_private_lan_token(&self.root)
            .map_err(|_| BridgeError::new("lan_token_unavailable"))?;
        let local_token = runtime_api::token::load_private_token(&self.root)
            .map_err(|_| BridgeError::new("lan_token_unavailable"))?;
        if lan_token.matches_authorization(local_token.bearer_header_value().as_bytes()) {
            return Err(BridgeError::new("lan_token_unavailable"));
        }
        Ok(lan_token)
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
        let token = runtime_api::token::load_private_token(&self.root)
            .map_err(|_| BridgeError::new("credentials_unavailable"))?;
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
        let mut connection =
            VerifiedConnection::connect(discovery.listen, discovery.instance_id, token).await?;
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
            self.close_download().await?;
            self.close_chat().await?;
            self.close_library().await?;
            self.close_load().await?;
            let _work = tokio::time::timeout(Duration::from_secs(10), self.work.lock())
                .await
                .map_err(|_| BridgeError::new("desktop_busy"))?;
            if self.import_disconnected.load(Ordering::Acquire)
                || self.load_disconnected.load(Ordering::Acquire)
            {
                let load_disconnected = self.load_disconnected.load(Ordering::Acquire);
                tokio::time::timeout(Duration::from_secs(10), async {
                    loop {
                        let status = self.status().await?;
                        if !status.registry_busy
                            && !(load_disconnected
                                && matches!(
                                    status.state,
                                    RuntimeState::Loading
                                        | RuntimeState::Unloading
                                        | RuntimeState::Generating
                                ))
                        {
                            return Ok::<_, BridgeError>(());
                        }
                        // Registry ownership is not exposed. A competing
                        // client's busy registry is never cancelled by us.
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                })
                .await
                .map_err(|_| {
                    BridgeError::new(if load_disconnected {
                        "desktop_busy"
                    } else {
                        "import_cleanup_unconfirmed"
                    })
                })??;
                self.import_disconnected.store(false, Ordering::Release);
                self.load_disconnected.store(false, Ordering::Release);
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
#[cfg(windows)]
fn runtime_creation_flags() -> u32 {
    use windows_sys::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS};
    // Respect inherited host Job limits. We do not request breakaway, create a
    // UI-owned kill-on-close Job, or claim survival beyond an external Job.
    CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS
}
/// Native directory picker preflight. Does not expose a path-taking invoke.
pub fn validate_model_directory_path(path: &Path) -> Result<()> {
    model_store::library::validate_directory_candidate(path)
        .map_err(|error| BridgeError::new(error.code.as_str()))
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

#[cfg(all(test, windows))]
mod windows_launch_tests {
    #[test]
    fn runtime_flags_detach_console_but_respect_inherited_job() {
        use windows_sys::Win32::System::Threading::{
            CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS,
        };
        let flags = super::runtime_creation_flags();
        assert_eq!(flags, CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
        assert_eq!(flags & CREATE_BREAKAWAY_FROM_JOB, 0);
    }
}
