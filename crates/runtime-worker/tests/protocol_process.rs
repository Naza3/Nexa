//! Real OS pipes and the production binary, without loading a model. These
//! tests verify transport behavior, not parent-process native-link isolation.
use runtime_ipc::{
    Frame, Hello, MAX_EVENT_FRAME_BYTES, Message, SessionId, encode_frame, read_frame,
};
use std::{
    io::{BufReader, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

struct Worker {
    child: Child,
    input: Option<ChildStdin>,
    events: mpsc::Receiver<Option<Frame>>,
}
impl Worker {
    fn spawn() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_ai-runtime-worker"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take();
        let stdout = child.stdout.take().unwrap();
        let (events, receiver) = mpsc::channel();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                match read_frame(&mut reader, MAX_EVENT_FRAME_BYTES) {
                    Ok(Some(frame)) => {
                        if events.send(Some(frame)).is_err() {
                            break;
                        }
                    }
                    _ => {
                        let _ = events.send(None);
                        break;
                    }
                }
            }
        });
        Self {
            child,
            input,
            events: receiver,
        }
    }
    fn send(&mut self, bytes: &[u8]) {
        self.input.as_mut().unwrap().write_all(bytes).unwrap();
        self.input.as_mut().unwrap().flush().unwrap();
    }
    fn frame(&mut self, frame: &Frame) {
        self.send(&encode_frame(frame, runtime_ipc::MAX_REQUEST_FRAME_BYTES).unwrap());
    }
    fn hello(&mut self) -> SessionId {
        let session = SessionId::new_v4();
        self.frame(&Frame::hello(session, Hello::expected()));
        let frame = self
            .events
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        assert_eq!(frame.session_id, session);
        assert!(matches!(frame.message, Message::Hello(hello) if hello == Hello::expected()));
        session
    }
    fn wait(&mut self, success: bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert_eq!(status.success(), success);
                break;
            }
            assert!(
                Instant::now() < deadline,
                "worker failed to exit within bounded grace"
            );
            thread::sleep(Duration::from_millis(10));
        }
        let mut diagnostics = String::new();
        use std::io::Read;
        self.child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut diagnostics)
            .unwrap();
        assert!(diagnostics.len() <= 80, "stderr must be bounded");
        assert!(!diagnostics.contains("private-user-text"));
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
#[test]
fn production_worker_handshake_and_shutdown_reap() {
    let mut worker = Worker::spawn();
    let session = worker.hello();
    worker.frame(&Frame::command(session, 0, None, Message::Shutdown {}));
    worker.wait(true);
}
#[test]
fn eof_before_or_after_handshake_exits_cleanly() {
    let mut worker = Worker::spawn();
    worker.input.take();
    worker.wait(true);
    let mut worker = Worker::spawn();
    worker.hello();
    worker.input.take();
    worker.wait(true);
}
#[test]
fn rejects_prehandshake_duplicate_and_wrong_version() {
    let mut worker = Worker::spawn();
    worker.frame(&Frame::command(
        SessionId::new_v4(),
        1,
        None,
        Message::Unload {},
    ));
    worker.wait(false);
    let mut worker = Worker::spawn();
    let session = worker.hello();
    worker.frame(&Frame::hello(session, Hello::expected()));
    worker.wait(false);
    let mut worker = Worker::spawn();
    let mut hello = Frame::hello(SessionId::new_v4(), Hello::expected());
    hello.protocol_version += 1;
    let mut bytes = serde_json::to_vec(&hello).unwrap();
    bytes.push(b'\n');
    worker.send(&bytes);
    worker.wait(false);
}
#[test]
fn rejects_malformed_unknown_truncated_and_oversized_frames_without_echo() {
    for frame in [
        b"private-user-text\n".to_vec(),
        b"{\"unknown\":\"private-user-text\"}\n".to_vec(),
        b"{\"kind\":".to_vec(),
    ] {
        let mut worker = Worker::spawn();
        worker.send(&frame);
        worker.input.take();
        worker.wait(false);
    }
    let mut worker = Worker::spawn();
    // The bounded reader stops at the advertised limit, before LF arrives.
    let oversized = vec![b' '; runtime_ipc::MAX_REQUEST_FRAME_BYTES + 1];
    let _ = worker.input.as_mut().unwrap().write_all(&oversized);
    worker.wait(false);
}
#[test]
fn rejects_all_production_arguments() {
    let status = Command::new(env!("CARGO_BIN_EXE_ai-runtime-worker"))
        .arg("--fixture-mode")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(!status.success());
}

#[test]
fn native_load_failure_is_bounded_and_session_remains_usable() {
    let mut worker = Worker::spawn();
    let session = worker.hello();
    worker.frame(&Frame::command(
        session,
        1,
        None,
        Message::Load {
            model: runtime_types::ResolvedModel {
                id: runtime_types::ModelId::new("missing").unwrap(),
                path: std::env::temp_dir().join(format!(
                    "private-user-text-{}-missing.gguf",
                    SessionId::new_v4()
                )),
                projector_path: None,
                context_limit: 2048,
                default_context: 2048,
                loadable: true,
            },
            options: runtime_types::LoadOptions {
                context_size: 2048,
                batch_size: 512,
                threads: 2,
            },
        },
    ));
    let failure = worker
        .events
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
        .unwrap();
    assert_eq!(failure.operation_id, 1);
    assert_eq!(failure.seq, Some(1));
    let Message::Event {
        event: runtime_core::ExecutorEvent::Failed(error),
        credit_id: None,
    } = failure.message
    else {
        panic!("expected native load failure");
    };
    assert!(!error.message.contains("private-user-text"));
    worker.frame(&Frame::command(session, 2, None, Message::Unload {}));
    let unloaded = worker
        .events
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
        .unwrap();
    assert_eq!(unloaded.seq, Some(2));
    assert!(matches!(
        unloaded.message,
        Message::Event {
            event: runtime_core::ExecutorEvent::Unloaded,
            credit_id: None
        }
    ));
    worker.frame(&Frame::command(session, 0, None, Message::Shutdown {}));
    worker.wait(true);
}
