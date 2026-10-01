use crate::output::Output;
use runtime_types::{
    FinishReason, GenerationRequest, LoadOptions, ResolvedModel, RuntimeError, Usage,
};
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
/// start must return promptly. A dedicated executor owns all native resources.
/// Exactly one command is active; Loaded/Completed/Failed/Unloaded end it.
/// Faulted reports an unrecoverable executor failure. No callback may panic.
pub trait Executor: Send + 'static {
    fn start(
        &mut self,
        command: ExecutorCommand,
        events: ExecutionEvents,
    ) -> Result<CancellationHandle, RuntimeError>;
}
#[derive(Clone, Debug)]
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
}
#[derive(Clone)]
pub struct ExecutionEvents {
    pub(crate) operation: u64,
    pub(crate) sender: SyncSender<Envelope>,
    pub(crate) output: Option<Arc<Output>>,
}
pub(crate) struct Envelope {
    pub output: Option<Arc<Output>>,
    pub emitted_at: std::time::Instant,
    pub operation: u64,
    pub event: ExecutorEvent,
    pub charge: usize,
}
impl ExecutionEvents {
    /// Text is split at UTF-8 boundaries and held against the request's byte
    /// budget until the client consumes it. Returns false after cancellation,
    /// disconnect, a slow consumer, or scheduler shutdown. Never retry text.
    pub fn emit(&self, event: ExecutorEvent) -> bool {
        self.emit_inner(event)
    }
    pub fn text_delta(&self, text: &str) -> bool {
        self.emit_text(text)
    }
}
