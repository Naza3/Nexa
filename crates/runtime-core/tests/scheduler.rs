//! Deterministic executor-controlled scheduling tests. These do not claim model inference.
use runtime_core::*;
use runtime_types::*;
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

struct Pending {
    command: ExecutorCommand,
    events: ExecutionEvents,
    cancelled: Arc<AtomicBool>,
}
impl Pending {
    fn prepared(&self) {
        assert!(
            self.events
                .emit(ExecutorEvent::Prepared { prompt_tokens: 8 })
        );
    }
    fn complete(&self) {
        assert!(self.events.emit(ExecutorEvent::Completed {
            usage: Usage {
                prompt_tokens: 8,
                completion_tokens: 1
            },
            finish_reason: FinishReason::Stop
        }));
    }
    fn fail(&self, code: ErrorCode) {
        assert!(self.events.emit(ExecutorEvent::GenerationFailed {
            error: RuntimeError::new(code, "controlled failure"),
            usage: Usage {
                prompt_tokens: 8,
                completion_tokens: 0
            }
        }));
    }
    fn id(&self) -> RequestId {
        match &self.command {
            ExecutorCommand::Generate { request } => request.request_id,
            _ => panic!("not generation"),
        }
    }
}
struct Controlled {
    sender: mpsc::Sender<Pending>,
    auto_load: bool,
    current: Arc<Mutex<Option<ExecutionEvents>>>,
}
impl Executor for Controlled {
    fn start(
        &mut self,
        command: ExecutorCommand,
        events: ExecutionEvents,
    ) -> Result<CancellationHandle, RuntimeError> {
        *self.current.lock().unwrap() = Some(events.clone());
        if matches!(command, ExecutorCommand::Unload) {
            events.emit(ExecutorEvent::Unloaded);
            return Ok(CancellationHandle::noop());
        }
        if self.auto_load && matches!(command, ExecutorCommand::Load { .. }) {
            events.emit(ExecutorEvent::Loaded);
            return Ok(CancellationHandle::noop());
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = cancelled.clone();
        self.sender
            .send(Pending {
                command,
                events,
                cancelled,
            })
            .unwrap();
        Ok(CancellationHandle::new(move || {
            flag.store(true, Ordering::SeqCst);
        }))
    }
}
struct Harness {
    runtime: Option<Runtime>,
    handle: RuntimeHandle,
    commands: mpsc::Receiver<Pending>,
    current: Arc<Mutex<Option<ExecutionEvents>>>,
}
impl Harness {
    fn new(config: RuntimeConfig, auto_load: bool) -> Self {
        let (sender, commands) = mpsc::channel();
        let current = Arc::new(Mutex::new(None));
        let resolver = |id: &ModelId| {
            Ok(ResolvedModel {
                id: id.clone(),
                path: PathBuf::from("controlled.gguf"),
                context_limit: 4096,
                default_context: 4096,
                validated: true,
            })
        };
        let runtime = Runtime::spawn(
            config,
            resolver,
            Controlled {
                sender,
                auto_load,
                current: current.clone(),
            },
        )
        .unwrap();
        let handle = runtime.handle();
        Self {
            runtime: Some(runtime),
            handle,
            commands,
            current,
        }
    }
    fn pending(&self) -> Pending {
        self.commands
            .recv_timeout(Duration::from_secs(2))
            .expect("executor operation")
    }
    fn finish(mut self) {
        self.runtime.take().unwrap().shutdown().unwrap();
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        if self.runtime.is_some()
            && let Some(events) = self.current.lock().unwrap().take()
        {
            events.emit(ExecutorEvent::Faulted(RuntimeError::new(
                ErrorCode::ExecutorUnavailable,
                "test teardown",
            )));
        }
    }
}
fn config() -> RuntimeConfig {
    RuntimeConfig {
        idle_unload: Duration::from_secs(60),
        ..RuntimeConfig::default()
    }
}
fn request() -> GenerationRequest {
    GenerationRequest {
        request_id: RequestId::new(),
        model: ModelId::new("qa-small").unwrap(),
        messages: vec![Message::new(Role::User, "synthetic")],
        options: GenerationOptions::default(),
    }
}
fn events(receiver: &EventReceiver) -> Vec<RequestEvent> {
    let mut result = Vec::new();
    loop {
        let event = receiver.recv_timeout(Duration::from_secs(3)).unwrap();
        let terminal = event.kind.is_terminal();
        result.push(event);
        if terminal {
            break;
        }
    }
    for (i, event) in result.iter().enumerate() {
        assert_eq!(event.seq, i as u64 + 1);
    }
    assert_eq!(result.iter().filter(|e| e.kind.is_terminal()).count(), 1);
    assert!(receiver.recv().is_none());
    result
}
fn wait(mut condition: impl FnMut() -> bool) {
    let start = Instant::now();
    while !condition() {
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "condition timed out"
        );
        thread::sleep(Duration::from_millis(2));
    }
}
fn terminal(receiver: &EventReceiver) -> RequestEventKind {
    events(receiver).pop().unwrap().kind
}

