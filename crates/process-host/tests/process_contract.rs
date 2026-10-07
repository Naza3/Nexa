use process_host::{ProcessDiagnostics, ProcessHost, ProcessHostConfig};
use runtime_core::{
    EventReceiver, ExecutionEventSink, ExecutionEvents, Executor, ExecutorCommand, ExecutorEvent,
    Runtime, RuntimeHandle,
};
use runtime_types::*;
use std::{
    path::PathBuf,
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant},
};
fn fixture() -> &'static str {
    env!("CARGO_BIN_EXE_nexa-fault-worker")
}
fn options() -> LoadOptions {
    LoadOptions {
        context_size: 2048,
        threads: 2,
        batch_size: 128,
    }
}
fn model() -> ModelId {
    ModelId::new("fixture").unwrap()
}
fn request() -> GenerationRequest {
    GenerationRequest {
        request_id: RequestId::new(),
        model: model(),
        messages: vec![Message::new(Role::User, "fixture input")],
        options: GenerationOptions {
            max_tokens: 64,
            ..Default::default()
        },
    }
}
fn resolved() -> ResolvedModel {
    ResolvedModel {
        id: model(),
        path: PathBuf::from("fixture.gguf"),
        projector_path: None,
        context_limit: 2048,
        default_context: 2048,
        loadable: true,
    }
}
struct Setup {
    runtime: Runtime,
    handle: RuntimeHandle,
    diagnostics: ProcessDiagnostics,
    _directory: tempfile::TempDir,
}
fn setup(case: &str) -> Setup {
    let directory = tempfile::tempdir().unwrap();
    let mut host_config = ProcessHostConfig::new(fixture());
    host_config.worker_args = vec![case.into(), directory.path().join("pid").into_os_string()];
    host_config.handshake_timeout = Duration::from_millis(500);
    host_config.unload_timeout = Duration::from_millis(400);
    host_config.shutdown_timeout = Duration::from_millis(400);
    let host = ProcessHost::new(host_config).unwrap();
    let diagnostics = host.diagnostics();
    let config = RuntimeConfig {
        load_options: options(),
        slow_consumer_timeout: Duration::from_millis(350),
        execution_timeout: Duration::from_secs(30),
        load_timeout: Duration::from_secs(2),
        ..Default::default()
    };
    let runtime = Runtime::spawn(config, |_id: &ModelId| Ok(resolved()), host).unwrap();
    let handle = runtime.handle();
    Setup {
        runtime,
        handle,
        diagnostics,
        _directory: directory,
    }
}
fn collect(events: &EventReceiver) -> Vec<RequestEvent> {
    let mut result = Vec::new();
    let until = Instant::now() + Duration::from_secs(9);
    while Instant::now() < until {
        match events.recv_timeout(Duration::from_secs(1)) {
            Ok(event) => {
                let terminal = event.kind.is_terminal();
                result.push(event);
                if terminal {
                    assert!(events.recv_timeout(Duration::from_millis(20)).is_err());
                    return result;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    panic!("missing terminal: {result:?}");
}
fn await_state(handle: &RuntimeHandle, state: ModelState) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if handle.status().unwrap().state == state {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!(
        "state did not reach {state:?}: {:?}",
        handle.status().unwrap()
    );
}
fn finish(setup: Setup) {
    setup.runtime.shutdown().unwrap();
    assert_eq!(
        setup.diagnostics.sessions_started(),
        setup.diagnostics.sessions_reaped()
    );
    assert!(setup.diagnostics.worker_pid().is_none());
}
#[test]
fn wrong_malformed_oversized_missing_hello_and_load_crash_reap() {
    for case in [
        "wrong_hello",
        "malformed_hello",
        "oversized_hello",
        "silent_hello",
        "crash_hello",
        "crash_load",
    ] {
        let s = setup(case);
        assert!(s.handle.load(model(), options()).is_err(), "{case}");
        await_state(&s.handle, ModelState::Faulted);
        assert!(
            s.diagnostics.worker_pid().is_none(),
            "reap before Faulted: {case}"
        );
        finish(s);
    }
}
#[test]
fn malformed_oversized_stale_and_crashed_generation_fault_once() {
    for case in [
        "malformed_event",
        "oversized_event",
        "blocked_stdout",
        "stale_session",
        "stale_operation",
        "duplicate_credit",
        "crash_generate",
        "alive_faulted",
    ] {
        let s = setup(case);
        s.handle.load(model(), options()).unwrap();
        let events = s.handle.submit(request()).unwrap();
        let output = collect(&events);
        assert_eq!(
            output.iter().filter(|e| e.kind.is_terminal()).count(),
            1,
            "{case}"
        );
        assert!(
            matches!(output.last().unwrap().kind, RequestEventKind::Failed { .. }),
            "{case}"
        );
        await_state(&s.handle, ModelState::Faulted);
        assert!(s.diagnostics.worker_pid().is_none());
        finish(s);
    }
}
#[test]
fn ordinary_generation_failure_preserves_live_worker() {
    let s = setup("generation_failed");
    s.handle.load(model(), options()).unwrap();
    let pid = s.diagnostics.worker_pid();
    let events = s.handle.submit(request()).unwrap();
    let output = collect(&events);
    assert!(
        matches!(&output.last().unwrap().kind, RequestEventKind::Failed { error, .. } if error.code == ErrorCode::ContextLengthExceeded)
    );
    await_state(&s.handle, ModelState::Ready);
    assert_eq!(s.diagnostics.worker_pid(), pid);
    s.handle.unload().unwrap();
    finish(s);
}
#[test]
fn idle_worker_death_is_reported_without_new_request() {
    let s = setup("idle_crash");
    s.handle.load(model(), options()).unwrap();
    await_state(&s.handle, ModelState::Faulted);
    assert!(s.diagnostics.worker_pid().is_none());
    let sessions = s.diagnostics.sessions_started();
    assert_eq!(
        s.handle.unload().unwrap_err().code,
        ErrorCode::RuntimeFaulted
    );
    assert_eq!(s.diagnostics.sessions_started(), sessions);
    finish(s);
}
#[test]
fn queued_requests_terminate_once_and_recovery_is_explicit_without_replay() {
    let s = setup("crash_once");
    s.handle.load(model(), options()).unwrap();
    let first = s.handle.submit(request()).unwrap();
    let queued: Vec<_> = (0..3)
        .map(|_| s.handle.submit(request()).unwrap())
        .collect();
    let output = collect(&first);
    assert!(
        output
            .iter()
            .any(|e| matches!(e.kind, RequestEventKind::TextDelta(_)))
    );
    assert!(matches!(
        output.last().unwrap().kind,
        RequestEventKind::Failed { .. }
    ));
    for events in queued {
        let output = collect(&events);
        assert_eq!(output.iter().filter(|e| e.kind.is_terminal()).count(), 1);
        assert!(matches!(
            output.last().unwrap().kind,
            RequestEventKind::Failed { .. }
        ));
    }
    await_state(&s.handle, ModelState::Faulted);
    let sessions = s.diagnostics.sessions_started();
    assert_eq!(
        s.handle.unload().unwrap_err().code,
        ErrorCode::RuntimeFaulted
    );
    assert_eq!(s.diagnostics.sessions_started(), sessions);
    s.handle.load(model(), options()).unwrap();
    assert_eq!(s.diagnostics.sessions_started(), sessions + 1);
    let recovered = s.handle.submit(request()).unwrap();
    assert!(matches!(
        collect(&recovered).last().unwrap().kind,
        RequestEventKind::Completed { .. }
    ));
    finish(s);
}
#[test]
fn repeated_cancel_does_not_extend_five_second_kill_grace() {
    let s = setup("ignore_cancel");
    s.handle.load(model(), options()).unwrap();
    let request = request();
    let id = request.request_id;
    let events = s.handle.submit(request).unwrap();
    loop {
        if matches!(
            events.recv_timeout(Duration::from_secs(2)).unwrap().kind,
            RequestEventKind::Started { .. }
        ) {
            break;
        }
    }
    let start = Instant::now();
    s.handle.cancel(id).unwrap();
    for _ in 0..4 {
        thread::sleep(Duration::from_millis(600));
        s.handle.cancel(id).unwrap();
    }
    let output = collect(&events);
    let elapsed = start.elapsed();
    assert!(
        elapsed >= Duration::from_millis(4900) && elapsed < Duration::from_millis(6500),
        "{elapsed:?}"
    );
    assert!(matches!(
        output.last().unwrap().kind,
        RequestEventKind::Cancelled {
            reason: ErrorCode::RequestCancelled,
            ..
        }
    ));
    await_state(&s.handle, ModelState::Faulted);
    assert!(s.diagnostics.worker_pid().is_none());
    finish(s);
}
#[test]
fn blocked_stdin_cannot_block_actor_or_cancel_caller() {
    let s = setup("block_stdin");
    s.handle.load(model(), options()).unwrap();
    let mut request = request();
    request.messages[0].content = "x".repeat(900_000);
    let id = request.request_id;
    let events = s.handle.submit(request).unwrap();
    thread::sleep(Duration::from_millis(50));
    let now = Instant::now();
    s.handle.status().unwrap();
    s.handle.cancel(id).unwrap();
    assert!(now.elapsed() < Duration::from_millis(250));
    let output = collect(&events);
    assert!(matches!(
        output.last().unwrap().kind,
        RequestEventKind::Cancelled {
            reason: ErrorCode::RequestCancelled,
            ..
        }
    ));
    assert!(s.diagnostics.worker_pid().is_none());
    finish(s);
}
#[test]
fn leases_retain_credit_until_consumption_and_slow_consumer_keeps_reason() {
    let s = setup("stream");
    s.handle.load(model(), options()).unwrap();
    let events = s.handle.submit(request()).unwrap();
    let mut held = Vec::new();
    while held.len() < 2 {
        let lease = events.recv_timeout_leased(Duration::from_secs(2)).unwrap();
        if matches!(lease.kind, RequestEventKind::TextDelta(_)) {
            held.push(lease);
        }
    }
    assert!(events.buffered_bytes() >= 240 * 1024);
    assert!(events.buffered_bytes() <= runtime_core::MAX_BUFFERED_TEXT_BYTES);
    let output = collect(&events);
    assert!(matches!(
        output.last().unwrap().kind,
        RequestEventKind::Cancelled {
            reason: ErrorCode::SlowConsumer,
            ..
        }
    ));
    assert_eq!(events.buffered_bytes(), 240 * 1024);
    drop(held);
    assert_eq!(events.buffered_bytes(), 0);
    finish(s);
}
#[test]
fn unload_crash_and_deadline_are_finite_and_dead_unload_does_not_respawn() {
    for case in ["crash_unload", "hang_unload"] {
        let s = setup(case);
        s.handle.load(model(), options()).unwrap();
        let start = Instant::now();
        assert!(s.handle.unload().is_err());
        assert!(start.elapsed() < Duration::from_secs(2));
        let count = s.diagnostics.sessions_started();
        assert_eq!(
            s.handle.unload().unwrap_err().code,
            ErrorCode::RuntimeFaulted
        );
        assert_eq!(s.diagnostics.sessions_started(), count);
        finish(s);
    }
}
#[test]
fn shutdown_deadline_kills_uncooperative_child_and_reaps() {
    let s = setup("hang_shutdown");
    s.handle.load(model(), options()).unwrap();
    let start = Instant::now();
    finish(s);
    assert!(start.elapsed() < Duration::from_secs(2));
}
struct Sink(mpsc::Sender<ExecutorEvent>);
impl ExecutionEventSink for Sink {
    fn emit(&self, event: ExecutorEvent) -> bool {
        self.0.send(event).is_ok()
    }
}
#[test]
fn immediate_cancel_during_hello_never_dispatches_unsent_load() {
    for cancel_during_hello in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let pid_file = directory.path().join("pid");
        let mut config = ProcessHostConfig::new(fixture());
        config.worker_args = vec!["delayed_hello".into(), pid_file.clone().into_os_string()];
        let mut host = ProcessHost::new(config).unwrap();
        let diagnostics = host.diagnostics();
        let (tx, rx) = mpsc::channel();
        let now = Instant::now();
        let cancel = host
            .start(
                ExecutorCommand::Load {
                    model: resolved(),
                    options: options(),
                },
                ExecutionEvents::from_sink(1, Arc::new(Sink(tx))),
            )
            .unwrap();
        assert!(now.elapsed() < Duration::from_millis(100));
        if cancel_during_hello {
            let deadline = Instant::now() + Duration::from_secs(1);
            while diagnostics.worker_pid().is_none() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(1));
            }
            assert!(diagnostics.worker_pid().is_some());
        }
        cancel.cancel();
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            ExecutorEvent::Failed(RuntimeError {
                code: ErrorCode::RequestCancelled,
                ..
            })
        ));
        assert!(!directory.path().join("pid.load").exists());
        host.close().unwrap();
        assert!(diagnostics.worker_pid().is_none());
        assert!(rx.try_recv().is_err());
    }
}
#[test]
fn encoded_request_bound_is_checked_before_any_write() {
    let s = setup("normal");
    s.handle.load(model(), options()).unwrap();
    let mut request = request();
    request.messages[0].content = "\0".repeat(500_000);
    let events = s.handle.submit(request).unwrap();
    let output = collect(&events);
    assert!(
        !output
            .iter()
            .any(|e| matches!(e.kind, RequestEventKind::Started { .. }))
    );
    assert!(matches!(
        output.last().unwrap().kind,
        RequestEventKind::Failed { .. }
    ));
    finish(s);
}

