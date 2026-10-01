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

#[test]
fn shutdown_calls_executor_close_and_surfaces_reaping_failure() {
    use std::sync::atomic::{AtomicBool, Ordering};
    struct CloseFailure(Arc<AtomicBool>);
    impl Executor for CloseFailure {
        fn start(
            &mut self,
            _: ExecutorCommand,
            _: ExecutionEvents,
        ) -> Result<CancellationHandle, RuntimeError> {
            unreachable!()
        }
        fn close(&mut self) -> Result<(), RuntimeError> {
            self.0.store(true, Ordering::SeqCst);
            Err(RuntimeError::new(
                ErrorCode::ExecutorUnavailable,
                "synthetic reap failure",
            ))
        }
    }
    let closed = Arc::new(AtomicBool::new(false));
    let runtime = Runtime::spawn(
        RuntimeConfig::default(),
        |_: &ModelId| Err(RuntimeError::invalid("unused")),
        CloseFailure(closed.clone()),
    )
    .unwrap();
    let error = runtime.shutdown().unwrap_err();
    assert_eq!(error.code, ErrorCode::ExecutorUnavailable);
    assert!(closed.load(Ordering::SeqCst));
}

#[test]
fn faulted_recovery_uses_internal_unload_while_public_unload_stays_rejected() {
    let h = Harness::new(config(), true);
    let receiver = h.handle.submit(request()).unwrap();
    let pending = h.pending();
    pending
        .events
        .emit(ExecutorEvent::Faulted(RuntimeError::new(
            ErrorCode::ExecutorUnavailable,
            "already reaped",
        )));
    assert!(matches!(
        terminal(&receiver),
        RequestEventKind::Failed { .. }
    ));
    assert_eq!(
        h.handle.unload().unwrap_err().code,
        ErrorCode::RuntimeFaulted
    );
    assert_eq!(h.handle.status().unwrap().state, ModelState::Faulted);
    assert_eq!(
        h.handle.submit(request()).err().unwrap().code,
        ErrorCode::RuntimeFaulted
    );
    assert!(h.commands.try_recv().is_err());
    h.handle
        .load(ModelId::new("qa-small").unwrap(), LoadOptions::default())
        .unwrap();
    assert_eq!(h.handle.status().unwrap().state, ModelState::Ready);
    h.finish();
}

#[test]
fn cancel_before_prepared_preserves_usage_without_public_started_or_text() {
    let h = Harness::new(config(), true);
    let req = request();
    let receiver = h.handle.submit(req.clone()).unwrap();
    let pending = h.pending();
    h.handle.cancel(req.request_id).unwrap();
    pending.prepared();
    assert!(!pending.events.text_delta("too late"));
    pending.fail(ErrorCode::RequestCancelled);
    let delivered = events(&receiver);
    assert!(delivered.iter().all(|event| !matches!(
        event.kind,
        RequestEventKind::Started { .. } | RequestEventKind::TextDelta(_)
    )));
    assert_eq!(
        delivered
            .iter()
            .filter(|event| event.kind.is_terminal())
            .count(),
        1
    );
    assert!(matches!(
        &delivered.last().unwrap().kind,
        RequestEventKind::Cancelled {
            reason: ErrorCode::RequestCancelled,
            usage: Usage {
                prompt_tokens: 8,
                completion_tokens: 0
            },
            ..
        }
    ));
    assert_eq!(h.handle.status().unwrap().state, ModelState::Ready);
    h.finish();
}

#[test]
fn unconfirmed_cleanup_overrides_cancellation_and_permanently_disables_recovery() {
    let mut h = Harness::new(config(), true);
    let req = request();
    let current = h.handle.submit(req.clone()).unwrap();
    let pending = h.pending();
    let queued = h.handle.submit(request()).unwrap();
    h.handle.cancel(req.request_id).unwrap();
    pending
        .events
        .emit(ExecutorEvent::CleanupUnconfirmed(RuntimeError::new(
            ErrorCode::ExecutorCleanupUnconfirmed,
            "synthetic OS wait failure",
        )));
    for receiver in [&current, &queued] {
        let delivered = events(receiver);
        assert_eq!(
            delivered
                .iter()
                .filter(|event| event.kind.is_terminal())
                .count(),
            1
        );
        assert!(
            matches!(&delivered.last().unwrap().kind, RequestEventKind::Failed { error, .. } if error.code == ErrorCode::ExecutorCleanupUnconfirmed)
        );
    }
    let status = h.handle.status().unwrap();
    assert_eq!(status.state, ModelState::Faulted);
    assert!(status.active_request.is_none());
    assert_eq!(status.queued_jobs, 0);
    assert!(
        status
            .last_error
            .unwrap()
            .message
            .contains("request_cancelled")
    );
    // A late normal terminal/fault must neither recover nor hide the stronger error.
    pending.complete();
    pending
        .events
        .emit(ExecutorEvent::Faulted(RuntimeError::new(
            ErrorCode::ExecutorUnavailable,
            "late fault",
        )));
    assert_eq!(
        h.handle
            .load(ModelId::new("qa-small").unwrap(), LoadOptions::default())
            .unwrap_err()
            .code,
        ErrorCode::ExecutorCleanupUnconfirmed
    );
    assert!(h.commands.try_recv().is_err());
    assert_eq!(
        h.handle.status().unwrap().last_error.unwrap().code,
        ErrorCode::ExecutorCleanupUnconfirmed
    );
    assert_eq!(
        h.runtime.take().unwrap().shutdown().unwrap_err().code,
        ErrorCode::ExecutorCleanupUnconfirmed
    );
}

