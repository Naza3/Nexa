use crate::*;
use model_store::library::{LIBRARY_FILE, ModelLibrary, ScanControl};
use std::sync::Arc;
use uuid::Uuid;

pub(crate) type DirectoryValidator = dyn Fn(&Path) -> Result<()> + Send + Sync + 'static;
#[derive(Default)]
pub(crate) struct LibrarySlot {
    current: Option<Arc<LibraryTask>>,
    previous: Option<Arc<LibraryTask>>,
}
struct LibraryTask {
    state: Mutex<LibraryOperationState>,
    control: Arc<ScanControl>,
    changed: tokio::sync::Notify,
    owns_instance: AtomicBool,
}
impl LibrarySlot {
    fn task(&self, id: Uuid) -> Result<Arc<LibraryTask>> {
        self.current
            .iter()
            .chain(self.previous.iter())
            .find(|task| task.state.lock().unwrap().operation_id == id)
            .cloned()
            .ok_or_else(|| BridgeError::new("request_not_owned"))
    }
    pub(crate) fn owns_instance(&self) -> bool {
        self.current
            .as_ref()
            .is_some_and(|task| task.owns_instance.load(Ordering::Acquire))
    }
}
impl LibraryTask {
    fn snapshot(&self) -> LibraryOperationState {
        let mut state = self.state.lock().unwrap().clone();
        if !state.terminal {
            let progress = self.control.progress();
            state.phase = match progress.phase {
                "enumerating" => LibraryOperationPhase::Enumerating,
                "verifying" => LibraryOperationPhase::Verifying,
                "committing" => LibraryOperationPhase::Committing,
                _ => LibraryOperationPhase::Checking,
            };
            state.examined_entries = progress.examined_entries;
            state.candidate_files = progress.candidate_files;
            state.verified_files = progress.verified_files;
            state.file_errors = file_errors(&progress);
        }
        state
    }
    fn finish(&self, result: Result<LibraryOperationResult>) {
        let progress = self.control.progress();
        let mut state = self.state.lock().unwrap();
        state.terminal = true;
        state.phase = LibraryOperationPhase::Finished;
        state.examined_entries = progress.examined_entries;
        state.candidate_files = progress.candidate_files;
        state.verified_files = progress.verified_files;
        state.file_errors = file_errors(&progress);
        match result {
            Ok(result) => {
                state.status = if result.rejected_files == 0 {
                    LibraryOperationStatus::Completed
                } else {
                    LibraryOperationStatus::Partial
                };
                state.result = Some(result);
            }
            Err(error) => {
                state.status = if error.code == "model_scan_cancelled" {
                    LibraryOperationStatus::Cancelled
                } else {
                    LibraryOperationStatus::Failed
                };
                if state.status == LibraryOperationStatus::Failed
                    && matches!(
                        error.code.as_str(),
                        "invalid_manifest"
                            | "unsupported_model"
                            | "unsupported_chat_template"
                            | "invalid_argument"
                            | "model_file_changed"
                            | "model_file_unavailable"
                            | "model_file_in_use"
                    )
                {
                    state.failed_file_name = progress
                        .current_file_name
                        .filter(|name| name.len() <= 1024 && !name.contains(['/', '\\', '\0']));
                }
                state.error = Some(error);
            }
        }
        drop(state);
        self.changed.notify_waiters();
    }
}
struct OwnsInstance(Arc<LibraryTask>);
impl Drop for OwnsInstance {
    fn drop(&mut self) {
        self.0.owns_instance.store(false, Ordering::Release);
    }
}
fn error(error: runtime_types::RuntimeError) -> BridgeError {
    BridgeError::new(error.code.as_str())
}
fn file_errors(progress: &model_store::library::ScanProgress) -> Vec<LibraryFileError> {
    progress
        .file_errors
        .iter()
        .map(|failure| {
            let error = BridgeError::new(failure.reason.as_str());
            LibraryFileError {
                file_name: failure.file_name.clone(),
                code: error.code,
                message: error.message,
            }
        })
        .collect()
}
impl DesktopBridge {
    pub fn with_default_model_directory(mut self, path: PathBuf) -> Self {
        self.default_model_directory = Some(path);
        self
    }
    /// Startup admission uses the same guarded transaction as an explicit pick.
    /// A configured missing/stale directory never falls back to another location.
    pub fn directory_discover(self: &Arc<Self>) -> Result<Option<LibraryOperationHandle>> {
        self.open()?;
        if ModelLibrary::read(&self.root).map_err(error)?.is_some() {
            return Ok(None);
        }
        let Some(path) = &self.default_model_directory else {
            return Ok(None);
        };
        match fs::symlink_metadata(path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(BridgeError::new("model_directory_unavailable")),
            Ok(_) => (),
        }
        self.begin_library(Some(path.clone()), true).map(Some)
    }

