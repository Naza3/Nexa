use crate::{
    Config, ServiceShutdown,
    dto::{ImportModelRequest, ModelSummary},
    errors::ApiError,
};
use model_store::{ImportCancellation, ImportRequest, ModelManifest, ModelSource, ModelStore};
use process_host::ProcessDiagnostics;
use runtime_core::{EventReceiver, Runtime, RuntimeHandle};
use runtime_types::{
    ErrorCode, GenerationOptions, GenerationRequest, LoadOptions, Message, ModelId, RequestId,
    RuntimeError,
};
use std::{
    path::PathBuf,
    sync::{Arc, RwLock},
};
use tokio::sync::Semaphore;

pub(crate) type ProbeBuild = Arc<
    std::sync::OnceLock<Result<(Vec<model_store::library::FileIdentity>, String), &'static str>>,
>;

struct RegistrySnapshot {
    models: Vec<ModelSummary>,
    generation: uuid::Uuid,
}

#[derive(Clone)]
pub struct ApiState {
    pub runtime: RuntimeHandle,
    pub config: Arc<Config>,
    active_configuration: Arc<RwLock<crate::configuration::Document>>,
    configuration_gate: Arc<tokio::sync::Mutex<()>>,
    pub shutdown: ServiceShutdown,
    pub diagnostics: Option<ProcessDiagnostics>,
    lan_listening: Arc<std::sync::atomic::AtomicBool>,
    pub(crate) store: Arc<ModelStore>,
    pub(crate) probe_build: ProbeBuild,
    pub(crate) load_operations: Arc<std::sync::Mutex<crate::load_operations::LoadOperations>>,
    registry: Arc<RwLock<RegistrySnapshot>>,
    thread_selection: Arc<RwLock<Option<(LoadOptions, &'static str)>>>,
    requests: Arc<Semaphore>,
    controls: Arc<Semaphore>,
    storage: Arc<Semaphore>,
}
impl ApiState {
    /// No filesystem I/O occurs here. Call initialize_registry before accepting
    /// connections so startup hashing/listing also stays outside the reactor.
    pub fn new(
        runtime: Runtime,
        store: Arc<ModelStore>,
        config: Config,
        diagnostics: Option<ProcessDiagnostics>,
    ) -> Self {
        let import_cancel = ImportCancellation::default();
        let config_bytes = config
            .to_toml()
            .expect("validated startup configuration")
            .into_bytes();
        let active = crate::configuration::Document {
            config: config.clone(),
            revision: crate::configuration::revision(&config_bytes),
            bytes: config_bytes,
        };
        Self {
            runtime: runtime.handle(),
            config: Arc::new(config),
            active_configuration: Arc::new(RwLock::new(active)),
            configuration_gate: Arc::new(tokio::sync::Mutex::new(())),
            shutdown: ServiceShutdown::new(runtime, import_cancel.clone()),
            diagnostics,
            lan_listening: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            store,
            probe_build: Arc::new(std::sync::OnceLock::new()),
            load_operations: Arc::new(std::sync::Mutex::new(Default::default())),
            registry: Arc::new(RwLock::new(RegistrySnapshot {
                models: Vec::new(),
                generation: uuid::Uuid::new_v4(),
            })),
            thread_selection: Arc::new(RwLock::new(None)),
            requests: Arc::new(Semaphore::new(32)),
            controls: Arc::new(Semaphore::new(8)),
            storage: Arc::new(Semaphore::new(1)),
        }
    }
    pub fn active_config(&self) -> Result<Config, ApiError> {
        Ok(self
            .active_configuration
            .read()
            .map_err(|_| ApiError::internal())?
            .config
            .clone())
    }
    pub async fn configuration_get(
        &self,
    ) -> Result<crate::configuration::ConfigurationSnapshot, ApiError> {
        let _gate = self.configuration_gate.lock().await;
        let active = self
            .active_configuration
            .read()
            .map_err(|_| ApiError::internal())?
            .clone();
        let root = self.store.data_directory().to_owned();
        tokio::task::spawn_blocking(move || {
            let saved = crate::configuration::read(&root)?;
            crate::configuration::snapshot(&root, &saved, Some(&active))
        })
        .await
        .map_err(|_| ApiError::internal())?
        .map_err(Into::into)
    }
    pub async fn configuration_save(
        &self,
        request: crate::configuration::ConfigurationSaveRequest,
    ) -> Result<crate::configuration::ConfigurationSnapshot, ApiError> {
        self.ensure_running()?;
        let gate = self
            .configuration_gate
            .clone()
            .try_lock_owned()
            .map_err(|_| crate::configuration::ConfigurationError {
                code: "configuration_busy",
                param: None,
            })?;
        let active = self
            .active_configuration
            .read()
            .map_err(|_| ApiError::internal())?
            .clone();
        let root = self.store.data_directory().to_owned();
        let limits = self
            .registry
            .read()
            .map_err(|_| ApiError::internal())?
            .models
            .iter()
            .map(|m| (m.id.clone(), m.context_limit))
            .collect();
        let cache = self.active_configuration.clone();
        tokio::task::spawn_blocking(move || {
            // The detached blocking transaction owns the cache publication and
            // gate even if the HTTP client disconnects while disk I/O runs.
            let _gate = gate;
            let next = crate::configuration::save(&root, request, Some(&active), &limits)?;
            *cache
                .write()
                .map_err(|_| crate::configuration::ConfigurationError {
                    code: "configuration_unavailable",
                    param: None,
                })? = next.clone();
            crate::configuration::snapshot(&root, &next, Some(&next))
        })
        .await
        .map_err(|_| ApiError::internal())?
        .map_err(Into::into)
    }
    pub async fn configuration_model_get(
        &self,
        id: ModelId,
    ) -> Result<crate::configuration::ModelConfiguration, ApiError> {
        let _gate = self.configuration_gate.lock().await;
        let root = self.store.data_directory().to_owned();
        let saved = tokio::task::spawn_blocking(move || crate::configuration::read(&root))
            .await
            .map_err(|_| ApiError::internal())??;
        let limit = self
            .registry
            .read()
            .map_err(|_| ApiError::internal())?
            .models
            .iter()
            .find(|m| m.id == id)
            .map(|m| m.context_limit);
        let status = self.control(|runtime| runtime.status()).await?;
        crate::configuration::model_configuration(&saved, id, Some(&status), limit)
            .map_err(Into::into)
    }
    pub async fn open_store(path: PathBuf) -> Result<Arc<ModelStore>, RuntimeError> {
        tokio::task::spawn_blocking(move || ModelStore::open(path).map(Arc::new))
            .await
            .map_err(|_| {
                RuntimeError::new(
                    ErrorCode::RuntimeFaulted,
                    "model-store initialization task failed",
                )
            })?
    }
    pub async fn initialize_registry(&self) -> Result<(), ApiError> {
        let root = self.store.data_directory().to_owned();
        if root.join("config.toml").exists() {
            let document = tokio::task::spawn_blocking(move || crate::configuration::read(&root))
                .await
                .map_err(|_| ApiError::internal())??;
            if document.config != *self.config {
                return Err(crate::configuration::ConfigurationError {
                    code: "configuration_restart_required",
                    param: None,
                }
                .into());
            }
            *self
                .active_configuration
                .write()
                .map_err(|_| ApiError::internal())? = document;
        }
        let build = self.probe_build.clone();
        tokio::task::spawn_blocking(move || {
            build.get_or_init(|| {
                let path = std::env::current_exe().map_err(|_| "validation_engine_unavailable")?;
                model_store::local_validation::engine_identity(&path)
                    .map_err(|_| "validation_engine_unavailable")
            });
        })
        .await
        .map_err(|_| ApiError::internal())?;
        let store = self.store.clone();
        let models = self
            .storage(move || {
                store.list().map(|models| {
                    models
                        .into_iter()
                        .map(|model| ModelSummary::from_store(model, &store))
                        .collect()
                })
            })
            .await?;
        *self.registry.write().map_err(|_| ApiError::internal())? = RegistrySnapshot {
            models,
            generation: uuid::Uuid::new_v4(),
        };
        Ok(())
    }
    pub fn models_page(
        &self,
        limit: usize,
        after: Option<&runtime_types::ModelId>,
        available_only: bool,
        expected_generation: Option<uuid::Uuid>,
    ) -> Result<
        (
            Vec<ModelSummary>,
            Option<runtime_types::ModelId>,
            uuid::Uuid,
        ),
        ApiError,
    > {
        let registry = self.registry.read().map_err(|_| ApiError::internal())?;
        if expected_generation.is_some_and(|g| g != registry.generation) {
            return Err(model_store::library::library_error(ErrorCode::ModelListChanged).into());
        }
        let mut models = registry.models.iter().filter(|model| {
            (!available_only || model.available) && after.is_none_or(|after| model.id > *after)
        });
        let page: Vec<_> = models.by_ref().take(limit).cloned().collect();
        let next_after = if models.next().is_some() {
            page.last().map(|model| model.id.clone())
        } else {
            None
        };
        Ok((page, next_after, registry.generation))
    }
    pub fn model_library_info(&self) -> Option<model_store::library::LibraryDirectoryInfo> {
        self.store.library_info()
    }
    pub fn selected_display_name(&self, id: Option<&ModelId>) -> Option<String> {
        let id = id?;
        self.registry
            .read()
            .ok()?
            .models
            .iter()
            .find(|model| &model.id == id)
            .map(|model| model.display_name.clone())
    }
    pub async fn wait_shutdown(&self) -> Result<(), RuntimeError> {
        self.shutdown.wait().await?;
        self.store.release_external_after_shutdown();
        Ok(())
    }
    pub(crate) async fn prepare_external(&self, id: ModelId) -> Result<(), ApiError> {
        match self.store.needs_external_preparation(&id) {
            Ok(false) => return Ok(()),
            Ok(true) => (),
            Err(error) => {
                publish_preparation_result(&self.registry, &id, &Err(error.clone()))?;
                return Err(error.into());
            }
        }
        let store = self.store.clone();
        let requested = id.clone();
        self.prepare_guarded(id, move |control| {
            store.prepare_external(&requested, &control)
        })
        .await
    }
    pub(crate) async fn prepare_external_for_onboarding(
        &self,
        id: ModelId,
    ) -> Result<(), ApiError> {
        if !self.store.needs_external_preparation(&id)? {
            return Ok(());
        }
        let store = self.store.clone();
        let requested = id.clone();
        self.prepare_guarded_with_condition(
            id,
            move |control| store.prepare_external(&requested, &control),
            true,
            None,
        )
        .await
    }
    pub(crate) async fn prepare_external_controlled(
        &self,
        id: ModelId,
        cancel: runtime_core::LoadControl,
        only_if_unloaded: bool,
    ) -> Result<(), ApiError> {
        crate::load_operations::check_cancelled(&cancel)?;
        match self.store.needs_external_preparation(&id) {
            Ok(false) => return Ok(()),
            Ok(true) => (),
            Err(error) => {
                publish_preparation_result(&self.registry, &id, &Err(error.clone()))?;
                return Err(error.into());
            }
        }
        let store = self.store.clone();
        let requested = id.clone();
        self.prepare_guarded_with_condition(
            id,
            move |control| store.prepare_external(&requested, &control),
            only_if_unloaded,
            Some(cancel),
        )
        .await
    }
    async fn prepare_guarded<F>(&self, id: ModelId, action: F) -> Result<(), ApiError>
    where
        F: FnOnce(Arc<model_store::library::ScanControl>) -> Result<(), RuntimeError>
            + Send
            + 'static,
    {
        self.prepare_guarded_with_condition(id, action, false, None)
            .await
    }
    async fn prepare_guarded_with_condition<F>(
        &self,
        id: ModelId,
        action: F,
        only_if_unloaded: bool,
        cancellation: Option<runtime_core::LoadControl>,
    ) -> Result<(), ApiError>
    where
        F: FnOnce(Arc<model_store::library::ScanControl>) -> Result<(), RuntimeError>
            + Send
            + 'static,
    {
        let permit = self
            .storage
            .clone()
            .try_acquire_owned()
            .map_err(|_| ApiError::busy())?;
        let lease = self
            .control(move |runtime| {
                if only_if_unloaded {
                    runtime.reserve_registry_if_unloaded()
                } else {
                    runtime.reserve_registry()
                }
            })
            .await?;
        let control = Arc::new(model_store::library::ScanControl::with_timeout(
            self.config.model_verification_timeout(),
        ));
        let shutdown = self.shutdown.clone();
        let shutdown_control = control.clone();
        if cancellation
            .as_ref()
            .is_some_and(runtime_core::LoadControl::is_cancelled)
        {
            control.cancel();
        }
        let watcher = tokio::spawn(async move {
            loop {
                if cancellation
                    .as_ref()
                    .is_some_and(runtime_core::LoadControl::is_cancelled)
                {
                    shutdown_control.cancel();
                    return;
                }
                tokio::select! {
                    _ = shutdown.requested() => { shutdown_control.cancel(); return; },
                    _ = tokio::time::sleep(std::time::Duration::from_millis(20)) => (),
                }
            }
        });
        let mut guard = PreparationGuard {
            control: control.clone(),
            watcher,
            finished: false,
        };
        let registry = self.registry.clone();
        let result = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let _lease = lease; // Remains reserved through cancellation and cleanup.
            let result = action(control);
            publish_preparation_result(&registry, &id, &result)?;
            result
        })
        .await
        .map_err(|_| ApiError::internal())?;
        guard.finished = true;
        result?;
        self.ensure_running()
    }
    pub fn ensure_running(&self) -> Result<(), ApiError> {
        if self.shutdown.is_stopping() {
            Err(RuntimeError::new(ErrorCode::RuntimeShutdown, "stopping").into())
        } else {
            Ok(())
        }
    }
    pub async fn load(
        &self,
        model: ModelId,
        options: LoadOptions,
        explicit_threads: bool,
    ) -> Result<(), ApiError> {
        self.ensure_running()?;
        self.prepare_external(model.clone()).await?;
        self.load_prepared(model, options, explicit_threads, None, false)
            .await
    }
    pub(crate) async fn load_prepared(
        &self,
        model: ModelId,
        options: LoadOptions,
        explicit_threads: bool,
        cancel: Option<runtime_core::LoadControl>,
        only_if_unloaded: bool,
    ) -> Result<(), ApiError> {
        let source = if explicit_threads {
            "request"
        } else if self.config.inference.threads.is_some() {
            "configuration"
        } else {
            "automatic_min_4_available_parallelism_fallback_1"
        };
        let selection = self.thread_selection.clone();
        let self_schema_one = self.config.schema_version == 1;
        self.execute(move |runtime| {
            match cancel {
                Some(cancel) if only_if_unloaded => {
                    runtime.load_if_unloaded_controlled(model, options, cancel)?
                }
                Some(cancel) => runtime.load_controlled(model, options, cancel)?,
                None => runtime.load(model, options)?,
            }
            *selection.write().map_err(|_| {
                RuntimeError::new(
                    ErrorCode::RuntimeFaulted,
                    "thread selection observation failed",
                )
            })? = if explicit_threads || self_schema_one {
                Some((options, source))
            } else {
                None
            };
            Ok(())
        })
        .await
        .map_err(|mut error| {
            if error.error.code == "context_length_exceeded" {
                error.error.param = Some("context_size".into());
            }
            error
        })
    }
    pub fn thread_source(&self, options: Option<LoadOptions>) -> Option<&'static str> {
        let options = options?;
        if let Ok(selection) = self.thread_selection.read()
            && let Some((recorded, source)) = *selection
            && recorded == options
        {
            return Some(source);
        }
        if self.config.schema_version == 1 && options == self.config.load_options() {
            Some(if self.config.inference.threads.is_some() {
                "configuration"
            } else {
                "automatic_min_4_available_parallelism_fallback_1"
            })
        } else {
            None
        }
    }
    /// Called by the serving owner only after the configured LAN socket is bound.
    pub fn mark_lan_listening(&self, listening: bool) {
        self.lan_listening
            .store(listening, std::sync::atomic::Ordering::Release);
    }
    pub fn lan_is_listening(&self) -> bool {
        self.lan_listening
            .load(std::sync::atomic::Ordering::Acquire)
            && !self.shutdown.is_stopping()
    }
    pub async fn submit_loaded(
        &self,
        request: GenerationRequest,
    ) -> Result<EventReceiver, ApiError> {
        self.ensure_running()?;
        // No prepare_external, filesystem scan/hash, registry reservation or load.
        self.execute(move |runtime| runtime.submit_loaded(request))
            .await
    }
    pub async fn submit(&self, request: GenerationRequest) -> Result<EventReceiver, ApiError> {
        self.ensure_running()?;
        self.prepare_external(request.model.clone()).await?;
        let config = self.active_config()?;
        let options = crate::configuration::resolve_load_options(
            &config,
            &request.model,
            Default::default(),
        )?;
        self.execute(move |runtime| runtime.submit_with_load_options(request, options))
            .await
    }
    pub async fn submit_current(
        &self,
        request_id: RequestId,
        messages: Vec<Message>,
        options: GenerationOptions,
    ) -> Result<(ModelId, EventReceiver), ApiError> {
        self.ensure_running()?;
        // Current-model inference must not prepare external files or infer a
        // load target from a separate, potentially stale status observation.
        self.execute(move |runtime| runtime.submit_current(request_id, messages, options))
            .await
    }
    pub async fn execute<T, F>(&self, action: F) -> Result<T, ApiError>
    where
        T: Send + 'static,
        F: FnOnce(RuntimeHandle) -> Result<T, RuntimeError> + Send + 'static,
    {
        let runtime = self.runtime.clone();
        bounded(self.requests.clone(), move || action(runtime)).await
    }
    /// Status/cancel never wait behind a load, import, hash, or event receiver.
    pub async fn control<T, F>(&self, action: F) -> Result<T, ApiError>
    where
        T: Send + 'static,
        F: FnOnce(RuntimeHandle) -> Result<T, RuntimeError> + Send + 'static,
    {
        let runtime = self.runtime.clone();
        bounded(self.controls.clone(), move || action(runtime)).await
    }
    async fn storage<T, F>(&self, action: F) -> Result<T, ApiError>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T, RuntimeError> + Send + 'static,
    {
        bounded(self.storage.clone(), action).await
    }
    pub async fn unregister(
        &self,
        request: crate::dto::UnregisterModelRequest,
    ) -> Result<crate::dto::UnregisterModelResult, ApiError> {
        self.ensure_running()?;
        if self
            .load_operations
            .lock()
            .map_err(|_| ApiError::internal())?
            .active()
        {
            return Err(ApiError::busy());
        }
        let permit = self
            .storage
            .clone()
            .try_acquire_owned()
            .map_err(|_| ApiError::busy())?;
        let id = request.model_id.clone();
        let lease = self
            .control(move |runtime| runtime.reserve_unregister(id))
            .await
            .map_err(|error| {
                if error.error.code == "model_conflict" {
                    ApiError::new(
                        axum::http::StatusCode::CONFLICT,
                        "model_unregister_loaded",
                        "Unload this model before removing its registration.",
                        Some("model_id"),
                    )
                } else {
                    error
                }
            })?;
        let store = self.store.clone();
        let registry = self.registry.clone();
        let runtime = self.runtime.clone();
        let loads = self.load_operations.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let _lease = lease;
            if loads.lock().map_err(|_| ApiError::internal())?.active() {
                return Err(ApiError::busy());
            }
            // The actor lease excludes all loads/queue admission until disk,
            // resolver, inventory and historical selected identity agree.
            let mut snapshot = registry.write().map_err(|_| ApiError::internal())?;
            if snapshot.generation != request.generation {
                return Err(model_store::library::library_error(ErrorCode::ModelListChanged).into());
            }
            if !snapshot.models.iter().any(|model| model.id == request.model_id) {
                return Err(model_store::library::library_error(ErrorCode::ModelNotFound).into());
            }
            let mut committed = false;
            let outcome = store.unregister_with_commit(&request.model_id, || committed = true);
            if committed {
                snapshot.models.retain(|model| model.id != request.model_id);
                snapshot.generation = uuid::Uuid::new_v4();
                runtime.forget_unregistered(request.model_id.clone())?;
            }
            if outcome.is_err() && committed {
                return Err(ApiError::new(
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "model_unregister_durability_unconfirmed",
                    "Registration removal is visible but durability was not confirmed. Refresh before retrying.",
                    None,
                ));
            }
            outcome?;
            Ok(crate::dto::UnregisterModelResult {
                model_id: request.model_id,
                removed: true,
                files_preserved: true,
            })
        })
        .await
        .map_err(|_| ApiError::internal())?
    }

    pub async fn import(&self, request: ImportModelRequest) -> Result<ModelSummary, ApiError> {
        self.ensure_running()?;
        // Acquire bounded storage capacity first; actor decides whether the
        // reservation is legal atomically with all submit/load/unload commands.
        let permit = self
            .storage
            .clone()
            .try_acquire_owned()
            .map_err(|_| ApiError::busy())?;
        let lease = self.control(|runtime| runtime.reserve_registry()).await?;
        let store = self.store.clone();
        let cancel = ImportCancellation::default();
        let shutdown = self.shutdown.clone();
        let shutdown_cancel = cancel.clone();
        let watcher = tokio::spawn(async move {
            shutdown.requested().await;
            shutdown_cancel.cancel();
        });
        let mut cancel_guard = ImportGuard {
            cancel: cancel.clone(),
            watcher,
            finished: false,
        };
        let registry = self.registry.clone();
        let result = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let _lease = lease; // Last to drop, AFTER import cleanup / cache update.
            let mut import = ImportRequest::new(
                request.id.clone(),
                request
                    .display_name
                    .unwrap_or_else(|| request.id.to_string()),
                ModelSource::local("user-selected local file"),
            );
            import.expected_sha256 = request.expected_sha256;
            // Only exact ModelNotFound establishes that a later visible entry
            // was newly committed under this actor/store transaction.
            let was_missing = matches!(store.get(&request.id), Err(error) if error.code == ErrorCode::ModelNotFound);
            let imported = store.import_file(request.file, import, &cancel);
            let observed = (was_missing && imported.is_err()).then(|| store.get(&request.id));
            let executable = observed.as_ref().is_some_and(Result::is_ok)
                && store.resolve(&request.id).is_ok_and(|model| model.loadable);
            let outcome = import_outcome(&request.id, was_missing, imported, observed, executable);
            if let Some(summary) = &outcome.observed {
                let mut snapshot = registry.write().map_err(|_| ApiError::internal())?;
                snapshot.generation = uuid::Uuid::new_v4();
                let models = &mut snapshot.models;
                if let Some(previous) = models.iter_mut().find(|model| model.id == summary.id) {
                    *previous = summary.clone();
                } else {
                    models.push(summary.clone());
                    models.sort_by(|a, b| a.id.cmp(&b.id));
                }
            }
            if let Some(error) = outcome.error { Err(error) }
            else { outcome.observed.ok_or_else(ApiError::internal) }
        })
        .await
        .map_err(|_| ApiError::internal())?;
        cancel_guard.finished = true;
        result
    }
}
async fn bounded<T, F>(semaphore: Arc<Semaphore>, action: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, RuntimeError> + Send + 'static,
{
    let permit = semaphore
        .try_acquire_owned()
        .map_err(|_| ApiError::busy())?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        action()
    })
    .await
    .map_err(|_| ApiError::internal())?
    .map_err(ApiError::from)
}

