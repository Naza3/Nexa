//! A bounded, explicit, single catalog download. No executable content, URL input,
//! implicit source failover. Publication is followed by guarded registration;
//! optional text onboarding never evicts an existing actor selection.
use crate::*;
use download_engine::{
    DownloadControl, DownloadProgress, DownloadSpec, SidecarConfig, StagingPaths, TransferOptions,
    TransferReport,
};
use model_store::library::{
    ModelLibrary,
    download::{SidecarDownloadFile, VerifiedSidecarFile},
};
#[path = "source_adapter.rs"]
mod source_adapter;
use source_adapter::catalog;
use std::sync::Arc;
use tokio::sync::{Notify, watch};
use uuid::Uuid;

#[derive(Default)]
pub(crate) struct DownloadSlot {
    current: Option<Arc<DownloadTask>>,
    previous: Option<Arc<DownloadTask>>,
    verifier: Option<Arc<SidecarVerifier>>,
}
struct DownloadTask {
    state: Mutex<DownloadOperationState>,
    control: DownloadControl,
    onboarding_cancelled: AtomicBool,
    scan_control: Mutex<Option<Arc<model_store::library::ScanControl>>>,
    changed: Notify,
    retained: Mutex<Option<Box<dyn Send>>>,
}
type SidecarVerifier = dyn Fn() -> Result<VerifiedSidecar> + Send + Sync;
struct VerifiedSidecar {
    config: SidecarConfig,
    _guard: Box<dyn Send>,
}
impl Drop for DownloadTask {
    fn drop(&mut self) {
        if let Some(resources) = self.retained.get_mut().unwrap().take() {
            // Only an unrecoverable supervisor failure reaches quarantine. Keep
            // every guard and instance lock until process exit, never pretend
            // an unconfirmed writer stopped merely because its task was dropped.
            std::mem::forget(resources);
        }
    }
}
impl DownloadSlot {
    fn task(&self, id: Uuid) -> Result<Arc<DownloadTask>> {
        self.current
            .iter()
            .chain(self.previous.iter())
            .find(|task| task.state.lock().unwrap().operation_id == id)
            .cloned()
            .ok_or_else(|| BridgeError::new("request_not_owned"))
    }
}
impl DownloadTask {
    fn cancel(&self) {
        self.control.cancel();
        self.onboarding_cancelled.store(true, Ordering::Release);
        if let Some(control) = self.scan_control.lock().unwrap().as_ref() {
            control.cancel();
        }
        self.changed.notify_waiters();
    }
    async fn wait_onboarding_cancelled(&self) {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.onboarding_cancelled.load(Ordering::Acquire) {
                return;
            }
            changed.await;
        }
    }
    fn progress(&self, progress: DownloadProgress) {
        let mut state = self.state.lock().unwrap();
        // Identity and total belong to the admitted catalog operation, not transport.
        if progress.total_bytes != state.total_bytes
            || progress.attempt < state.attempt
            || !(1..=2).contains(&progress.attempt)
            || progress.written_bytes > state.total_bytes
            || (progress.attempt == state.attempt
                && progress.written_bytes < state.downloaded_bytes)
        {
            return;
        }
        state.downloaded_bytes = progress.written_bytes;
        state.attempt = progress.attempt;
        state.phase = match progress.phase {
            download_engine::DownloadPhase::Connecting => DownloadPhase::Connecting,
            download_engine::DownloadPhase::Downloading => DownloadPhase::Downloading,
            download_engine::DownloadPhase::Verifying => DownloadPhase::Verifying,
            download_engine::DownloadPhase::Committing => DownloadPhase::Committing,
        };
        drop(state);
        self.changed.notify_waiters();
    }
    fn phase(&self, phase: DownloadPhase) {
        self.state.lock().unwrap().phase = phase;
        self.changed.notify_waiters();
    }
    fn quarantine(&self, resources: impl Send + 'static) {
        *self.retained.lock().unwrap() = Some(Box::new(resources));
        self.state.lock().unwrap().error =
            Some(BridgeError::new("model_download_cleanup_unconfirmed"));
        self.changed.notify_waiters();
    }
    fn finish_saved(&self, outcome: DownloadResult) {
        debug_assert!(outcome.saved);
        let mut state = self.state.lock().unwrap();
        state.result = Some(outcome);
        state.status = DownloadStatus::Completed;
        state.phase = DownloadPhase::Finished;
        state.terminal = true;
        drop(state);
        self.changed.notify_waiters();
    }
    fn finish(&self, result: Result<bool>) {
        let mut state = self.state.lock().unwrap();
        state.terminal = true;
        state.phase = DownloadPhase::Finished;
        match result {
            Ok(cleaned) => {
                state.status = DownloadStatus::Completed;
                state.result = Some(DownloadResult {
                    saved: true,
                    registered: false,
                    registration_error: None,
                    local_validation: None,
                    file_name: state.file_name.clone(),
                    cleanup_warning: (!cleaned).then(|| "partial_cleanup_unconfirmed".into()),
                });
            }
            Err(error) => {
                state.status = if error.code == "model_download_cancelled" {
                    DownloadStatus::Cancelled
                } else {
                    DownloadStatus::Failed
                };
                state.error = Some(error);
            }
        }
        drop(state);
        self.changed.notify_waiters();
    }
}
fn store_error(error: runtime_types::RuntimeError) -> BridgeError {
    BridgeError::new(error.code.as_str())
}
impl DesktopBridge {
    /// Native-shell-only dependency injection, never an invoke argument. The
    /// verifier must validate and pin the fixed packaged binary, dependencies
    /// and every ancestor for the returned guard's entire lifetime.
    pub fn with_download_sidecar_verifier<F, G>(self, verifier: F) -> Self
    where
        F: Fn() -> Result<(PathBuf, G)> + Send + Sync + 'static,
        G: Send + 'static,
    {
        self.downloads.lock().unwrap().verifier = Some(Arc::new(move || {
            let (executable, guard) = verifier()?;
            if !executable.is_absolute()
                || executable.file_name() != Some(std::ffi::OsStr::new("nexa-aria2.exe"))
            {
                return Err(BridgeError::new("model_download_engine_unavailable"));
            }
            Ok(VerifiedSidecar {
                config: SidecarConfig { executable },
                _guard: Box::new(guard),
            })
        }));
        self
    }
    fn verified_download_sidecar(&self) -> Result<VerifiedSidecar> {
        let verifier = self
            .downloads
            .lock()
            .unwrap()
            .verifier
            .clone()
            .ok_or_else(|| BridgeError::new("model_download_engine_unavailable"))?;
        verifier()
    }
    pub fn model_catalog(&self) -> Result<ModelCatalog> {
        catalog()
    }
    pub(crate) fn download_active(&self) -> bool {
        self.downloads
            .lock()
            .unwrap()
            .current
            .as_ref()
            .is_some_and(|t| !t.state.lock().unwrap().terminal)
    }
    pub(crate) fn download_holds_instance(&self) -> bool {
        self.downloads
            .lock()
            .unwrap()
            .current
            .as_ref()
            .is_some_and(|task| {
                let state = task.state.lock().unwrap();
                !state.terminal && state.phase != DownloadPhase::Testing
            })
    }
    pub fn download_start(self: &Arc<Self>, catalog_id: String) -> Result<DownloadOperationHandle> {
        self.download_start_with_options(catalog_id, false)
    }
    pub fn download_start_with_options(
        self: &Arc<Self>,
        catalog_id: String,
        auto_test: bool,
    ) -> Result<DownloadOperationHandle> {
        self.open()?;
        // Admission is short and fail-fast; no shared async mutex is held across HTTP.
        let _work = self
            .work
            .try_lock()
            .map_err(|_| BridgeError::new("desktop_busy"))?;
        self.open()?;
        let entry = catalog()?
            .entries
            .into_iter()
            .find(|entry| entry.catalog_id == catalog_id)
            .ok_or_else(|| BridgeError::new("model_catalog_not_found"))?;
        if !self.root.exists() {
            return Err(BridgeError::new("model_directory_required"));
        }
        let lock = InstanceLock::try_acquire(&self.root)
            .map_err(|_| BridgeError::new("instance_unavailable"))?
            .ok_or_else(|| BridgeError::new("runtime_running"))?;
        if lock.has_discovery() {
            return Err(BridgeError::new("runtime_stop_unconfirmed"));
        }
        let source = settings::preferences(&self.root)?.download_source;
        let source_entry = entry
            .sources
            .iter()
            .find(|s| s.source == source)
            .cloned()
            .ok_or_else(|| BridgeError::new("model_download_source_unavailable"))?;
        let spec = source_adapter::specification(&entry, &source_entry)?;
        let library = ModelLibrary::read(&self.root)
            .map_err(store_error)?
            .ok_or_else(|| BridgeError::new("model_directory_required"))?;
        if let Some(validate) = &self.directory_validator {
            validate(library.configured_directory().map_err(store_error)?)?;
        }
        let sidecar = self.verified_download_sidecar()?;
        let id = Uuid::new_v4();
        let destination =
            SidecarDownloadFile::create(&library, &entry.file_name, id).map_err(store_error)?;
        let task = Arc::new(DownloadTask {
            state: Mutex::new(DownloadOperationState {
                operation_id: id,
                catalog_id,
                source,
                file_name: entry.file_name.clone(),
                directory_id: library
                    .directory_id
                    .ok_or_else(|| BridgeError::new("model_directory_required"))?,
                target_display_path: library
                    .configured_directory()
                    .map_err(store_error)?
                    .to_string_lossy()
                    .into_owned(),
                downloaded_bytes: 0,
                attempt: 1,
                total_bytes: entry.size_bytes,
                phase: DownloadPhase::Connecting,
                status: DownloadStatus::Running,
                terminal: false,
                result: None,
                error: None,
            }),
            control: DownloadControl::new(),
            onboarding_cancelled: AtomicBool::new(false),
            scan_control: Mutex::new(None),
            changed: Notify::new(),
            retained: Mutex::new(None),
        });
        self.register_download_task(task.clone())?;
        let bridge = self.clone();
        tokio::spawn(async move {
            let result = run_download(
                task.clone(),
                spec,
                ModelDownloadTarget {
                    destination,
                    _lock: lock,
                },
                sidecar,
                &ProductionSupervisor,
            )
            .await;
            // Writer and all protected handles have exited before publishing terminal.
            if let Some(result) = result {
                match result {
                    Err(error) => task.finish(Err(error)),
                    Ok(cleaned) => {
                        task.phase(DownloadPhase::Registering);
                        let registration = bridge.register_download(&task, &entry.sha256).await;
                        let mut outcome = DownloadResult {
                            saved: true,
                            registered: registration.is_ok(),
                            file_name: entry.file_name,
                            cleanup_warning: (!cleaned)
                                .then(|| "partial_cleanup_unconfirmed".into()),
                            registration_error: registration.as_ref().err().cloned(),
                            local_validation: None,
                        };
                        if auto_test && let Ok(id) = registration {
                            task.phase(DownloadPhase::Testing);
                            outcome.local_validation = Some(bridge.test_download(&task, id).await);
                        }
                        task.finish_saved(outcome);
                    }
                }
            }
        });
        Ok(DownloadOperationHandle { operation_id: id })
    }
    async fn register_download(
        &self,
        task: &Arc<DownloadTask>,
        sha: &str,
    ) -> Result<runtime_types::ModelId> {
        let root = self.root.clone();
        let sha = sha.to_owned();
        let task = task.clone();
        tokio::task::spawn_blocking(move || {
            let lock = InstanceLock::try_acquire(&root)
                .map_err(|_| BridgeError::new("instance_unavailable"))?
                .ok_or_else(|| BridgeError::new("runtime_running"))?;
            if lock.has_discovery() {
                return Err(BridgeError::new("runtime_stop_unconfirmed"));
            }
            // Transfer has finished. Freeze one new registration budget here;
            // do not charge download time or restart it for individual files.
            let control = Arc::new(model_store::library::ScanControl::with_timeout(
                settings::verification_timeout(&root)?,
            ));
            *task.scan_control.lock().unwrap() = Some(control.clone());
            if task.onboarding_cancelled.load(Ordering::Acquire) {
                control.cancel();
            }
            control.check().map_err(store_error)?;
            let previous = ModelLibrary::read(&root)
                .map_err(store_error)?
                .ok_or_else(|| BridgeError::new("model_directory_required"))?;
            let state = task.state.lock().unwrap().clone();
            if previous.directory_id != Some(state.directory_id) {
                return Err(BridgeError::new("model_library_changed"));
            }
            let selected = model_store::library::selected::SelectedFile::open_configured(
                &previous,
                &state.file_name,
            )
            .map_err(store_error)?;
            let results = Mutex::new(Vec::new());
            let scanned = model_store::library::selected::register_selected(
                &root,
                Some(&previous),
                vec![selected],
                &control,
                &results,
            )
            .map_err(store_error)?;
            let target = scanned
                .files
                .first()
                .and_then(|file| file.model_id.clone())
                .filter(|id| {
                    scanned
                        .library
                        .entry(id)
                        .is_some_and(|entry| entry.manifest.sha256 == sha)
                })
                .ok_or_else(|| BridgeError::new("model_scan_target_rejected"))?;
            let bytes = scanned.library.encode().map_err(store_error)?;
            scanned.check().map_err(store_error)?;
            control.begin_commit().map_err(store_error)?;
            settings::atomic_replace(&root.join(model_store::library::LIBRARY_FILE), &bytes)?;
            Ok(target)
        })
        .await
        .map_err(|_| BridgeError::new("model_library_write_failed"))?
    }
    async fn test_download(
        &self,
        task: &Arc<DownloadTask>,
        id: runtime_types::ModelId,
    ) -> LocalValidation {
        use model_store::local_validation::ValidationState;
        #[derive(serde::Deserialize)]
        struct TestedLoad {
            local_validation: LocalValidation,
        }
        let result:Result<LocalValidation>=async {
            if task.onboarding_cancelled.load(Ordering::Acquire){return Err(BridgeError::new("request_cancelled"));}
            self.start_inner(true,Some(&task.onboarding_cancelled)).await?;
            let preferences=settings::preferences(&self.root)?;
            let body=crate::onboarding::load_body(LoadModelRequest{model_id:id.to_string(),context_size:preferences.context_size,threads:preferences.threads,batch_size:preferences.batch_size})?;
            let loading=self.json::<TestedLoad>(Method::POST,"/runtime/load-if-unloaded",Some(&body));
            tokio::select! {
                biased;
                _=self.closing_requested()=>{self.load_disconnected.store(true,Ordering::Release);Err(BridgeError::new("request_cancelled"))},
                _=task.wait_onboarding_cancelled()=>{self.load_disconnected.store(true,Ordering::Release);Err(BridgeError::new("request_cancelled"))},
                status=loading=>Ok(status?.local_validation),
            }
        }.await;
        let loaded = self
            .local_observation(&id)
            .is_ok_and(|o| o.state != ValidationState::Stale && o.load_success);
        match result {
            Ok(observation) => observation,
            Err(error) => LocalValidation {
                state: if matches!(error.code.as_str(), "runtime_busy" | "model_conflict") {
                    ValidationState::Deferred
                } else {
                    ValidationState::Failed
                },
                checked_at_unix_ms: Some(model_store::local_validation::now_ms()),
                error_code: Some(error.code),
                load_success: loaded,
                generation_pass: false,
            },
        }
    }
    fn register_download_task(&self, task: Arc<DownloadTask>) -> Result<()> {
        let mut slot = self.downloads.lock().unwrap();
        // Synchronize final admission with close_download's read of this same
        // slot. Once close has observed an empty slot, no late task may appear.
        if self.closing.load(Ordering::Acquire) {
            return Err(BridgeError::new("desktop_closing"));
        }
        slot.previous = slot.current.take();
        slot.current = Some(task);
        Ok(())
    }
    pub async fn download_next(&self, id: Uuid) -> Result<DownloadOperationState> {
        let _poll = self
            .download_poll
            .try_lock()
            .map_err(|_| BridgeError::new("consumer_busy"))?;
        let task = self.downloads.lock().unwrap().task(id)?;
        let changed = task.changed.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        if !task.state.lock().unwrap().terminal {
            let _ = tokio::time::timeout(Duration::from_secs(1), changed).await;
        }
        Ok(task.state.lock().unwrap().clone())
    }
    pub async fn download_cancel(&self, id: Uuid) -> Result<DownloadStopping> {
        let task = self.downloads.lock().unwrap().task(id)?;
        task.cancel();
        Ok(DownloadStopping {
            stopping: !task.state.lock().unwrap().terminal,
        })
    }
    pub(crate) async fn close_download(&self) -> Result<()> {
        let task = self.downloads.lock().unwrap().current.clone();
        let Some(task) = task else {
            return Ok(());
        };
        task.cancel();
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let changed = task.changed.notified();
                tokio::pin!(changed);
                changed.as_mut().enable();
                if task.state.lock().unwrap().terminal {
                    break;
                }
                changed.await;
            }
        })
        .await
        .map_err(|_| BridgeError::new("model_download_cleanup_unconfirmed"))
    }
}
struct ModelDownloadTarget {
    destination: SidecarDownloadFile,
    _lock: InstanceLock,
}
struct VerifiedModelDownload {
    destination: VerifiedSidecarFile,
    _lock: InstanceLock,
}
trait Destination: Send + 'static {
    type Verified: Publication;
    fn begin_attempt(&mut self) -> Result<StagingPaths>;
    fn stopped(&mut self);
    fn observed_size(&self) -> Result<u64>;
    fn reset(&mut self) -> Result<()>;
    fn verify(self, spec: &DownloadSpec, control: &DownloadControl) -> Result<Self::Verified>;
    fn cleanup(self) -> bool;
}
trait Publication: Send + 'static {
    fn publish(self) -> Result<bool>;
}
impl Destination for ModelDownloadTarget {
    type Verified = VerifiedModelDownload;
    fn begin_attempt(&mut self) -> Result<StagingPaths> {
        let paths = self.destination.begin_attempt().map_err(store_error)?;
        Ok(StagingPaths {
            directory: paths.directory,
            file_name: "payload.part".into(),
        })
    }
    fn stopped(&mut self) {
        self.destination.confirm_writer_stopped();
    }
    fn observed_size(&self) -> Result<u64> {
        self.destination.observed_size().map_err(store_error)
    }
    fn reset(&mut self) -> Result<()> {
        self.destination.reset_for_retry().map_err(store_error)
    }
    fn verify(self, spec: &DownloadSpec, control: &DownloadControl) -> Result<Self::Verified> {
        let destination = self
            .destination
            .verify(spec.expected_size, spec.sha256, || {
                control.is_cancelled() || control.is_expired()
            })
            .map_err(|error| match error.code {
                runtime_types::ErrorCode::RequestCancelled => control_error(control)
                    .unwrap_or_else(|| BridgeError::new("model_download_cancelled")),
                runtime_types::ErrorCode::IntegrityFailure => {
                    BridgeError::new("model_download_identity_mismatch")
                }
                _ => store_error(error),
            })?;
        Ok(VerifiedModelDownload {
            destination,
            _lock: self._lock,
        })
    }
    fn cleanup(self) -> bool {
        self.destination.cleanup()
    }
}
impl Publication for VerifiedModelDownload {
    fn publish(self) -> Result<bool> {
        self.destination.publish().map_err(store_error)
    }
}
trait Supervisor: Sync {
    fn transfer(
        &self,
        spec: DownloadSpec,
        sidecar: SidecarConfig,
        staging: StagingPaths,
        control: DownloadControl,
        options: TransferOptions,
        progress: watch::Sender<DownloadProgress>,
    ) -> impl std::future::Future<
        Output = std::result::Result<TransferReport, download_engine::DownloadError>,
    > + Send;
}
struct ProductionSupervisor;
impl Supervisor for ProductionSupervisor {
    async fn transfer(
        &self,
        spec: DownloadSpec,
        sidecar: SidecarConfig,
        staging: StagingPaths,
        control: DownloadControl,
        options: TransferOptions,
        progress: watch::Sender<DownloadProgress>,
    ) -> std::result::Result<TransferReport, download_engine::DownloadError> {
        download_engine::transfer(spec, sidecar, staging, control, options, progress).await
    }
}
fn control_error(control: &DownloadControl) -> Option<BridgeError> {
    if control.is_cancelled() {
        Some(BridgeError::new("model_download_cancelled"))
    } else if control.is_expired() {
        Some(BridgeError::new("model_download_timeout"))
    } else {
        None
    }
}
fn cleanup_error(target: impl Destination, mut error: BridgeError) -> Result<bool> {
    if !target.cleanup() {
        error
            .message
            .push_str(" 临时文件清理未确认，请勿自动重试。");
    }
    Err(error)
}
async fn run_download<T: Destination>(
    task: Arc<DownloadTask>,
    spec: DownloadSpec,
    mut target: T,
    sidecar: VerifiedSidecar,
    supervisor: &impl Supervisor,
) -> Option<Result<bool>> {
    let mut attempt = 1;
    loop {
        if let Some(error) = control_error(&task.control) {
            return Some(cleanup_error(target, error));
        }
        let paths = match target.begin_attempt() {
            Ok(paths) => paths,
            Err(error) => return Some(cleanup_error(target, error)),
        };
        let mut initial = DownloadProgress::initial(spec.expected_size);
        initial.attempt = attempt;
        task.progress(initial);
        let (progress, mut receive) = watch::channel(initial);
        let transfer = supervisor.transfer(
            spec.clone(),
            sidecar.config.clone(),
            paths,
            task.control.clone(),
            TransferOptions { attempt },
            progress,
        );
        tokio::pin!(transfer);
        let mut progress_open = true;
        let mut watchdog = tokio::time::interval(Duration::from_millis(250));
        watchdog.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut watchdog_error = None;
        let result = loop {
            tokio::select! {
                result = &mut transfer => break result,
                _ = watchdog.tick(), if watchdog_error.is_none() => {
                    // This is a polling resource watchdog, not a per-write quota.
                    // Preserve prior user cancellation/deadline and keep awaiting
                    // actual OS/drainer shutdown after a watchdog stop request.
                    if control_error(&task.control).is_none() {
                        let error = match target.observed_size() {
                            Ok(size) if size > spec.expected_size => Some(BridgeError::new("model_download_size_mismatch")),
                            Err(error) => Some(error),
                            _ => None,
                        };
                        if error.is_some() && control_error(&task.control).is_none()
                            && task.control.try_cancel() {
                            watchdog_error = error;
                        }
                    }
                }
                changed = receive.changed(), if progress_open => {
                    if changed.is_ok() { task.progress(*receive.borrow_and_update()); }
                    else { progress_open = false; }
                }
            }
        };
        task.progress(*receive.borrow_and_update());
        if !match &result {
            Ok(report) => report.writer_stopped,
            Err(error) => error.writer_stopped(),
        } {
            task.quarantine((target, sidecar));
            return None;
        }
        target.stopped();
        if let Some(error) = watchdog_error {
            return Some(cleanup_error(target, error));
        }
        if let Some(error) = control_error(&task.control) {
            return Some(cleanup_error(target, error));
        }
        match result {
            Ok(report) if report.exit_code == 0 => break,
            Err(download_engine::DownloadError::SidecarExit { exit_code: 8, .. })
                if attempt == 1 =>
            {
                if let Err(error) = target.reset() {
                    return Some(cleanup_error(target, error));
                }
                attempt = 2;
            }
            Err(error) => return Some(cleanup_error(target, crate::error::download_error(error))),
            Ok(_) => {
                return Some(cleanup_error(
                    target,
                    BridgeError::new("model_download_incomplete"),
                ));
            }
        }
    }
    task.phase(DownloadPhase::Verifying);
    let verify_task = task.clone();
    let result = tokio::task::spawn_blocking(move || {
        let verified = target.verify(&spec, &verify_task.control)?;
        if !verify_task.control.begin_publish() {
            return Err(control_error(&verify_task.control)
                .unwrap_or_else(|| BridgeError::new("model_download_cancelled")));
        }
        verify_task.progress(DownloadProgress {
            phase: download_engine::DownloadPhase::Committing,
            written_bytes: spec.expected_size,
            total_bytes: spec.expected_size,
            attempt,
        });
        verified.publish()
    })
    .await
    .unwrap_or_else(|_| Err(BridgeError::new("model_download_write_failed")));
    // Binary/dependency pins outlive the sidecar, all drainers and verification.
    drop(sidecar);
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn task_for(entry: &CatalogEntry) -> Arc<DownloadTask> {
        Arc::new(DownloadTask {
            state: Mutex::new(DownloadOperationState {
                operation_id: Uuid::new_v4(),
                catalog_id: entry.catalog_id.clone(),
                source: DownloadSource::Modelscope,
                file_name: entry.file_name.clone(),
                directory_id: Uuid::new_v4(),
                target_display_path: "fixture".into(),
                downloaded_bytes: 0,
                attempt: 1,
                total_bytes: entry.size_bytes,
                phase: DownloadPhase::Connecting,
                status: DownloadStatus::Running,
                terminal: false,
                result: None,
                error: None,
            }),
            control: DownloadControl::new(),
            onboarding_cancelled: AtomicBool::new(false),
            scan_control: Mutex::new(None),
            changed: Notify::new(),
            retained: Mutex::new(None),
        })
    }
    fn tiny_gguf() -> Vec<u8> {
        fn string(bytes: &mut Vec<u8>, value: &str) {
            bytes.extend((value.len() as u64).to_le_bytes());
            bytes.extend(value.as_bytes());
        }
        let mut bytes = b"GGUF".to_vec();
        bytes.extend(3u32.to_le_bytes());
        bytes.extend(1u64.to_le_bytes());
        bytes.extend(4u64.to_le_bytes());
        for (key, value) in [
            ("general.architecture", "qwen3"),
            ("tokenizer.chat_template", "synthetic template"),
        ] {
            string(&mut bytes, key);
            bytes.extend(8u32.to_le_bytes());
            string(&mut bytes, value);
        }
        for (key, value) in [("general.file_type", 7u32), ("qwen3.context_length", 40960)] {
            string(&mut bytes, key);
            bytes.extend(4u32.to_le_bytes());
            bytes.extend(value.to_le_bytes());
        }
        string(&mut bytes, "synthetic.weight");
        bytes.extend(1u32.to_le_bytes());
        bytes.extend(32u64.to_le_bytes());
        bytes.extend(0u32.to_le_bytes());
        bytes.extend(0u64.to_le_bytes());
        bytes.resize(bytes.len().next_multiple_of(32) + 128, 0);
        bytes
    }
    #[tokio::test]
    async fn published_download_registers_only_its_bound_sha_and_failures_keep_file() {
        use sha2::{Digest, Sha256};
        for (bad_sha, cancelled, late_cancel) in [
            (false, false, false),
            (true, false, false),
            (false, true, false),
            (false, false, true),
        ] {
            let root = tempfile::tempdir().unwrap();
            let source = tempfile::tempdir().unwrap();
            let data = root.path().join("private");
            runtime_api::token::create_private_dir(&data).unwrap();
            let scan = model_store::library::scan_directory(
                &data,
                source.path(),
                None,
                &model_store::library::ScanControl::default(),
            )
            .unwrap();
            let library = scan.library().unwrap().clone();
            drop(scan);
            fs::write(
                data.join(model_store::library::LIBRARY_FILE),
                library.encode().unwrap(),
            )
            .unwrap();
            let bridge = DesktopBridge::new(
                data.clone(),
                root.path().join(if cfg!(windows) {
                    "ai-runtime.exe"
                } else {
                    "ai-runtime"
                }),
            )
            .unwrap();
            let task = task_for(&catalog().unwrap().entries.remove(0));
            let filename = task.state.lock().unwrap().file_name.clone();
            task.state.lock().unwrap().directory_id = library.directory_id.unwrap();
            let bytes = tiny_gguf();
            let target = source.path().join(&filename);
            fs::write(&target, &bytes).unwrap();
            fs::write(source.path().join("unselected-broken.gguf"), b"invalid").unwrap();
            let huge = fs::File::create(source.path().join("unselected-too-large.gguf")).unwrap();
            huge.set_len(model_store::library::MAX_MODEL_BYTES + 1)
                .unwrap();
            assert!(task.control.begin_publish());
            if cancelled {
                task.cancel();
            }
            let sha = if bad_sha {
                "0".repeat(64)
            } else {
                format!("{:x}", Sha256::digest(&bytes))
            };
            let result = bridge.register_download(&task, &sha).await;
            assert_eq!(fs::read(&target).unwrap(), bytes);
            if bad_sha || cancelled {
                assert!(result.is_err());
            } else {
                assert!(result.is_ok(), "registration failed: {:?}", result);
                let registered = ModelLibrary::read(&data).unwrap().unwrap();
                assert_eq!(registered.models.len(), 1);
                assert_eq!(&registered.models[0].manifest.id, result.as_ref().unwrap());
                assert!(!registered.models[0].manifest.validated);
            }
            let local_validation = if late_cancel {
                task.cancel();
                let observation = bridge
                    .test_download(&task, result.as_ref().unwrap().clone())
                    .await;
                assert_eq!(observation.error_code.as_deref(), Some("request_cancelled"));
                assert!(!data.join("config.toml").exists());
                Some(observation)
            } else {
                None
            };
            task.finish_saved(DownloadResult {
                saved: true,
                registered: result.is_ok(),
                registration_error: result.err(),
                local_validation,
                file_name: filename,
                cleanup_warning: None,
            });
            let state = task.state.lock().unwrap();
            assert_eq!(state.status, DownloadStatus::Completed);
            assert!(state.result.as_ref().unwrap().saved);
            assert_eq!(
                state.result.as_ref().unwrap().registered,
                !bad_sha && !cancelled
            );
        }
    }
    #[tokio::test]
    async fn active_task_keeps_snapshot_responsive_and_close_waits_for_terminal() {
        let root = tempfile::tempdir().unwrap();
        let bridge = DesktopBridge::new(
            root.path().join("private"),
            root.path().join(if cfg!(windows) {
                "ai-runtime.exe"
            } else {
                "ai-runtime"
            }),
        )
        .unwrap();
        let task = task_for(&catalog().unwrap().entries.remove(0));
        bridge.downloads.lock().unwrap().current = Some(task.clone());
        assert!(matches!(
            bridge.snapshot().await.unwrap().connection,
            ConnectionState::Stopped
        ));
        assert_eq!(
            bridge
                .settings_save(DesktopPreferences::default())
                .await
                .unwrap_err()
                .code,
            "model_download_active"
        );
        assert!(bridge.work.try_lock().is_ok());
        let page = bridge.models_page(None, None).await.unwrap();
        assert_eq!(page.source, ModelsSource::Local);
        assert!(page.data.is_empty());
        let task_copy = task.clone();
        let complete = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            assert!(task_copy.control.is_cancelled());
            task_copy.finish(Err(BridgeError::new("model_download_cancelled")));
        });
        bridge.close().await.unwrap();
        complete.await.unwrap();
        assert!(task.state.lock().unwrap().terminal);
    }
    #[tokio::test]
    async fn close_before_final_admission_rejects_late_task_and_waits_for_work() {
        let temp = tempfile::tempdir().unwrap();
        let bridge = Arc::new(
            DesktopBridge::new(
                temp.path().join("private"),
                temp.path().join(if cfg!(windows) {
                    "ai-runtime.exe"
                } else {
                    "ai-runtime"
                }),
            )
            .unwrap(),
        );
        // Pause an admitted start after open/work acquisition but before its
        // final slot registration, exactly the slow-directory-validation gap.
        bridge.open().unwrap();
        let work = bridge.work.clone().try_lock_owned().unwrap();
        bridge.open().unwrap();
        let closing_bridge = bridge.clone();
        let close = tokio::spawn(async move { closing_bridge.close().await });
        while !bridge.closing.load(Ordering::Acquire) {
            tokio::task::yield_now().await;
        }
        assert!(!close.is_finished());
        let task = task_for(&catalog().unwrap().entries.remove(0));
        assert_eq!(
            bridge.register_download_task(task).unwrap_err().code,
            "desktop_closing"
        );
        drop(work);
        close.await.unwrap().unwrap();
        assert!(!bridge.download_active());
    }
    #[test]
    fn old_preferences_default_to_modelscope_and_invalid_source_rejected() {
        let mut value = serde_json::to_value(DesktopPreferences::default()).unwrap();
        value.as_object_mut().unwrap().remove("download_source");
        assert_eq!(
            serde_json::from_value::<DesktopPreferences>(value.clone())
                .unwrap()
                .download_source,
            DownloadSource::Modelscope
        );
        value["download_source"] = json!("arbitrary-url");
        assert!(serde_json::from_value::<DesktopPreferences>(value).is_err());
    }
    #[test]
    fn progress_resets_only_with_new_attempt_and_keeps_identity_immutable() {
        let entry = catalog().unwrap().entries.remove(0);
        let task = task_for(&entry);
        let initial = task.state.lock().unwrap().clone();
        let mut progress = DownloadProgress::initial(entry.size_bytes);
        progress.written_bytes = 10;
        task.progress(progress);
        progress.written_bytes = 0;
        task.progress(progress);
        assert_eq!(task.state.lock().unwrap().downloaded_bytes, 10);
        progress.attempt = 2;
        task.progress(progress);
        let state = task.state.lock().unwrap().clone();
        assert_eq!(state.downloaded_bytes, 0);
        assert_eq!(state.attempt, 2);
        assert_eq!(state.source, initial.source);
        assert_eq!(state.directory_id, initial.directory_id);
        assert_eq!(state.file_name, initial.file_name);
        progress.total_bytes += 1;
        progress.attempt = 3;
        task.progress(progress);
        assert_eq!(task.state.lock().unwrap().attempt, 2);
    }
    #[test]
    fn older_state_defaults_to_first_attempt_and_cleanup_warning_is_truthful() {
        let task = task_for(&catalog().unwrap().entries.remove(0));
        let mut value = serde_json::to_value(task.state.lock().unwrap().clone()).unwrap();
        value.as_object_mut().unwrap().remove("attempt");
        assert_eq!(
            serde_json::from_value::<DownloadOperationState>(value)
                .unwrap()
                .attempt,
            1
        );
        task.finish(Ok(false));
        let state = task.state.lock().unwrap();
        assert_eq!(state.status, DownloadStatus::Completed);
        assert!(state.result.as_ref().unwrap().saved);
        assert_eq!(
            state.result.as_ref().unwrap().cleanup_warning.as_deref(),
            Some("partial_cleanup_unconfirmed")
        );
    }
    #[derive(Default)]
    struct FakeState {
        events: Vec<&'static str>,
        attempts: Vec<u8>,
        cancel_verification: bool,
        late_cancel: bool,
        cleanup_warning: bool,
        observed_size: u64,
    }
    struct FakeDestination {
        state: Arc<Mutex<FakeState>>,
        active: bool,
    }
    struct FakePublication {
        state: Arc<Mutex<FakeState>>,
        control: DownloadControl,
    }
    impl Destination for FakeDestination {
        type Verified = FakePublication;
        fn begin_attempt(&mut self) -> Result<StagingPaths> {
            assert!(!self.active);
            self.active = true;
            self.state.lock().unwrap().events.push("begin");
            Ok(StagingPaths {
                directory: PathBuf::from("fixture"),
                file_name: "payload.part".into(),
            })
        }
        fn stopped(&mut self) {
            assert!(self.active);
            self.active = false;
            self.state.lock().unwrap().events.push("stopped");
        }
        fn observed_size(&self) -> Result<u64> {
            Ok(self.state.lock().unwrap().observed_size)
        }
        fn reset(&mut self) -> Result<()> {
            assert!(!self.active);
            self.state.lock().unwrap().events.push("reset");
            Ok(())
        }
        fn verify(self, _: &DownloadSpec, control: &DownloadControl) -> Result<Self::Verified> {
            assert!(!self.active);
            let mut state = self.state.lock().unwrap();
            state.events.push("verify");
            if state.cancel_verification {
                control.cancel();
                return Err(BridgeError::new("model_download_cancelled"));
            }
            drop(state);
            Ok(FakePublication {
                state: self.state,
                control: control.clone(),
            })
        }
        fn cleanup(self) -> bool {
            assert!(!self.active);
            self.state.lock().unwrap().events.push("cleanup");
            true
        }
    }
    impl Publication for FakePublication {
        fn publish(self) -> Result<bool> {
            let mut state = self.state.lock().unwrap();
            state.events.push("publish");
            if state.late_cancel {
                self.control.cancel();
            }
            Ok(!state.cleanup_warning)
        }
    }
    struct FakeSupervisor {
        state: Arc<Mutex<FakeState>>,
        outcomes: Mutex<
            std::collections::VecDeque<
                std::result::Result<TransferReport, download_engine::DownloadError>,
            >,
        >,
    }
    impl Supervisor for FakeSupervisor {
        async fn transfer(
            &self,
            spec: DownloadSpec,
            _: SidecarConfig,
            staging: StagingPaths,
            _: DownloadControl,
            options: TransferOptions,
            progress: watch::Sender<DownloadProgress>,
        ) -> std::result::Result<TransferReport, download_engine::DownloadError> {
            assert_eq!(staging.file_name, "payload.part");
            self.state.lock().unwrap().attempts.push(options.attempt);
            let _ = progress.send(DownloadProgress {
                phase: download_engine::DownloadPhase::Downloading,
                written_bytes: 10.min(spec.expected_size),
                total_bytes: spec.expected_size,
                attempt: options.attempt,
            });
            self.outcomes.lock().unwrap().pop_front().unwrap()
        }
    }
    struct FakeGuard(Arc<Mutex<FakeState>>);
    impl Drop for FakeGuard {
        fn drop(&mut self) {
            self.0.lock().unwrap().events.push("guard_dropped");
        }
    }
    fn fake_fixture(
        outcomes: Vec<std::result::Result<TransferReport, download_engine::DownloadError>>,
    ) -> (
        Arc<DownloadTask>,
        DownloadSpec,
        FakeDestination,
        VerifiedSidecar,
        FakeSupervisor,
        Arc<Mutex<FakeState>>,
    ) {
        let entry = catalog().unwrap().entries.remove(0);
        let spec = source_adapter::specification(&entry, &entry.sources[0]).unwrap();
        let task = task_for(&entry);
        let state = Arc::new(Mutex::new(FakeState::default()));
        let target = FakeDestination {
            state: state.clone(),
            active: false,
        };
        let sidecar = VerifiedSidecar {
            config: SidecarConfig {
                executable: PathBuf::from("fixture-never-launched"),
            },
            _guard: Box::new(FakeGuard(state.clone())),
        };
        let supervisor = FakeSupervisor {
            state: state.clone(),
            outcomes: Mutex::new(outcomes.into()),
        };
        (task, spec, target, sidecar, supervisor, state)
    }
    fn successful_transfer() -> std::result::Result<TransferReport, download_engine::DownloadError>
    {
        Ok(TransferReport {
            exit_code: 0,
            writer_stopped: true,
        })
    }
    #[tokio::test]
    async fn only_exit_eight_restarts_once_after_confirmed_stop_and_reset() {
        use download_engine::DownloadError as E;
        let range = Err(E::SidecarExit {
            exit_code: 8,
            error_code: Some(8),
        });
        let (task, spec, target, sidecar, supervisor, state) =
            fake_fixture(vec![range, successful_transfer()]);
        let deadline = task.control.deadline();
        assert!(
            run_download(task.clone(), spec, target, sidecar, &supervisor)
                .await
                .unwrap()
                .unwrap()
        );
        assert_eq!(task.control.deadline(), deadline);
        {
            let state = state.lock().unwrap();
            assert_eq!(state.attempts, [1, 2]);
            assert_eq!(
                state.events,
                [
                    "begin",
                    "stopped",
                    "reset",
                    "begin",
                    "stopped",
                    "verify",
                    "publish",
                    "guard_dropped"
                ]
            );
            assert_eq!(task.state.lock().unwrap().attempt, 2);
        }
        let (task, spec, target, sidecar, supervisor, state) = fake_fixture(vec![range, range]);
        assert!(
            run_download(task, spec, target, sidecar, &supervisor)
                .await
                .unwrap()
                .is_err()
        );
        assert_eq!(state.lock().unwrap().attempts, [1, 2]);
        assert!(!state.lock().unwrap().events.contains(&"verify"));
        for code in [2, 6, 9, 19, 32] {
            let (task, spec, target, sidecar, supervisor, state) =
                fake_fixture(vec![Err(E::SidecarExit {
                    exit_code: code,
                    error_code: Some(8),
                })]);
            assert!(
                run_download(task, spec, target, sidecar, &supervisor)
                    .await
                    .unwrap()
                    .is_err()
            );
            assert_eq!(state.lock().unwrap().attempts, [1]);
            assert!(!state.lock().unwrap().events.contains(&"reset"));
        }
    }
    #[tokio::test]
    async fn verification_cancellation_prevents_publish_and_late_cancel_keeps_saved_warning() {
        let (task, spec, target, sidecar, supervisor, state) =
            fake_fixture(vec![successful_transfer()]);
        state.lock().unwrap().cancel_verification = true;
        let error = run_download(task, spec, target, sidecar, &supervisor)
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(error.code, "model_download_cancelled");
        assert!(!state.lock().unwrap().events.contains(&"publish"));
        let (task, spec, target, sidecar, supervisor, state) =
            fake_fixture(vec![successful_transfer()]);
        state.lock().unwrap().late_cancel = true;
        state.lock().unwrap().cleanup_warning = true;
        let result = run_download(task.clone(), spec, target, sidecar, &supervisor)
            .await
            .unwrap();
        assert!(!result.as_ref().unwrap());
        assert!(!task.control.is_cancelled());
        task.finish(result);
        let state = task.state.lock().unwrap();
        assert_eq!(state.status, DownloadStatus::Completed);
        assert!(state.result.as_ref().unwrap().saved);
        assert!(state.result.as_ref().unwrap().cleanup_warning.is_some());
    }
    #[tokio::test]
    async fn unconfirmed_writer_keeps_every_guard_and_never_claims_terminal() {
        let (task, spec, target, sidecar, supervisor, state) = fake_fixture(vec![Err(
            download_engine::DownloadError::CleanupUnconfirmed,
        )]);
        assert!(
            run_download(task.clone(), spec, target, sidecar, &supervisor)
                .await
                .is_none()
        );
        assert_eq!(state.lock().unwrap().events, ["begin"]);
        assert!(!task.state.lock().unwrap().terminal);
        assert!(task.retained.lock().unwrap().is_some());
        // Only this synthetic test has no OS writer; release its fake resources
        // explicitly rather than leaking any production handles during tests.
        drop(task.retained.lock().unwrap().take());
        assert_eq!(state.lock().unwrap().events, ["begin", "guard_dropped"]);
    }
    #[test]
    fn native_verifier_is_required_and_never_falls_back_to_path() {
        let root = tempfile::tempdir().unwrap();
        let bridge = DesktopBridge::new(
            root.path().join("private"),
            root.path().join(if cfg!(windows) {
                "ai-runtime.exe"
            } else {
                "ai-runtime"
            }),
        )
        .unwrap();
        assert_eq!(
            bridge.verified_download_sidecar().err().unwrap().code,
            "model_download_engine_unavailable"
        );
        let bad = root.path().join("aria2c.exe");
        let bridge = bridge.with_download_sidecar_verifier(move || Ok((bad.clone(), ())));
        assert_eq!(
            bridge.verified_download_sidecar().err().unwrap().code,
            "model_download_engine_unavailable"
        );
    }
    struct WaitingSupervisor {
        entered: Arc<Notify>,
        release: Arc<Notify>,
    }
    impl Supervisor for WaitingSupervisor {
        async fn transfer(
            &self,
            _: DownloadSpec,
            _: SidecarConfig,
            _: StagingPaths,
            control: DownloadControl,
            _: TransferOptions,
            _: watch::Sender<DownloadProgress>,
        ) -> std::result::Result<TransferReport, download_engine::DownloadError> {
            self.entered.notify_one();
            while !control.is_cancelled() && !control.is_expired() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            // Simulates an OS writer still exiting after the stop request.
            self.release.notified().await;
            Err(download_engine::DownloadError::Cancelled)
        }
    }
    #[tokio::test]
    async fn size_watchdog_waits_for_writer_and_preserves_prior_cancel_or_deadline() {
        let (task, spec, target, sidecar, _, state) = fake_fixture(vec![]);
        state.lock().unwrap().observed_size = spec.expected_size + 1;
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let supervisor = WaitingSupervisor {
            entered: entered.clone(),
            release: release.clone(),
        };
        let run_task = task.clone();
        let run = tokio::spawn(async move {
            run_download(run_task, spec, target, sidecar, &supervisor).await
        });
        entered.notified().await;
        tokio::time::timeout(Duration::from_secs(2), async {
            while !task.control.is_cancelled() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(!run.is_finished());
        assert_eq!(state.lock().unwrap().events, ["begin"]);
        release.notify_one();
        let error = run.await.unwrap().unwrap().unwrap_err();
        assert_eq!(error.code, "model_download_size_mismatch");
        assert_eq!(
            state.lock().unwrap().events,
            ["begin", "stopped", "cleanup", "guard_dropped"]
        );
        for expired in [false, true] {
            let (mut task, spec, target, sidecar, supervisor, state) = fake_fixture(vec![]);
            state.lock().unwrap().observed_size = spec.expected_size + 1;
            if expired {
                Arc::get_mut(&mut task).unwrap().control = DownloadControl::with_deadline(
                    std::time::Instant::now() - Duration::from_secs(1),
                );
            } else {
                task.cancel();
            }
            let error = run_download(task, spec, target, sidecar, &supervisor)
                .await
                .unwrap()
                .unwrap_err();
            assert_eq!(
                error.code,
                if expired {
                    "model_download_timeout"
                } else {
                    "model_download_cancelled"
                }
            );
            assert!(state.lock().unwrap().attempts.is_empty());
        }
    }
}