#[test]
fn unconfirmed_cleanup_unblocks_pending_load_and_shutdown_with_error() {
    let mut h = Harness::new(config(), false);
    let handle = h.handle.clone();
    let loader = thread::spawn(move || {
        handle.load(ModelId::new("qa-small").unwrap(), LoadOptions::default())
    });
    h.pending()
        .events
        .emit(ExecutorEvent::CleanupUnconfirmed(RuntimeError::new(
            ErrorCode::ExecutorCleanupUnconfirmed,
            "synthetic load cleanup",
        )));
    assert_eq!(
        loader.join().unwrap().unwrap_err().code,
        ErrorCode::ExecutorCleanupUnconfirmed
    );
    assert_eq!(
        h.runtime.take().unwrap().shutdown().unwrap_err().code,
        ErrorCode::ExecutorCleanupUnconfirmed
    );
}

#[test]
fn load_deadline_reason_survives_transport_force_kill_ack() {
    let mut cfg = config();
    cfg.load_timeout = Duration::from_millis(20);
    let h = Harness::new(cfg, false);
    let current = h.handle.submit(request()).unwrap();
    let pending = h.pending();
    let queued = h.handle.submit(request()).unwrap();
    wait(|| pending.cancelled.load(Ordering::SeqCst));
    pending
        .events
        .emit(ExecutorEvent::Faulted(RuntimeError::new(
            ErrorCode::RequestCancelled,
            "worker reaped after cancellation grace",
        )));
    for receiver in [&current, &queued] {
        assert!(
            matches!(terminal(receiver), RequestEventKind::Failed { error, .. } if error.code == ErrorCode::LoadTimeout)
        );
    }
    assert_eq!(
        h.handle.status().unwrap().last_error.unwrap().code,
        ErrorCode::LoadTimeout
    );
    h.finish();
}

