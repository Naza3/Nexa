//! Single-owner MNN execution using runtime-core's scheduler and output ledger.
//! Production composition intentionally has no admitted Android device profiles.
use mnn_adapter::{Cancellation, ErrorKind, Model, TextAction};
use mnn_model_store::{LoadLease, MnnRegistrySnapshot};
use runtime_core::{
    CancellationHandle, ExecutionEvents, Executor, ExecutorCommand, ExecutorEvent, ModelResolver,
};
use runtime_types::{ErrorCode, GenerationRequest, ModelId, ResolvedModel, RuntimeError, Usage};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
};
fn error(code: ErrorCode, message: &str) -> RuntimeError {
    RuntimeError::new(code, message)
}
fn unavailable() -> RuntimeError {
    error(ErrorCode::ExecutorUnavailable, "MNN owner is unavailable")
}
fn map_error(e: mnn_adapter::Error) -> RuntimeError {
    error(
        match e.kind {
            ErrorKind::Cancelled => ErrorCode::RequestCancelled,
            ErrorKind::Budget => ErrorCode::ContextLengthExceeded,
            ErrorKind::Invalid => ErrorCode::InvalidArgument,
            ErrorKind::Identity => ErrorCode::IntegrityFailure,
            ErrorKind::Busy => ErrorCode::RuntimeBusy,
            ErrorKind::WrongThread => ErrorCode::WrongThread,
            ErrorKind::Callback => ErrorCode::ConsumerStopped,
            ErrorKind::Native => ErrorCode::NativeFailure,
            _ => ErrorCode::NativeProtocol,
        },
        "MNN operation failed",
    )
}
#[derive(Clone)]
struct Cancel {
    native: Cancellation,
    requested: Arc<AtomicBool>,
}
impl Cancel {
    fn new() -> Result<Self, RuntimeError> {
        Ok(Self {
            native: Cancellation::new().map_err(map_error)?,
            requested: Arc::new(AtomicBool::new(false)),
        })
    }
    fn cancel(&self) {
        self.requested.store(true, Ordering::Release);
        self.native.cancel();
    }
    fn requested(&self) -> bool {
        self.requested.load(Ordering::Acquire)
    }
}
struct Job {
    command: ExecutorCommand,
    events: ExecutionEvents,
    cancel: Cancel,
}
#[derive(Default)]
struct State {
    busy: AtomicBool,
    loaded: AtomicBool,
    failed: AtomicBool,
    #[cfg(test)]
    progress_hook: std::sync::Mutex<Option<tests::ProgressHook>>,
}
impl State {
    fn progress(&self, _p: mnn_adapter::Progress) {
        #[cfg(test)]
        {
            let hook = self.progress_hook.lock().unwrap().clone();
            if let Some(hook) = hook {
                hook(_p);
            }
        }
    }
}
pub struct MnnModelResolver {
    snapshot: Arc<MnnRegistrySnapshot>,
    #[cfg(test)]
    research: Option<tests::ResearchCpuEvidence>,
}
impl ModelResolver for MnnModelResolver {
    fn resolve(&self, id: &ModelId) -> Result<ResolvedModel, RuntimeError> {
        let resolved = self.snapshot.resolve_candidate(id)?;
        #[cfg(test)]
        if self.research.is_some() {
            return Ok(ResolvedModel {
                loadable: true,
                ..resolved
            });
        }
        let _ = resolved;
        Err(error(
            ErrorCode::UnsupportedModel,
            "MNN candidate has no trusted Android device admission",
        ))
    }
}
pub struct MnnExecutor {
    sender: Option<SyncSender<Job>>,
    thread: Option<JoinHandle<()>>,
    state: Arc<State>,
    active: Option<Cancel>,
}
impl MnnExecutor {
    /// The only public factory binds resolver and owner to the same registry snapshot.
    /// Integrity-valid candidates remain unavailable through the product resolver.
    pub fn composition(
        snapshot: MnnRegistrySnapshot,
    ) -> Result<(MnnModelResolver, Self), RuntimeError> {
        let snapshot = Arc::new(snapshot);
        let executor = Self::new(snapshot.clone())?;
        Ok((
            MnnModelResolver {
                snapshot,
                #[cfg(test)]
                research: None,
            },
            executor,
        ))
    }
    fn new(snapshot: Arc<MnnRegistrySnapshot>) -> Result<Self, RuntimeError> {
        mnn_adapter::build_identity().map_err(map_error)?;
        let (sender, receiver) = mpsc::sync_channel(1);
        let state = Arc::new(State::default());
        let worker = state.clone();
        let thread = thread::Builder::new()
            .name("nexa-mnn-owner".into())
            .spawn(move || owner(receiver, snapshot, worker))
            .map_err(|_| unavailable())?;
        Ok(Self {
            sender: Some(sender),
            thread: Some(thread),
            state,
            active: None,
        })
    }
}
impl Executor for MnnExecutor {
    fn start(
        &mut self,
        command: ExecutorCommand,
        events: ExecutionEvents,
    ) -> Result<CancellationHandle, RuntimeError> {
        if self.state.failed.load(Ordering::Acquire) {
            return Err(unavailable());
        }
        let cancel = Cancel::new()?;
        let sender = self.sender.as_ref().ok_or_else(unavailable)?;
        if self
            .state
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(error(
                ErrorCode::RuntimeBusy,
                "MNN owner already has an operation",
            ));
        }
        if sender
            .try_send(Job {
                command,
                events,
                cancel: cancel.clone(),
            })
            .is_err()
        {
            self.state.busy.store(false, Ordering::Release);
            return Err(unavailable());
        }
        self.active = Some(cancel.clone());
        Ok(CancellationHandle::new(move || cancel.cancel()))
    }
    fn close(&mut self) -> Result<(), RuntimeError> {
        if self.state.failed.load(Ordering::Acquire) {
            return Err(error(
                ErrorCode::ExecutorCleanupUnconfirmed,
                "MNN owner cleanup was not confirmed",
            ));
        }
        if self.state.busy.load(Ordering::Acquire) || self.state.loaded.load(Ordering::Acquire) {
            return Err(error(
                ErrorCode::RuntimeBusy,
                "MNN owner requires acknowledged unload before close",
            ));
        }
        self.sender.take();
        if let Some(t) = self.thread.take() {
            t.join().map_err(|_| {
                error(
                    ErrorCode::ExecutorCleanupUnconfirmed,
                    "MNN owner cleanup was not confirmed",
                )
            })?;
        }
        if self.state.failed.load(Ordering::Acquire) {
            return Err(error(
                ErrorCode::ExecutorCleanupUnconfirmed,
                "MNN owner cleanup was not confirmed",
            ));
        }
        Ok(())
    }
}
impl Drop for MnnExecutor {
    fn drop(&mut self) {
        if let Some(c) = &self.active {
            c.cancel();
        }
        self.sender.take(); /* Never join a potentially non-returning kernel. Owner retains every resource. */
    }
}
fn finish(state: &State, events: &ExecutionEvents, event: ExecutorEvent) {
    state.busy.store(false, Ordering::Release);
    events.emit(event);
}
fn pin_unconfirmed<T>(resource: &mut Option<T>, state: &State) {
    // A later successful close cannot disprove an earlier ownership failure.
    std::mem::forget(resource.take());
    state.failed.store(true, Ordering::Release);
}
fn cleanup(model: &mut Option<(Model, LoadLease)>, state: &State) -> Result<(), RuntimeError> {
    if state.failed.load(Ordering::Acquire) {
        pin_unconfirmed(model, state);
        return Err(error(
            ErrorCode::ExecutorCleanupUnconfirmed,
            "MNN cleanup remains unconfirmed",
        ));
    }
    if let Some((mut native, lease)) = model.take() {
        if native.close().is_err() {
            // Native ownership is unconfirmed: pin both handle and its files permanently.
            std::mem::forget(native);
            std::mem::forget(lease);
            state.failed.store(true, Ordering::Release);
            return Err(error(
                ErrorCode::ExecutorCleanupUnconfirmed,
                "MNN native cleanup was not confirmed",
            ));
        }
        drop(native);
        drop(lease);
    }
    state.loaded.store(false, Ordering::Release);
    Ok(())
}
fn owner(receiver: Receiver<Job>, snapshot: Arc<MnnRegistrySnapshot>, state: Arc<State>) {
    let mut model: Option<(Model, LoadLease)> = None;
    while let Ok(job) = receiver.recv() {
        let terminal =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match job.command {
                ExecutorCommand::Load {
                    model: resolved,
                    options,
                } => {
                    if model.is_some() {
                        ExecutorEvent::Failed(error(
                            ErrorCode::RuntimeBusy,
                            "MNN model is already loaded",
                        ))
                    } else {
                        let loaded = (|| {
                            let lease =
                                snapshot.acquire(&resolved, options, || job.cancel.requested())?;
                            let path = lease.runtime_config().to_str().ok_or_else(|| {
                                error(ErrorCode::InvalidManifest, "MNN config path is not UTF-8")
                            })?;
                            let mut native = Model::load(
                                &mnn_adapter::LoadOptions {
                                    runtime_config_path: path,
                                    artifact_sha256: lease.artifact_digest(),
                                    logical_context: options.context_size,
                                    threads: options.threads,
                                    prefill_chunk: options.batch_size,
                                },
                                &job.cancel.native,
                                |p| state.progress(p),
                            )
                            .map_err(map_error)?;
                            if job.cancel.requested() {
                                if native.close().is_err() {
                                    std::mem::forget(native);
                                    std::mem::forget(lease);
                                    state.failed.store(true, Ordering::Release);
                                    return Err(error(
                                        ErrorCode::ExecutorCleanupUnconfirmed,
                                        "MNN cancelled load cleanup unconfirmed",
                                    ));
                                }
                                return Err(error(
                                    ErrorCode::RequestCancelled,
                                    "MNN load cancelled",
                                ));
                            }
                            Ok((native, lease))
                        })();
                        match loaded {
                            Ok(pair) => {
                                model = Some(pair);
                                state.loaded.store(true, Ordering::Release);
                                ExecutorEvent::Loaded
                            }
                            Err(e) => {
                                if state.failed.load(Ordering::Acquire) {
                                    ExecutorEvent::CleanupUnconfirmed(e)
                                } else {
                                    ExecutorEvent::Failed(e)
                                }
                            }
                        }
                    }
                }
                ExecutorCommand::Generate { request } => match model.as_mut() {
                    None => ExecutorEvent::Failed(error(
                        ErrorCode::RuntimeFaulted,
                        "MNN model is not loaded",
                    )),
                    Some((native, _)) => {
                        let (event, fault) =
                            generate(native, request, &job.cancel, &job.events, &state);
                        if matches!(event, ExecutorEvent::CleanupUnconfirmed(_)) {
                            pin_unconfirmed(&mut model, &state);
                            event
                        } else if fault {
                            if let Err(e) = cleanup(&mut model, &state) {
                                ExecutorEvent::CleanupUnconfirmed(e)
                            } else {
                                let fault = match event {
                                    ExecutorEvent::Failed(e)
                                    | ExecutorEvent::GenerationFailed { error: e, .. } => e,
                                    _ => error(ErrorCode::NativeFailure, "MNN owner faulted"),
                                };
                                // Existing core Faulted has no usage payload. Never emit
                                // GenerationFailed first: that would acknowledge twice.
                                ExecutorEvent::Faulted(fault)
                            }
                        } else {
                            event
                        }
                    }
                },
                ExecutorCommand::Unload => match cleanup(&mut model, &state) {
                    Ok(()) => ExecutorEvent::Unloaded,
                    Err(e) => ExecutorEvent::CleanupUnconfirmed(e),
                },
            }));
        let terminal = match terminal {
            Ok(event) => event,
            Err(payload) => {
                // A panic is not proof that every temporary native resource was
                // destroyed. Retain the generation and permanently fail closed.
                std::mem::forget(payload);
                let _ = cleanup(&mut model, &state);
                std::mem::forget(snapshot.clone());
                state.failed.store(true, Ordering::Release);
                ExecutorEvent::CleanupUnconfirmed(error(
                    ErrorCode::ExecutorCleanupUnconfirmed,
                    "MNN owner panic left cleanup unconfirmed",
                ))
            }
        };
        finish(&state, &job.events, terminal);
        if state.failed.load(Ordering::Acquire) {
            break;
        }
    }
    let _ = cleanup(&mut model, &state);
}
fn generate(
    model: &mut Model,
    request: GenerationRequest,
    cancel: &Cancel,
    events: &ExecutionEvents,
    state: &State,
) -> (ExecutorEvent, bool) {
    if request.model.as_str() != mnn_model_store::MODEL_ID {
        return (
            ExecutorEvent::Failed(error(
                ErrorCode::UnsupportedModel,
                "MNN request model mismatch",
            )),
            false,
        );
    }
    if let Err(e) = request.validate() {
        return (ExecutorEvent::Failed(e), false);
    }
    let messages: Vec<_> = request
        .messages
        .iter()
        .map(|m| mnn_adapter::Message {
            role: match m.role {
                runtime_types::Role::System => mnn_adapter::Role::System,
                runtime_types::Role::User => mnn_adapter::Role::User,
                runtime_types::Role::Assistant => mnn_adapter::Role::Assistant,
            },
            content: &m.content,
        })
        .collect();
    let stops: Vec<_> = request.options.stops.iter().map(String::as_str).collect();
    let prepared = match model.prepare(
        &mnn_adapter::Request {
            messages: &messages,
            stops: &stops,
            max_tokens: request.options.max_tokens,
            temperature: request.options.temperature,
            top_p: request.options.top_p,
            seed: if request.options.seed == u32::MAX {
                mnn_adapter::Seed::Random
            } else {
                mnn_adapter::Seed::Fixed(request.options.seed)
            },
        },
        &cancel.native,
        |p| state.progress(p),
    ) {
        Ok(p) => p,
        Err(e) => {
            let fault = !matches!(
                e.kind,
                ErrorKind::Cancelled | ErrorKind::Budget | ErrorKind::Invalid | ErrorKind::Callback
            );
            return (ExecutorEvent::Failed(map_error(e)), fault);
        }
    };
    let prompt = prepared.info().prompt_tokens as u32;
    if !events.emit(ExecutorEvent::Prepared {
        prompt_tokens: prompt,
    }) {
        cancel.cancel();
    }
    match prepared.generate(
        &cancel.native,
        |text| {
            if events.text_delta(text) {
                TextAction::Continue
            } else {
                cancel.cancel();
                TextAction::Cancel
            }
        },
        |p| state.progress(p),
    ) {
        Ok(g) => {
            let usage = Usage {
                prompt_tokens: g.prompt_tokens as u32,
                completion_tokens: g.completion_tokens as u32,
            };
            match g.finish_reason {
                mnn_adapter::FinishReason::Eos | mnn_adapter::FinishReason::Stop => (
                    ExecutorEvent::Completed {
                        usage,
                        finish_reason: runtime_types::FinishReason::Stop,
                    },
                    false,
                ),
                mnn_adapter::FinishReason::Length => (
                    ExecutorEvent::Completed {
                        usage,
                        finish_reason: runtime_types::FinishReason::Length,
                    },
                    false,
                ),
                mnn_adapter::FinishReason::Cancelled => (
                    ExecutorEvent::GenerationFailed {
                        error: error(ErrorCode::RequestCancelled, "MNN generation cancelled"),
                        usage,
                    },
                    false,
                ),
                _ => (
                    ExecutorEvent::GenerationFailed {
                        error: error(ErrorCode::NativeFailure, "MNN generation failed"),
                        usage,
                    },
                    true,
                ),
            }
        }
        Err(f) => generation_failure(f),
    }
}
#[cfg(test)]
mod tests;

