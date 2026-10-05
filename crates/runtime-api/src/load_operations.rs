//! Bounded, capability-scoped explicit load attempts. Cancellation never derives
//! a target from global runtime status, selected model, or active generation.
use crate::{ApiError, ApiState};
use model_store::local_validation::LocalValidation;
use runtime_core::LoadControl;
use runtime_types::{ErrorCode, LoadOptions, ModelId, RuntimeError};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

#[derive(Default)]
pub(crate) struct LoadOperations {
    current: Option<Arc<LoadOperation>>,
    previous: Option<Arc<LoadOperation>>,
}
pub(crate) struct LoadOperation {
    id: Uuid,
    model: ModelId,
    options: LoadOptions,
    only_if_unloaded: bool,
    explicit_threads: bool,
    cancel: LoadControl,
    state: Mutex<Observation>,
}
struct Observation {
    phase: &'static str,
    status: &'static str,
    terminal: bool,
    runtime: Option<Value>,
    validation: Option<LocalValidation>,
    error: Option<ApiError>,
}
pub(crate) fn check_cancelled(cancel: &LoadControl) -> Result<(), ApiError> {
    if cancel.is_cancelled() {
        Err(RuntimeError::new(ErrorCode::RequestCancelled, "load operation cancelled").into())
    } else {
        Ok(())
    }
}
impl LoadOperation {
    fn phase(&self, phase: &'static str) {
        self.state.lock().unwrap().phase = phase;
    }
    fn finish(&self, result: Result<LocalValidation, ApiError>, runtime: Option<Value>) {
        let mut state = self.state.lock().unwrap();
        if state.terminal {
            return;
        }
        state.terminal = true;
        state.phase = "finished";
        state.runtime = runtime;
        match result {
            Ok(validation) => {
                // A fully committed result wins over a later stop click.
                state.status = "completed";
                state.validation = Some(validation);
            }
            Err(error) => {
                state.status = if matches!(
                    error.error.code.as_str(),
                    "request_cancelled" | "model_scan_cancelled"
                ) {
                    "cancelled"
                } else {
                    "failed"
                };
                state.error = Some(if state.status == "cancelled" {
                    RuntimeError::new(ErrorCode::RequestCancelled, "load operation cancelled")
                        .into()
                } else {
                    error
                });
            }
        }
    }
    fn snapshot(&self) -> Value {
        let state = self.state.lock().unwrap();
        json!({"operation_id":self.id,"model_id":self.model,"phase":state.phase,"status":state.status,"terminal":state.terminal,"runtime":state.runtime,"local_validation":state.validation,"error":state.error.as_ref().map(|e| &e.error)})
    }
}
impl ApiState {
    pub(crate) fn load_operation_start(
        &self,
        id: Uuid,
        model: ModelId,
        options: LoadOptions,
        only_if_unloaded: bool,
        explicit_threads: bool,
    ) -> Result<Uuid, ApiError> {
        self.ensure_running()?;
        let task = Arc::new(LoadOperation {
            id,
            model,
            options,
            only_if_unloaded,
            explicit_threads,
            cancel: LoadControl::default(),
            state: Mutex::new(Observation {
                phase: "preparing",
                status: "running",
                terminal: false,
                runtime: None,
                validation: None,
                error: None,
            }),
        });
        {
            let mut slots = self
                .load_operations
                .lock()
                .map_err(|_| ApiError::internal())?;
            if let Some(existing) = slots
                .current
                .iter()
                .chain(slots.previous.iter())
                .find(|existing| existing.id == id)
            {
                return if existing.model == task.model
                    && existing.options == options
                    && existing.only_if_unloaded == only_if_unloaded
                    && existing.explicit_threads == explicit_threads
                {
                    Ok(id)
                } else {
                    Err(ApiError::invalid(
                        "operation_id",
                        "This operation identity has different load parameters.",
                    ))
                };
            }
            if slots
                .current
                .as_ref()
                .is_some_and(|task| !task.state.lock().unwrap().terminal)
            {
                return Err(ApiError::busy());
            }
            slots.previous = slots.current.take();
            slots.current = Some(task.clone());
        }
        let state = self.clone();
        let id = task.id;
        let work = task.clone();
        let run = tokio::spawn(async move {
            state.ensure_running()?;
            state
                .prepare_external_controlled(
                    work.model.clone(),
                    work.cancel.clone(),
                    only_if_unloaded,
                )
                .await?;
            check_cancelled(&work.cancel)?;
            work.phase("loading");
            let model = work.model.clone();
            let cancel = work.cancel.clone();
            state
                .load_prepared(
                    model,
                    options,
                    explicit_threads,
                    Some(cancel),
                    only_if_unloaded,
                )
                .await?;
            check_cancelled(&work.cancel)?;
            work.phase("testing");
            state
                .probe_after_load(work.model.clone(), options, Some(work.cancel.clone()))
                .await
        });
        let state = self.clone();
        tokio::spawn(async move {
            // Joining in a separate supervisor also retires a panicked task.
            let result = run.await.unwrap_or_else(|_| Err(ApiError::internal()));
            let runtime = state
                .control(|runtime| runtime.status())
                .await
                .ok()
                .map(|status| crate::routes::status_json(&state, status));
            task.finish(result, runtime);
        });
        Ok(id)
    }
    fn load_operation(&self, id: Uuid) -> Result<Arc<LoadOperation>, ApiError> {
        let slots = self
            .load_operations
            .lock()
            .map_err(|_| ApiError::internal())?;
        slots
            .current
            .iter()
            .chain(slots.previous.iter())
            .find(|task| task.id == id)
            .cloned()
            .ok_or_else(|| {
                RuntimeError::new(ErrorCode::RequestNotFound, "load operation not found").into()
            })
    }
    pub(crate) fn load_operation_next(&self, id: Uuid) -> Result<Value, ApiError> {
        Ok(self.load_operation(id)?.snapshot())
    }
    pub(crate) fn load_operation_cancel(&self, id: Uuid) -> Result<bool, ApiError> {
        let task = self.load_operation(id)?;
        let mut state = task.state.lock().map_err(|_| ApiError::internal())?;
        if state.terminal {
            return Ok(false);
        }
        task.cancel.cancel();
        state.status = "cancelling";
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use runtime_core::{
        CancellationHandle, ExecutionEvents, Executor, ExecutorCommand, ExecutorEvent, Runtime,
    };
    use runtime_types::{ResolvedModel, RuntimeConfig};
    use std::{sync::mpsc, time::Duration};
    struct Fixture(mpsc::Sender<(ExecutorCommand, ExecutionEvents)>);
    impl Executor for Fixture {
        fn start(
            &mut self,
            command: ExecutorCommand,
            events: ExecutionEvents,
        ) -> Result<CancellationHandle, RuntimeError> {
            if matches!(command, ExecutorCommand::Unload) {
                events.emit(ExecutorEvent::Unloaded);
            } else {
                self.0.send((command, events)).unwrap();
            }
            Ok(CancellationHandle::noop())
        }
    }
    fn setup() -> (
        tempfile::TempDir,
        ApiState,
        mpsc::Receiver<(ExecutorCommand, ExecutionEvents)>,
    ) {
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(model_store::ModelStore::open(root.path()).unwrap());
        let (tx, rx) = mpsc::channel();
        let runtime = Runtime::spawn(
            RuntimeConfig::default(),
            |id: &ModelId| {
                Ok(ResolvedModel {
                    id: id.clone(),
                    path: "fixture.gguf".into(),
                    context_limit: 4096,
                    default_context: 4096,
                    loadable: true,
                })
            },
            Fixture(tx),
        )
        .unwrap();
        (
            root,
            ApiState::new(runtime, store, crate::Config::default(), None),
            rx,
        )
    }
    async fn command(
        rx: &mpsc::Receiver<(ExecutorCommand, ExecutionEvents)>,
    ) -> (ExecutorCommand, ExecutionEvents) {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Ok(command) = rx.try_recv() {
                    return command;
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .unwrap()
    }
    async fn terminal(state: &ApiState, id: Uuid) -> Value {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let result = state.load_operation_next(id).unwrap();
                if result["terminal"] == true {
                    return result;
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .unwrap()
    }
    fn model() -> ModelId {
        ModelId::new("fixture").unwrap()
    }
    #[tokio::test]
    async fn stop_is_scoped_idempotent_and_waits_for_load_cleanup() {
        let (_root, state, commands) = setup();
        let id = Uuid::new_v4();
        state
            .load_operation_start(id, model(), Default::default(), false, false)
            .unwrap();
        let (_, load) = command(&commands).await;
        assert_eq!(
            state
                .load_operation_cancel(Uuid::new_v4())
                .unwrap_err()
                .error
                .code,
            "request_not_found"
        );
        assert_eq!(
            state
                .load_operation_start(id, model(), Default::default(), false, false)
                .unwrap(),
            id
        );
        assert!(
            state
                .load_operation_start(
                    id,
                    ModelId::new("different").unwrap(),
                    Default::default(),
                    false,
                    false
                )
                .is_err()
        );
        assert!(state.load_operation_cancel(id).unwrap());
        assert!(state.load_operation_cancel(id).unwrap());
        assert_eq!(
            state.load_operation_next(id).unwrap()["status"],
            "cancelling"
        );
        assert_eq!(state.load_operation_next(id).unwrap()["terminal"], false);
        load.emit(ExecutorEvent::Failed(RuntimeError::new(
            ErrorCode::RequestCancelled,
            "cleaned",
        )));
        let result = terminal(&state, id).await;
        assert_eq!(result["status"], "cancelled");
        assert_eq!(result["runtime"]["state"], "unloaded");
        assert!(!state.load_operation_cancel(id).unwrap());
        let next = Uuid::new_v4();
        state
            .load_operation_start(next, model(), Default::default(), false, false)
            .unwrap();
        let (_, load) = command(&commands).await;
        assert!(!state.load_operation_cancel(id).unwrap());
        assert_eq!(
            state.load_operation_next(next).unwrap()["status"],
            "running"
        );
        state.load_operation_cancel(next).unwrap();
        load.emit(ExecutorEvent::Failed(RuntimeError::new(
            ErrorCode::RequestCancelled,
            "cleaned",
        )));
        terminal(&state, next).await;
        state.shutdown.begin();
        state.wait_shutdown().await.unwrap();
    }
    #[tokio::test]
    async fn stopped_probe_preserves_load_and_never_persists_failed_evidence() {
        let (root, state, commands) = setup();
        let id = Uuid::new_v4();
        state
            .load_operation_start(id, model(), Default::default(), false, false)
            .unwrap();
        command(&commands).await.1.emit(ExecutorEvent::Loaded);
        let (command, generation) = command(&commands).await;
        assert!(matches!(command, ExecutorCommand::Generate { .. }));
        generation.emit(ExecutorEvent::Prepared { prompt_tokens: 8 });
        state.load_operation_cancel(id).unwrap();
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert_eq!(state.load_operation_next(id).unwrap()["terminal"], false);
        generation.emit(ExecutorEvent::GenerationFailed {
            error: RuntimeError::new(ErrorCode::RequestCancelled, "cleaned"),
            usage: Default::default(),
        });
        let result = terminal(&state, id).await;
        assert_eq!(result["status"], "cancelled");
        assert_eq!(result["runtime"]["state"], "ready");
        assert!(
            model_store::local_validation::Receipts::read(root.path())
                .unwrap()
                .entries
                .is_empty()
        );
        state.shutdown.begin();
        state.wait_shutdown().await.unwrap();
    }
    #[tokio::test]
    async fn stopped_probe_preserves_native_failure_and_cleanup_unconfirmed() {
        for code in [
            ErrorCode::NativeFailure,
            ErrorCode::ExecutorCleanupUnconfirmed,
        ] {
            let (root, state, commands) = setup();
            let id = Uuid::new_v4();
            state
                .load_operation_start(id, model(), Default::default(), false, false)
                .unwrap();
            command(&commands).await.1.emit(ExecutorEvent::Loaded);
            let (_, generation) = command(&commands).await;
            generation.emit(ExecutorEvent::Prepared { prompt_tokens: 8 });
            state.load_operation_cancel(id).unwrap();
            tokio::time::sleep(Duration::from_millis(40)).await;
            generation.emit(if code == ErrorCode::ExecutorCleanupUnconfirmed {
                ExecutorEvent::CleanupUnconfirmed(RuntimeError::new(code, "fixture"))
            } else {
                ExecutorEvent::GenerationFailed {
                    error: RuntimeError::new(code, "fixture"),
                    usage: Default::default(),
                }
            });
            let result = terminal(&state, id).await;
            assert_eq!(result["status"], "failed");
            assert_eq!(result["error"]["code"], code.as_str());
            assert!(
                model_store::local_validation::Receipts::read(root.path())
                    .unwrap()
                    .entries
                    .is_empty()
            );
            state.shutdown.begin();
            if code == ErrorCode::ExecutorCleanupUnconfirmed {
                assert!(state.wait_shutdown().await.is_err());
            } else {
                state.wait_shutdown().await.unwrap();
            }
        }
    }
    #[tokio::test]
    async fn automatic_attempt_never_evicts_existing_selection() {
        let (_root, state, commands) = setup();
        let handle = state.runtime.clone();
        let load = tokio::task::spawn_blocking(move || handle.load(model(), Default::default()));
        command(&commands).await.1.emit(ExecutorEvent::Loaded);
        load.await.unwrap().unwrap();
        let id = Uuid::new_v4();
        state
            .load_operation_start(
                id,
                ModelId::new("other").unwrap(),
                Default::default(),
                true,
                false,
            )
            .unwrap();
        let result = terminal(&state, id).await;
        assert_eq!(result["status"], "failed");
        assert_eq!(result["error"]["code"], "runtime_busy");
        assert_eq!(
            state.runtime.status().unwrap().selected_model,
            Some(model())
        );
        assert!(commands.try_recv().is_err());
        state.shutdown.begin();
        state.wait_shutdown().await.unwrap();
    }
}