    pub fn with_directory_validator(
        mut self,
        validator: impl Fn(&Path) -> Result<()> + Send + Sync + 'static,
    ) -> Self {
        self.directory_validator = Some(Arc::new(validator));
        self
    }
    pub fn library_active(&self) -> Option<Uuid> {
        self.library
            .lock()
            .unwrap()
            .current
            .as_ref()
            .and_then(|task| {
                let state = task.state.lock().unwrap();
                (!state.terminal).then_some(state.operation_id)
            })
    }
    pub fn directory_apply(self: &Arc<Self>, path: PathBuf) -> Result<LibraryOperationHandle> {
        self.begin_library(Some(path), false)
    }
    pub fn models_scan(self: &Arc<Self>) -> Result<LibraryOperationHandle> {
        self.begin_library(None, false)
    }
    fn begin_library(
        self: &Arc<Self>,
        candidate: Option<PathBuf>,
        discovery: bool,
    ) -> Result<LibraryOperationHandle> {
        self.open()?;
        let ownership = self
            .work
            .clone()
            .try_lock_owned()
            .map_err(|_| BridgeError::new("desktop_busy"))?;
        let mut slot = self.library.lock().unwrap();
        if slot
            .current
            .as_ref()
            .is_some_and(|task| !task.state.lock().unwrap().terminal)
        {
            return Err(BridgeError::new("desktop_busy"));
        }
        let id = Uuid::new_v4();
        let task = Arc::new(LibraryTask {
            state: Mutex::new(LibraryOperationState {
                operation_id: id,
                status: LibraryOperationStatus::Running,
                phase: LibraryOperationPhase::Checking,
                examined_entries: 0,
                candidate_files: 0,
                verified_files: 0,
                failed_file_name: None,
                file_errors: Vec::new(),
                terminal: false,
                result: None,
                error: None,
            }),
            control: Arc::new(ScanControl::default()),
            changed: tokio::sync::Notify::new(),
            owns_instance: AtomicBool::new(false),
        });
        slot.previous = slot.current.take();
        slot.current = Some(task.clone());
        let bridge = self.clone();
        tokio::spawn(async move {
            let result = bridge
                .run_library(candidate, discovery, task.clone(), ownership)
                .await;
            task.finish(result);
        });
        Ok(LibraryOperationHandle { operation_id: id })
    }
    async fn run_library(
        &self,
        candidate: Option<PathBuf>,
        discovery: bool,
        task: Arc<LibraryTask>,
        _ownership: tokio::sync::OwnedMutexGuard<()>,
    ) -> Result<LibraryOperationResult> {
        if self.closing.load(Ordering::Acquire) {
            task.control.cancel();
        }
        task.control.check().map_err(error)?;
        self.open()
            .map_err(|_| BridgeError::new("model_scan_cancelled"))?;
        let root = self.root.clone();
        let validator = self.directory_validator.clone();
        tokio::task::spawn_blocking(move || {
            task.control.check().map_err(error)?;
            runtime_api::token::create_private_dir(&root)
                .map_err(|_| BridgeError::new("data_directory_unavailable"))?;
            let lock = InstanceLock::try_acquire(&root)
                .map_err(|_| BridgeError::new("instance_unavailable"))?
                .ok_or_else(|| BridgeError::new("runtime_running"))?;
            if lock.has_discovery() {
                return Err(BridgeError::new("runtime_stop_unconfirmed"));
            }
            task.owns_instance.store(true, Ordering::Release);
            let _owner = OwnsInstance(task.clone());
            let previous = ModelLibrary::read(&root).map_err(error)?;
            if discovery && previous.is_some() {
                return Err(BridgeError::new("model_directory_already_configured"));
            }
            let rescan = candidate.is_none();
            let directory = candidate
                .or_else(|| previous.as_ref().map(|old| old.directory.clone()))
                .ok_or_else(|| BridgeError::new("model_directory_required"))?;
            if let Some(validator) = validator {
                validator(&directory)?;
            }
            task.control.check().map_err(error)?;
            let scanned = if rescan {
                model_store::library::rescan_directory(
                    &root,
                    previous
                        .as_ref()
                        .ok_or_else(|| BridgeError::new("model_directory_required"))?,
                    &task.control,
                )
            } else {
                model_store::library::scan_directory(
                    &root,
                    &directory,
                    previous.as_ref(),
                    &task.control,
                )
            }
            .map_err(error)?;
            // Resolve publication or abandonment while all guards and the lock
            // are held. Publish the terminal only after this scope is released.
            (|| {
                task.control.check().map_err(error)?;
                let library = scanned
                    .library()
                    .ok_or_else(|| BridgeError::new("model_scan_no_usable_files"))?;
                let diagnostics = file_errors(&task.control.progress());
                if serde_json::to_vec(&diagnostics)
                    .map_err(|_| BridgeError::new("model_library_limit"))?
                    .len()
                    > model_store::library::MAX_SCAN_DIAGNOSTIC_BYTES
                {
                    return Err(BridgeError::new("model_library_limit"));
                }
                let bytes = library.encode().map_err(error)?;
                task.control.begin_commit().map_err(error)?;
                settings::atomic_replace(&root.join(LIBRARY_FILE), &bytes).map_err(|cause| {
                    if cause.code == "settings_durability_unconfirmed" {
                        cause
                    } else {
                        BridgeError::new("model_library_write_failed")
                    }
                })?;
                Ok(LibraryOperationResult {
                    library_generation: library.library_generation,
                    directory_id: library.directory_id,
                    registered_files: library.models.len(),
                    available_files: library
                        .models
                        .iter()
                        .filter(|entry| entry.manifest.load_candidate())
                        .count(),
                    rejected_files: diagnostics.len(),
                })
            })()
        })
        .await
        .map_err(|_| BridgeError::new("model_library_write_failed"))?
    }
    pub async fn library_next(&self, id: Uuid) -> Result<LibraryOperationState> {
        let _consumer = self
            .library_poll
            .try_lock()
            .map_err(|_| BridgeError::new("consumer_busy"))?;
        let task = self.library.lock().unwrap().task(id)?;
        let changed = task.changed.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        if !task.state.lock().unwrap().terminal {
            let _ = tokio::time::timeout(Duration::from_secs(1), changed).await;
        }
        let snapshot = task.snapshot();
        // Diagnostics have a separate 512 KiB bound; the complete private
        // operation response, including fixed terminal metadata, stays bounded.
        if serde_json::to_vec(&snapshot)
            .map_err(|_| BridgeError::new("model_library_limit"))?
            .len()
            > 1024 * 1024
        {
            return Err(BridgeError::new("model_library_limit"));
        }
        Ok(snapshot)
    }
    pub async fn library_cancel(&self, id: Uuid) -> Result<LibraryStopping> {
        let task = self.library.lock().unwrap().task(id)?;
        if !task.state.lock().unwrap().terminal {
            task.control.cancel();
        }
        Ok(LibraryStopping {
            operation_id: id,
            status: "stopping".into(),
        })
    }
    pub(crate) async fn close_library(&self) -> Result<()> {
        let task = self.library.lock().unwrap().current.clone();
        let Some(task) = task else {
            return Ok(());
        };
        task.control.cancel();
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
        .map_err(|_| BridgeError::new("desktop_busy"))
    }
}
pub(crate) fn directory_snapshot(
    root: &Path,
    runtime: Option<&RuntimeStatus>,
) -> Result<ModelDirectorySnapshot> {
    let library = ModelLibrary::read(root).map_err(error)?;
    let directory_problem =
        library
            .as_ref()
            .and_then(|library| match library.directory_presence() {
                Ok(false) => Some(ModelDirectoryState::Missing),
                Err(_) => Some(ModelDirectoryState::Unavailable),
                Ok(true) => None,
            });
    let configured = library.map(|library| {
        let info = library.info();
        ModelDirectoryInfo {
            directory_id: info.directory_id,
            display_path: info.display_path,
            library_generation: info.library_generation,
        }
    });
    let effective = runtime
        .and_then(|status| status.model_library.as_ref())
        .and_then(|library| library.directory.clone());
    let state = match runtime {
        Some(status)
            if status
                .model_library
                .as_ref()
                .is_none_or(|library| !library.supported) =>
        {
            if configured.is_some() {
                ModelDirectoryState::Unsupported
            } else {
                ModelDirectoryState::Default
            }
        }
        Some(_) if configured != effective => ModelDirectoryState::Stale,
        Some(_) => {
            if configured.is_some() {
                ModelDirectoryState::Ready
            } else {
                ModelDirectoryState::Default
            }
        }
        None => {
            if configured.is_some() {
                ModelDirectoryState::Stopped
            } else {
                ModelDirectoryState::Default
            }
        }
    };
    Ok(ModelDirectorySnapshot {
        configured,
        effective,
        state: if matches!(
            state,
            ModelDirectoryState::Stale | ModelDirectoryState::Unsupported
        ) {
            state
        } else {
            directory_problem.unwrap_or(state)
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn status(library: Option<ModelLibraryObservation>) -> RuntimeStatus {
        RuntimeStatus {
            lan_api: None,
            state: RuntimeState::Unloaded,
            selected_model: None,
            selected_model_display_name: None,
            model_library: library,
            load_options: None,
            active_request: None,
            queued_jobs: 0,
            stopping: false,
            registry_busy: false,
            configured_backend: "cpu".into(),
            backend: None,
            backend_observation: "unavailable".into(),
            last_error: None,
            threads_source: None,
            available_parallelism: Some(2),
            threads_exceed_available_parallelism: None,
            worker: WorkerStatus {
                pid: None,
                sessions_started: Some(0),
                sessions_reaped: Some(0),
            },
            memory: MemoryStatus {
                api_private_bytes: None,
                worker_private_bytes: None,
                gpu_bytes: None,
                observation: "unavailable".into(),
            },
        }
    }
    #[test]
    fn stale_or_unsupported_identity_takes_priority_over_missing_source() {
        let root = tempfile::tempdir().unwrap();
        let sources = tempfile::tempdir().unwrap();
        let directory = sources.path().join("models");
        std::fs::create_dir(&directory).unwrap();
        let scan = model_store::library::scan_directory(
            root.path(),
            &directory,
            None,
            &ScanControl::default(),
        )
        .unwrap();
        let library = scan.library().unwrap().clone();
        drop(scan);
        std::fs::write(root.path().join(LIBRARY_FILE), library.encode().unwrap()).unwrap();
        let configured = directory_snapshot(root.path(), None)
            .unwrap()
            .configured
            .unwrap();
        std::fs::remove_dir(&directory).unwrap();
        let mut wrong = configured.clone();
        wrong.library_generation = Uuid::new_v4();
        let stale = status(Some(ModelLibraryObservation {
            supported: true,
            directory: Some(wrong),
        }));
        assert_eq!(
            directory_snapshot(root.path(), Some(&stale)).unwrap().state,
            ModelDirectoryState::Stale
        );
        let old = status(None);
        assert_eq!(
            directory_snapshot(root.path(), Some(&old)).unwrap().state,
            ModelDirectoryState::Unsupported
        );
        let matching = status(Some(ModelLibraryObservation {
            supported: true,
            directory: Some(configured),
        }));
        assert_eq!(
            directory_snapshot(root.path(), Some(&matching))
                .unwrap()
                .state,
            ModelDirectoryState::Missing
        );
        assert_eq!(
            directory_snapshot(root.path(), None).unwrap().state,
            ModelDirectoryState::Missing
        );
    }
}