#[test]
fn dead_backend_unload_acknowledges_reclaimed_state_without_respawn() {
    let mut config = ProcessHostConfig::new(fixture());
    config.worker_args = vec!["crash_load".into()];
    let mut host = ProcessHost::new(config).unwrap();
    let diagnostics = host.diagnostics();
    let (tx, rx) = mpsc::channel();
    host.start(
        ExecutorCommand::Load {
            model: resolved(),
            options: options(),
        },
        ExecutionEvents::from_sink(1, Arc::new(Sink(tx.clone()))),
    )
    .unwrap();
    assert!(matches!(
        rx.recv_timeout(Duration::from_secs(2)).unwrap(),
        ExecutorEvent::Faulted(_)
    ));
    assert!(diagnostics.worker_pid().is_none());
    let count = diagnostics.sessions_started();
    host.start(
        ExecutorCommand::Unload,
        ExecutionEvents::from_sink(2, Arc::new(Sink(tx))),
    )
    .unwrap();
    assert!(matches!(
        rx.recv_timeout(Duration::from_secs(1)).unwrap(),
        ExecutorEvent::Unloaded
    ));
    assert_eq!(diagnostics.sessions_started(), count);
    host.close().unwrap();
}

#[test]
fn cancelling_before_or_during_generate_preserves_healthy_model() {
    let s = setup("wait_cancel");
    s.handle.load(model(), options()).unwrap();
    let pid = s.diagnostics.worker_pid();
    let request = request();
    let id = request.request_id;
    let events = s.handle.submit(request).unwrap();
    s.handle.cancel(id).unwrap();
    assert!(matches!(
        collect(&events).last().unwrap().kind,
        RequestEventKind::Cancelled {
            reason: ErrorCode::RequestCancelled,
            ..
        }
    ));
    await_state(&s.handle, ModelState::Ready);
    assert_eq!(s.diagnostics.worker_pid(), pid);
    finish(s);
}
#[cfg(target_os = "linux")]
#[test]
fn escaped_linux_pipe_fails_closed_instead_of_hanging_or_claiming_reaped() {
    let s = setup("escaped_pipe");
    s.handle.load(model(), options()).unwrap();
    let pid_path = s._directory.path().join("pid.descendant");
    let deadline = Instant::now() + Duration::from_secs(2);
    while !pid_path.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    let escaped: u32 = std::fs::read_to_string(pid_path).unwrap().parse().unwrap();
    struct Cleanup(u32);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            unsafe {
                libc::kill(self.0 as i32, libc::SIGKILL);
            }
        }
    }
    let _cleanup = Cleanup(escaped);
    let events = s.handle.submit(request()).unwrap();
    let now = Instant::now();
    let output = collect(&events);
    assert!(now.elapsed() < Duration::from_secs(7));
    assert!(
        matches!(&output.last().unwrap().kind, RequestEventKind::Failed { error, .. } if error.code == ErrorCode::ExecutorCleanupUnconfirmed)
    );
    assert_eq!(s.diagnostics.sessions_reaped(), 0);
    assert!(s.diagnostics.worker_pid().is_some());
    assert_eq!(
        s.handle.load(model(), options()).unwrap_err().code,
        ErrorCode::ExecutorCleanupUnconfirmed
    );
    let now = Instant::now();
    assert_eq!(
        s.runtime.shutdown().unwrap_err().code,
        ErrorCode::ExecutorCleanupUnconfirmed
    );
    assert!(now.elapsed() < Duration::from_secs(2));
}
#[test]
fn load_deadline_keeps_load_timeout_after_forced_process_cleanup() {
    let mut config = ProcessHostConfig::new(fixture());
    config.worker_args = vec!["hang_load".into()];
    let host = ProcessHost::new(config).unwrap();
    let diagnostics = host.diagnostics();
    let runtime = Runtime::spawn(
        RuntimeConfig {
            load_timeout: Duration::from_millis(100),
            ..Default::default()
        },
        |_id: &ModelId| Ok(resolved()),
        host,
    )
    .unwrap();
    let start = Instant::now();
    let error = runtime.handle().load(model(), options()).unwrap_err();
    assert_eq!(error.code, ErrorCode::LoadTimeout);
    assert!(start.elapsed() < Duration::from_secs(7));
    assert!(diagnostics.worker_pid().is_none());
    runtime.shutdown().unwrap();
}

