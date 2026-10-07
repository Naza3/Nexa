//! Opt-in production worker test using the exact fixed GGUF. This test process
//! is native-linked by Cargo; the process-host management harness supplies the
//! separate proof that the real management executable does not link llama.
use runtime_core::ExecutorEvent;
use runtime_ipc::{
    Frame, Hello, MAX_EVENT_FRAME_BYTES, MAX_REQUEST_FRAME_BYTES, Message, SessionId, read_frame,
    write_frame,
};
use runtime_types::{
    GenerationOptions, GenerationRequest, LoadOptions, ModelId, RequestId, ResolvedModel, Role,
};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{BufReader, Read},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
const MODEL_SHA256: &str = "9465e63a22add5354d9bb4b99e90117043c7124007664907259bd16d043bb031";
struct Worker {
    child: Child,
    input: ChildStdin,
    events: mpsc::Receiver<Frame>,
    session: SessionId,
    seq: u64,
}
impl Worker {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_ai-runtime-worker"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let (events, receiver) = mpsc::channel();
        thread::spawn(move || {
            let mut output = BufReader::new(output);
            while let Ok(Some(frame)) = read_frame(&mut output, MAX_EVENT_FRAME_BYTES) {
                if events.send(frame).is_err() {
                    break;
                }
            }
        });
        let mut worker = Self {
            child,
            input,
            events: receiver,
            session: SessionId::new_v4(),
            seq: 0,
        };
        write_frame(
            &mut worker.input,
            &Frame::hello(worker.session, Hello::expected()),
            MAX_REQUEST_FRAME_BYTES,
        )
        .unwrap();
        let hello = worker.events.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(hello.session_id, worker.session);
        assert!(matches!(hello.message, Message::Hello(hello) if hello == Hello::expected()));
        worker
    }
    fn send(&mut self, operation: u64, request: Option<RequestId>, message: Message) {
        write_frame(
            &mut self.input,
            &Frame::command(self.session, operation, request, message),
            MAX_REQUEST_FRAME_BYTES,
        )
        .unwrap();
    }
    fn event(
        &mut self,
        operation: u64,
        request: Option<RequestId>,
    ) -> (ExecutorEvent, Option<u64>) {
        let frame = self
            .events
            .recv_timeout(Duration::from_secs(120))
            .expect("production worker event timeout");
        self.seq += 1;
        assert_eq!(frame.seq, Some(self.seq));
        assert_eq!(frame.session_id, self.session);
        assert_eq!(frame.operation_id, operation);
        assert_eq!(frame.request_id, request);
        let Message::Event { event, credit_id } = frame.message else {
            panic!("expected worker event")
        };
        (event, credit_id)
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn model() -> ResolvedModel {
    let path = PathBuf::from(
        std::env::var_os("NEXA_TEST_MODEL").expect("set NEXA_TEST_MODEL to the locked Qwen3 GGUF"),
    );
    let mut file = File::open(&path).unwrap();
    let mut sha = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let length = file.read(&mut buffer).unwrap();
        if length == 0 {
            break;
        }
        sha.update(&buffer[..length]);
    }
    assert_eq!(format!("{:x}", sha.finalize()), MODEL_SHA256);
    ResolvedModel {
        id: ModelId::new("qwen3-fixed").unwrap(),
        path,
        projector_path: None,
        context_limit: 2048,
        default_context: 2048,
        loadable: true,
    }
}
fn request(model: &ModelId, prompt: &str) -> GenerationRequest {
    GenerationRequest {
        request_id: RequestId::new(),
        model: model.clone(),
        messages: vec![runtime_types::Message::new(Role::User, prompt)],
        options: GenerationOptions {
            max_tokens: 64,
            temperature: 0.0,
            seed: 42,
            ..Default::default()
        },
    }
}
#[test]
#[ignore = "requires exact locked Qwen3 GGUF and production native worker"]
fn real_credit_wait_cancel_and_single_use_streaming() {
    let model = model();
    let threads = std::env::var("NEXA_TEST_THREADS")
        .unwrap_or_else(|_| "2".into())
        .parse()
        .unwrap();
    let mut worker = Worker::start();
    worker.send(
        1,
        None,
        Message::Load {
            model: model.clone(),
            options: LoadOptions {
                context_size: 2048,
                batch_size: 512,
                threads,
            },
        },
    );
    assert!(matches!(
        worker.event(1, None),
        (ExecutorEvent::Loaded, None)
    ));
    let chinese = request(&model.id, "请用中文解释为什么天空是蓝色的。");
    let request_id = chinese.request_id;
    worker.send(2, Some(request_id), Message::Generate { request: chinese });
    assert!(matches!(
        worker.event(2, Some(request_id)),
        (ExecutorEvent::Prepared { .. }, None)
    ));
    // Zero credits means no generated text can leave the worker, even though
    // native inference has an independent thread. Cancel must still be read.
    assert!(matches!(
        worker.events.recv_timeout(Duration::from_millis(500)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    let cancel_started = Instant::now();
    worker.send(2, Some(request_id), Message::Cancel {});
    assert!(matches!(
        worker.event(2, Some(request_id)),
        (ExecutorEvent::GenerationFailed { .. }, None)
    ));
    let cancel_duration = cancel_started.elapsed();
    assert!(cancel_duration < Duration::from_secs(5));
    let english = request(
        &model.id,
        "Explain in English why the sky is blue, in one sentence.",
    );
    let request_id = english.request_id;
    worker.send(3, Some(request_id), Message::Generate { request: english });
    worker.send(3, Some(request_id), Message::Credit { credit_id: 1 });
    let mut credit = 1;
    let mut text_bytes = 0;
    let mut prepared = false;
    loop {
        match worker.event(3, Some(request_id)) {
            (ExecutorEvent::Prepared { .. }, None) => {
                assert!(!prepared);
                prepared = true;
            }
            (ExecutorEvent::TextDelta(text), Some(id)) => {
                assert!(prepared);
                assert_eq!(id, credit);
                assert!(!text.is_empty() && text.len() <= 4096);
                text_bytes += text.len();
                credit += 1;
                worker.send(3, Some(request_id), Message::Credit { credit_id: credit });
            }
            (ExecutorEvent::Completed { usage, .. }, None) => {
                assert!(usage.completion_tokens > 0);
                break;
            }
            _ => panic!("unexpected generation event"),
        }
    }
    assert!(text_bytes > 0);
    worker.send(4, None, Message::Unload {});
    assert!(matches!(
        worker.event(4, None),
        (ExecutorEvent::Unloaded, None)
    ));
    worker.send(0, None, Message::Shutdown {});
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = worker.child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    eprintln!(
        "real worker credit/cancel: pass; emitted_bytes={text_bytes}; cancel_ms={}",
        cancel_duration.as_millis()
    );
}