#[test]
fn every_actual_load_rechecks_model_identity_validation_and_context_without_native_dispatch() {
    use std::sync::atomic::AtomicUsize;
    struct Recording(Arc<Mutex<Vec<(ResolvedModel, LoadOptions)>>>);
    impl Executor for Recording {
        fn start(
            &mut self,
            command: ExecutorCommand,
            events: ExecutionEvents,
        ) -> Result<CancellationHandle, RuntimeError> {
            match command {
                ExecutorCommand::Load { model, options } => {
                    self.0.lock().unwrap().push((model, options));
                    events.emit(ExecutorEvent::Loaded);
                }
                ExecutorCommand::Unload => {
                    events.emit(ExecutorEvent::Unloaded);
                }
                ExecutorCommand::Generate { .. } => {
                    panic!("unverified reload must never reach generation")
                }
            }
            Ok(CancellationHandle::noop())
        }
    }
    // Modes simulate the model-store resolver rejecting a replaced/unverified
    // file, changed identity/context, or a changed metadata fingerprint.
    for idle_unload in [false, true] {
        for (mode, expected) in [
            (1, ErrorCode::UnsupportedModel),
            (2, ErrorCode::UnsupportedModel),
            (3, ErrorCode::ContextLengthExceeded),
            (4, ErrorCode::IntegrityFailure),
        ] {
            let identity = Arc::new(AtomicUsize::new(0));
            let source = identity.clone();
            let loads = Arc::new(Mutex::new(Vec::new()));
            let mut cfg = config();
            if idle_unload {
                cfg.idle_unload = Duration::from_millis(5);
            }
            let runtime = Runtime::spawn(
                cfg,
                move |id: &ModelId| {
                    let mode = source.load(Ordering::SeqCst);
                    if mode == 4 {
                        return Err(RuntimeError::new(
                            ErrorCode::IntegrityFailure,
                            "metadata fingerprint changed; verify outside actor",
                        ));
                    }
                    Ok(ResolvedModel {
                        id: if mode == 2 {
                            ModelId::new("other").unwrap()
                        } else {
                            id.clone()
                        },
                        path: if mode == 5 {
                            "reverified-replacement.gguf".into()
                        } else {
                            "original.gguf".into()
                        },
                        context_limit: if mode == 3 { 1024 } else { 4096 },
                        default_context: 2048,
                        validated: mode != 1,
                    })
                },
                Recording(loads.clone()),
            )
            .unwrap();
            let handle = runtime.handle();
            let id = ModelId::new("qa-small").unwrap();
            let options = LoadOptions::default();
            handle.load(id.clone(), options).unwrap();
            if idle_unload {
                wait(|| handle.status().unwrap().state == ModelState::Unloaded);
            } else {
                handle.unload().unwrap();
            }
            identity.store(mode, Ordering::SeqCst);
            let events = handle.submit(request()).unwrap();
            assert!(
                matches!(terminal(&events), RequestEventKind::Failed { error, .. } if error.code == expected)
            );
            let status = handle.status().unwrap();
            assert_eq!(status.state, ModelState::Faulted);
            assert_eq!(status.selected_model, Some(id.clone()));
            assert_eq!(status.load_options, Some(options));
            assert_eq!(
                loads.lock().unwrap().len(),
                1,
                "no native load of changed model"
            );
            assert!(handle.load(id.clone(), options).is_err());
            assert_eq!(loads.lock().unwrap().len(), 1);
            // Explicit outside-actor reverification is represented by mode 5.
            identity.store(5, Ordering::SeqCst);
            handle.load(id, options).unwrap();
            let recorded = loads.lock().unwrap();
            assert_eq!(recorded.len(), 2);
            assert_eq!(
                recorded[1].0.path,
                PathBuf::from("reverified-replacement.gguf")
            );
            assert_eq!(recorded[1].1, options, "context is never silently reduced");
            drop(recorded);
            runtime.shutdown().unwrap();
        }
    }
}

#[test]
fn registry_reservation_is_atomic_and_does_not_block_control() {
    let h = Harness::new(config(), true);
    let lease = h.handle.reserve_registry().unwrap();
    assert!(h.handle.status().unwrap().registry_busy);
    assert_eq!(
        h.handle.reserve_registry().err().unwrap().code,
        ErrorCode::RuntimeBusy
    );
    assert_eq!(
        h.handle.submit(request()).err().unwrap().code,
        ErrorCode::RuntimeBusy
    );
    assert_eq!(
        h.handle
            .load(ModelId::new("other").unwrap(), LoadOptions::default())
            .unwrap_err()
            .code,
        ErrorCode::RuntimeBusy
    );
    assert_eq!(h.handle.unload().unwrap_err().code, ErrorCode::RuntimeBusy);
    assert_eq!(
        h.handle.cancel(RequestId::new()).unwrap_err().code,
        ErrorCode::RequestNotFound
    );
    assert!(!lease.cancellation_requested());
    drop(lease);
    assert!(!h.handle.status().unwrap().registry_busy);
    h.finish();
}

#[test]
fn shutdown_requests_import_cancel_and_waits_for_actual_lease_cleanup() {
    let mut h = Harness::new(config(), true);
    let lease = h.handle.reserve_registry().unwrap();
    let runtime = h.runtime.take().unwrap();
    let (tx, rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        tx.send(runtime.shutdown()).unwrap();
    });
    let until = Instant::now() + Duration::from_secs(2);
    while !lease.cancellation_requested() {
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(2));
    }
    assert!(h.handle.status().unwrap().stopping);
    assert!(rx.recv_timeout(Duration::from_millis(50)).is_err());
    drop(lease);
    rx.recv_timeout(Duration::from_secs(2)).unwrap().unwrap();
    worker.join().unwrap();
}

#[test]
fn idle_unload_cannot_cross_registry_reservation() {
    let mut cfg = config();
    cfg.idle_unload = Duration::from_millis(40);
    let h = Harness::new(cfg, true);
    h.handle
        .load(ModelId::new("qa-small").unwrap(), LoadOptions::default())
        .unwrap();
    let lease = h.handle.reserve_registry().unwrap();
    thread::sleep(Duration::from_millis(80));
    assert_eq!(h.handle.status().unwrap().state, ModelState::Ready);
    drop(lease);
    let until = Instant::now() + Duration::from_secs(2);
    while h.handle.status().unwrap().state != ModelState::Unloaded {
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(2));
    }
    h.finish();
}
