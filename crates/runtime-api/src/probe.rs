//! A private, bounded text probe. It bypasses chat history/SSE and never stores
//! prompt or output. Actor admission is atomic and refuses any queued work.
use crate::{ApiState, errors::ApiError};
use model_store::local_validation::{
    self, LocalValidation, Receipt, Receipts, Scope, ValidationState,
};
use runtime_core::{DisconnectHandle, EventReceiver};
use runtime_types::{
    GenerationOptions, GenerationRequest, LoadOptions, Message, ModelId, RequestEventKind,
    RequestId, Role,
};
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_OUTPUT: u32 = 24;
const MAX_BYTES: usize = 16 * 1024;
static RECEIPT_WRITE: Mutex<()> = Mutex::new(());
struct DisconnectOnDrop(DisconnectHandle);
impl Drop for DisconnectOnDrop {
    fn drop(&mut self) {
        self.0.disconnect();
    }
}
impl ApiState {
    fn probe_scope(&self, model: &ModelId, options: LoadOptions) -> Result<Scope, &'static str> {
        let executable = std::env::current_exe().map_err(|_| "validation_engine_unavailable")?;
        let stamps = local_validation::engine_file_stamps(&executable)
            .map_err(|_| "validation_engine_unavailable")?;
        let (expected, build) = self
            .probe_build
            .get()
            .ok_or("validation_engine_unavailable")?
            .as_ref()
            .map_err(|code| *code)?;
        if &stamps != expected {
            return Err("validation_scope_changed");
        }
        self.store
            .resolve(model)
            .map_err(|_| "validation_scope_unavailable")?;
        let registered = self
            .store
            .get(model)
            .map_err(|_| "validation_scope_unavailable")?;
        let inventory = model_store::inventory::read(self.store.data_directory())
            .map_err(|_| "validation_scope_unavailable")?;
        let entry = inventory
            .entries
            .iter()
            .find(|entry| &entry.manifest.id == model && entry.availability_error.is_none())
            .ok_or("validation_scope_unavailable")?;
        if registered != entry.manifest {
            return Err("validation_scope_changed");
        }
        self.store
            .resolve(model)
            .map_err(|_| "validation_scope_unavailable")?;
        local_validation::scope(self.store.data_directory(), entry, options, build)
            .map_err(|_| "validation_scope_unavailable")
    }
    fn save_observation(
        &self,
        scope: Scope,
        observation: LocalValidation,
    ) -> Result<(), &'static str> {
        let _guard = RECEIPT_WRITE
            .lock()
            .map_err(|_| "validation_record_write_failed")?;
        let root = self.store.data_directory();
        let mut receipts = Receipts::read(root).map_err(|_| "validation_record_read_failed")?;
        receipts
            .record(root, Receipt { scope, observation })
            .map_err(|_| "validation_record_write_failed")
    }
    pub(crate) async fn desktop_load(
        &self,
        model: ModelId,
        options: LoadOptions,
        explicit_threads: bool,
        only_if_unloaded: bool,
    ) -> Result<LocalValidation, ApiError> {
        // Preparation has its own actor reservation and cannot mutate a loaded
        // session. Final admission is checked again by LoadIfUnloaded.
        if only_if_unloaded {
            self.ensure_running()?;
            self.prepare_external_for_onboarding(model.clone()).await?;
            let requested = model.clone();
            self.execute(move |runtime| runtime.load_if_unloaded(requested, options))
                .await?;
        } else {
            self.load(model.clone(), options, explicit_threads).await?;
        }
        let state = self.clone();
        let requested = model.clone();
        let loaded_scope = tokio::task::spawn_blocking(move || {
            let scope = state.probe_scope(&requested, options)?;
            state.save_observation(
                scope.clone(),
                LocalValidation {
                    state: ValidationState::Loaded,
                    checked_at_unix_ms: Some(local_validation::now_ms()),
                    error_code: None,
                    load_success: true,
                    generation_pass: false,
                },
            )?;
            Ok::<_, &'static str>(scope)
        })
        .await
        .map_err(|_| ApiError::internal())?;
        let (mut observation, needs_record) = match self.model_probe(model, options).await {
            Ok(observation) => {
                let deferred = observation.state == ValidationState::Deferred;
                (observation, deferred)
            }
            Err(error) => (
                LocalValidation {
                    state: ValidationState::Failed,
                    error_code: Some(error.error.code),
                    ..LocalValidation::default()
                },
                true,
            ),
        };
        observation.load_success = true;
        if observation.checked_at_unix_ms.is_none() {
            observation.checked_at_unix_ms = Some(local_validation::now_ms());
        }
        if needs_record {
            let state = self.clone();
            let saved = observation.clone();
            let persisted = tokio::task::spawn_blocking(move || {
                loaded_scope.and_then(|scope| state.save_observation(scope, saved))
            })
            .await
            .map_err(|_| ApiError::internal())?;
            if let Err(code) = persisted {
                evidence_unavailable(&mut observation, code);
            }
        }
        Ok(observation)
    }
    pub(crate) async fn model_probe(
        &self,
        model: ModelId,
        options: LoadOptions,
    ) -> Result<LocalValidation, ApiError> {
        self.ensure_running()?;
        let state = self.clone();
        let requested = model.clone();
        let scope = tokio::task::spawn_blocking(move || state.probe_scope(&requested, options))
            .await
            .map_err(|_| ApiError::internal())?;
        let request = GenerationRequest {
            request_id: RequestId::new(),
            model,
            messages: vec![Message::new(Role::User, "Reply with one short greeting.")],
            options: GenerationOptions {
                max_tokens: MAX_OUTPUT,
                temperature: 0.0,
                seed: 0,
                ..GenerationOptions::default()
            },
        };
        let receiver = match self
            .execute(move |runtime| runtime.submit_if_idle(request, options))
            .await
        {
            Ok(receiver) => receiver,
            Err(error)
                if matches!(error.error.code.as_str(), "runtime_busy" | "model_conflict") =>
            {
                return Ok(LocalValidation {
                    state: ValidationState::Deferred,
                    error_code: Some("runtime_busy".into()),
                    ..LocalValidation::default()
                });
            }
            Err(error) => return Err(error),
        };
        let _disconnect = DisconnectOnDrop(receiver.disconnect_handle());
        let state = self.clone();
        let observation = tokio::task::spawn_blocking(move || {
            let mut observation = consume_probe(receiver, PROBE_TIMEOUT, options.context_size);
            match scope {
                Ok(scope) => {
                    match state.probe_scope(&scope.model_id, scope.options) {
                        Ok(current) if current == scope => (),
                        current => {
                            observation.state = ValidationState::Stale;
                            observation.generation_pass = false;
                            observation.error_code =
                                Some(current.err().unwrap_or("validation_scope_changed").into());
                        }
                    }
                    if let Err(code) = state.save_observation(scope, observation.clone()) {
                        evidence_unavailable(&mut observation, code);
                    }
                }
                Err(code) => evidence_unavailable(&mut observation, code),
            }
            observation
        })
        .await
        .map_err(|_| ApiError::internal())?;
        Ok(observation)
    }
}
fn evidence_unavailable(observation: &mut LocalValidation, code: &'static str) {
    observation.error_code = Some(code.into());
    // A generation result without bound, durable evidence cannot grant Passed.
    // Preserve the independently observed successful load.
    if observation.generation_pass {
        observation.state = ValidationState::Loaded;
        observation.generation_pass = false;
    }
}
fn consume_probe(receiver: EventReceiver, timeout: Duration, context_size: u32) -> LocalValidation {
    let deadline = Instant::now() + timeout;
    let mut nonempty = false;
    let mut bytes = 0usize;
    let mut started = None;
    let result = loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break Err("execution_timeout".to_string());
        }
        let event = match receiver.recv_timeout(remaining) {
            Ok(event) => event,
            Err(_) => {
                break Err(if Instant::now() >= deadline {
                    "execution_timeout"
                } else {
                    "consumer_stopped"
                }
                .into());
            }
        };
        match event.kind {
            RequestEventKind::Started { prompt_tokens } => started = Some(prompt_tokens),
            RequestEventKind::TextDelta(text) => {
                bytes = bytes.saturating_add(text.len());
                nonempty |= !text.trim().is_empty();
                if bytes > MAX_BYTES {
                    break Err("response_too_large".into());
                }
            }
            RequestEventKind::Completed { usage, .. } => {
                if nonempty
                    && started == Some(usage.prompt_tokens)
                    && usage.prompt_tokens > 0
                    && usage.completion_tokens > 0
                    && usage.completion_tokens <= MAX_OUTPUT
                    && usage.total_tokens() <= context_size as u64
                {
                    break Ok(());
                } else {
                    break Err("model_test_empty_or_invalid".into());
                }
            }
            RequestEventKind::Cancelled { reason, .. } => break Err(reason.as_str().into()),
            RequestEventKind::Failed { error, .. } => break Err(error.code.as_str().into()),
            _ => (),
        }
    };
    let passed = result.is_ok();
    LocalValidation {
        state: if passed {
            ValidationState::Passed
        } else {
            ValidationState::Failed
        },
        checked_at_unix_ms: Some(local_validation::now_ms()),
        error_code: result.err(),
        load_success: true,
        generation_pass: passed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use runtime_core::{
        CancellationHandle, ExecutionEvents, Executor, ExecutorCommand, ExecutorEvent, Runtime,
    };
    use runtime_types::{
        ErrorCode, FinishReason, ResolvedModel, RuntimeConfig, RuntimeError, Usage,
    };
    #[derive(Clone, Copy)]
    enum Mode {
        Pass,
        Empty,
        BadUsage,
        Cancel,
        Wait,
    }
    struct Fixture(Mode);
    impl Executor for Fixture {
        fn start(
            &mut self,
            command: ExecutorCommand,
            events: ExecutionEvents,
        ) -> Result<CancellationHandle, RuntimeError> {
            match command {
                ExecutorCommand::Load { .. } => {
                    events.emit(ExecutorEvent::Loaded);
                }
                ExecutorCommand::Unload => {
                    events.emit(ExecutorEvent::Unloaded);
                }
                ExecutorCommand::Generate { .. } => {
                    events.emit(ExecutorEvent::Prepared { prompt_tokens: 8 });
                    if matches!(self.0, Mode::Pass | Mode::BadUsage) {
                        events.text_delta("private-generated-body-canary");
                    }
                    match self.0 {
                        Mode::Wait => {
                            return Ok(CancellationHandle::new(move || {
                                events.emit(ExecutorEvent::GenerationFailed {
                                    error: RuntimeError::new(
                                        ErrorCode::RequestCancelled,
                                        "cancelled",
                                    ),
                                    usage: Usage {
                                        prompt_tokens: 8,
                                        completion_tokens: 0,
                                    },
                                });
                            }));
                        }
                        Mode::Cancel => {
                            events.emit(ExecutorEvent::GenerationFailed {
                                error: RuntimeError::new(ErrorCode::RequestCancelled, "cancelled"),
                                usage: Usage {
                                    prompt_tokens: 8,
                                    completion_tokens: 0,
                                },
                            });
                        }
                        _ => {
                            events.emit(ExecutorEvent::Completed {
                                usage: Usage {
                                    prompt_tokens: 8,
                                    completion_tokens: if matches!(self.0, Mode::BadUsage) {
                                        MAX_OUTPUT + 1
                                    } else {
                                        1
                                    },
                                },
                                finish_reason: FinishReason::Stop,
                            });
                        }
                    }
                }
            }
            Ok(CancellationHandle::noop())
        }
    }
    fn stream(mode: Mode) -> (Runtime, EventReceiver) {
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
            Fixture(mode),
        )
        .unwrap();
        let model = ModelId::new("fixture").unwrap();
        let options = LoadOptions::default();
        runtime.handle().load(model.clone(), options).unwrap();
        let receiver = runtime
            .handle()
            .submit_if_idle(
                GenerationRequest {
                    request_id: RequestId::new(),
                    model,
                    messages: vec![Message::new(Role::User, "private-prompt-body-canary")],
                    options: GenerationOptions {
                        max_tokens: MAX_OUTPUT,
                        ..Default::default()
                    },
                },
                options,
            )
            .unwrap();
        (runtime, receiver)
    }
    #[test]
    fn only_valid_nonempty_terminal_passes_and_observation_has_no_body() {
        for (mode, pass) in [
            (Mode::Pass, true),
            (Mode::Empty, false),
            (Mode::BadUsage, false),
            (Mode::Cancel, false),
        ] {
            let (runtime, receiver) = stream(mode);
            let observation = consume_probe(receiver, Duration::from_secs(1), 4096);
            assert_eq!(observation.generation_pass, pass);
            assert!(observation.load_success);
            let json = serde_json::to_string(&observation).unwrap();
            assert!(!json.contains("canary"));
            assert!(!json.contains("private"));
            runtime.shutdown().unwrap();
        }
    }
    #[test]
    fn timeout_and_disconnected_consumer_never_pass_and_cancel_owned_request() {
        let (runtime, receiver) = stream(Mode::Wait);
        let observation = consume_probe(receiver, Duration::from_millis(10), 4096);
        assert!(!observation.generation_pass);
        assert_eq!(observation.error_code.as_deref(), Some("execution_timeout"));
        runtime.shutdown().unwrap();
        let (runtime, receiver) = stream(Mode::Wait);
        receiver.disconnect_handle().disconnect();
        let observation = consume_probe(receiver, Duration::from_secs(1), 4096);
        assert!(!observation.generation_pass);
        runtime.shutdown().unwrap();
    }
    #[test]
    fn evidence_failures_preserve_only_observed_load_success() {
        for code in [
            "validation_engine_unavailable",
            "validation_scope_unavailable",
            "validation_scope_changed",
            "validation_record_read_failed",
            "validation_record_write_failed",
        ] {
            let mut observation = LocalValidation {
                state: ValidationState::Passed,
                checked_at_unix_ms: Some(1),
                load_success: true,
                generation_pass: true,
                error_code: None,
            };
            evidence_unavailable(&mut observation, code);
            assert_eq!(observation.state, ValidationState::Loaded);
            assert!(observation.load_success);
            assert!(!observation.generation_pass);
            assert_eq!(observation.error_code.as_deref(), Some(code));
            let encoded = serde_json::to_string(&observation).unwrap();
            assert!(!encoded.contains("private"));
        }
    }
    #[tokio::test]
    async fn corrupt_receipts_report_read_stage_and_remain_intact() {
        use model_store::{ModelStore, library::FileIdentity};
        use std::sync::Arc;
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(ModelStore::open(root.path()).unwrap());
        let runtime = Runtime::spawn(
            RuntimeConfig::default(),
            |_: &ModelId| Err(RuntimeError::new(ErrorCode::ModelNotFound, "fixture")),
            Fixture(Mode::Pass),
        )
        .unwrap();
        let state = ApiState::new(runtime, store, crate::Config::default(), None);
        let scope = Scope {
            model_id: ModelId::new("fixture").unwrap(),
            model_sha256: "0".repeat(64),
            template_sha256: "1".repeat(64),
            file_identity: FileIdentity {
                volume: 1,
                file: 2,
                size: 3,
                modified_seconds: 4,
                modified_nanos: 5,
            },
            engine_build: "fixture".into(),
            platform: "fixture".into(),
            installation: "2".repeat(64),
            options: LoadOptions::default(),
        };
        let observation = LocalValidation {
            state: ValidationState::Loaded,
            load_success: true,
            ..LocalValidation::default()
        };
        state
            .save_observation(scope.clone(), observation.clone())
            .unwrap();
        assert_eq!(Receipts::read(root.path()).unwrap().entries.len(), 1);
        let mut invalid = observation.clone();
        invalid.error_code = Some("private/path-canary".into());
        assert_eq!(
            state.save_observation(scope.clone(), invalid).unwrap_err(),
            "validation_record_write_failed"
        );
        let path = root.path().join("local-model-validation.json");
        std::fs::write(&path, b"private-corrupt-canary").unwrap();
        assert_eq!(
            state.save_observation(scope, observation).unwrap_err(),
            "validation_record_read_failed"
        );
        assert_eq!(std::fs::read(path).unwrap(), b"private-corrupt-canary");
        state.shutdown.begin();
        state.wait_shutdown().await.unwrap();
    }
    #[tokio::test]
    async fn successful_generation_without_engine_identity_cannot_claim_a_receipt() {
        use model_store::ModelStore;
        use std::sync::Arc;
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(ModelStore::open(root.path()).unwrap());
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
            Fixture(Mode::Pass),
        )
        .unwrap();
        let model = ModelId::new("fixture").unwrap();
        let options = LoadOptions::default();
        runtime.handle().load(model.clone(), options).unwrap();
        let state = ApiState::new(runtime, store, crate::Config::default(), None);
        state
            .probe_build
            .set(Err("validation_engine_unavailable"))
            .unwrap();
        let observation = state.model_probe(model, options).await.unwrap();
        assert_eq!(observation.state, ValidationState::Loaded);
        assert!(observation.load_success && !observation.generation_pass);
        assert_eq!(
            observation.error_code.as_deref(),
            Some("validation_engine_unavailable")
        );
        assert!(Receipts::read(root.path()).unwrap().entries.is_empty());
        state.shutdown.begin();
        state.wait_shutdown().await.unwrap();
    }
}