#[test]
fn a06_one_active_eight_waiting_fifo_and_duplicate_ids() {
    let h = Harness::new(config(), true);
    let mut submitted = Vec::new();
    for _ in 0..9 {
        let r = request();
        let id = r.request_id;
        let receiver = h.handle.submit(r.clone()).unwrap();
        assert_eq!(
            h.handle.submit(r).err().unwrap().code,
            ErrorCode::DuplicateRequestId
        );
        submitted.push((id, receiver));
    }
    assert_eq!(
        h.handle.submit(request()).err().unwrap().code,
        ErrorCode::QueueFull
    );
    assert_eq!(h.handle.status().unwrap().queued_jobs, 8);
    for (id, receiver) in submitted {
        let pending = h.pending();
        assert_eq!(pending.id(), id);
        pending.prepared();
        pending.complete();
        assert!(matches!(
            terminal(&receiver),
            RequestEventKind::Completed { .. }
        ));
    }
    h.finish();
}
#[test]
fn android_one_waiting_and_zero_waiting_config() {
    for capacity in [0, 1] {
        let mut c = RuntimeConfig::android();
        c.max_queued_jobs = capacity;
        let h = Harness::new(c, true);
        let first = h.handle.submit(request()).unwrap();
        let pending = h.pending();
        let queued = if capacity == 1 {
            Some(h.handle.submit(request()).unwrap())
        } else {
            None
        };
        assert_eq!(
            h.handle.submit(request()).err().unwrap().code,
            ErrorCode::QueueFull
        );
        pending.prepared();
        pending.complete();
        terminal(&first);
        if let Some(receiver) = queued {
            let pending = h.pending();
            pending.prepared();
            pending.complete();
            terminal(&receiver);
        }
        h.finish();
    }
}
#[test]
fn a07_cancel_queued_never_executes_and_does_not_cancel_active() {
    let h = Harness::new(config(), true);
    let first = h.handle.submit(request()).unwrap();
    let pending = h.pending();
    let q = request();
    let receiver = h.handle.submit(q.clone()).unwrap();
    h.handle.cancel(q.request_id).unwrap();
    assert!(matches!(
        terminal(&receiver),
        RequestEventKind::Cancelled {
            reason: ErrorCode::RequestCancelled,
            ..
        }
    ));
    assert!(!pending.cancelled.load(Ordering::SeqCst));
    assert_eq!(h.handle.status().unwrap().queued_jobs, 0);
    pending.prepared();
    pending.complete();
    terminal(&first);
    h.finish();
}
#[test]
fn a08_cancel_keeps_native_slot_until_cleanup_ack() {
    let h = Harness::new(config(), true);
    let r = request();
    let receiver = h.handle.submit(r.clone()).unwrap();
    let pending = h.pending();
    pending.prepared();
    let next = h.handle.submit(request()).unwrap();
    h.handle.cancel(r.request_id).unwrap();
    assert!(pending.cancelled.load(Ordering::SeqCst));
    assert_eq!(
        h.handle.status().unwrap().active_request,
        Some(r.request_id)
    );
    assert!(h.commands.try_recv().is_err());
    pending.fail(ErrorCode::RequestCancelled);
    assert!(matches!(
        terminal(&receiver),
        RequestEventKind::Cancelled {
            usage: Usage {
                prompt_tokens: 8,
                ..
            },
            ..
        }
    ));
    let pending = h.pending();
    pending.prepared();
    pending.complete();
    terminal(&next);
    h.finish();
}
#[test]
fn a05_prepare_rejection_does_not_start_and_next_request_recovers() {
    let h = Harness::new(config(), true);
    let receiver = h.handle.submit(request()).unwrap();
    h.pending().fail(ErrorCode::ContextLengthExceeded);
    let observed = events(&receiver);
    assert!(
        !observed
            .iter()
            .any(|e| matches!(e.kind, RequestEventKind::Started { .. }))
    );
    assert!(matches!(
        observed.last().unwrap().kind,
        RequestEventKind::Failed {
            error: RuntimeError {
                code: ErrorCode::ContextLengthExceeded,
                ..
            },
            ..
        }
    ));
    let receiver = h.handle.submit(request()).unwrap();
    let pending = h.pending();
    pending.prepared();
    pending.complete();
    terminal(&receiver);
    h.finish();
}
#[test]
fn a09_disconnected_client_cancels_without_delivery_or_slot_reuse() {
    let h = Harness::new(config(), true);
    let receiver = h.handle.submit(request()).unwrap();
    let pending = h.pending();
    pending.prepared();
    drop(receiver);
    wait(|| pending.cancelled.load(Ordering::SeqCst));
    assert_eq!(h.handle.status().unwrap().state, ModelState::Generating);
    pending.fail(ErrorCode::RequestCancelled);
    wait(|| h.handle.status().unwrap().state == ModelState::Ready);
    h.finish();
}
#[test]
fn a09_backpressure_is_bounded_utf8_and_slow_consumer_recovers() {
    let mut c = config();
    c.slow_consumer_timeout = Duration::from_millis(40);
    let h = Harness::new(c, true);
    let receiver = h.handle.submit(request()).unwrap();
    let pending = h.pending();
    pending.prepared();
    let sink = pending.events.clone();
    let writer = thread::spawn(move || {
        let chunk = "🙂".repeat(1024);
        for _ in 0..1000 {
            if !sink.text_delta(&chunk) {
                return;
            }
        }
        panic!("unbounded writer");
    });
    wait(|| pending.cancelled.load(Ordering::SeqCst));
    assert!(receiver.buffered_bytes() <= MAX_BUFFERED_TEXT_BYTES);
    writer.join().unwrap();
    pending.fail(ErrorCode::ConsumerStopped);
    let observed = events(&receiver);
    assert!(observed.iter().all(|e| match &e.kind {
        RequestEventKind::TextDelta(text) =>
            text.len() <= MAX_DELTA_BYTES && text.chars().all(|c| c == '🙂'),
        _ => true,
    }));
    assert!(matches!(
        observed.last().unwrap().kind,
        RequestEventKind::Cancelled {
            reason: ErrorCode::SlowConsumer,
            ..
        }
    ));
    let receiver = h.handle.submit(request()).unwrap();
    let pending = h.pending();
    pending.prepared();
    pending.complete();
    terminal(&receiver);
    h.finish();
}
#[test]
fn a09_cancel_wakes_blocked_output_without_waiting_ten_seconds() {
    let h = Harness::new(config(), true);
    let r = request();
    let receiver = h.handle.submit(r.clone()).unwrap();
    let pending = h.pending();
    pending.prepared();
    let sink = pending.events.clone();
    let writer = thread::spawn(move || sink.text_delta(&"x".repeat(MAX_BUFFERED_TEXT_BYTES * 2)));
    wait(|| receiver.buffered_bytes() == MAX_BUFFERED_TEXT_BYTES);
    let before = Instant::now();
    h.handle.cancel(r.request_id).unwrap();
    assert!(!writer.join().unwrap());
    assert!(before.elapsed() < Duration::from_secs(1));
    pending.fail(ErrorCode::RequestCancelled);
    terminal(&receiver);
    h.finish();
}
#[test]
fn a10_busy_switch_unload_and_model_conflict() {
    let h = Harness::new(config(), true);
    let receiver = h.handle.submit(request()).unwrap();
    let pending = h.pending();
    let other = ModelId::new("other").unwrap();
    assert_eq!(
        h.handle
            .load(other.clone(), LoadOptions::default())
            .unwrap_err()
            .code,
        ErrorCode::RuntimeBusy
    );
    assert_eq!(h.handle.unload().unwrap_err().code, ErrorCode::RuntimeBusy);
    let mut r = request();
    r.model = other;
    assert_eq!(
        h.handle.submit(r).err().unwrap().code,
        ErrorCode::ModelConflict
    );
    pending.prepared();
    pending.complete();
    terminal(&receiver);
    h.handle.unload().unwrap();
    assert_eq!(
        h.handle.status().unwrap().selected_model,
        Some(ModelId::new("qa-small").unwrap())
    );
    h.finish();
}
#[test]
fn a11_idle_unload_retains_selection_and_next_request_reloads() {
    let mut c = config();
    c.idle_unload = Duration::from_millis(20);
    let h = Harness::new(c, true);
    let receiver = h.handle.submit(request()).unwrap();
    let pending = h.pending();
    pending.prepared();
    pending.complete();
    terminal(&receiver);
    wait(|| h.handle.status().unwrap().state == ModelState::Unloaded);
    let receiver = h.handle.submit(request()).unwrap();
    let pending = h.pending();
    pending.prepared();
    pending.complete();
    let observed = events(&receiver);
    assert!(
        observed
            .iter()
            .any(|e| matches!(e.kind, RequestEventKind::Loading))
    );
    h.finish();
}
#[test]
fn a12_queue_timeout_is_independent_of_active_execution() {
    let mut c = config();
    c.queue_timeout = Duration::from_millis(20);
    let h = Harness::new(c, true);
    let receiver = h.handle.submit(request()).unwrap();
    let pending = h.pending();
    let queued = h.handle.submit(request()).unwrap();
    assert!(matches!(
        terminal(&queued),
        RequestEventKind::Failed {
            error: RuntimeError {
                code: ErrorCode::QueueTimeout,
                ..
            },
            ..
        }
    ));
    assert!(!pending.cancelled.load(Ordering::SeqCst));
    pending.prepared();
    pending.complete();
    terminal(&receiver);
    h.finish();
}
#[test]
fn a12_execution_timeout_waits_cleanup_then_fails_once() {
    let mut c = config();
    c.execution_timeout = Duration::from_millis(20);
    let h = Harness::new(c, true);
    let receiver = h.handle.submit(request()).unwrap();
    let pending = h.pending();
    wait(|| pending.cancelled.load(Ordering::SeqCst));
    assert_eq!(h.handle.status().unwrap().state, ModelState::Generating);
    pending.fail(ErrorCode::RequestCancelled);
    assert!(matches!(
        terminal(&receiver),
        RequestEventKind::Failed {
            error: RuntimeError {
                code: ErrorCode::ExecutionTimeout,
                ..
            },
            ..
        }
    ));
    h.finish();
}
#[test]
fn a12_load_timeout_fails_batch_and_explicit_load_recovers() {
    let mut c = config();
    c.load_timeout = Duration::from_millis(20);
    let h = Harness::new(c, false);
    let receiver = h.handle.submit(request()).unwrap();
    let pending = h.pending();
    let queued = h.handle.submit(request()).unwrap();
    wait(|| pending.cancelled.load(Ordering::SeqCst));
    pending.events.emit(ExecutorEvent::Failed(RuntimeError::new(
        ErrorCode::RequestCancelled,
        "cancelled load",
    )));
    for receiver in [&receiver, &queued] {
        assert!(matches!(
            terminal(receiver),
            RequestEventKind::Failed {
                error: RuntimeError {
                    code: ErrorCode::LoadTimeout,
                    ..
                },
                ..
            }
        ));
    }
    assert_eq!(h.handle.status().unwrap().state, ModelState::Faulted);
    assert_eq!(
        h.handle.submit(request()).err().unwrap().code,
        ErrorCode::RuntimeFaulted
    );
    let handle = h.handle.clone();
    let loader = thread::spawn(move || {
        handle.load(ModelId::new("qa-small").unwrap(), LoadOptions::default())
    });
    h.pending().events.emit(ExecutorEvent::Loaded);
    loader.join().unwrap().unwrap();
    h.finish();
}
#[test]
fn cancelling_one_loading_demand_preserves_others() {
    let h = Harness::new(config(), false);
    let r = request();
    let receiver = h.handle.submit(r.clone()).unwrap();
    let loading = h.pending();
    let queued = h.handle.submit(request()).unwrap();
    h.handle.cancel(r.request_id).unwrap();
    terminal(&receiver);
    assert!(!loading.cancelled.load(Ordering::SeqCst));
    loading.events.emit(ExecutorEvent::Loaded);
    let pending = h.pending();
    pending.prepared();
    pending.complete();
    terminal(&queued);
    h.finish();
}
#[test]
fn cancelling_last_load_demand_and_new_arrival_safely_reloads() {
    let h = Harness::new(config(), false);
    let r = request();
    let receiver = h.handle.submit(r.clone()).unwrap();
    let loading = h.pending();
    h.handle.cancel(r.request_id).unwrap();
    terminal(&receiver);
    assert!(loading.cancelled.load(Ordering::SeqCst));
    let next = h.handle.submit(request()).unwrap();
    loading.events.emit(ExecutorEvent::Failed(RuntimeError::new(
        ErrorCode::RequestCancelled,
        "aborted load",
    )));
    let loading = h.pending();
    assert!(matches!(loading.command, ExecutorCommand::Load { .. }));
    loading.events.emit(ExecutorEvent::Loaded);
    let pending = h.pending();
    pending.prepared();
    pending.complete();
    terminal(&next);
    h.finish();
}
#[test]
fn executor_fault_terminates_active_and_queue_and_never_replays() {
    let h = Harness::new(config(), true);
    let receiver = h.handle.submit(request()).unwrap();
    let pending = h.pending();
    pending.prepared();
    pending.events.text_delta("partial");
    let queued = h.handle.submit(request()).unwrap();
    pending
        .events
        .emit(ExecutorEvent::Faulted(RuntimeError::new(
            ErrorCode::ExecutorUnavailable,
            "simulated transport loss",
        )));
    for receiver in [&receiver, &queued] {
        assert!(matches!(
            terminal(receiver),
            RequestEventKind::Failed {
                error: RuntimeError {
                    code: ErrorCode::ExecutorUnavailable,
                    ..
                },
                ..
            }
        ));
    }
    assert_eq!(h.handle.status().unwrap().state, ModelState::Faulted);
    assert!(h.commands.try_recv().is_err());
    h.finish();
}

