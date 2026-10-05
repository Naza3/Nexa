//! Window-owned attempts retain their opaque identity even if the start response
//! is lost. No global status/model/request identifier is used to cancel work.
use crate::*;
use std::sync::Arc;
use uuid::Uuid;

type AutomaticLoadOwner<'a> = (&'a AtomicBool, &'a (dyn Fn(&str) + Send + Sync));

#[derive(Default)]
pub(crate) struct LoadSlot {
    current: Option<Arc<LoadTask>>,
    previous: Option<Arc<LoadTask>>,
}
struct LoadTask {
    state: Mutex<ModelLoadOperationState>,
    cancel: AtomicBool,
    poll_error: Mutex<Option<BridgeError>>,
    changed: tokio::sync::Notify,
}
impl LoadSlot {
    fn task(&self, id: Uuid) -> Result<Arc<LoadTask>> {
        self.current
            .iter()
            .chain(self.previous.iter())
            .find(|task| task.state.lock().unwrap().operation_id == id)
            .cloned()
            .ok_or_else(|| BridgeError::new("request_not_owned"))
    }
}
impl LoadTask {
    fn snapshot(&self) -> ModelLoadOperationState {
        self.state.lock().unwrap().clone()
    }
    fn fail(&self, error: BridgeError) {
        let mut state = self.state.lock().unwrap();
        state.phase = "finished".into();
        state.status = "failed".into();
        state.terminal = true;
        state.error = Some(error);
        self.changed.notify_waiters();
    }
}
impl DesktopBridge {
    pub fn model_load_start(
        self: &Arc<Self>,
        request: ModelLoadStartRequest,
    ) -> Result<ModelLoadOperationHandle> {
        let body = onboarding::load_body(request.load)?;
        self.start_load_task(body, false, request.operation_id)
    }
    pub fn model_load_profile_start(
        self: &Arc<Self>,
        request: ModelLoadProfileStartRequest,
    ) -> Result<ModelLoadOperationHandle> {
        let id = runtime_types::ModelId::new(request.load.model_id)
            .map_err(|_| BridgeError::new("invalid_request"))?;
        let mut body = serde_json::to_value(request.load.load_overrides)
            .map_err(|_| BridgeError::new("settings_invalid"))?;
        body["model"] = json!(id);
        body["backend"] = json!("cpu");
        body["gpu_layers"] = json!(0);
        self.start_load_task(body, true, request.operation_id)
    }
    fn start_load_task(
        self: &Arc<Self>,
        mut body: Value,
        profile: bool,
        id: Uuid,
    ) -> Result<ModelLoadOperationHandle> {
        self.open()?;
        if id.is_nil() {
            return Err(BridgeError::new("invalid_request"));
        }
        if self.loads.lock().unwrap().task(id).is_ok() {
            return Err(BridgeError::new("duplicate_request_id"));
        }
        let work = self
            .work
            .clone()
            .try_lock_owned()
            .map_err(|_| BridgeError::new("desktop_busy"))?;
        let model = body["model"]
            .as_str()
            .ok_or_else(|| BridgeError::new("invalid_request"))?
            .to_owned();
        body["operation_id"] = json!(id);
        let task = Arc::new(LoadTask {
            state: Mutex::new(ModelLoadOperationState {
                operation_id: id,
                model_id: model,
                phase: "preparing".into(),
                status: "running".into(),
                terminal: false,
                runtime: None,
                local_validation: None,
                error: None,
            }),
            cancel: AtomicBool::new(false),
            poll_error: Mutex::new(None),
            changed: tokio::sync::Notify::new(),
        });
        {
            let mut slots = self.loads.lock().unwrap();
            slots.previous = slots.current.take();
            slots.current = Some(task.clone());
        }
        let bridge = self.clone();
        let worker = task.clone();
        let run = tokio::spawn(async move { bridge.track_load(worker, body, profile, None).await });
        tokio::spawn(async move {
            let _work = work; // Including the uncertain-start recovery period.
            if run.await.is_err() {
                task.fail(BridgeError::new("model_load_interrupted"));
            }
        });
        Ok(ModelLoadOperationHandle { operation_id: id })
    }
    async fn load_json<T: DeserializeOwned>(
        &self,
        instance: Uuid,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<T> {
        let mut connection = self.connect().await?;
        if connection.instance_id() != instance {
            return Err(BridgeError::new("runtime_shutdown"));
        }
        serde_json::from_value(connection.json(method, path, body).await?)
            .map_err(|_| BridgeError::new("response_invalid"))
    }
    async fn track_load(
        &self,
        task: Arc<LoadTask>,
        body: Value,
        profile: bool,
        automatic: Option<AutomaticLoadOwner<'_>>,
    ) {
        let mut connection = match self.connect().await {
            Ok(connection) => connection,
            Err(error) => {
                task.fail(error);
                return;
            }
        };
        let instance = connection.instance_id();
        if profile {
            let checked = tokio::time::timeout(Duration::from_secs(3), async {
                let config: ConfigurationSnapshot = serde_json::from_value(
                    connection
                        .json(Method::GET, "/runtime/configuration", None)
                        .await?,
                )
                .map_err(|_| BridgeError::new("response_invalid"))?;
                if config.schema_version != 2 {
                    return Err(BridgeError::new("configuration_migration_required"));
                }
                if config.pending_restart {
                    return Err(BridgeError::new("configuration_restart_required"));
                }
                Ok(())
            })
            .await
            .unwrap_or_else(|_| Err(BridgeError::new("connection_failed")));
            if let Err(error) = checked {
                task.fail(error);
                return;
            }
        }
        if task.cancel.load(Ordering::Acquire)
            || self.closing.load(Ordering::Acquire)
            || automatic.is_some_and(|(cancel, _)| cancel.load(Ordering::Acquire))
        {
            let mut state = task.state.lock().unwrap();
            state.status = "cancelled".into();
            state.phase = "finished".into();
            state.terminal = true;
            state.error = Some(BridgeError::new("request_cancelled"));
            task.changed.notify_waiters();
            return;
        }
        let id = task.snapshot().operation_id;
        // Keep the POST alive, but concurrently recover/poll/cancel this exact
        // ID. A withheld start response cannot delay manual Stop for 750s.
        let mut start = Box::pin(async {
            let value = connection
                .json(Method::POST, "/runtime/load-operations", Some(&body))
                .await
                .map_err(BridgeError::from)?;
            serde_json::from_value::<ModelLoadOperationHandle>(value)
                .map_err(|_| BridgeError::new("response_invalid"))
        });
        let mut start_finished = false;
        let mut admitted = false;
        let path = format!("/runtime/load-operations/{id}");
        loop {
            if automatic.is_some_and(|(cancel, _)| cancel.load(Ordering::Acquire)) {
                task.cancel.store(true, Ordering::Release);
            }
            if !start_finished {
                tokio::select! {
                    result = &mut start => {
                        start_finished = true;
                        match result {
                            Ok(handle) if handle.operation_id == id => admitted = true,
                            Ok(_) => (), // A mismatched handle is uncertain too.
                            Err(error) if !matches!(error.code.as_str(), "connection_failed" | "response_invalid") => { task.fail(error); return; },
                            Err(_) => (),
                        }
                    },
                    _ = tokio::time::sleep(Duration::from_millis(100)) => (),
                }
            }
            if task.cancel.load(Ordering::Acquire) || self.closing.load(Ordering::Acquire) {
                let _ = tokio::time::timeout(
                    Duration::from_secs(1),
                    self.load_json::<ModelLoadStopping>(
                        instance,
                        Method::POST,
                        &format!("{path}/cancel"),
                        Some(&json!({})),
                    ),
                )
                .await;
            }
            match tokio::time::timeout(
                Duration::from_secs(1),
                self.load_json::<ModelLoadOperationState>(instance, Method::GET, &path, None),
            )
            .await
            {
                Ok(Ok(mut next))
                    if next.operation_id == id
                        && next.model_id == task.snapshot().model_id
                        && valid_observation(&next) =>
                {
                    admitted = true;
                    if let Some((_, progress)) = automatic
                        && !next.terminal
                    {
                        progress(&next.phase);
                    }
                    *task.poll_error.lock().unwrap() = None;
                    // Translate only known API error codes; never surface server text.
                    next.error = next.error.map(|error| BridgeError::api(Some(&error.code)));
                    let terminal = next.terminal;
                    let mut state = task.state.lock().unwrap();
                    if !terminal && task.cancel.load(Ordering::Acquire) {
                        next.status = "cancelling".into();
                    }
                    *state = next;
                    task.changed.notify_waiters();
                    if terminal {
                        return;
                    }
                }
                Ok(Err(error))
                    if error.code == "runtime_shutdown"
                        || (admitted && error.code == "request_not_found") =>
                {
                    task.fail(BridgeError::new(if error.code == "runtime_shutdown" {
                        "runtime_shutdown"
                    } else {
                        "model_load_result_unavailable"
                    }));
                    return;
                }
                _ => {
                    *task.poll_error.lock().unwrap() =
                        Some(BridgeError::new("model_load_interrupted"));
                    // Do not infer cleanup from a transport error or a 404 racing
                    // the original POST. Only confirmed stopped service retires it.
                    if matches!(InstanceLock::observe(&self.root), Ok(runtime_cli::instance::InstanceObservation::Stopped(ref guard)) if guard.as_ref().is_none_or(|guard| !guard.has_discovery()))
                    {
                        task.fail(BridgeError::new("runtime_shutdown"));
                        return;
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    /// Used inside the existing download/library owner; never reacquires work.
    pub(crate) async fn automatic_load_operation(
        &self,
        mut body: Value,
        cancel: &AtomicBool,
        progress: impl Fn(&str) + Send + Sync,
    ) -> Result<LocalValidation> {
        let id = Uuid::new_v4();
        let model = body["model"]
            .as_str()
            .ok_or_else(|| BridgeError::new("invalid_request"))?
            .to_owned();
        body["operation_id"] = json!(id);
        body["only_if_unloaded"] = json!(true);
        let task = Arc::new(LoadTask {
            state: Mutex::new(ModelLoadOperationState {
                operation_id: id,
                model_id: model.clone(),
                phase: "preparing".into(),
                status: "running".into(),
                terminal: false,
                runtime: None,
                local_validation: None,
                error: None,
            }),
            cancel: AtomicBool::new(false),
            poll_error: Mutex::new(None),
            changed: tokio::sync::Notify::new(),
        });
        progress("preparing");
        self.track_load(task.clone(), body, false, Some((cancel, &progress)))
            .await;
        let result = task.snapshot();
        if result.status == "cancelled" {
            return Ok(LocalValidation {
                state: model_store::local_validation::ValidationState::Deferred,
                checked_at_unix_ms: Some(model_store::local_validation::now_ms()),
                error_code: Some("request_cancelled".into()),
                load_success: result.runtime.as_ref().is_some_and(|status| {
                    matches!(status.state, RuntimeState::Ready | RuntimeState::Generating)
                        && status
                            .selected_model
                            .as_ref()
                            .is_some_and(|id| id.as_str() == model)
                }),
                generation_pass: false,
            });
        }
        result.local_validation.ok_or_else(|| {
            result
                .error
                .unwrap_or_else(|| BridgeError::new("model_load_interrupted"))
        })
    }
    pub async fn model_load_next(&self, id: Uuid) -> Result<ModelLoadOperationState> {
        let task = self.loads.lock().unwrap().task(id)?;
        let state = task.snapshot();
        if !state.terminal
            && let Some(error) = task.poll_error.lock().unwrap().clone()
        {
            return Err(error);
        }
        Ok(state)
    }
    pub async fn model_load_cancel(&self, id: Uuid) -> Result<ModelLoadStopping> {
        let task = self.loads.lock().unwrap().task(id)?;
        let mut state = task.state.lock().unwrap();
        if state.terminal {
            return Ok(ModelLoadStopping { stopping: false });
        }
        task.cancel.store(true, Ordering::Release);
        state.status = "cancelling".into();
        Ok(ModelLoadStopping { stopping: true })
    }
    pub(crate) async fn close_load(&self) -> Result<()> {
        let task = self.loads.lock().unwrap().current.clone();
        if let Some(task) = task {
            task.cancel.store(true, Ordering::Release);
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    let changed = task.changed.notified();
                    if task.snapshot().terminal { return; }
                    tokio::select! { _ = changed => (), _ = tokio::time::sleep(Duration::from_millis(50)) => () }
                }
            }).await.map_err(|_| BridgeError::new("model_load_interrupted"))?;
        }
        Ok(())
    }
}

fn valid_observation(state: &ModelLoadOperationState) -> bool {
    match (state.status.as_str(), state.terminal, state.phase.as_str()) {
        ("running" | "cancelling", false, "preparing" | "loading" | "testing") => {
            state.error.is_none() && state.local_validation.is_none()
        }
        ("completed", true, "finished") => {
            state.error.is_none() && state.local_validation.is_some()
        }
        ("cancelled", true, "finished") => {
            state
                .error
                .as_ref()
                .is_some_and(|error| error.code == "request_cancelled")
                && state.local_validation.is_none()
        }
        ("failed", true, "finished") => state.error.is_some(),
        _ => false,
    }
}
