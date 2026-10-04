use crate::output::{Output, TextPermit};
use runtime_types::Usage;
use runtime_types::{FinishReason, GenerationRequest, LoadOptions, ResolvedModel, RuntimeError};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, mpsc::SyncSender};

/// Only this thread-safe control handle crosses native thread ownership boundaries.
#[derive(Clone)]
pub struct CancellationHandle(Arc<dyn Fn() + Send + Sync>);
impl CancellationHandle {
    pub fn new(cancel: impl Fn() + Send + Sync + 'static) -> Self {
        Self(Arc::new(cancel))
    }
    pub fn cancel(&self) {
        (self.0)()
    }
    pub fn noop() -> Self {
        Self::new(|| {})
    }
}
#[derive(Clone, Debug)]
pub enum ExecutorCommand {
    Load {
        model: ResolvedModel,
        options: LoadOptions,
    },
    Generate {
        request: GenerationRequest,
    },
    Unload,
}
/// start must return promptly. Exactly one command is active until its cleanup
/// acknowledgment. Faulted means an unrecoverable executor has stopped safely.
pub trait Executor: Send + 'static {
    fn start(
        &mut self,
        command: ExecutorCommand,
        events: ExecutionEvents,
    ) -> Result<CancellationHandle, RuntimeError>;
    /// Called only after active operations/resources have acknowledged cleanup.
    /// Process implementations must wait for and reap their child before success.
    fn close(&mut self) -> Result<(), RuntimeError> {
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "event",
    content = "data",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ExecutorEvent {
    Loaded,
    Prepared {
        prompt_tokens: u32,
    },
    TextDelta(String),
    Completed {
        usage: Usage,
        finish_reason: FinishReason,
    },
    Failed(RuntimeError),
    GenerationFailed {
        error: RuntimeError,
        usage: Usage,
    },
    Unloaded,
    Faulted(RuntimeError),
    /// Parent-local emergency: child/resource exit could NOT be confirmed.
    /// This irrevocably disables recovery and is never a worker wire event.
    #[serde(skip)]
    CleanupUnconfirmed(RuntimeError),
}
/// A transport sink owns its own bounded/cancellable emission. A worker uses it
/// directly with EngineHost; it must not start another scheduler or text ledger.
pub trait ExecutionEventSink: Send + Sync + 'static {
    fn emit(&self, event: ExecutorEvent) -> bool;
}
#[derive(Clone)]
pub struct ExecutionEvents {
    pub(crate) operation: u64,
    pub(crate) destination: EventDestination,
}
#[derive(Clone)]
pub(crate) enum EventDestination {
    Actor {
        sender: SyncSender<Envelope>,
        output: Option<Arc<Output>>,
    },
    Sink(Arc<dyn ExecutionEventSink>),
}
pub(crate) struct Envelope {
    pub emitted_at: std::time::Instant,
    pub operation: u64,
    pub event: ExecutorEvent,
    pub permit: Option<TextPermit>,
}
impl ExecutionEvents {
    pub fn from_sink(operation_id: u64, sink: Arc<dyn ExecutionEventSink>) -> Self {
        Self {
            operation: operation_id,
            destination: EventDestination::Sink(sink),
        }
    }
    pub fn operation_id(&self) -> u64 {
        self.operation
    }
    pub(crate) fn for_actor(
        operation: u64,
        sender: SyncSender<Envelope>,
        output: Option<Arc<Output>>,
    ) -> Self {
        Self {
            operation,
            destination: EventDestination::Actor { sender, output },
        }
    }
    /// Text is split on UTF-8 boundaries and held until consumption/drop.
    /// False means cancellation, disconnect, slow consumer, or shutdown. No replay.
    pub fn emit(&self, event: ExecutorEvent) -> bool {
        self.emit_inner(event)
    }
    pub fn text_delta(&self, text: &str) -> bool {
        self.emit_text(text)
    }
}