/// Future cancellation (including disconnect) cancels only this import. The
/// blocking task retains its reservation until the store has removed partials.
fn publish_preparation_result(
    registry: &RwLock<RegistrySnapshot>,
    id: &ModelId,
    result: &Result<(), RuntimeError>,
) -> Result<(), RuntimeError> {
    if result.as_ref().is_err_and(|error| {
        matches!(
            error.code,
            ErrorCode::ModelScanCancelled | ErrorCode::ModelScanTimeout
        )
    }) {
        return Ok(());
    }
    let mut registry = registry
        .write()
        .map_err(|_| RuntimeError::new(ErrorCode::RuntimeFaulted, "model registry unavailable"))?;
    let Some(model) = registry.models.iter_mut().find(|model| &model.id == id) else {
        return Err(RuntimeError::new(
            ErrorCode::ModelNotFound,
            "model registration unavailable",
        ));
    };
    // Compatibility remains a separate registered-metadata observation. A
    // source failure takes precedence; successful preparation must not clear a
    // model's load eligibility failure (including in synthetic/test observations).
    let code = result
        .as_ref()
        .err()
        .map(|error| error.code.as_str().to_owned())
        .or_else(|| (!model.loadable).then(|| ErrorCode::UnsupportedModel.as_str().into()));
    let available = result.is_ok() && model.loadable;
    if model.available != available || model.availability_error != code {
        model.available = available;
        model.availability_error = code;
        registry.generation = uuid::Uuid::new_v4();
    }
    Ok(())
}