fn generation_failure(f: mnn_adapter::GenerationFailure) -> (ExecutorEvent, bool) {
    if f.cleanup_error.is_some() {
        return (
            ExecutorEvent::CleanupUnconfirmed(error(
                ErrorCode::ExecutorCleanupUnconfirmed,
                "MNN prepared cleanup was not confirmed",
            )),
            true,
        );
    }
    let fault = !matches!(
        f.error.kind,
        ErrorKind::Cancelled | ErrorKind::Budget | ErrorKind::Invalid | ErrorKind::Callback
    );
    (
        ExecutorEvent::GenerationFailed {
            error: map_error(f.error),
            usage: Usage {
                prompt_tokens: f.usage.prompt_tokens as u32,
                completion_tokens: f.usage.completion_tokens as u32,
            },
        },
        fault,
    )
}
#[cfg(test)]
mod cleanup_regression {
    use super::*;
    fn failure(cleanup: bool) -> mnn_adapter::GenerationFailure {
        mnn_adapter::GenerationFailure {
            error: mnn_adapter::Error {
                kind: ErrorKind::Cancelled,
            },
            cleanup_error: cleanup.then_some(mnn_adapter::Error {
                kind: ErrorKind::Native,
            }),
            usage: mnn_adapter::Generation {
                prompt_tokens: 2,
                completion_tokens: 1,
                resolved_seed: 0,
                finish_reason: mnn_adapter::FinishReason::Cancelled,
            },
        }
    }
    #[test]
    fn prepared_cleanup_failure_cannot_become_recoverable_cancellation() {
        let (event, fault) = generation_failure(failure(true));
        assert!(fault);
        assert!(matches!(event, ExecutorEvent::CleanupUnconfirmed(_)));
        let (event, fault) = generation_failure(failure(false));
        assert!(!fault);
        assert!(matches!(event, ExecutorEvent::GenerationFailed { .. }));
    }
    #[test]
    fn unconfirmed_resources_are_pinned_and_cleanup_never_clears_failure() {
        struct Resource(Arc<AtomicBool>);
        impl Drop for Resource {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        let dropped = Arc::new(AtomicBool::new(false));
        let state = State::default();
        state.loaded.store(true, Ordering::Release);
        let mut resource = Some(Resource(dropped.clone()));
        pin_unconfirmed(&mut resource, &state);
        assert!(resource.is_none());
        assert!(!dropped.load(Ordering::Acquire));
        assert!(state.failed.load(Ordering::Acquire));
        assert!(cleanup(&mut None, &state).is_err());
        assert!(state.loaded.load(Ordering::Acquire));
    }
}