#[test]
fn malformed_progress_does_not_reuse_slot_until_terminal_cleanup() {
    let h = Harness::new(config(), true);
    let receiver = h.handle.submit(request()).unwrap();
    let pending = h.pending();
    pending.prepared();
    pending.prepared();
    wait(|| pending.cancelled.load(Ordering::SeqCst));
    assert_eq!(h.handle.status().unwrap().state, ModelState::Faulted);
    assert_eq!(
        h.handle
            .load(ModelId::new("qa-small").unwrap(), LoadOptions::default())
            .unwrap_err()
            .code,
        ErrorCode::RuntimeBusy
    );
    pending.fail(ErrorCode::RequestCancelled);
    assert!(matches!(
        terminal(&receiver),
        RequestEventKind::Failed {
            error: RuntimeError {
                code: ErrorCode::NativeProtocol,
                ..
            },
            ..
        }
    ));
    h.finish();
}
#[test]
fn late_load_success_after_timeout_is_unloaded_before_explicit_recovery() {
    let mut c = config();
    c.load_timeout = Duration::from_millis(20);
    let h = Harness::new(c, false);
    let receiver = h.handle.submit(request()).unwrap();
    let pending = h.pending();
    wait(|| pending.cancelled.load(Ordering::SeqCst));
    pending.events.emit(ExecutorEvent::Loaded);
    assert!(matches!(
        terminal(&receiver),
        RequestEventKind::Failed {
            error: RuntimeError {
                code: ErrorCode::LoadTimeout,
                ..
            },
            ..
        }
    ));
    wait(|| h.handle.status().unwrap().state == ModelState::Faulted);
    assert!(h.commands.try_recv().is_err());
    h.finish();
}
#[test]
fn explicit_load_is_idempotent_and_model_switch_requires_idle() {
    let h = Harness::new(config(), true);
    let id = ModelId::new("qa-small").unwrap();
    h.handle.load(id.clone(), LoadOptions::default()).unwrap();
    h.handle.load(id, LoadOptions::default()).unwrap();
    h.handle
        .load(ModelId::new("other").unwrap(), LoadOptions::default())
        .unwrap();
    assert_eq!(
        h.handle.status().unwrap().selected_model.unwrap().as_str(),
        "other"
    );
    h.finish();
}