struct PreparationGuard {
    control: Arc<model_store::library::ScanControl>,
    watcher: tokio::task::JoinHandle<()>,
    finished: bool,
}
impl Drop for PreparationGuard {
    fn drop(&mut self) {
        if !self.finished {
            self.control.cancel();
        }
        self.watcher.abort();
    }
}
struct ImportGuard {
    cancel: ImportCancellation,
    watcher: tokio::task::JoinHandle<()>,
    finished: bool,
}
impl Drop for ImportGuard {
    fn drop(&mut self) {
        if !self.finished {
            self.cancel.cancel();
        }
        self.watcher.abort();
    }
}

struct ImportOutcome {
    observed: Option<ModelSummary>,
    error: Option<ApiError>,
}
/// Pure classification of observations made while the exclusive actor lease is
/// still held. The store's independent commit-failure test proves rename/fsync
/// semantics; this layer never injects filesystem failures into production HTTP.
fn import_outcome(
    id: &ModelId,
    was_missing: bool,
    imported: Result<ModelManifest, RuntimeError>,
    observed: Option<Result<ModelManifest, RuntimeError>>,
    executable: bool,
) -> ImportOutcome {
    match imported {
        Ok(model) => ImportOutcome {
            observed: Some(model.into()),
            error: None,
        },
        Err(original) => {
            if was_missing
                && let Some(Ok(model)) = observed
                && model.id == *id
            {
                let mut summary = ModelSummary::from(model);
                summary.available &= executable;
                return ImportOutcome {
                    observed: Some(summary),
                    error: Some(ApiError::new(
                        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                        "import_committed_durability_unconfirmed",
                        "The model is registered, but import finalization could not be confirmed. Check /runtime/models before retrying; no rollback or retry was performed.",
                        Some("id"),
                    )),
                };
            }
            ImportOutcome {
                observed: None,
                error: Some(original.into()),
            }
        }
    }
}

