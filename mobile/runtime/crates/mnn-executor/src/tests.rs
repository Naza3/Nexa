mod research_receipt;
use super::*;
use std::os::unix::fs::PermissionsExt;
#[test]
fn production_rejects_empty_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let store = mnn_model_store::MnnModelStore::open(dir.path()).unwrap();
    let (r, mut e) = MnnExecutor::composition(store.snapshot().unwrap()).unwrap();
    assert!(
        r.resolve(&ModelId::new(mnn_model_store::MODEL_ID).unwrap())
            .is_err()
    );
    e.close().unwrap();
    e.close().unwrap();
}

use mnn_model_store::{MODEL_ID, MnnModelStore};
use runtime_core::Runtime;
use runtime_types::{
    GenerationOptions, LoadOptions, Message, RequestEventKind, RequestId, Role, RuntimeConfig,
};
use std::time::Duration;
const MODEL_DIGEST: &str = "1ec59d439451738b4992f2ea5b06438788d752da81d55f11e1e8866d03fa7a57";
// This evidence is compiled ONLY in the unit-test target, never a product feature.
// The exact B1 Linux artifact was exercised by its real native/adapter suite.
pub(super) struct ResearchCpuEvidence;
impl ResearchCpuEvidence {
    fn verify(build: &mnn_adapter::BuildIdentity) -> Result<Self, RuntimeError> {
        // CI must consume same-run B1 proof; a missing/bad receipt never falls
        // back to any static record. These environment reads compile only here.
        if std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true")
            || std::env::var_os("NEXA_MNN_B2_RESEARCH_RECEIPT").is_some()
        {
            research_receipt::verify(build)
                .map_err(|_| error(ErrorCode::UnsupportedModel, "research receipt rejected"))?;
            return Ok(Self);
        }
        // Explicit local Debian evidence independently exercised by native owner
        // and root adapter checks. No current-build self-admission is performed.
        let (compiler, artifact, patch) = match (
            build.compiler.as_str(),
            build.artifact_manifest_sha256.as_str(),
        ) {
            (
                "c++ (Debian 14.2.0-19) 14.2.0",
                "53eab05ec35465082f784af6e305635376e9e12f8284cc4d208d1f5f40448450",
            ) => (
                "c++ (Debian 14.2.0-19) 14.2.0",
                "53eab05ec35465082f784af6e305635376e9e12f8284cc4d208d1f5f40448450",
                "43cc33146e2036ff452bd02d5ec352bb099d143ed4a4cdeb6ff55335987f9ce0",
            ),
            (
                "c++ (Debian 14.2.0-19) 14.2.0",
                "b8b4d8efb06388f49c6457d177997f2bf630c5dceeb9ec190d3cc245a313372d",
            ) => (
                "c++ (Debian 14.2.0-19) 14.2.0",
                "b8b4d8efb06388f49c6457d177997f2bf630c5dceeb9ec190d3cc245a313372d",
                "dfe571d08b1583e39d7ce271eb289ebc91c06b88261a3c83fdef1e53dc062b80",
            ),
            _ => {
                return Err(error(
                    ErrorCode::UnsupportedModel,
                    "research compiler has no evidence",
                ));
            }
        };
        let expected = mnn_adapter::BuildIdentity {
            upstream_commit: "d407447ed56c4121a11ccbd266dc184ca1ead0c2".into(),
            patch_sha256: patch.into(),
            policy_sha256: "ea06621b78e67e58f97f98951b26db0a8a893ded4112da3e5a762b98566fa328"
                .into(),
            artifact_manifest_sha256: artifact.into(),
            target: "x86_64-unknown-linux-gnu".into(),
            compiler: compiler.into(),
        };
        if build != &expected || mnn_model_store::candidate_digest() != MODEL_DIGEST {
            return Err(error(
                ErrorCode::UnsupportedModel,
                "research evidence identity mismatch",
            ));
        }
        Ok(Self)
    }
    fn composition(
        self,
        snapshot: MnnRegistrySnapshot,
    ) -> Result<(MnnModelResolver, MnnExecutor), RuntimeError> {
        let (mut resolver, executor) = MnnExecutor::composition(snapshot)?;
        resolver.research = Some(self);
        Ok((resolver, executor))
    }
}
fn research(snapshot: MnnRegistrySnapshot) -> (MnnModelResolver, MnnExecutor) {
    ResearchCpuEvidence::verify(&mnn_adapter::build_identity().unwrap())
        .unwrap()
        .composition(snapshot)
        .unwrap()
}
fn private_temp() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    d
}
fn model_id() -> ModelId {
    ModelId::new(MODEL_ID).unwrap()
}
fn load_options() -> LoadOptions {
    LoadOptions {
        context_size: 2048,
        threads: 2,
        batch_size: 32,
    }
}
fn config() -> RuntimeConfig {
    RuntimeConfig {
        load_options: load_options(),
        ..RuntimeConfig::android()
    }
}
fn request(text: &str, max_tokens: u32) -> GenerationRequest {
    GenerationRequest {
        request_id: RequestId::new(),
        model: model_id(),
        messages: vec![Message::new(Role::User, text)],
        options: GenerationOptions {
            max_tokens,
            temperature: 0.,
            top_p: 1.,
            seed: 0,
            stops: vec![],
        },
    }
}
fn collect(receiver: runtime_core::EventReceiver) -> Vec<runtime_types::RequestEvent> {
    let mut result = Vec::new();
    while let Ok(e) = receiver.recv_timeout(Duration::from_secs(120)) {
        let done = e.kind.is_terminal();
        result.push(e);
        if done {
            assert!(receiver.recv_timeout(Duration::from_secs(1)).is_err());
            break;
        }
    }
    assert_eq!(result.iter().filter(|e| e.kind.is_terminal()).count(), 1);
    result
}
fn complete(events: &[runtime_types::RequestEvent]) -> Usage {
    match &events.last().unwrap().kind {
        RequestEventKind::Completed { usage, .. } => *usage,
        other => panic!("expected completion, got {other:?}"),
    }
}
fn imported() -> (tempfile::TempDir, MnnModelStore) {
    let dir = private_temp();
    let mut store = MnnModelStore::open(dir.path()).unwrap();
    let source = std::env::var_os("NEXA_MNN_TEST_MODEL")
        .expect("set NEXA_MNN_TEST_MODEL to the fixed candidate directory");
    store
        .import_candidate(std::path::PathBuf::from(source), || false)
        .unwrap();
    (dir, store)
}
#[test]
fn research_rejects_changed_identity() {
    let original = mnn_adapter::build_identity().unwrap();
    for field in 0..6 {
        let mut b = original.clone();
        let value = match field {
            0 => &mut b.artifact_manifest_sha256,
            1 => &mut b.upstream_commit,
            2 => &mut b.patch_sha256,
            3 => &mut b.policy_sha256,
            4 => &mut b.target,
            _ => &mut b.compiler,
        };
        value.push('x');
        assert!(ResearchCpuEvidence::verify(&b).is_err());
    }
}
#[test]
#[ignore = "requires explicit fixed real candidate and audited Linux artifact"]
fn real_store_core_lifecycle() {
    let (_dir, mut store) = imported();
    let snapshot = store.snapshot().unwrap();
    let candidate = snapshot.resolve_candidate(&model_id()).unwrap();
    assert!(!candidate.validated);
    assert_eq!(candidate.path.file_name().unwrap(), "manifest.json");
    let (production, mut executor) = MnnExecutor::composition(snapshot.clone()).unwrap();
    assert_eq!(
        production.resolve(&model_id()).unwrap_err().code,
        ErrorCode::UnsupportedModel
    );
    executor.close().unwrap();
    drop(production);
    drop(executor);
    assert_eq!(
        store.remove_generation(&model_id()).unwrap_err().code,
        ErrorCode::ModelFileInUse
    );
    let (r, e) = research(snapshot.clone());
    let runtime = Runtime::spawn(config(), r, e).unwrap();
    let handle = runtime.handle();
    handle.load(model_id(), load_options()).unwrap();
    let q = request(
        "Please follow this instruction carefully. I am checking that a small local language model can answer a simple English question. Reply with one short greeting.",
        8,
    );
    let baseline = collect(handle.submit(q.clone()).unwrap());
    let usage = complete(&baseline);
    assert!(usage.prompt_tokens > 0 && usage.completion_tokens > 0 && usage.completion_tokens <= 8);
    let mut multi = request("那二加二呢？", 8);
    multi.messages = vec![
        Message::new(Role::System, "Answer briefly."),
        Message::new(Role::User, "What is 1+1?"),
        Message::new(Role::Assistant, "2"),
        Message::new(Role::User, "那二加二呢？"),
    ];
    complete(&collect(handle.submit(multi).unwrap()));
    // Exact logical budget: same prompt accepts equality and rejects one over.
    let mut exact = request(
        "Please follow this instruction carefully. I am checking that a small local language model can answer a simple English question. Reply with one short greeting.",
        1,
    );
    handle.unload().unwrap();
    let options = LoadOptions {
        context_size: usage.prompt_tokens + 1,
        ..load_options()
    };
    handle.load(model_id(), options).unwrap();
    let result = complete(&collect(handle.submit(exact.clone()).unwrap()));
    assert_eq!(
        result.prompt_tokens + result.completion_tokens,
        options.context_size
    );
    exact.request_id = RequestId::new();
    exact.options.max_tokens = 2;
    assert!(
        matches!(collect(handle.submit(exact).unwrap()).last().unwrap().kind,RequestEventKind::Failed{ref error,..} if error.code==ErrorCode::ContextLengthExceeded)
    );
    handle.unload().unwrap();
    handle.load(model_id(), load_options()).unwrap();
    let mut stop = request(
        "Please follow this instruction carefully. I am checking that a small local language model can answer a simple English question. Reply with one short greeting.",
        32,
    );
    let output = baseline
        .iter()
        .filter_map(|e| match &e.kind {
            RequestEventKind::TextDelta(s) => Some(s.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert!(!output.is_empty());
    stop.options.stops = vec![output.chars().next().unwrap().to_string()];
    let stopped = collect(handle.submit(stop).unwrap());
    complete(&stopped);
    assert!(
        !stopped
            .iter()
            .any(|e| matches!(e.kind, RequestEventKind::TextDelta(_)))
    );
    let q = request("Count from 1 to 100, separated by commas.", 128);
    let id = q.request_id;
    let stream = handle.submit(q).unwrap();
    loop {
        let e = stream.recv_timeout(Duration::from_secs(120)).unwrap();
        assert!(!e.kind.is_terminal());
        if matches!(e.kind, RequestEventKind::TextDelta(_)) {
            handle.cancel(id).unwrap();
            break;
        }
    }
    let cancelled = collect(stream);
    assert!(matches!(
        cancelled.last().unwrap().kind,
        RequestEventKind::Cancelled {
            reason: ErrorCode::RequestCancelled,
            ..
        }
    ));
    complete(&collect(handle.submit(request("Say hello.", 4)).unwrap()));
    // Consumer disappearance requests cancellation; the next request recovers.
    let disconnected = handle.submit(request("Count from 1 to 100.", 128)).unwrap();
    drop(disconnected);
    complete(&collect(handle.submit(request("Say hi.", 4)).unwrap()));
    handle.unload().unwrap();
    runtime.shutdown().unwrap();
    drop(snapshot);
    store.remove_generation(&model_id()).unwrap();
    assert!(
        store
            .snapshot()
            .unwrap()
            .resolve_candidate(&model_id())
            .is_err()
    );
}

pub(super) type ProgressHook = Arc<dyn Fn(mnn_adapter::Progress) + Send + Sync>;

#[derive(Default)]
struct Pressure {
    first: AtomicBool,
    full: AtomicBool,
    bytes: std::sync::atomic::AtomicUsize,
}
struct PressureExecutor {
    inner: MnnExecutor,
    pressure: Arc<Pressure>,
}
struct PressureSink {
    events: ExecutionEvents,
    pressure: Arc<Pressure>,
}
impl runtime_core::ExecutionEventSink for PressureSink {
    fn emit(&self, event: ExecutorEvent) -> bool {
        if matches!(event, ExecutorEvent::TextDelta(_))
            && !self.pressure.first.swap(true, Ordering::AcqRel)
        {
            let synthetic = "x".repeat(runtime_core::MAX_DELTA_BYTES);
            for _ in 0..runtime_core::MAX_BUFFERED_TEXT_BYTES / runtime_core::MAX_DELTA_BYTES {
                if !self.events.text_delta(&synthetic) {
                    return false;
                }
                self.pressure
                    .bytes
                    .fetch_add(synthetic.len(), Ordering::AcqRel);
            }
            self.pressure.full.store(true, Ordering::Release);
        }
        self.events.emit(event)
    }
}
impl Executor for PressureExecutor {
    fn start(
        &mut self,
        command: ExecutorCommand,
        events: ExecutionEvents,
    ) -> Result<CancellationHandle, RuntimeError> {
        let events = if matches!(command, ExecutorCommand::Generate { .. }) {
            ExecutionEvents::from_sink(
                events.operation_id(),
                Arc::new(PressureSink {
                    events,
                    pressure: self.pressure.clone(),
                }),
            )
        } else {
            events
        };
        self.inner.start(command, events)
    }
    fn close(&mut self) -> Result<(), RuntimeError> {
        self.inner.close()
    }
}
fn wait_until(mut predicate: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(90);
    while !predicate() {
        assert!(
            std::time::Instant::now() < deadline,
            "bounded test wait expired"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn pressure_reset(p: &Pressure) {
    p.full.store(false, Ordering::Release);
    p.first.store(false, Ordering::Release);
    p.bytes.store(0, Ordering::Release);
}
#[test]
#[ignore = "real native owner plus explicitly synthetic saturation of the core ledger"]
fn real_owner_backpressure_cancel_disconnect_shutdown() {
    let (_dir, store) = imported();
    let (r, e) = research(store.snapshot().unwrap());
    let pressure = Arc::new(Pressure::default());
    let runtime = Runtime::spawn(
        config(),
        r,
        PressureExecutor {
            inner: e,
            pressure: pressure.clone(),
        },
    )
    .unwrap();
    let handle = runtime.handle();
    handle.load(model_id(), load_options()).unwrap();
    let stream = handle.submit(request("Say hello.", 8)).unwrap();
    wait_until(|| pressure.full.load(Ordering::Acquire));
    assert_eq!(pressure.bytes.load(Ordering::Acquire), 256 * 1024);
    let started = std::time::Instant::now();
    wait_until(|| handle.status().unwrap().active_request.is_none());
    assert!(started.elapsed() >= Duration::from_secs(9));
    let result = collect(stream);
    assert!(matches!(
        result.last().unwrap().kind,
        RequestEventKind::Cancelled {
            reason: ErrorCode::SlowConsumer,
            ..
        }
    ));
    // Once full, cancellation must wake the original core Output wait immediately.
    pressure_reset(&pressure);
    let q = request("Say hello.", 8);
    let id = q.request_id;
    let stream = handle.submit(q).unwrap();
    wait_until(|| pressure.full.load(Ordering::Acquire));
    let start = std::time::Instant::now();
    handle.cancel(id).unwrap();
    let result = collect(stream);
    assert!(start.elapsed() < Duration::from_secs(3));
    assert!(matches!(
        result.last().unwrap().kind,
        RequestEventKind::Cancelled {
            reason: ErrorCode::RequestCancelled,
            ..
        }
    ));
    pressure_reset(&pressure);
    let stream = handle.submit(request("Say hello.", 8)).unwrap();
    wait_until(|| pressure.full.load(Ordering::Acquire));
    drop(stream);
    wait_until(|| handle.status().unwrap().active_request.is_none());
    // Disable pressure for a genuine unmodified recovery generation.
    pressure.first.store(true, Ordering::Release);
    complete(&collect(handle.submit(request("Say hello.", 4)).unwrap()));
    pressure_reset(&pressure);
    let stream = handle.submit(request("Say hello.", 8)).unwrap();
    wait_until(|| pressure.full.load(Ordering::Acquire));
    runtime.shutdown().unwrap();
    let result = collect(stream);
    assert!(matches!(
        result.last().unwrap().kind,
        RequestEventKind::Cancelled {
            reason: ErrorCode::RuntimeShutdown,
            ..
        }
    ));
}
#[test]
#[ignore = "requires real native owner, fixed candidate and audited Linux artifact"]
fn real_owner_fault_reload_load_timeout_and_idle() {
    let (_dir, mut store) = imported();
    let (r, e) = research(store.snapshot().unwrap());
    let state = e.state.clone();
    let runtime = Runtime::spawn(config(), r, e).unwrap();
    let handle = runtime.handle();
    handle.load(model_id(), load_options()).unwrap();
    *state.progress_hook.lock().unwrap() = Some(Arc::new(|p| {
        if p.phase == mnn_adapter::Phase::Decode {
            panic!("synthetic fatal callback injection");
        }
    }));
    let failed = collect(
        handle
            .submit(request("Count from one to twenty.", 16))
            .unwrap(),
    );
    assert!(matches!(
        failed.last().unwrap().kind,
        RequestEventKind::Failed {
            error: RuntimeError {
                code: ErrorCode::NativeProtocol,
                ..
            },
            ..
        }
    ));
    assert_eq!(
        handle.status().unwrap().state,
        runtime_types::ModelState::Faulted
    );
    assert_eq!(
        handle.submit(request("Say hi.", 2)).err().unwrap().code,
        ErrorCode::RuntimeFaulted
    );
    *state.progress_hook.lock().unwrap() = None;
    handle.load(model_id(), load_options()).unwrap();
    complete(&collect(handle.submit(request("Say hi.", 2)).unwrap()));
    runtime.shutdown().unwrap();
    // Hold a real native load checkpoint beyond core's deadline. No early cleanup ack.
    let (r, e) = research(store.snapshot().unwrap());
    let state = e.state.clone();
    let entered = Arc::new(AtomicBool::new(false));
    let released = Arc::new(AtomicBool::new(false));
    let a = entered.clone();
    struct ReleaseOnDrop(Arc<AtomicBool>);
    impl Drop for ReleaseOnDrop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }
    let _release_guard = ReleaseOnDrop(released.clone());
    let b = released.clone();
    *state.progress_hook.lock().unwrap() = Some(Arc::new(move |p| {
        if p.phase == mnn_adapter::Phase::Load && !a.swap(true, Ordering::AcqRel) {
            while !b.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }));
    let mut cfg = config();
    cfg.load_timeout = Duration::from_secs(30);
    let runtime = Runtime::spawn(cfg, r, e).unwrap();
    let handle = runtime.handle();
    let other = handle.clone();
    let (tx, rx) = mpsc::sync_channel(1);
    let loader =
        std::thread::spawn(move || tx.send(other.load(model_id(), load_options())).unwrap());
    eprintln!("waiting for real load checkpoint");
    wait_until(|| entered.load(Ordering::Acquire));
    eprintln!("real load checkpoint held");
    std::thread::sleep(Duration::from_secs(31));
    assert!(rx.try_recv().is_err());
    assert_eq!(
        handle.status().unwrap().state,
        runtime_types::ModelState::Loading
    );
    released.store(true, Ordering::Release);
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(30))
            .unwrap()
            .unwrap_err()
            .code,
        ErrorCode::LoadTimeout
    );
    loader.join().unwrap();
    runtime.shutdown().unwrap();
    let (r, e) = research(store.snapshot().unwrap());
    let mut cfg = config();
    cfg.idle_unload = Duration::from_millis(100);
    let runtime = Runtime::spawn(cfg, r, e).unwrap();
    let h = runtime.handle();
    h.load(model_id(), load_options()).unwrap();
    wait_until(|| h.status().unwrap().state == runtime_types::ModelState::Unloaded);
    runtime.shutdown().unwrap();

    // Dropping a busy mailbox does not join or free its native model on the
    // caller. The real owner retains the generation until its checkpoint returns.
    struct Sink(SyncSender<ExecutorEvent>);
    impl runtime_core::ExecutionEventSink for Sink {
        fn emit(&self, event: ExecutorEvent) -> bool {
            if matches!(event, ExecutorEvent::TextDelta(_)) {
                return true;
            }
            self.0.try_send(event).is_ok()
        }
    }
    let snapshot = store.snapshot().unwrap();
    let candidate = snapshot.resolve_candidate(&model_id()).unwrap();
    let (resolver, mut executor) = MnnExecutor::composition(snapshot).unwrap();
    drop(resolver);
    let (tx, rx) = mpsc::sync_channel(8);
    let sink = Arc::new(Sink(tx));
    executor
        .start(
            ExecutorCommand::Load {
                model: candidate,
                options: load_options(),
            },
            ExecutionEvents::from_sink(1, sink.clone()),
        )
        .unwrap();
    assert!(matches!(
        rx.recv_timeout(Duration::from_secs(90)).unwrap(),
        ExecutorEvent::Loaded
    ));
    assert_eq!(executor.close().unwrap_err().code, ErrorCode::RuntimeBusy);
    let entered = Arc::new(AtomicBool::new(false));
    let released = Arc::new(AtomicBool::new(false));
    let a = entered.clone();
    let b = released.clone();
    let _release_guard = ReleaseOnDrop(released.clone());
    *executor.state.progress_hook.lock().unwrap() = Some(Arc::new(move |p| {
        if p.phase == mnn_adapter::Phase::Prefill && !a.swap(true, Ordering::AcqRel) {
            while !b.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }));
    executor
        .start(
            ExecutorCommand::Generate {
                request: request("Say hello.", 8),
            },
            ExecutionEvents::from_sink(2, sink),
        )
        .unwrap();
    wait_until(|| entered.load(Ordering::Acquire));
    assert_eq!(executor.close().unwrap_err().code, ErrorCode::RuntimeBusy);
    let owner = executor.thread.take().unwrap();
    let started = std::time::Instant::now();
    drop(executor);
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(
        store.remove_generation(&model_id()).unwrap_err().code,
        ErrorCode::ModelFileInUse
    );
    released.store(true, Ordering::Release);
    owner.join().unwrap();
    store.remove_generation(&model_id()).unwrap();
}

#[test]
fn ci_receipt_missing_or_invalid_never_uses_static_evidence() {
    if std::env::var_os("NEXA_RECEIPT_NEGATIVE_CHILD").is_some() {
        assert!(ResearchCpuEvidence::verify(&mnn_adapter::build_identity().unwrap()).is_err());
        return;
    }
    for ci in ["true", "false"] {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child
            .args([
                "--exact",
                "tests::ci_receipt_missing_or_invalid_never_uses_static_evidence",
            ])
            .env("NEXA_RECEIPT_NEGATIVE_CHILD", "1")
            .env("GITHUB_ACTIONS", ci)
            .env_remove("NEXA_MNN_B2_CONTEXT")
            .env_remove("NEXA_MNN_B2_RESEARCH_RECEIPT");
        if ci == "false" {
            child.env("NEXA_MNN_B2_RESEARCH_RECEIPT", "/nonexistent/receipt.json");
        }
        assert!(child.output().unwrap().status.success());
    }
}