#[test]
fn manual_stop_of_uncooperative_load_reaps_only_its_worker_and_allows_retry() {
    let mut config = ProcessHostConfig::new(fixture());
    let directory = tempfile::tempdir().unwrap();
    let pid = directory.path().join("worker");
    let loading_marker = directory.path().join("worker.loading");
    config.worker_args = vec!["hang_load_once".into(), pid.into_os_string()];
    let host = ProcessHost::new(config).unwrap();
    let diagnostics = host.diagnostics();
    let runtime = Runtime::spawn(
        RuntimeConfig {
            load_timeout: Duration::from_secs(30),
            ..Default::default()
        },
        |_id: &ModelId| Ok(resolved()),
        host,
    )
    .unwrap();
    let control = runtime_core::LoadControl::default();
    let stop = control.clone();
    let handle = runtime.handle();
    let loading = thread::spawn(move || handle.load_controlled(model(), options(), control));
    let until = Instant::now() + Duration::from_secs(2);
    while !loading_marker.exists() {
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(5));
    }
    let now = Instant::now();
    stop.cancel();
    assert_eq!(
        loading.join().unwrap().unwrap_err().code,
        ErrorCode::RequestCancelled
    );
    assert!(now.elapsed() < Duration::from_secs(7));
    assert!(diagnostics.worker_pid().is_none());
    assert_eq!(
        diagnostics.sessions_started(),
        diagnostics.sessions_reaped()
    );
    assert_eq!(
        runtime.handle().status().unwrap().state,
        ModelState::Unloaded
    );
    assert!(!runtime.handle().status().unwrap().stopping);
    // The same supervisor starts a new healthy worker for the next load.
    runtime
        .handle()
        .load_controlled(model(), options(), runtime_core::LoadControl::default())
        .unwrap();
    assert_eq!(runtime.handle().status().unwrap().state, ModelState::Ready);
    assert_eq!(diagnostics.sessions_started(), 2);
    stop.cancel(); // A stale old token does not touch the replacement session.
    assert_eq!(runtime.handle().status().unwrap().state, ModelState::Ready);
    runtime.shutdown().unwrap();
}