#[cfg(test)]
mod import_outcome_tests {
    use super::*;
    pub(super) fn model() -> ModelManifest {
        ModelManifest {
            schema_version: 1,
            storage: model_store::ModelStorage::Managed,
            id: ModelId::new("new-model").unwrap(),
            display_name: "Synthetic metadata".into(),
            relative_file: "model.gguf".into(),
            size_bytes: 64,
            sha256: "0".repeat(64),
            source: ModelSource::local("test"),
            architecture: "qwen3".into(),
            quantization: "Q8_0".into(),
            gguf_file_type: 7,
            template_sha256: "1".repeat(64),
            context_limit: 40960,
            default_context: 2048,
            validated_llama_commit: None,
            capabilities: Default::default(),
            validated: false,
            validation: None,
            extra: Default::default(),
        }
    }
    fn failure() -> RuntimeError {
        RuntimeError::new(ErrorCode::Io, "private/source/path must never escape")
    }
    #[test]
    fn confirmed_new_registration_remains_visible_but_is_not_reported_successful() {
        let manifest = model();
        let outcome = import_outcome(
            &manifest.id,
            true,
            Err(failure()),
            Some(Ok(manifest.clone())),
            false,
        );
        assert_eq!(outcome.observed.unwrap().id, manifest.id);
        let error = outcome.error.unwrap();
        assert_eq!(error.status, axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(error.error.code, "import_committed_durability_unconfirmed");
        assert!(
            !serde_json::to_string(&error)
                .unwrap()
                .contains("private/source/path")
        );
    }
    #[test]
    fn uncertain_or_preexisting_observations_do_not_claim_a_commit() {
        let manifest = model();
        for (missing, observed) in [
            (false, Some(Ok(manifest.clone()))),
            (true, Some(Err(failure()))),
            (true, None),
        ] {
            let outcome = import_outcome(&manifest.id, missing, Err(failure()), observed, false);
            assert!(outcome.observed.is_none());
            assert_eq!(outcome.error.unwrap().error.code, "internal_error");
        }
        let outcome = import_outcome(
            &ModelId::new("other-id").unwrap(),
            true,
            Err(failure()),
            Some(Ok(manifest)),
            false,
        );
        assert!(outcome.observed.is_none());
    }
    #[test]
    fn an_unconfirmed_cache_never_advertises_a_model_as_available() {
        let mut manifest = model();
        // Trusted observations are classified here; this synthetic mutation is
        // not a manifest validation test and cannot register model bytes.
        manifest.validated = true;
        manifest.capabilities.chat = true;
        let outcome = import_outcome(
            &manifest.id,
            true,
            Err(failure()),
            Some(Ok(manifest.clone())),
            false,
        );
        assert!(!outcome.observed.unwrap().available);
        let id = manifest.id.clone();
        let success = import_outcome(&id, true, Ok(manifest), None, false);
        assert!(success.error.is_none());
    }
}

#[cfg(test)]
#[path = "state/preparation_tests.rs"]
mod preparation_tests;
