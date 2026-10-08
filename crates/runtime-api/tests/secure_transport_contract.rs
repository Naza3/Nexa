//! Actual TCP/HTTP/1 transport contracts with a deterministic protocol executor.
//! These synthetic events are NOT evidence of model inference quality.
use axum::http::HeaderValue;
use runtime_api::{
    ApiState, Config, router, security::SecurityContext, token::SecretToken, transport,
};
use runtime_core::{
    CancellationHandle, ExecutionEvents, Executor, ExecutorCommand, ExecutorEvent, Runtime,
};
use runtime_types::{
    ErrorCode, FinishReason, ModelId, ModelState, ResolvedModel, RuntimeError, Usage,
};
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
#[derive(Clone, Copy, Debug)]
enum Mode {
    Success,
    Length,
    BeforeError,
    AfterError,
    LoadWait,
    PrepareWait,
    StartedWait,
    Long,
    Overflow,
}
#[derive(Clone, Copy, Debug)]
struct DisconnectCase {
    rst: bool,
    mode: Mode,
    stream: bool,
    phase: usize,
}
impl std::fmt::Display for DisconnectCase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "rst={}, mode={:?}, stream={}, phase={}",
            self.rst, self.mode, self.stream, self.phase
        )
    }
}
#[derive(Default)]
struct Observed {
    phase: AtomicUsize,
    cancel_received: AtomicBool,
    hold_cancel_completion: AtomicBool,
    cancelled: AtomicBool,
    events: Mutex<Option<ExecutionEvents>>,
    request: Mutex<Option<runtime_types::GenerationRequest>>,
    peak: AtomicUsize,
    resolutions: AtomicUsize,
    loads: AtomicUsize,
    unloads: AtomicUsize,
}
struct CancelCompletionGate(Arc<Observed>);
impl CancelCompletionGate {
    fn new(observed: Arc<Observed>) -> Self {
        observed
            .hold_cancel_completion
            .store(true, Ordering::SeqCst);
        Self(observed)
    }
}
impl Drop for CancelCompletionGate {
    fn drop(&mut self) {
        self.0.hold_cancel_completion.store(false, Ordering::SeqCst);
    }
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
                observed.cancel_received.store(true, Ordering::SeqCst);
                while observed.hold_cancel_completion.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(2));
                }
                observed.cancelled.store(true, Ordering::SeqCst);
            };
            match command {
                ExecutorCommand::Load { .. } => {
                    observed.loads.fetch_add(1, Ordering::SeqCst);
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
                    observed.unloads.fetch_add(1, Ordering::SeqCst);
                    events.emit(ExecutorEvent::Unloaded);
                }
                ExecutorCommand::Generate { request } => {
                    *observed.request.lock().unwrap() = Some(request);
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
                            timings: Some(runtime_types::InferenceTimings {
                                prepare_us: 100,
                                prefill_us: 3000,
                                decode_us: 2000,
                                output_callback_us: 400,
                            }),
                            usage,
                            finish_reason: if matches!(mode, Mode::Length) {
                                FinishReason::Length
                            } else {
                                FinishReason::Stop
                            },
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
    authority: SocketAddr,
    bearer: HeaderValue,
    instance_id: uuid::Uuid,
    observed: Arc<Observed>,
    server: tokio::task::JoinHandle<std::io::Result<()>>,
}
impl Harness {
    async fn new(mode: Mode) -> Self {
        Self::with_lan(mode, false).await
    }
    async fn with_lan(mode: Mode, lan: bool) -> Self {
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
        let resolved = observed.clone();
        let runtime = Runtime::spawn(
            config.runtime_config(),
            move |id: &ModelId| {
                resolved.resolutions.fetch_add(1, Ordering::SeqCst);
                if id.as_str() == "missing-model" {
                    return Err(RuntimeError::new(
                        ErrorCode::ModelNotFound,
                        "fixture missing",
                    ));
                }
                Ok(ResolvedModel {
                    id: id.clone(),
                    path: "synthetic-not-read".into(),
                    projector_path: None,
                    context_limit: 4096,
                    default_context: 2048,
                    loadable: true,
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
        let instance_id = uuid::Uuid::new_v4();
        let security = Arc::new(SecurityContext::new(token, instance_id, address, vec![]).unwrap());
        let (app, authority) = if lan {
            // Test-only transport adapter: the real TCP connection stays on
            // loopback, while the independent LAN middleware sees private socket
            // fixtures. Production transport has no peer override or unsafe flag.
            let config = runtime_api::LanApiConfig {
                enabled: true,
                listen: Some("192.168.10.2:18081".parse().unwrap()),
                allowed_cidrs: vec!["192.168.10.3/32".into()],
            };
            let authority = config.listen.unwrap();
            let lan_token = runtime_api::token::SecretToken::generate().unwrap();
            // Use the same fixture authorization string for LAN harness callers.
            let key = lan_token.bearer_header_value();
            let context =
                Arc::new(runtime_api::lan::LanSecurityContext::new(lan_token, &config).unwrap());
            let app = runtime_api::routes::lan_router(state.clone(), context)
                .layer(axum::middleware::from_fn(move |mut request: axum::extract::Request, next: axum::middleware::Next| async move {
                    request.extensions_mut().insert(runtime_api::security::PeerEndpoints { client: "192.168.10.3:25000".parse().unwrap(), server: authority });
                    next.run(request).await
                }));
            (Some((app, key)), authority)
        } else {
            (None, address)
        };
        let (app, bearer) = app.unwrap_or_else(|| (router(state.clone(), security), bearer));
        let shutdown = state.shutdown.clone();
        let server = tokio::spawn(transport::serve(listener, app, shutdown));
        Self {
            _root: root,
            state,
            address,
            authority,
            bearer,
            instance_id,
            observed,
            server,
        }
    }
    async fn send(&self, streaming: bool) -> TcpStream {
        self.send_with_policy(streaming, true).await
    }
    async fn send_with_policy(&self, streaming: bool, close: bool) -> TcpStream {
        let body = json!({"model":"fixture","messages":[{"role":"user","content":"protocol test"}],"stream":streaming,"max_tokens":128,"stream_options":if streaming{json!({"include_usage":true})}else{Value::Null}});
        let mut body = body.as_object().unwrap().clone();
        if !streaming {
            body.remove("stream_options");
        }
        let body = serde_json::to_string(&body).unwrap();
        self.send_body(&body, Some(self.bearer.to_str().unwrap()), close)
            .await
    }
    async fn send_body(&self, body: &str, bearer: Option<&str>, close: bool) -> TcpStream {
        let connection = if close { "Connection: close\r\n" } else { "" };
        let authorization =
            bearer.map_or_else(String::new, |value| format!("Authorization: {value}\r\n"));
        let mut socket = TcpStream::connect(self.address).await.unwrap();
        let request = format!(
            "POST /v1/chat/completions HTTP/1.1\r\nHost: {}\r\n{authorization}Content-Type: application/json\r\nContent-Length: {}\r\n{connection}\r\n{}",
            self.authority,
            body.len(),
            body
        );
        socket.write_all(request.as_bytes()).await.unwrap();
        socket
    }
    async fn reply_body(&self, body: &str, bearer: Option<&str>) -> (u16, String, String) {
        let mut socket = self.send_body(body, bearer, true).await;
        let mut wire = Vec::new();
        tokio::time::timeout(Duration::from_secs(5), socket.read_to_end(&mut wire))
            .await
            .unwrap()
            .unwrap();
        decode(&wire)
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
    async fn disconnected(&self, case: DisconnectCase) {
        let mut last = None;
        let completed = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let status = self.state.control(|r| r.status()).await.unwrap();
                let bytes = self
                    .observed
                    .events
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map_or(0, ExecutionEvents::buffered_bytes);
                let cancelled = self.observed.cancelled.load(Ordering::SeqCst);
                last = Some((
                    status.state,
                    status.active_request.is_some(),
                    status.queued_jobs,
                    bytes,
                    cancelled,
                    self.observed.phase.load(Ordering::SeqCst),
                ));
                if cancelled
                    && status.active_request.is_none()
                    && status.queued_jobs == 0
                    && bytes == 0
                    && !matches!(
                        status.state,
                        ModelState::Loading | ModelState::Generating | ModelState::Unloading
                    )
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .is_ok();
        assert!(
            completed,
            "disconnect cleanup exceeded 3s: {case}, last(state,active,queued,bytes,cancelled,phase)={last:?}"
        );
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
#[tokio::test]
async fn disconnect_wait_does_not_accept_a_still_loading_request() {
    let h = Harness::new(Mode::LoadWait).await;
    let gate = CancelCompletionGate::new(h.observed.clone());
    let socket = h.send_with_policy(true, false).await;
    h.phase(1).await;
    drop(socket);
    tokio::time::timeout(Duration::from_secs(3), async {
        while !h.observed.cancel_received.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("load executor must receive disconnect cancellation");
    let status = h.state.control(|r| r.status()).await.unwrap();
    assert_eq!(status.state, ModelState::Loading);
    assert!(status.active_request.is_none());
    assert!(h.observed.events.lock().unwrap().is_none());
    assert!(!h.observed.cancelled.load(Ordering::SeqCst));

    let completed_while_loading = {
        let wait = h.disconnected(DisconnectCase {
            rst: false,
            mode: Mode::LoadWait,
            stream: true,
            phase: 1,
        });
        tokio::pin!(wait);
        // Core has removed the active job, but the executor cannot finish Load
        // until the gate opens. Empty request/budget fields are not cleanup.
        let early = tokio::time::timeout(Duration::from_millis(100), &mut wait)
            .await
            .is_ok();
        drop(gate);
        if !early {
            wait.await;
        }
        early
    };
    h.close().await;
    assert!(
        !completed_while_loading,
        "disconnect wait accepted Loading with active_request=None, no generation events, and cancelled=false"
    );
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
    let (status, _, body) = lan_raw(
        &h,
        "GET",
        "/runtime/performance",
        Some(h.bearer.to_str().unwrap()),
        "",
    )
    .await;
    assert_eq!(status, 200);
    let history: runtime_api::performance::PerformanceSnapshot =
        serde_json::from_str(&body).unwrap();
    assert!(history.is_valid());
    assert_eq!(history.instance_id, h.instance_id);
    assert_eq!(history.records.len(), 2);
    assert!(history.records.iter().all(|record| {
        record.status == runtime_types::PerformanceStatus::Completed
            && record.modality == runtime_types::PerformanceModality::Text
            && record.performance.unwrap().timings.prefill_us == 3000
            && record.performance.unwrap().load_options.context_size == 2048
    }));
    assert!(!body.contains("protocol test"));
    assert!(!body.contains("你好"));
    assert_eq!(
        lan_raw(&h, "GET", "/runtime/performance", None, "").await.0,
        401
    );
    h.clean().await;
    h.close().await;
}

// Fixed dsh/pi-ai text profile. The HTTP executor is synthetic; this does not
// claim a real model, the DSH adapter, or an agent tool loop was executed.
const HARNESS_TEXT: &str = include_str!("../../../examples/harness/fixtures/text-request.json");

const OCR_PIXEL: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/lX8AAAAASUVORK5CYII=";
fn image_request(stream: bool) -> Value {
    json!({"model":"fixture","messages":[{"role":"user","content":[
        {"type":"image_url","image_url":{"url":OCR_PIXEL}},
        {"type":"text","text":"Text Recognition:"}
    ]}],"stream":stream,"max_tokens":128})
}

#[tokio::test]
async fn local_image_reaches_actor_unchanged_with_existing_json_and_sse_contracts() {
    let h = Harness::new(Mode::Success).await;
    for stream in [false, true] {
        let (status, _, body) = h
            .reply_body(
                &image_request(stream).to_string(),
                Some(h.bearer.to_str().unwrap()),
            )
            .await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body.contains("data: [DONE]"), stream);
        let request = h.observed.request.lock().unwrap().clone().unwrap();
        assert_eq!(request.messages[0].content, "Text Recognition:");
        assert_eq!(
            request.messages[0].image.as_ref().unwrap().data_url(),
            OCR_PIXEL
        );
        assert!(!request.messages[0].image.as_ref().unwrap().after_text);
        h.clean().await;
    }
    let (_, _, body) = lan_raw(
        &h,
        "GET",
        "/runtime/performance",
        Some(h.bearer.to_str().unwrap()),
        "",
    )
    .await;
    let history: runtime_api::performance::PerformanceSnapshot =
        serde_json::from_str(&body).unwrap();
    assert!(history.is_valid());
    assert_eq!(history.records.len(), 2);
    assert!(
        history
            .records
            .iter()
            .all(|record| record.modality == runtime_types::PerformanceModality::Image)
    );
    assert!(!body.contains("data:image"));
    assert!(!body.contains("Text Recognition"));
    h.close().await;
}

#[tokio::test]
async fn local_image_envelope_limit_does_not_expand_text_or_lan_permissions() {
    // Trailing whitespace makes an otherwise valid image envelope exceed the
    // text cap without manufacturing a decodable oversized image fixture.
    let h = Harness::new(Mode::Success).await;
    let mut image = image_request(false).to_string();
    image.push_str(&" ".repeat(1024 * 1024));
    let (status, _, body) = h.reply_body(&image, Some(h.bearer.to_str().unwrap())).await;
    assert_eq!(status, 200, "{body}");
    h.clean().await;
    let mut text =
        json!({"model":"fixture","messages":[{"role":"user","content":"hello"}]}).to_string();
    text.push_str(&" ".repeat(1024 * 1024));
    assert_eq!(
        h.reply_body(&text, Some(h.bearer.to_str().unwrap()))
            .await
            .0,
        413
    );
    h.close().await;
    let h = Harness::with_lan(Mode::Success, true).await;
    let (status, _, body) = h
        .reply_body(
            &image_request(false).to_string(),
            Some(h.bearer.to_str().unwrap()),
        )
        .await;
    assert_eq!(status, 400, "{body}");
    assert!(body.contains("unsupported_parameter"));
    assert_eq!(h.observed.loads.load(Ordering::SeqCst), 0);
    assert!(h.observed.request.lock().unwrap().is_none());
    h.close().await;
}

#[tokio::test]
#[ignore = "requires the isolated examples/harness/client npm lock and NEXA_PI_AI_ROOT"]
async fn harness_official_pi_ai_consumes_actual_nexa_http() {
    let client_root = std::env::var_os("NEXA_PI_AI_ROOT")
        .expect("set NEXA_PI_AI_ROOT to the isolated client directory");
    let h = Harness::new(Mode::Success).await;
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/harness/verify-pi-ai.mjs");
    let endpoint = format!("http://{}/v1", h.address);
    let token = h
        .bearer
        .to_str()
        .unwrap()
        .strip_prefix("Bearer ")
        .unwrap()
        .to_owned();
    let output = tokio::task::spawn_blocking(move || {
        let mut command = std::process::Command::new("node");
        command.env_clear();
        for key in ["PATH", "SystemRoot", "SYSTEMROOT", "WINDIR", "TEMP", "TMP"] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command.env("NEXA_TEST_TOKEN", token);
        command.arg(script).arg(client_root).arg(endpoint).output()
    })
    .await
    .unwrap()
    .expect("run the pinned official client with existing Node.js");
    assert!(
        output.status.success(),
        "official client failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["mode"], "pi-ai-to-nexa-synthetic-executor");
    assert_eq!(report["version"], "0.87.1");
    assert_eq!(report["outcomes"][0]["terminal"], "done");
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    let request = h.observed.request.lock().unwrap().clone().unwrap();
    let expected: Value = serde_json::from_str(HARNESS_TEXT).unwrap();
    assert_eq!(
        serde_json::to_value(request.messages).unwrap(),
        expected["messages"]
    );
    h.clean().await;
    h.close().await;
}

#[tokio::test]
async fn harness_text_wire_preserves_messages_and_stop_length_usage_tails() {
    for (mode, finish) in [(Mode::Success, "stop"), (Mode::Length, "length")] {
        let h = Harness::new(mode).await;
        for include_usage in [true, false] {
            let mut request: Value = serde_json::from_str(HARNESS_TEXT).unwrap();
            if !include_usage {
                request.as_object_mut().unwrap().remove("stream_options");
            }
            let (status, headers, body) = h
                .reply_body(&request.to_string(), Some(h.bearer.to_str().unwrap()))
                .await;
            assert_eq!(status, 200);
            assert!(
                headers
                    .to_ascii_lowercase()
                    .contains("content-type: text/event-stream")
            );
            assert!(headers.to_ascii_lowercase().contains("x-request-id:"));
            let frames: Vec<_> = body.split("\n\n").filter(|part| !part.is_empty()).collect();
            assert_eq!(frames.len(), if include_usage { 5 } else { 4 });
            assert_eq!(frames.last(), Some(&"data: [DONE]"));
            assert_eq!(body.matches("[DONE]").count(), 1);
            let chunks: Vec<Value> = frames[..frames.len() - 1]
                .iter()
                .map(|frame| serde_json::from_str(frame.strip_prefix("data: ").unwrap()).unwrap())
                .collect();
            assert_eq!(chunks[0]["choices"][0]["delta"]["role"], "assistant");
            assert!(chunks[0]["choices"][0]["finish_reason"].is_null());
            assert_eq!(chunks[1]["choices"][0]["delta"]["content"], "你好🙂\n\"\\");
            assert!(chunks[1]["choices"][0]["finish_reason"].is_null());
            assert_eq!(chunks[2]["choices"][0]["finish_reason"], finish);
            assert_eq!(chunks[2]["choices"][0]["delta"], json!({}));
            for chunk in &chunks {
                assert_eq!(chunk["id"], chunks[0]["id"]);
                assert_eq!(chunk["created"], chunks[0]["created"]);
                assert_eq!(chunk["model"], "fixture");
                assert_eq!(chunk["object"], "chat.completion.chunk");
            }
            if include_usage {
                assert_eq!(chunks[3]["choices"], json!([]));
                assert_eq!(
                    chunks[3]["usage"],
                    json!({"prompt_tokens":3,"completion_tokens":1,"total_tokens":4})
                );
            } else {
                assert!(chunks.iter().all(|chunk| chunk.get("usage").is_none()));
            }
            let observed = h.observed.request.lock().unwrap().clone().unwrap();
            assert_eq!(
                serde_json::to_value(observed.messages).unwrap(),
                request["messages"]
            );
            assert_eq!(observed.options.max_tokens, 128);
            assert_eq!(observed.options.temperature, 0.0);
            h.clean().await;
        }
        h.close().await;
    }
}

#[tokio::test]
async fn harness_text_wire_auth_model_and_unsupported_fields_fail_before_inference() {
    let h = Harness::new(Mode::Success).await;
    for bearer in [None, Some("Bearer not-a-runtime-token")] {
        let (status, _, body) = h.reply_body(HARNESS_TEXT, bearer).await;
        assert_eq!(status, 401);
        let body: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["error"]["code"], "invalid_api_key");
        assert!(body["error"]["message"].is_string());
        assert_eq!(body["error"]["type"], "invalid_request_error");
    }
    let cases = [
        (
            "/model",
            json!("missing-model"),
            404,
            "model_not_found",
            "model",
        ),
        (
            "/store",
            json!(false),
            400,
            "unsupported_parameter",
            "store",
        ),
        ("/tools", json!([]), 400, "unsupported_parameter", "tools"),
        (
            "/tools",
            json!([{"type":"function","function":{"name":"noop","parameters":{"type":"object"}}}]),
            400,
            "unsupported_parameter",
            "tools",
        ),
        (
            "/messages/1/content",
            json!([{"type":"text","text":"hello"}]),
            400,
            "unsupported_parameter",
            "messages.1.content",
        ),
        (
            "/messages/1/content",
            Value::Null,
            400,
            "unsupported_parameter",
            "messages.1.content",
        ),
        (
            "/messages/0/role",
            json!("developer"),
            400,
            "unsupported_parameter",
            "messages.0.role",
        ),
        ("/surprise", json!(true), 400, "invalid_request", "surprise"),
    ];
    for (pointer, value, expected_status, code, param) in cases {
        let mut request: Value = serde_json::from_str(HARNESS_TEXT).unwrap();
        if let Some(target) = request.pointer_mut(pointer) {
            *target = value;
        } else {
            request
                .as_object_mut()
                .unwrap()
                .insert(pointer.trim_start_matches('/').into(), value);
        }
        let (status, headers, body) = h
            .reply_body(&request.to_string(), Some(h.bearer.to_str().unwrap()))
            .await;
        assert_eq!(status, expected_status, "{pointer}: {body}");
        assert!(
            headers
                .to_ascii_lowercase()
                .contains("content-type: application/json")
        );
        assert!(headers.to_ascii_lowercase().contains("x-request-id:"));
        assert!(!body.contains("data:"));
        let body: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["error"]["code"], code);
        assert_eq!(body["error"]["param"], param);
        assert!(body["error"]["message"].is_string());
        assert_eq!(body["error"]["type"], "invalid_request_error");
    }
    assert_eq!(h.observed.phase.load(Ordering::SeqCst), 0);
    assert!(h.observed.request.lock().unwrap().is_none());
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
            let case = DisconnectCase {
                rst,
                mode,
                stream,
                phase,
            };
            eprintln!("disconnect case: {case}");
            let h = Harness::new(mode).await;
            let mut socket = h.send_with_policy(stream, false).await;
            h.phase(phase).await;
            if stream && phase == 4 {
                let mut headers = [0; 1024];
                let n = tokio::time::timeout(Duration::from_secs(2), socket.read(&mut headers))
                    .await
                    .unwrap()
                    .unwrap();
                assert!(n > 0, "stream response headers missing: {case}");
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
            h.disconnected(case).await;
            let (status, _, body) = h.reply(false).await;
            assert_eq!(status, 200, "next request must recover: {case}");
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
        "POST /v1/chat/completions HTTP/1.1\r\nHost: {}\r\nAuthorization: {}\r\nContent-Type: application/json\r\nContent-Length: 8388609\r\n\r\n",
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
            8388609
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
        "POST /v1/chat/completions HTTP/1.1\r\nHost: {}\r\nAuthorization: {}\r\nContent-Type: application/json\r\nContent-Length: 8388609\r\n\r\n",
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
        write.write_all(&vec![b' '; 8388609]).await?;
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

#[tokio::test]
async fn private_model_probe_fin_and_rst_cancel_real_waiting_json_routes() {
    for route in [
        "/runtime/model-test",
        "/runtime/load-and-test",
        "/runtime/load-if-unloaded",
    ] {
        for rst in [false, true] {
            let h = Harness::new(Mode::StartedWait).await;
            let options = h.state.config.load_options();
            if route == "/runtime/model-test" {
                h.state
                    .load(ModelId::new("fixture").unwrap(), options, false)
                    .await
                    .unwrap();
            }
            let body=json!({"model":"fixture","context_size":options.context_size,"threads":options.threads,"batch_size":options.batch_size}).to_string();
            let mut socket = TcpStream::connect(h.address).await.unwrap();
            let request = format!(
                "POST {route} HTTP/1.1\r\nHost: {}\r\nAuthorization: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                h.address,
                h.bearer.to_str().unwrap(),
                body.len()
            );
            socket.write_all(request.as_bytes()).await.unwrap();
            h.phase(4).await;
            let owned = h.observed.request.lock().unwrap().as_ref().unwrap().clone();
            assert_eq!(owned.options.max_tokens, 24);
            assert_eq!(owned.messages.len(), 1);
            if rst {
                socket2::SockRef::from(&socket)
                    .set_linger(Some(Duration::ZERO))
                    .unwrap();
                drop(socket);
            } else {
                socket.shutdown().await.unwrap();
                drop(socket);
            }
            h.disconnected(DisconnectCase {
                rst,
                mode: Mode::StartedWait,
                stream: false,
                phase: 4,
            })
            .await;
            // The fake resolver has no verified artifact/build scope, so no
            // observation from this synthetic executor may become a receipt.
            assert!(
                !h._root
                    .path()
                    .join("models/local-model-validation.json")
                    .exists()
            );
            let (status, _, _) = h.reply(false).await;
            assert_eq!(status, 200);
            h.clean().await;
            h.close().await;
        }
    }
}

async fn lan_raw(
    h: &Harness,
    method: &str,
    path: &str,
    authorization: Option<&str>,
    extra: &str,
) -> (u16, String, String) {
    let auth = authorization
        .map(|v| format!("Authorization: {v}\r\n"))
        .unwrap_or_default();
    let mut socket = TcpStream::connect(h.address).await.unwrap();
    socket
        .write_all(
            format!(
                "{method} {path} HTTP/1.1\r\nHost: {}\r\n{auth}{extra}Connection: close\r\n\r\n",
                h.authority
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut wire = Vec::new();
    tokio::time::timeout(Duration::from_secs(3), socket.read_to_end(&mut wire))
        .await
        .unwrap()
        .unwrap();
    decode(&wire)
}
#[tokio::test]
async fn lan_inference_requires_local_load_and_preserves_sse_and_nonstream_contracts() {
    let h = Harness::with_lan(Mode::Success, true).await;
    let key = h.bearer.to_str().unwrap();
    let (code, _, body) = lan_raw(&h, "GET", "/v1/models", Some(key), "").await;
    assert_eq!(code, 200);
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap()["data"],
        json!([])
    );
    let (code, _, body) = h.reply(false).await;
    assert_eq!(code, 409);
    assert!(body.contains("model_not_loaded"));
    assert_eq!(h.observed.phase.load(Ordering::SeqCst), 0);
    h.state
        .control(|runtime| {
            runtime.load(
                ModelId::new("fixture").unwrap(),
                runtime_types::LoadOptions {
                    context_size: 2048,
                    ..Default::default()
                },
            )
        })
        .await
        .unwrap();
    let (code, _, body) = lan_raw(&h, "GET", "/v1/models", Some(key), "").await;
    assert_eq!(code, 200);
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap()["data"][0]["id"],
        "fixture"
    );
    for streaming in [false, true] {
        let (code, headers, body) = h.reply(streaming).await;
        assert_eq!(code, 200, "{body}");
        assert!(!headers.contains("x-nexa-server-proof"));
        if streaming {
            assert!(body.contains("data: [DONE]"));
            assert!(body.contains("chat.completion.chunk"));
        } else {
            assert_eq!(
                serde_json::from_str::<Value>(&body).unwrap()["object"],
                "chat.completion"
            );
        }
    }
    h.state.control(|runtime| runtime.unload()).await.unwrap();
    let (code, _, body) = h.reply(false).await;
    assert_eq!(code, 409);
    assert!(body.contains("model_not_loaded"));
    let history = h
        .state
        .control(|runtime| runtime.performance())
        .await
        .unwrap();
    // Both LAN response modes share history; rejected requests do not.
    assert_eq!(history.records.len(), 2);
    assert!(
        history
            .records
            .iter()
            .all(|record| record.performance.is_some())
    );
    h.close().await;
}
#[tokio::test]
async fn lan_router_has_no_management_proof_or_ambient_authentication() {
    let h = Harness::with_lan(Mode::Success, true).await;
    let key = h.bearer.to_str().unwrap();
    for path in [
        "/healthz",
        "/runtime/status",
        "/runtime/performance",
        "/runtime/configuration",
        "/runtime/configuration/models/fixture",
        "/runtime/models",
        "/runtime/models/import",
        "/runtime/load",
        "/runtime/unload",
        "/runtime/shutdown",
        "/runtime/requests/00000000-0000-0000-0000-000000000000/cancel",
        "/runtime/load-and-test",
        "/runtime/model-test",
    ] {
        for method in ["GET", "POST", "PUT"] {
            let (code, headers, _) = lan_raw(&h, method, path, Some(key), "").await;
            assert_eq!(code, 404, "{method} {path}");
            assert!(!headers.contains("x-nexa-server-proof"));
        }
    }
    for auth in [None, Some("Bearer wrong")] {
        assert_eq!(lan_raw(&h, "GET", "/v1/models", auth, "").await.0, 401);
    }
    let local = SecretToken::generate().unwrap();
    assert_eq!(
        lan_raw(
            &h,
            "GET",
            "/v1/models",
            Some(local.bearer_header_value().to_str().unwrap()),
            ""
        )
        .await
        .0,
        401
    );
    assert_eq!(
        lan_raw(
            &h,
            "GET",
            "/v1/models?api_key=ignored",
            None,
            &format!("Cookie: {key}\r\n")
        )
        .await
        .0,
        401
    );
    for header in [
        "Origin: http://192.168.10.2:18081\r\n",
        "Forwarded: for=192.168.10.3\r\n",
        "X-Forwarded-For: 192.168.10.3\r\n",
        "X-Nexa-Server-Challenge: 0000000000000000000000000000000000000000000000000000000000000000\r\n",
        "X-Request-ID: 00000000-0000-0000-0000-000000000000\r\n",
    ] {
        let (code, headers, _) = lan_raw(&h, "GET", "/v1/models", Some(key), header).await;
        assert_eq!(code, 403);
        assert!(!headers.contains("x-nexa-server-proof"));
    }
    assert_eq!(h.observed.phase.load(Ordering::SeqCst), 0);
    h.close().await;
}
#[tokio::test]
async fn lan_real_fin_rst_cancel_preparing_sse_and_nonstream_and_release_budget() {
    for rst in [false, true] {
        for (mode, stream, phase) in [
            (Mode::PrepareWait, true, 2),
            (Mode::StartedWait, true, 4),
            (Mode::StartedWait, false, 4),
        ] {
            let h = Harness::with_lan(mode, true).await;
            h.state
                .control(|runtime| {
                    runtime.load(
                        ModelId::new("fixture").unwrap(),
                        runtime_types::LoadOptions {
                            context_size: 2048,
                            ..Default::default()
                        },
                    )
                })
                .await
                .unwrap();
            let mut socket = h.send_with_policy(stream, false).await;
            h.phase(phase).await;
            if stream && phase == 4 {
                let mut bytes = [0; 1024];
                assert!(socket.read(&mut bytes).await.unwrap() > 0);
            }
            if rst {
                socket2::SockRef::from(&socket)
                    .set_linger(Some(Duration::ZERO))
                    .unwrap();
            } else {
                socket.shutdown().await.unwrap();
            }
            drop(socket);
            h.disconnected(DisconnectCase {
                rst,
                mode,
                stream,
                phase,
            })
            .await;
            assert_eq!(h.reply(false).await.0, 200);
            h.clean().await;
            h.close().await;
        }
    }
}
#[tokio::test]
async fn lan_shutdown_cancels_active_and_queued_requests_and_closes_socket() {
    let h = Harness::with_lan(Mode::StartedWait, true).await;
    h.state
        .control(|runtime| {
            runtime.load(
                ModelId::new("fixture").unwrap(),
                runtime_types::LoadOptions {
                    context_size: 2048,
                    ..Default::default()
                },
            )
        })
        .await
        .unwrap();
    let mut active = h.send_with_policy(true, false).await;
    h.phase(4).await;
    let mut queued = h.send_with_policy(false, false).await;
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if h.state.control(|r| r.status()).await.unwrap().queued_jobs == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    h.state.shutdown.begin();
    h.state.shutdown.wait().await.unwrap();
    for socket in [&mut active, &mut queued] {
        let mut bytes = Vec::new();
        let _ = tokio::time::timeout(Duration::from_secs(3), socket.read_to_end(&mut bytes))
            .await
            .unwrap();
    }
    assert!(h.observed.cancelled.load(Ordering::SeqCst));
    h.close().await;
}
#[tokio::test]
async fn lan_body_limit_and_total_read_deadline_apply_before_inference() {
    let h = Harness::with_lan(Mode::Success, true).await;
    assert_eq!(
        lan_raw(
            &h,
            "GET",
            "/v1/models",
            Some(h.bearer.to_str().unwrap()),
            "Content-Length: 1048577\r\n"
        )
        .await
        .0,
        413
    );
    let mut socket = TcpStream::connect(h.address).await.unwrap();
    socket.write_all(format!("POST /v1/chat/completions HTTP/1.1\r\nHost: {}\r\nAuthorization: {}\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n{{",h.authority,h.bearer.to_str().unwrap()).as_bytes()).await.unwrap();
    let mut wire = Vec::new();
    tokio::time::timeout(
        runtime_api::lan::BODY_READ_TIMEOUT + Duration::from_secs(2),
        socket.read_to_end(&mut wire),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(decode(&wire).0, 408);
    assert_eq!(h.observed.phase.load(Ordering::SeqCst), 0);
    h.close().await;
}

fn current_chat_body(model: Option<&str>, streaming: bool) -> Value {
    let mut body =
        json!({"messages":[{"role":"user","content":"current model test"}],"stream":streaming});
    if let Some(model) = model {
        body["model"] = json!(model);
    }
    if streaming {
        body["stream_options"] = json!({"include_usage":true});
    }
    body
}

#[tokio::test]
async fn current_model_wire_binds_actual_id_for_all_sse_frames_and_nonstream_responses() {
    for lan in [false, true] {
        let h = Harness::with_lan(Mode::Success, lan).await;
        for actual in ["first-loaded", "second-loaded"] {
            h.state
                .control(move |runtime| {
                    runtime.load(
                        ModelId::new(actual).unwrap(),
                        runtime_types::LoadOptions::default(),
                    )
                })
                .await
                .unwrap();
            let resolutions = h.observed.resolutions.load(Ordering::SeqCst);
            let loads = h.observed.loads.load(Ordering::SeqCst);
            let unloads = h.observed.unloads.load(Ordering::SeqCst);
            for selector in [None, Some(""), Some(" \t\r\n\u{2003}")] {
                for streaming in [false, true] {
                    let request = current_chat_body(selector, streaming);
                    let (code, headers, body) = h
                        .reply_body(&request.to_string(), Some(h.bearer.to_str().unwrap()))
                        .await;
                    assert_eq!(code, 200, "lan={lan} model={selector:?}: {body}");
                    assert!(headers.to_ascii_lowercase().contains("x-request-id:"));
                    if streaming {
                        assert!(
                            headers
                                .to_ascii_lowercase()
                                .contains("content-type: text/event-stream")
                        );
                        let frames: Vec<_> = body
                            .split("\n\n")
                            .filter(|frame| !frame.is_empty())
                            .collect();
                        assert_eq!(frames.len(), 5);
                        assert_eq!(frames.last(), Some(&"data: [DONE]"));
                        for frame in &frames[..frames.len() - 1] {
                            let chunk: Value =
                                serde_json::from_str(frame.strip_prefix("data: ").unwrap())
                                    .unwrap();
                            assert_eq!(chunk["model"], actual);
                        }
                    } else {
                        assert_eq!(
                            serde_json::from_str::<Value>(&body).unwrap()["model"],
                            actual
                        );
                    }
                    h.clean().await;
                    let generated = h.observed.request.lock().unwrap().clone().unwrap();
                    assert_eq!(generated.model.as_str(), actual);
                    assert_eq!(
                        serde_json::to_value(generated.messages).unwrap(),
                        request["messages"]
                    );
                }
            }
            // Explicit IDs remain strict conflicts, with no fallback or switch.
            let (code, _, body) = h
                .reply_body(
                    &current_chat_body(Some("different-model"), false).to_string(),
                    Some(h.bearer.to_str().unwrap()),
                )
                .await;
            assert_eq!(code, 409, "{body}");
            assert_eq!(
                serde_json::from_str::<Value>(&body).unwrap()["error"]["code"],
                "model_conflict"
            );
            assert_eq!(h.observed.resolutions.load(Ordering::SeqCst), resolutions);
            assert_eq!(h.observed.loads.load(Ordering::SeqCst), loads);
            assert_eq!(h.observed.unloads.load(Ordering::SeqCst), unloads);
        }
        h.close().await;
    }
}

#[tokio::test]
async fn current_model_wire_fails_before_inference_when_unloaded_or_model_type_is_invalid() {
    for lan in [false, true] {
        let h = Harness::with_lan(Mode::Success, lan).await;
        for retained_selection in [false, true] {
            if retained_selection {
                h.state
                    .control(|runtime| {
                        runtime.load(
                            ModelId::new("fixture").unwrap(),
                            runtime_types::LoadOptions::default(),
                        )
                    })
                    .await
                    .unwrap();
                h.state.control(|runtime| runtime.unload()).await.unwrap();
            }
            let resolutions = h.observed.resolutions.load(Ordering::SeqCst);
            let loads = h.observed.loads.load(Ordering::SeqCst);
            for selector in [None, Some(""), Some(" \t\r\n")] {
                for streaming in [false, true] {
                    let (code, headers, body) = h
                        .reply_body(
                            &current_chat_body(selector, streaming).to_string(),
                            Some(h.bearer.to_str().unwrap()),
                        )
                        .await;
                    assert_eq!(code, 409, "{body}");
                    assert!(
                        headers
                            .to_ascii_lowercase()
                            .contains("content-type: application/json")
                    );
                    let body: Value = serde_json::from_str(&body).unwrap();
                    assert_eq!(body["error"]["code"], "model_not_loaded");
                    assert_eq!(body["error"]["param"], "model");
                }
            }
            for model in [
                Value::Null,
                json!(0),
                json!(false),
                json!([]),
                json!({}),
                json!(" fixture"),
                json!("fixture "),
                json!("../fixture"),
            ] {
                let mut request = current_chat_body(None, true);
                request["model"] = model;
                let (code, _, body) = h
                    .reply_body(&request.to_string(), Some(h.bearer.to_str().unwrap()))
                    .await;
                assert_eq!(code, 400, "{body}");
                let body: Value = serde_json::from_str(&body).unwrap();
                assert_eq!(body["error"]["code"], "invalid_request");
                assert_eq!(body["error"]["param"], "model");
            }
            assert_eq!(h.observed.resolutions.load(Ordering::SeqCst), resolutions);
            assert_eq!(h.observed.loads.load(Ordering::SeqCst), loads);
            assert!(h.observed.request.lock().unwrap().is_none());
        }
        h.close().await;
    }
}

#[tokio::test]
async fn current_model_wire_disconnect_cancels_active_work_without_loading() {
    for lan in [false, true] {
        for stream in [false, true] {
            let h = Harness::with_lan(Mode::StartedWait, lan).await;
            h.state
                .control(|runtime| {
                    runtime.load(
                        ModelId::new("fixture").unwrap(),
                        runtime_types::LoadOptions::default(),
                    )
                })
                .await
                .unwrap();
            let resolutions = h.observed.resolutions.load(Ordering::SeqCst);
            let socket = h
                .send_body(
                    &current_chat_body(None, stream).to_string(),
                    Some(h.bearer.to_str().unwrap()),
                    false,
                )
                .await;
            h.phase(4).await;
            drop(socket);
            h.disconnected(DisconnectCase {
                rst: false,
                mode: Mode::StartedWait,
                stream,
                phase: 4,
            })
            .await;
            assert_eq!(h.observed.resolutions.load(Ordering::SeqCst), resolutions);
            assert_eq!(h.observed.loads.load(Ordering::SeqCst), 1);
            h.close().await;
        }
    }
}