#[test]
fn manual_stop_cannot_hide_malformed_ipc_or_native_fault_after_load_dispatch() {
    for (case, expected) in [
        ("malformed_cancel_load", ErrorCode::NativeProtocol),
        ("native_fault_cancel_load", ErrorCode::NativeFailure),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let mut config = ProcessHostConfig::new(fixture());
        config.worker_args = vec![
            case.into(),
            directory.path().join("worker").into_os_string(),
        ];
        let host = ProcessHost::new(config).unwrap();
        let diagnostics = host.diagnostics();
        let runtime = Runtime::spawn(
            RuntimeConfig::default(),
            |_id: &ModelId| Ok(resolved()),
            host,
        )
        .unwrap();
        let control = runtime_core::LoadControl::default();
        let stop = control.clone();
        let handle = runtime.handle();
        let loading = thread::spawn(move || handle.load_controlled(model(), options(), control));
        let until = Instant::now() + Duration::from_secs(2);
        while !directory.path().join("worker.loading").exists() {
            assert!(Instant::now() < until);
            thread::sleep(Duration::from_millis(2));
        }
        stop.cancel();
        assert_eq!(loading.join().unwrap().unwrap_err().code, expected);
        assert_eq!(
            runtime.handle().status().unwrap().state,
            ModelState::Faulted
        );
        assert_eq!(
            runtime.handle().status().unwrap().last_error.unwrap().code,
            expected
        );
        assert!(diagnostics.worker_pid().is_none());
        assert_eq!(
            diagnostics.sessions_started(),
            diagnostics.sessions_reaped()
        );
        runtime.shutdown().unwrap();
    }
}
