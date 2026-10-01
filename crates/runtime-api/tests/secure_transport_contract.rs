//! Actual TCP/HTTP/1 transport contracts with a deterministic protocol executor.
//! These synthetic events are NOT evidence of model inference quality.
use axum::http::HeaderValue;
use runtime_api::{
    ApiState, Config, router, security::SecurityContext, token::SecretToken, transport,
};
use runtime_core::{
    CancellationHandle, ExecutionEvents, Executor, ExecutorCommand, ExecutorEvent, Runtime,
};
use runtime_types::{ErrorCode, FinishReason, ModelId, ResolvedModel, RuntimeError, Usage};
use serde_json::{Value, json};
use std::{
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
#[derive(Clone, Copy)]
enum Mode {
    Success,
    BeforeError,
    AfterError,
    LoadWait,
    PrepareWait,
    StartedWait,
    Long,
    Overflow,
}
#[derive(Default)]
struct Observed {
    phase: AtomicUsize,
    cancelled: AtomicBool,
    events: Mutex<Option<ExecutionEvents>>,
    peak: AtomicUsize,
}
struct ProtocolExecutor {
    mode: Mode,
    observed: Arc<Observed>,
}
impl Executor for ProtocolExecutor {
    fn start(
        &mut self,
        command: ExecutorCommand,
        events: ExecutionEvents,
    ) -> Result<CancellationHandle, RuntimeError> {
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = cancelled.clone();
        let observed = self.observed.clone();
        let mode = if self.observed.cancelled.load(Ordering::SeqCst) {
            Mode::Success
        } else {
            self.mode
        };
        std::thread::spawn(move || {
            let wait = |phase| {
                observed.phase.store(phase, Ordering::SeqCst);
                while !cancelled.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(2));
                }
                observed.cancelled.store(true, Ordering::SeqCst);
            };
            match command {
                ExecutorCommand::Load { .. } => {
                    if matches!(mode, Mode::LoadWait) {
                        wait(1);
                        events.emit(ExecutorEvent::Failed(RuntimeError::new(
                            ErrorCode::RequestCancelled,
                            "cancel",
                        )));
                    } else {
                        events.emit(ExecutorEvent::Loaded);
                    }
                }
                ExecutorCommand::Unload => {
                    events.emit(ExecutorEvent::Unloaded);
                }
                ExecutorCommand::Generate { .. } => {
                    *observed.events.lock().unwrap() = Some(events.clone());
                    let scratch = events.try_reserve_text(16 * 1024).unwrap();
                    if matches!(mode, Mode::PrepareWait) {
                        wait(2);
                        events.emit(ExecutorEvent::GenerationFailed {
                            error: RuntimeError::new(ErrorCode::RequestCancelled, "cancel"),
                            usage: Usage::default(),
                        });
                        return;
                    }
                    if matches!(mode, Mode::BeforeError) {
                        events.emit(ExecutorEvent::GenerationFailed {
                            error: RuntimeError::new(ErrorCode::ContextLengthExceeded, "budget"),
                            usage: Usage::default(),
                        });
                        return;
                    }
                    events.emit(ExecutorEvent::Prepared { prompt_tokens: 3 });
                    observed.phase.store(3, Ordering::SeqCst);
                    let count = match mode {
                        Mode::Long => 20,
                        Mode::Overflow => 40,
                        _ => 1,
                    };
                    for _ in 0..count {
                        let text = match mode {
                            Mode::Long | Mode::Overflow => "x".repeat(4096),
                            _ => "你好🙂\n\"\\".into(),
                        };
                        let permit = loop {
                            if let Some(p) = events.try_reserve_text(120 * 1024) {
                                break Some(p);
                            }
                            if cancelled.load(Ordering::SeqCst)
                                || events.cancellation_reason().is_some()
                            {
                                break None;
                            }
                            std::thread::sleep(Duration::from_millis(1));
                        };
                        let Some(permit) = permit else {
                            break;
                        };
                        observed
                            .peak
                            .fetch_max(events.buffered_bytes(), Ordering::SeqCst);
                        if !events.emit_reserved_text(text, permit) {
                            break;
                        }
                    }
                    if matches!(mode, Mode::StartedWait) {
                        wait(4);
                    }
                    let usage = Usage {
                        prompt_tokens: 3,
                        completion_tokens: count,
                    };
                    if matches!(mode, Mode::AfterError) {
                        events.emit(ExecutorEvent::GenerationFailed {
                            error: RuntimeError::new(ErrorCode::NativeFailure, "decode"),
                            usage,
                        });
                    } else if cancelled.load(Ordering::SeqCst)
                        || events.cancellation_reason().is_some()
                    {
                        observed.cancelled.store(true, Ordering::SeqCst);
                        events.emit(ExecutorEvent::GenerationFailed {
                            error: RuntimeError::new(ErrorCode::RequestCancelled, "cancel"),
                            usage,
                        });
                    } else {
                        events.emit(ExecutorEvent::Completed {
                            usage,
                            finish_reason: FinishReason::Stop,
                        });
                    }
                    drop(scratch);
                }
            }
        });
        Ok(CancellationHandle::new(move || {
            stop.store(true, Ordering::SeqCst)
        }))
    }
}
struct Harness {
    _root: tempfile::TempDir,
    state: ApiState,
    address: SocketAddr,
    bearer: HeaderValue,
    observed: Arc<Observed>,
    server: tokio::task::JoinHandle<std::io::Result<()>>,
}
impl Harness {
    async fn new(mode: Mode) -> Self {
        let root = tempfile::tempdir().unwrap();
        let store = ApiState::open_store(root.path().join("models"))
            .await
            .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut config = Config::default();
        config.api.listen = address;
        config.inference.context_size = 2048;
        let observed = Arc::new(Observed::default());
        let runtime = Runtime::spawn(
            config.runtime_config(),
            |id: &ModelId| {
                Ok(ResolvedModel {
                    id: id.clone(),
                    path: "synthetic-not-read".into(),
                    context_limit: 4096,
                    default_context: 2048,
                    validated: true,
                })
            },
            ProtocolExecutor {
                mode,
                observed: observed.clone(),
            },
        )
        .unwrap();
        let state = ApiState::new(runtime, store, config, None);
        let token = SecretToken::generate().unwrap();
        let bearer = token.bearer_header_value();
        let security =
            Arc::new(SecurityContext::new(token, uuid::Uuid::new_v4(), address, vec![]).unwrap());
        let app = router(state.clone(), security);
        let shutdown = state.shutdown.clone();
        let server = tokio::spawn(transport::serve(listener, app, shutdown));
        Self {
            _root: root,
            state,
            address,
            bearer,
            observed,
            server,
        }
    }
    async fn send(&self, streaming: bool) -> TcpStream {
        self.send_with_policy(streaming, true).await
    }
    async fn send_with_policy(&self, streaming: bool, close: bool) -> TcpStream {
        let connection = if close { "Connection: close\r\n" } else { "" };
        let body = json!({"model":"fixture","messages":[{"role":"user","content":"protocol test"}],"stream":streaming,"max_tokens":128,"stream_options":if streaming{json!({"include_usage":true})}else{Value::Null}});
        let mut body = body.as_object().unwrap().clone();
        if !streaming {
            body.remove("stream_options");
        }
        let body = serde_json::to_string(&body).unwrap();
        let mut socket = TcpStream::connect(self.address).await.unwrap();
        let request = format!(
            "POST /v1/chat/completions HTTP/1.1\r\nHost: {}\r\nAuthorization: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{connection}\r\n{}",
            self.address,
            self.bearer.to_str().unwrap(),
            body.len(),
            body
        );
        socket.write_all(request.as_bytes()).await.unwrap();
        socket
    }
    async fn reply(&self, streaming: bool) -> (u16, String, String) {
        let mut socket = self.send(streaming).await;
        let mut wire = Vec::new();
        tokio::time::timeout(Duration::from_secs(5), socket.read_to_end(&mut wire))
            .await
            .unwrap()
            .unwrap();
        decode(&wire)
    }
    async fn phase(&self, phase: usize) {
        let until = Instant::now() + Duration::from_secs(3);
        while self.observed.phase.load(Ordering::SeqCst) != phase {
            assert!(Instant::now() < until, "executor phase not reached");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
    async fn clean(&self) {
        let until = Instant::now() + Duration::from_secs(3);
        loop {
            let status = self.state.control(|r| r.status()).await.unwrap();
            let bytes = self
                .observed
                .events
                .lock()
                .unwrap()
                .as_ref()
                .map_or(0, ExecutionEvents::buffered_bytes);
            if status.active_request.is_none() && bytes == 0 {
                break;
            }
            assert!(
                Instant::now() < until,
                "disconnect did not release request/budget: {bytes}"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
    async fn close(self) {
        self.state.shutdown.begin();
        self.state.shutdown.wait().await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), self.server)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
}
fn decode(wire: &[u8]) -> (u16, String, String) {
    let split = wire
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("HTTP headers");
    let headers = String::from_utf8(wire[..split].to_vec()).unwrap();
    let status = headers.split_whitespace().nth(1).unwrap().parse().unwrap();
    let mut body = &wire[split + 4..];
    let mut out = Vec::new();
    if headers
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        loop {
            let line = body.windows(2).position(|w| w == b"\r\n").unwrap();
            let n = usize::from_str_radix(std::str::from_utf8(&body[..line]).unwrap(), 16).unwrap();
            body = &body[line + 2..];
            if n == 0 {
                break;
            }
            out.extend_from_slice(&body[..n]);
            body = &body[n + 2..];
        }
    } else {
        out.extend_from_slice(body);
    }
    (status, headers, String::from_utf8(out).unwrap())
}
#[tokio::test]
async fn actual_http_stream_order_and_nonstream_json_are_exact() {
    let h = Harness::new(Mode::Success).await;
    let (status, _, body) = h.reply(true).await;
    assert_eq!(status, 200);
    let frames: Vec<_> = body.split("\n\n").filter(|s| !s.is_empty()).collect();
    assert_eq!(frames.len(), 5);
    assert_eq!(frames[4], "data: [DONE]");
    let json = frames[..4]
        .iter()
        .map(|f| serde_json::from_str::<Value>(f.strip_prefix("data: ").unwrap()).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(json[0]["choices"][0]["delta"]["role"], "assistant");
    assert_eq!(json[1]["choices"][0]["delta"]["content"], "你好🙂\n\"\\");
    assert_eq!(json[2]["choices"][0]["finish_reason"], "stop");
    assert_eq!(json[3]["choices"], json!([]));
    assert_eq!(json[3]["usage"]["total_tokens"], 4);
    assert!(
        json.iter()
            .all(|j| j["id"] == json[0]["id"] && j["created"] == json[0]["created"])
    );
    let (status, _, body) = h.reply(false).await;
    assert_eq!(status, 200);
    let value: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(value["choices"][0]["message"]["content"], "你好🙂\n\"\\");
    h.clean().await;
    h.close().await;
}
#[tokio::test]
async fn before_started_is_real_http_error_and_after_started_has_no_success_tail() {
    let h = Harness::new(Mode::BeforeError).await;
    let (status, headers, body) = h.reply(true).await;
    assert_eq!(status, 400);
    assert!(headers.to_ascii_lowercase().contains("x-request-id:"));
    assert!(!body.contains("data:"));
    assert!(body.contains("context_length_exceeded"));
    h.close().await;
    let h = Harness::new(Mode::AfterError).await;
    let (status, _, body) = h.reply(true).await;
    assert_eq!(status, 200);
    assert_eq!(body.matches("\"error\"").count(), 1);
    assert!(body.contains("internal_error"));
    assert!(!body.contains("[DONE]"));
    assert!(!body.contains("\"finish_reason\":\"stop\""));
    assert!(!body.contains("\"usage\""));
    h.clean().await;
    h.close().await;
}
#[tokio::test]
async fn nonstream_long_output_progresses_under_one_retained_credit_and_overflow_cancels() {
    let h = Harness::new(Mode::Long).await;
    let (status, _, body) = h.reply(false).await;
    assert_eq!(status, 200);
    let value: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        value["choices"][0]["message"]["content"]
            .as_str()
            .unwrap()
            .len(),
        20 * 4096
    );
    assert!(body.len() <= 96 * 1024);
    assert!(h.observed.peak.load(Ordering::SeqCst) <= 256 * 1024);
    h.clean().await;
    h.close().await;
    let h = Harness::new(Mode::Overflow).await;
    let (status, _, body) = h.reply(false).await;
    assert_eq!(status, 400);
    let value: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(value["error"]["code"], "response_too_large");
    assert_eq!(value["error"]["param"], "stream");
    h.clean().await;
    assert!(h.observed.cancelled.load(Ordering::SeqCst));
    h.close().await;
}
#[tokio::test]
async fn fin_and_rst_cancel_load_prepare_stream_and_nonstream_without_stranded_pumps() {
    for rst in [false, true] {
        for (mode, stream, phase) in [
            (Mode::LoadWait, true, 1),
            (Mode::PrepareWait, true, 2),
            (Mode::StartedWait, true, 4),
            (Mode::StartedWait, false, 4),
        ] {
            let h = Harness::new(mode).await;
            let mut socket = h.send_with_policy(stream, false).await;
            h.phase(phase).await;
            if stream && phase == 4 {
                let mut headers = [0; 1024];
                let n = tokio::time::timeout(Duration::from_secs(2), socket.read(&mut headers))
                    .await
                    .unwrap()
                    .unwrap();
                assert!(n > 0);
            }
            if rst {
                socket2::SockRef::from(&socket)
                    .set_linger(Some(Duration::ZERO))
                    .unwrap();
                drop(socket);
            } else {
                socket.shutdown().await.unwrap();
                drop(socket);
            }
            h.clean().await;
            assert!(h.observed.cancelled.load(Ordering::SeqCst));
            let (status, _, body) = h.reply(false).await;
            assert_eq!(status, 200, "next request after disconnect must recover");
            let value: Value = serde_json::from_str(&body).unwrap();
            assert_eq!(value["object"], "chat.completion");
            h.clean().await;
            h.close().await;
        }
    }
}
#[tokio::test]
async fn common_body_limit_counts_chunked_get_before_route_or_json() {
    let h = Harness::new(Mode::Success).await;
    let mut socket = TcpStream::connect(h.address).await.unwrap();
    let request = format!(
        "GET /unknown HTTP/1.1\r\nHost: {}\r\nAuthorization: {}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
        h.address,
        h.bearer.to_str().unwrap()
    );
    socket.write_all(request.as_bytes()).await.unwrap();
    for _ in 0..17 {
        socket.write_all(b"10000\r\n").await.unwrap();
        socket.write_all(&vec![b'x'; 65536]).await.unwrap();
        socket.write_all(b"\r\n").await.unwrap();
    }
    let _ = socket.write_all(b"0\r\n\r\n").await;
    let mut wire = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(3), socket.read_to_end(&mut wire))
        .await
        .unwrap();
    let (status, _, body) = decode(&wire);
    assert_eq!(status, 413);
    assert!(body.contains("request_too_large"));
    assert_eq!(h.observed.phase.load(Ordering::SeqCst), 0);
    h.close().await;
}

#[tokio::test]
async fn wire_authority_authentication_and_ambient_credentials_fail_closed() {
    let h = Harness::new(Mode::Success).await;
    let bearer = h.bearer.to_str().unwrap();
    let token = bearer.strip_prefix("Bearer ").unwrap();
    let cases = [
        (
            format!(
                "GET /healthz HTTP/1.1\r\nHost: {}\r\nHost: {}\r\n",
                h.address, h.address
            ),
            vec![400, 403],
        ),
        ("GET /healthz HTTP/1.1\r\n".to_owned(), vec![400, 403]),
        (
            format!(
                "GET http://attacker.invalid/healthz HTTP/1.1\r\nHost: {}\r\n",
                h.address
            ),
            vec![403],
        ),
        (
            format!(
                "GET /runtime/status HTTP/1.1\r\nHost: {}\r\nAuthorization: {bearer}\r\nAuthorization: {bearer}\r\n",
                h.address
            ),
            vec![400],
        ),
        (
            format!(
                "GET /runtime/status?api_key={token} HTTP/1.1\r\nHost: {}\r\nCookie: Authorization={bearer}\r\nForwarded: for=127.0.0.1\r\n",
                h.address
            ),
            vec![401],
        ),
        (
            format!(
                "GET /runtime/status HTTP/1.1\r\nHost: {}\r\nAuthorization: {bearer}\r\nOrigin: null\r\n",
                h.address
            ),
            vec![403],
        ),
        (
            format!(
                "GET /healthz HTTP/1.1\r\nHost: {}\r\nX-Nexa-Server-Challenge: {}\r\nX-Nexa-Server-Challenge: {}\r\n",
                h.address,
                "a".repeat(64),
                "a".repeat(64)
            ),
            vec![400],
        ),
    ];
    for (mut request, statuses) in cases {
        request.push_str("Connection: close\r\n\r\n");
        let mut socket = TcpStream::connect(h.address).await.unwrap();
        socket.write_all(request.as_bytes()).await.unwrap();
        let mut bytes = Vec::new();
        tokio::time::timeout(Duration::from_secs(2), socket.read_to_end(&mut bytes))
            .await
            .unwrap()
            .unwrap();
        let (status, _, body) = decode(&bytes);
        if request.contains("Authorization:")
            || request.contains("X-Nexa-Server-Challenge:")
            || status == 401
        {
            let envelope: Value = serde_json::from_str(&body).unwrap();
            assert!(envelope["error"]["code"].is_string());
        }
        assert!(statuses.contains(&status), "unexpected status {status}");
    }
    assert_eq!(h.observed.phase.load(Ordering::SeqCst), 0);
    h.close().await;
}

// A test-only read gate models a client driver that is briefly scheduled to
// upload before it gets to consume an early error response. No production
// timeout, retry, authentication, or server behavior is changed by this fixture.
struct ReadGate {
    open: AtomicBool,
    waker: Mutex<Option<std::task::Waker>>,
}
struct GatedSocket {
    socket: TcpStream,
    gate: Arc<ReadGate>,
}
impl tokio::io::AsyncRead for GatedSocket {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        if !self.gate.open.load(Ordering::SeqCst) {
            *self.gate.waker.lock().unwrap() = Some(cx.waker().clone());
            if !self.gate.open.load(Ordering::SeqCst) {
                return std::task::Poll::Pending;
            }
        }
        std::pin::Pin::new(&mut self.socket).poll_read(cx, buf)
    }
}
impl tokio::io::AsyncWrite for GatedSocket {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bytes: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.socket).poll_write(cx, bytes)
    }
    fn poll_write_vectored(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bytes: &[std::io::IoSlice<'_>],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.socket).poll_write_vectored(cx, bytes)
    }
    fn is_write_vectored(&self) -> bool {
        true
    }
    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.socket).poll_flush(cx)
    }
    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.socket).poll_shutdown(cx)
    }
}
fn safe_hyper_category(stage: &str, error: &hyper::Error) -> String {
    use std::error::Error;
    let mut source = error.source();
    let mut io = "none";
    while let Some(value) = source {
        if let Some(error) = value.downcast_ref::<std::io::Error>() {
            io = match error.kind() {
                std::io::ErrorKind::ConnectionReset => "connection_reset",
                std::io::ErrorKind::BrokenPipe => "broken_pipe",
                std::io::ErrorKind::UnexpectedEof => "unexpected_eof",
                std::io::ErrorKind::TimedOut => "timed_out",
                _ => "other",
            };
            break;
        }
        source = value.source();
    }
    format!(
        "stage={stage}, canceled={}, closed={}, incomplete={}, body_write_aborted={}, io={io}",
        error.is_canceled(),
        error.is_closed(),
        error.is_incomplete_message(),
        error.is_body_write_aborted()
    )
}
#[tokio::test]
async fn oversize_declared_header_without_upload_returns_complete_413() {
    let h = Harness::new(Mode::Success).await;
    let mut socket = TcpStream::connect(h.address).await.unwrap();
    let headers = format!(
        "POST /v1/chat/completions HTTP/1.1\r\nHost: {}\r\nAuthorization: {}\r\nContent-Type: application/json\r\nContent-Length: 1048577\r\n\r\n",
        h.address,
        h.bearer.to_str().unwrap()
    );
    socket.write_all(headers.as_bytes()).await.unwrap();
    let mut wire = Vec::new();
    tokio::time::timeout(Duration::from_secs(2), socket.read_to_end(&mut wire))
        .await
        .unwrap()
        .unwrap();
    let (status, headers, body) = decode(&wire);
    let envelope: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(status, 413);
    assert!(headers.to_ascii_lowercase().contains("connection: close"));
    assert_eq!(envelope["error"]["code"], "request_too_large");
    assert_eq!(h.observed.phase.load(Ordering::SeqCst), 0);
    drop(socket);
    h.close().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oversize_eager_upload_with_delayed_client_read_preserves_complete_413() {
    use http_body_util::BodyExt;
    let h = Harness::new(Mode::Success).await;
    let socket = TcpStream::connect(h.address).await.unwrap();
    socket.set_nodelay(true).unwrap();
    socket2::SockRef::from(&socket)
        .set_send_buffer_size(4096)
        .unwrap();
    let gate = Arc::new(ReadGate {
        open: AtomicBool::new(false),
        waker: Mutex::new(None),
    });
    let io = GatedSocket {
        socket,
        gate: gate.clone(),
    };
    let (mut sender, connection) =
        hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(io))
            .await
            .unwrap();
    let driver = tokio::spawn(connection);
    sender.ready().await.unwrap();
    let request = hyper::Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("host", h.address.to_string())
        .header("authorization", h.bearer.clone())
        .header("content-type", "application/json")
        .body(http_body_util::Full::new(bytes::Bytes::from(vec![
            b' ';
            1048577
        ])))
        .unwrap();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        gate.open.store(true, Ordering::SeqCst);
        if let Some(waker) = gate.waker.lock().unwrap().take() {
            waker.wake();
        }
    });
    let outcome = tokio::time::timeout(Duration::from_secs(3), async {
        let response = sender
            .send_request(request)
            .await
            .map_err(|e| safe_hyper_category("send", &e))?;
        let status = response.status();
        let mut body = response.into_body();
        let mut bytes = Vec::new();
        while let Some(frame) = body.frame().await {
            let frame = frame.map_err(|e| safe_hyper_category("body", &e))?;
            if let Ok(data) = frame.into_data() {
                if bytes.len() + data.len() > 8192 {
                    return Err("stage=body, oversize_error_envelope".into());
                }
                bytes.extend_from_slice(&data);
            }
        }
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| "stage=json, malformed_error_envelope".to_owned())?;
        if status != 413 || value["error"]["code"] != "request_too_large" {
            return Err(format!(
                "stage=response, status={}, expected_error_code={}",
                status.as_u16(),
                value["error"]["code"] == "request_too_large"
            ));
        }
        Ok::<_, String>(())
    })
    .await
    .map_err(|_| "stage=deadline".to_owned())
    .and_then(|r| r);
    driver.abort();
    assert_eq!(h.observed.phase.load(Ordering::SeqCst), 0);
    h.close().await;
    assert!(
        outcome.is_ok(),
        "eager upload lost its exact 413: {}",
        outcome.unwrap_err()
    );
}

