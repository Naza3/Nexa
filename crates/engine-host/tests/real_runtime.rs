//! End-to-end, opt-in T02 verification with a managed copy of the locked GGUF.
//! Run with NEXA_TEST_MODEL and NEXA_TEST_THREADS=2, --ignored --test-threads=1.
//! No inference or storage operation in this test is replaced by a fake.
use engine_host::{EngineHost, Operation};
use model_store::{ImportCancellation, ImportRequest, ModelSource, ModelStore};
use runtime_core::{EventReceiver, Runtime, RuntimeHandle};
use runtime_types::*;
use std::{
    path::PathBuf,
    sync::{Arc, mpsc::RecvTimeoutError},
    time::{Duration, Instant},
};

const HASH: &str = "9465e63a22add5354d9bb4b99e90117043c7124007664907259bd16d043bb031";
struct TestDirectory(PathBuf);
impl TestDirectory {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("nexa-real-runtime-{}", RequestId::new())))
    }
}
impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn request(model: &ModelId, text: &str, max_tokens: u32) -> GenerationRequest {
    GenerationRequest {
        request_id: RequestId::new(),
        model: model.clone(),
        messages: vec![Message::new(Role::User, text)],
        options: GenerationOptions {
            max_tokens,
            temperature: 0.0,
            seed: 42,
            ..Default::default()
        },
    }
}
fn collect(receiver: &EventReceiver) -> Vec<RequestEvent> {
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut events = Vec::new();
    loop {
        let event = receiver
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("runtime did not produce a terminal event within 120 seconds");
        assert_eq!(event.seq, events.len() as u64 + 1);
        let done = event.kind.is_terminal();
        events.push(event);
        if done {
            break;
        }
    }
    assert_eq!(
        receiver.recv_timeout(Duration::from_millis(20)),
        Err(RecvTimeoutError::Disconnected),
        "request must have exactly one terminal event"
    );
    events
}
fn assert_completed(events: &[RequestEvent]) -> String {
    assert!(matches!(
        events.first().unwrap().kind,
        RequestEventKind::Accepted
    ));
    let start = events
        .iter()
        .position(|event| matches!(event.kind, RequestEventKind::Started { .. }))
        .expect("exact native prepare must produce Started");
    let mut text = String::new();
    for (index, event) in events.iter().enumerate() {
        if let RequestEventKind::TextDelta(piece) = &event.kind {
            assert!(index > start, "text was published before Started");
            assert!(!piece.is_empty() && piece.len() <= 4096);
            text.push_str(piece);
        }
    }
    match &events.last().unwrap().kind {
        RequestEventKind::Completed {
            usage, performance, ..
        } => {
            assert!(usage.prompt_tokens > 0 && usage.completion_tokens > 0);
            let performance = performance
                .as_deref()
                .expect("real engine phase measurements");
            assert!(performance.timings.is_valid());
            assert!(performance.timings.prefill_us > 0 && performance.timings.decode_us > 0);
            assert!(performance.load_options.threads > 0);
        }
        terminal => panic!("expected real completion, got {terminal:?}"),
    }
    assert!(!text.is_empty());
    assert!(!text.contains("<think>") && !text.contains("</think>"));
    text
}
fn wait_state(handle: &RuntimeHandle, state: ModelState) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if handle.status().unwrap().state == state {
            return;
        }
        assert!(Instant::now() < deadline, "runtime did not reach {state:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "requires the exact locked Qwen3 GGUF; real model-store + scheduler + native integration"]
fn real_store_runtime_queue_cancel_budget_and_idle_reload() {
    let source =
        PathBuf::from(std::env::var_os("NEXA_TEST_MODEL").expect("NEXA_TEST_MODEL is required"));
    let threads = std::env::var("NEXA_TEST_THREADS")
        .unwrap_or_else(|_| "2".into())
        .parse::<u32>()
        .unwrap();
    assert_eq!(
        threads, 2,
        "this acceptance test deliberately covers the validated two-thread configuration only"
    );
    let directory = TestDirectory::new();
    let store = Arc::new(ModelStore::open(&directory.0).unwrap());
    let model_id = ModelId::new("qwen3-0.6b-q8_0").unwrap();
    let mut import = ImportRequest::new(
        model_id.clone(),
        "Qwen3 0.6B Q8_0",
        ModelSource::local("official fixed public fixture"),
    );
    import.expected_sha256 = Some(HASH.into());
    let manifest = store
        .import_file(&source, import, &ImportCancellation::default())
        .unwrap();
    assert!(manifest.validated);
    assert_eq!(manifest.sha256, HASH);
    assert_eq!(manifest.size_bytes, 639_446_688);
    assert_eq!(
        manifest.template_sha256,
        "57f1fd00f0013a2be96aa79b857391f27e23df5b5f847072b524c897e24d0361"
    );
    let resolved = store.resolve(&model_id).unwrap();
    assert_ne!(resolved.path, source);
    assert!(resolved.loadable);
    assert_eq!(resolved.context_limit, 40960);
    let mut engine = EngineHost::new().unwrap();
    let reports = engine.take_diagnostics().unwrap();
    let resolver = store.clone();
    let options = LoadOptions {
        context_size: 2048,
        threads,
        batch_size: 128,
    };
    let runtime = Runtime::spawn(
        RuntimeConfig {
            max_queued_jobs: 1,
            queue_timeout: Duration::from_secs(30),
            execution_timeout: Duration::from_secs(60),
            load_timeout: Duration::from_secs(60),
            idle_unload: Duration::from_millis(250),
            load_options: options,
            ..Default::default()
        },
        move |id: &ModelId| resolver.resolve(id),
        engine,
    )
    .unwrap();
    let handle = runtime.handle();
    assert_eq!(handle.status().unwrap().state, ModelState::Unloaded);
    assert!(handle.status().unwrap().selected_model.is_none());

    // The native exact template budget rejects before Started/normal streaming.
    let over = handle
        .submit(request(&model_id, "Say hello.", 2048))
        .unwrap();
    let events = collect(&over);
    assert!(
        events
            .iter()
            .any(|e| matches!(e.kind, RequestEventKind::Loading))
    );
    assert!(!events.iter().any(|e| matches!(
        e.kind,
        RequestEventKind::Started { .. } | RequestEventKind::TextDelta(_)
    )));
    assert!(
        matches!(&events.last().unwrap().kind,RequestEventKind::Failed{error,..} if error.code==ErrorCode::ContextLengthExceeded)
    );
    assert_eq!(handle.status().unwrap().state, ModelState::Ready);

    let running = request(
        &model_id,
        "List fifty different common fruits and vegetables, one item per line.",
        128,
    );
    let running_id = running.request_id;
    let active = handle.submit(running).unwrap();
    let waiting = request(
        &model_id,
        "This queued request must never begin inference.",
        16,
    );
    let waiting_id = waiting.request_id;
    let queued = handle.submit(waiting).unwrap();
    assert_eq!(
        handle
            .submit(request(&model_id, "queue overflow", 16))
            .err()
            .unwrap()
            .code,
        ErrorCode::QueueFull
    );
    assert_eq!(handle.unload().unwrap_err().code, ErrorCode::RuntimeBusy);
    assert_eq!(
        handle
            .submit(request(
                &ModelId::new("other-model").unwrap(),
                "conflict",
                16
            ))
            .err()
            .unwrap()
            .code,
        ErrorCode::ModelConflict
    );
    handle.cancel(waiting_id).unwrap();
    let queued_events = collect(&queued);
    assert!(
        queued_events
            .iter()
            .any(|e| matches!(e.kind, RequestEventKind::Queued))
    );
    assert!(
        !queued_events
            .iter()
            .any(|e| matches!(e.kind, RequestEventKind::Started { .. }))
    );
    assert!(matches!(
        queued_events.last().unwrap().kind,
        RequestEventKind::Cancelled {
            reason: ErrorCode::RequestCancelled,
            ..
        }
    ));

    // Cancel only after native text proves decode, then await its cleanup ack.
    let mut active_events = Vec::new();
    loop {
        let event = active.recv_timeout(Duration::from_secs(60)).unwrap();
        let text = matches!(event.kind, RequestEventKind::TextDelta(_));
        assert!(
            !event.kind.is_terminal(),
            "fixture must still be running at first delta"
        );
        active_events.push(event);
        if text {
            break;
        }
    }
    let cancellation = Instant::now();
    handle.cancel(running_id).unwrap();
    loop {
        let event = active.recv_timeout(Duration::from_secs(5)).unwrap();
        let terminal = event.kind.is_terminal();
        active_events.push(event);
        if terminal {
            break;
        }
    }
    let cancellation_latency = cancellation.elapsed();
    assert!(cancellation_latency < Duration::from_secs(1));
    for (index, event) in active_events.iter().enumerate() {
        assert_eq!(event.seq, index as u64 + 1);
    }
    assert!(
        matches!(active_events.last().unwrap().kind,RequestEventKind::Cancelled{reason:ErrorCode::RequestCancelled,usage,..} if usage.completion_tokens>0)
    );
    assert_eq!(
        active.recv_timeout(Duration::from_millis(20)),
        Err(RecvTimeoutError::Disconnected)
    );
    assert_eq!(handle.status().unwrap().state, ModelState::Ready);

    let recovery = handle
        .submit(request(&model_id, "用一句中文说明本地模型的用途。", 24))
        .unwrap();
    let chinese = assert_completed(&collect(&recovery));
    assert!(!chinese.is_ascii());
    wait_state(&handle, ModelState::Unloaded);
    assert_eq!(
        handle.status().unwrap().selected_model,
        Some(model_id.clone())
    );
    let reload = handle
        .submit(request(&model_id, "Name one common fruit in English.", 12))
        .unwrap();
    let reload_events = collect(&reload);
    assert!(
        reload_events
            .iter()
            .any(|e| matches!(e.kind, RequestEventKind::Loading))
    );
    assert_completed(&reload_events);
    handle.unload().unwrap();
    assert_eq!(handle.status().unwrap().state, ModelState::Unloaded);
    handle.load(model_id.clone(), options).unwrap();
    handle.load(model_id.clone(), options).unwrap();
    assert_eq!(handle.status().unwrap().state, ModelState::Ready);
    runtime.shutdown().unwrap();
    assert_eq!(
        handle.status().unwrap_err().code,
        ErrorCode::RuntimeShutdown
    );
    assert!(
        source.is_file(),
        "managed import must preserve its original source"
    );

    let diagnostics: Vec<_> = reports.try_iter().collect();
    assert!(
        diagnostics
            .iter()
            .any(|r| r.operation == Operation::Load && r.load > Duration::ZERO)
    );
    assert!(
        diagnostics
            .iter()
            .any(|r| r.operation == Operation::Generate
                && r.error.is_none()
                && r.prepare > Duration::ZERO
                && r.prefill > Duration::ZERO
                && r.decode > Duration::ZERO)
    );
    for report in diagnostics {
        eprintln!(
            "{}",
            serde_json::json!({"test":"T02_real_runtime","operation":format!("{:?}",report.operation),"load_ms":report.load.as_secs_f64()*1000.0,"prepare_ms":report.prepare.as_secs_f64()*1000.0,"prefill_ms":report.prefill.as_secs_f64()*1000.0,"decode_ms":report.decode.as_secs_f64()*1000.0,"prompt_tokens":report.usage.prompt_tokens,"completion_tokens":report.usage.completion_tokens,"error":report.error.map(|e|e.as_str())})
        );
    }
    eprintln!(
        "{}",
        serde_json::json!({"test":"T02_real_runtime_verified","inference_threads":threads,"context_size":2048,"import_sha256":HASH,"queued_cancel":true,"decode_cancel_to_terminal_ms":cancellation_latency.as_secs_f64()*1000.0,"budget_before_started":true,"idle_reload":true,"safe_shutdown":true})
    );
    drop(store);
}