#[tokio::test]
async fn rejected_upload_discards_pipelined_followup_without_dispatch() {
    let h = Harness::new(Mode::Success).await;
    let socket = TcpStream::connect(h.address).await.unwrap();
    let (mut read, mut write) = socket.into_split();
    let first = format!(
        "POST /v1/chat/completions HTTP/1.1\r\nHost: {}\r\nAuthorization: {}\r\nContent-Type: application/json\r\nContent-Length: 1048577\r\n\r\n",
        h.address,
        h.bearer.to_str().unwrap()
    );
    let body=json!({"model":"fixture","messages":[{"role":"user","content":"must not dispatch"}],"max_tokens":1}).to_string();
    let second = format!(
        "POST /v1/chat/completions HTTP/1.1\r\nHost: {}\r\nAuthorization: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        h.address,
        h.bearer.to_str().unwrap(),
        body.len(),
        body
    );
    let writer = tokio::spawn(async move {
        write.write_all(first.as_bytes()).await?;
        write.write_all(&vec![b' '; 1048577]).await?;
        write.write_all(second.as_bytes()).await?;
        write.shutdown().await
    });
    let mut wire = Vec::new();
    tokio::time::timeout(Duration::from_secs(2), read.read_to_end(&mut wire))
        .await
        .unwrap()
        .unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(2), writer)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        wire.windows(9)
            .filter(|bytes| *bytes == b"HTTP/1.1 ")
            .count(),
        1
    );
    let (status, headers, body) = decode(&wire);
    assert_eq!(status, 413);
    assert!(headers.to_ascii_lowercase().contains("connection: close"));
    let value: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(value["error"]["code"], "request_too_large");
    assert_eq!(h.observed.phase.load(Ordering::SeqCst), 0);
    drop(read);
    h.close().await;
}
#[tokio::test]
async fn huge_declared_body_silent_peer_does_not_hold_shutdown_beyond_linger_bound() {
    let h = Harness::new(Mode::Success).await;
    let mut socket = TcpStream::connect(h.address).await.unwrap();
    let headers = format!(
        "POST /v1/chat/completions HTTP/1.1\r\nHost: {}\r\nAuthorization: {}\r\nContent-Length: 1073741824\r\n\r\n",
        h.address,
        h.bearer.to_str().unwrap()
    );
    socket.write_all(headers.as_bytes()).await.unwrap();
    let mut wire = Vec::new();
    tokio::time::timeout(Duration::from_secs(2), socket.read_to_end(&mut wire))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(decode(&wire).0, 413);
    let started = Instant::now();
    h.close().await;
    assert!(started.elapsed() < Duration::from_secs(2));
    drop(socket);
}
