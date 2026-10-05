//! Adversarial real TCP fixtures. These never stand in for model inference.
use desktop_bridge::{ChatEvent, ChatStartRequest, DesktopBridge};
use runtime_api::{
    Config,
    proof::{ProofContext, create_server_proof, decode_hex, encode_hex},
    token::{init_private_token, load_private_token, write_private_new},
};
use runtime_cli::instance::{Discovery, InstanceLock};
use runtime_types::{Message, Role};
use serde_json::json;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use uuid::Uuid;
#[derive(Clone, Copy)]
enum Mode {
    Valid,
    BadProof,
    Redirect,
    Eof,
    Huge,
    WrongId,
    NoUsage,
    Pending,
    StreamError,
    LargeValid,
    OverReply,
    OverHistory,
    PendingImport,
    SlowLoad,
    PendingPreparation,
    PendingChatPreparation,
    ProbeEvidenceError(&'static str),
    HeldLoadStart,
    LostLoadStart,
    InvalidLoadState,
}
struct Fixture {
    _temp: tempfile::TempDir,
    _lock: Option<InstanceLock>,
    bridge: Arc<DesktopBridge>,
    server: tokio::task::JoinHandle<()>,
    auth: Arc<AtomicUsize>,
    chats: Arc<AtomicUsize>,
    cancels: Arc<AtomicUsize>,
    disconnected: Arc<AtomicUsize>,
    gate: Arc<tokio::sync::Notify>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines().find_map(|s| {
        let (k, v) = s.split_once(':')?;
        k.eq_ignore_ascii_case(name).then(|| v.trim())
    })
}
async fn read_request(socket: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut b = [0u8; 1];
    while bytes.len() < 32 * 1024 {
        if socket.read_exact(&mut b).await.is_err() {
            return String::new();
        }
        bytes.push(b[0]);
        if bytes.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let head = String::from_utf8(bytes).unwrap();
    let size = header(&head, "content-length")
        .unwrap_or("0")
        .parse::<usize>()
        .unwrap();
    assert!(size <= 512 * 1024);
    let mut body = vec![0; size];
    let _ = socket.read_exact(&mut body).await;
    format!("{head}{}", String::from_utf8(body).unwrap())
}
fn chunk(id: &str, choices: serde_json::Value, usage: Option<serde_json::Value>) -> String {
    let mut value = json!({"id":format!("chatcmpl-{id}"),"object":"chat.completion.chunk","model":"a","created":1,"choices":choices});
    if let Some(usage) = usage {
        value["usage"] = usage;
    }
    format!("data: {value}\n\n")
}
impl Fixture {
    async fn new(mode: Mode) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private");
        init_private_token(&root).unwrap();
        write_private_new(
            &root.join("config.toml"),
            Config::default().to_toml().unwrap().as_bytes(),
        )
        .unwrap();
        let lock = InstanceLock::try_acquire(&root).unwrap().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let instance = Uuid::new_v4();
        lock.publish(&Discovery::current(instance, address).unwrap())
            .unwrap();
        let bridge = Arc::new(
            DesktopBridge::new(
                root.clone(),
                temp.path().join(if cfg!(windows) {
                    "ai-runtime.exe"
                } else {
                    "ai-runtime"
                }),
            )
            .unwrap(),
        );
        let token = Arc::new(load_private_token(&root).unwrap());
        let auth = Arc::new(AtomicUsize::new(0));
        let chats = Arc::new(AtomicUsize::new(0));
        let cancels = Arc::new(AtomicUsize::new(0));
        let a = auth.clone();
        let c = chats.clone();
        let cc = cancels.clone();
        let disconnected = Arc::new(AtomicUsize::new(0));
        let dc = disconnected.clone();
        let gate = Arc::new(tokio::sync::Notify::new());
        let g = gate.clone();
        let load_operation = Arc::new(std::sync::Mutex::new(None::<Uuid>));
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut socket, peer)) = listener.accept().await else {
                    break;
                };
                let token = token.clone();
                let a = a.clone();
                let c = c.clone();
                let cc = cc.clone();
                let dc = dc.clone();
                let g = g.clone();
                let load_operation = load_operation.clone();
                tokio::spawn(async move {
                    let first = read_request(&mut socket).await;
                    if first.is_empty() {
                        return;
                    }
                    assert!(first.starts_with("GET /healthz "));
                    assert!(header(&first, "authorization").is_none());
                    let nonce =
                        decode_hex::<32>(header(&first, "x-nexa-server-challenge").unwrap())
                            .unwrap();
                    let proof = create_server_proof(
                        &token,
                        &ProofContext {
                            instance_id: *instance.as_bytes(),
                            nonce,
                            client: peer,
                            server: address,
                        },
                    );
                    let proof = if matches!(mode, Mode::BadProof) {
                        "0".repeat(64)
                    } else {
                        encode_hex(&proof)
                    };
                    let proof_response = format!(
                        "HTTP/1.1 200 OK\r\nX-Nexa-Instance-ID: {instance}\r\nX-Nexa-Protocol-Version: 1\r\nX-Nexa-Server-Proof: {proof}\r\nCache-Control: no-store\r\nContent-Length: 2\r\n\r\n{{}}"
                    );
                    if socket.write_all(proof_response.as_bytes()).await.is_err() {
                        return;
                    }
                    let request = read_request(&mut socket).await;
                    if request.is_empty() {
                        return;
                    }
                    assert!(token.matches_authorization(
                        header(&request, "authorization").unwrap().as_bytes()
                    ));
                    a.fetch_add(1, Ordering::SeqCst);
                    if request.starts_with("POST /runtime/load-operations ") {
                        let body: serde_json::Value =
                            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1)
                                .unwrap();
                        let id = Uuid::parse_str(body["operation_id"].as_str().unwrap()).unwrap();
                        *load_operation.lock().unwrap() = Some(id);
                        c.fetch_add(1, Ordering::SeqCst);
                        if matches!(mode, Mode::LostLoadStart) {
                            return;
                        }
                        if matches!(mode, Mode::HeldLoadStart) {
                            let mut byte = [0];
                            let _ = socket.read(&mut byte).await;
                            return;
                        }
                        let body = json!({"operation_id":id}).to_string();
                        let _ = socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",body.len()).as_bytes()).await;
                        return;
                    }
                    if request.starts_with("POST /runtime/load-operations/") {
                        let id = load_operation.lock().unwrap().unwrap();
                        assert!(
                            request.starts_with(&format!(
                                "POST /runtime/load-operations/{id}/cancel "
                            ))
                        );
                        cc.fetch_add(1, Ordering::SeqCst);
                        let body = r#"{"stopping":true}"#;
                        let _ = socket
                            .write_all(
                                format!(
                                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                                    body.len()
                                )
                                .as_bytes(),
                            )
                            .await;
                        return;
                    }
                    if request.starts_with("GET /runtime/load-operations/") {
                        let id = *load_operation.lock().unwrap();
                        let Some(id) = id else {
                            let body = r#"{"error":{"code":"request_not_found"}}"#;
                            let _ = socket.write_all(format!("HTTP/1.1 404 Not Found\r\nContent-Length: {}\r\n\r\n{body}",body.len()).as_bytes()).await;
                            return;
                        };
                        assert!(
                            request.starts_with(&format!("GET /runtime/load-operations/{id} "))
                        );
                        let cancelled = cc.load(Ordering::SeqCst) > 0;
                        let invalid = !cancelled && matches!(mode, Mode::InvalidLoadState);
                        let body = json!({"operation_id":id,"model_id":"a","phase":if cancelled || invalid {"finished"}else{"loading"},"status":if cancelled {"cancelled"}else{"running"},"terminal":cancelled || invalid,"runtime":null,"local_validation":null,"error":if cancelled{json!({"code":"request_cancelled","message":"private fake text must not escape"})}else{json!(null)}}).to_string();
                        let _ = socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",body.len()).as_bytes()).await;
                        return;
                    }
                    if request.starts_with("POST /runtime/shutdown ") {
                        let body = r#"{"status":"stopped"}"#;
                        let _=socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",body.len()).as_bytes()).await;
                        return;
                    }
                    if request.starts_with("POST /runtime/model-test ") {
                        let error = match mode {
                            Mode::ProbeEvidenceError(code) => Some(code),
                            _ => None,
                        };
                        let body = json!({"state":if error.is_some() { "loaded" } else { "passed" }, "load_success":true, "generation_pass":error.is_none(), "checked_at_unix_ms":1, "error_code":error}).to_string();
                        let _ = socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()).as_bytes()).await;
                        return;
                    }
                    if request.starts_with("GET /runtime/status ")
                        || (request.starts_with("POST /runtime/load ")
                            || request.starts_with("POST /runtime/load-and-test "))
                    {
                        if request.starts_with("POST /runtime/load ")
                            || request.starts_with("POST /runtime/load-and-test ")
                        {
                            if matches!(mode, Mode::PendingPreparation) {
                                let mut byte = [0];
                                let _ = socket.read(&mut byte).await;
                                dc.fetch_add(1, Ordering::SeqCst);
                                return;
                            }
                            g.notified().await;
                            dc.fetch_add(1, Ordering::SeqCst);
                        }
                        let state =
                            if matches!(mode, Mode::SlowLoad) && dc.load(Ordering::SeqCst) == 0 {
                                "loading"
                            } else {
                                "unloaded"
                            };
                        let busy = matches!(
                            mode,
                            Mode::PendingPreparation | Mode::PendingChatPreparation
                        ) && dc.load(Ordering::SeqCst) == 0;
                        let body=json!({"local_validation":{"state":"deferred","load_success":true,"generation_pass":false,"checked_at_unix_ms":1,"error_code":"runtime_busy"},"state":state,"selected_model":null,"load_options":null,"active_request":null,"queued_jobs":0,"stopping":false,"registry_busy":busy,"configured_backend":"cpu","backend":null,"backend_observation":"unavailable","last_error":null,"threads_source":null,"available_parallelism":2,"threads_exceed_available_parallelism":null,"worker":{"pid":null,"sessions_started":0,"sessions_reaped":0},"memory":{"api_private_bytes":null,"worker_private_bytes":null,"gpu_bytes":null,"observation":"unavailable"}}).to_string();
                        let _ = socket
                            .write_all(
                                format!(
                                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                                    body.len()
                                )
                                .as_bytes(),
                            )
                            .await;
                        return;
                    }
                    if request.starts_with("POST /runtime/models/import ")
                        && matches!(mode, Mode::PendingImport)
                    {
                        let mut byte = [0];
                        let _ = socket.read(&mut byte).await;
                        dc.fetch_add(1, Ordering::SeqCst);
                        return;
                    }
                    if request.starts_with("POST /runtime/requests/") {
                        cc.fetch_add(1, Ordering::SeqCst);
                        let _=socket.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 38\r\n\r\n{\"error\":{\"code\":\"request_not_found\"}}").await;
                        return;
                    }
                    assert!(request.starts_with("POST /v1/chat/completions "));
                    c.fetch_add(1, Ordering::SeqCst);
                    if matches!(mode, Mode::Redirect) {
                        let _=socket.write_all(b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/stolen\r\nContent-Length: 2\r\n\r\n{}").await;
                        return;
                    }
                    if matches!(mode, Mode::Pending | Mode::PendingChatPreparation) {
                        let mut b = [0; 1];
                        let _ = socket.read(&mut b).await;
                        dc.fetch_add(1, Ordering::SeqCst);
                        return;
                    }
                    let actual = header(&request, "x-request-id").unwrap();
                    let id = if matches!(mode, Mode::WrongId) {
                        "other"
                    } else {
                        actual
                    };
                    let mut body = chunk(
                        id,
                        json!([{"index":0,"delta":{"role":"assistant"},"finish_reason":null}]),
                        None,
                    );
                    if matches!(mode, Mode::Huge) {
                        body = format!("data: {}\n\n", "x".repeat(33 * 1024));
                    } else if matches!(mode, Mode::StreamError) {
                        body.push_str("data: {\"error\":{\"code\":\"context_length_exceeded\",\"message\":\"MUST NOT LEAK secret/path\"}}\n\n");
                    } else {
                        if matches!(mode, Mode::LargeValid | Mode::OverReply | Mode::OverHistory) {
                            let count = if matches!(mode, Mode::OverReply) {
                                90
                            } else if matches!(mode, Mode::OverHistory) {
                                4
                            } else {
                                40
                            };
                            for _ in 0..count {
                                body.push_str(&chunk(id,json!([{"index":0,"delta":{"content":"文".repeat(1024)},"finish_reason":null}]),None));
                            }
                        } else {
                            body.push_str(&chunk(id,json!([{"index":0,"delta":{"content":"你好😀"},"finish_reason":null}]),None));
                        }
                        if !matches!(mode, Mode::Eof) {
                            body.push_str(&chunk(
                                id,
                                json!([{"index":0,"delta":{},"finish_reason":"stop"}]),
                                None,
                            ));
                            if !matches!(mode, Mode::NoUsage) {
                                body.push_str(&chunk(id,json!([]),Some(json!({"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}))));
                            }
                            body.push_str("data: [DONE]\n\n");
                        }
                    }
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(head.as_bytes()).await;
                    // Deliberately split all UTF-8 code points and JSON/SSE separators.
                    let transport_chunk =
                        if matches!(mode, Mode::LargeValid | Mode::OverReply | Mode::OverHistory) {
                            body.len()
                        } else {
                            17
                        };
                    for part in body.as_bytes().chunks(transport_chunk) {
                        if socket.write_all(part).await.is_err() {
                            return;
                        }
                    }
                });
            }
        });
        Self {
            _temp: temp,
            _lock: Some(lock),
            bridge,
            server,
            auth,
            chats,
            cancels,
            disconnected,
            gate,
        }
    }
    fn start(&self) -> Uuid {
        self.bridge
            .chat_start(ChatStartRequest {
                model_id: "a".into(),
                messages: vec![Message::new(Role::User, "hello")],
                max_output_tokens: 16,
            })
            .unwrap()
            .request_id
    }
    async fn terminal(&self, id: Uuid) -> Vec<ChatEvent> {
        tokio::time::timeout(Duration::from_secs(10), async {
            let mut events = vec![];
            loop {
                let batch = self.bridge.chat_next(id).await.unwrap();
                events.extend(batch.events);
                if batch.terminal {
                    break events;
                }
            }
        })
        .await
        .unwrap()
    }
}
#[tokio::test]
async fn valid_same_connection_unicode_usage_and_repeat_terminal() {
    let f = Fixture::new(Mode::Valid).await;
    let id = f.start();
    let events = f.terminal(id).await;
    assert_eq!(events[0], ChatEvent::Started);
    assert!(
        events
            .iter()
            .any(|e| matches!(e,ChatEvent::Delta{text} if text=="你好😀"))
    );
    assert!(matches!(events.last(),Some(ChatEvent::Completed{usage,..}) if usage.total_tokens==5));
    let again = f.bridge.chat_next(id).await.unwrap();
    assert_eq!(again.events, vec![events.last().unwrap().clone()]);
    assert_eq!(f.auth.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn proof_failure_never_releases_bearer() {
    let f = Fixture::new(Mode::BadProof).await;
    let id = f.start();
    let events = f.terminal(id).await;
    assert!(matches!(events.last(),Some(ChatEvent::Failed{code,..}) if code=="connection_failed"));
    assert_eq!(f.auth.load(Ordering::SeqCst), 0);
    assert_eq!(f.chats.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn redirect_eof_huge_frame_wrong_identity_and_missing_usage_never_succeed() {
    for mode in [
        Mode::Redirect,
        Mode::Eof,
        Mode::Huge,
        Mode::WrongId,
        Mode::NoUsage,
    ] {
        let f = Fixture::new(mode).await;
        let id = f.start();
        let events = f.terminal(id).await;
        assert!(matches!(events.last(), Some(ChatEvent::Failed { .. })));
        assert_eq!(f.chats.load(Ordering::SeqCst), 1);
    }
}
#[tokio::test]
async fn stream_error_is_controlled_and_keeps_partial_reply() {
    let f = Fixture::new(Mode::StreamError).await;
    let id = f.start();
    let events = f.terminal(id).await;
    let serialized = serde_json::to_string(&events).unwrap();
    assert!(!serialized.contains("MUST NOT LEAK"));
    assert!(
        matches!(events.last(),Some(ChatEvent::Failed{code,..}) if code=="context_length_exceeded")
    );
}
#[tokio::test]
async fn cancel_before_send_registers_once_and_sends_nothing() {
    let f = Fixture::new(Mode::Valid).await;
    let id = f.start();
    f.bridge.chat_cancel(id).await.unwrap();
    assert_eq!(f.terminal(id).await, vec![ChatEvent::Cancelled]);
    assert_eq!(f.auth.load(Ordering::SeqCst), 0);
    assert_eq!(f.chats.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn duplicate_send_consumer_and_foreign_cancel_rejected() {
    let f = Fixture::new(Mode::Pending).await;
    let id = f.start();
    let duplicate = f.bridge.chat_start(ChatStartRequest {
        model_id: "a".into(),
        messages: vec![Message::new(Role::User, "hello")],
        max_output_tokens: 1,
    });
    assert_eq!(duplicate.unwrap_err().code, "desktop_busy");
    assert_eq!(
        f.bridge.chat_cancel(Uuid::new_v4()).await.unwrap_err().code,
        "request_not_owned"
    );
    let b = f.bridge.clone();
    let next = tokio::spawn(async move { b.chat_next(id).await });
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        f.bridge.chat_next(id).await.unwrap_err().code,
        "consumer_busy"
    );
    f.bridge.chat_cancel(id).await.unwrap();
    let result = next.await.unwrap().unwrap();
    if !result.terminal {
        assert_eq!(f.terminal(id).await, vec![ChatEvent::Cancelled]);
    } else {
        assert_eq!(result.events, vec![ChatEvent::Cancelled]);
    }
    assert!(f.cancels.load(Ordering::SeqCst) <= 1);
}
#[tokio::test]
async fn close_cancels_only_owned_request_and_repeated_close_merges() {
    let f = Fixture::new(Mode::Pending).await;
    let id = f.start();
    tokio::time::timeout(Duration::from_secs(3), async {
        while f.chats.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let (a, b) = tokio::join!(f.bridge.close_ui_only(), f.bridge.close_ui_only());
    a.unwrap();
    b.unwrap();
    assert_eq!(
        f.bridge.chat_next(id).await.unwrap().events,
        vec![ChatEvent::Cancelled]
    );
    assert_eq!(f.chats.load(Ordering::SeqCst), 1);
    assert!(f.cancels.load(Ordering::SeqCst) <= 1);
}
#[tokio::test]
async fn serialized_and_message_bounds_are_enforced_before_network() {
    let f = Fixture::new(Mode::Valid).await;
    for text in ["x".repeat(512 * 1024), "\n".repeat(270 * 1024)] {
        let result = f.bridge.chat_start(ChatStartRequest {
            model_id: "a".into(),
            messages: vec![Message::new(Role::User, text)],
            max_output_tokens: 1,
        });
        assert_eq!(result.unwrap_err().code, "history_limit");
    }
    assert_eq!(f.auth.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn many_valid_events_in_one_transport_write_are_incrementally_consumed() {
    let f = Fixture::new(Mode::LargeValid).await;
    let id = f.start();
    let events = f.terminal(id).await;
    let bytes: usize = events
        .iter()
        .map(|e| {
            if let ChatEvent::Delta { text } = e {
                text.len()
            } else {
                0
            }
        })
        .sum();
    assert_eq!(bytes, 40 * 3 * 1024);
    assert!(matches!(events.last(), Some(ChatEvent::Completed { .. })));
}
#[tokio::test]
async fn response_limit_preserves_bounded_partial_and_fails() {
    let f = Fixture::new(Mode::OverReply).await;
    let id = f.start();
    let events = f.terminal(id).await;
    let bytes: usize = events
        .iter()
        .map(|e| {
            if let ChatEvent::Delta { text } = e {
                text.len()
            } else {
                0
            }
        })
        .sum();
    assert!(bytes <= 256 * 1024);
    assert!(bytes > 0);
    assert!(matches!(events.last(),Some(ChatEvent::Failed{code,..}) if code=="response_limit"));
}
#[tokio::test]
async fn history_limit_includes_current_reply() {
    let f = Fixture::new(Mode::OverHistory).await;
    let id = f
        .bridge
        .chat_start(ChatStartRequest {
            model_id: "a".into(),
            messages: vec![Message::new(Role::User, "x".repeat(510 * 1024))],
            max_output_tokens: 16,
        })
        .unwrap()
        .request_id;
    let events = f.terminal(id).await;
    assert!(matches!(events.last(),Some(ChatEvent::Failed{code,..}) if code=="history_limit"));
}
#[tokio::test]
async fn preferences_save_is_success_even_when_existing_instance_proof_fails() {
    let f = Fixture::new(Mode::BadProof).await;
    let root = f._temp.path().join("private");
    let original = std::fs::read(root.join("config.toml")).unwrap();
    let token = std::fs::read(root.join("secrets/api-token")).unwrap();
    let snapshot = f
        .bridge
        .settings_save(desktop_bridge::DesktopPreferences {
            threads: 3,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(matches!(
        snapshot.connection,
        desktop_bridge::ConnectionState::Error
    ));
    assert_eq!(snapshot.settings.threads, 3);
    assert_eq!(std::fs::read(root.join("config.toml")).unwrap(), original);
    assert_eq!(
        std::fs::read(root.join("secrets/api-token")).unwrap(),
        token
    );
    assert_eq!(
        f.bridge.start(false).await.unwrap_err().code,
        "connection_failed"
    );
    assert_eq!(f.auth.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn close_disconnects_only_this_import_and_observes_registry_idle() {
    let f = Fixture::new(Mode::PendingImport).await;
    let file = f._temp.path().join("selected.gguf");
    std::fs::write(&file, b"fixture").unwrap();
    let b = f.bridge.clone();
    let import = tokio::spawn(async move { b.import_model(file, "a".into()).await });
    while f.auth.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    f.bridge.close().await.unwrap();
    assert_eq!(
        import.await.unwrap().unwrap_err().code,
        "import_interrupted"
    );
    tokio::time::timeout(Duration::from_secs(1), async {
        while f.disconnected.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(f.cancels.load(Ordering::SeqCst), 0);
    assert_eq!(f.chats.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn close_waits_for_control_and_keeps_window_open_after_bounded_failure() {
    let f = Fixture::new(Mode::SlowLoad).await;
    let b = f.bridge.clone();
    let load = tokio::spawn(async move {
        b.load_model(desktop_bridge::LoadModelRequest {
            model_id: "a".into(),
            context_size: 2048,
            threads: 2,
            batch_size: 128,
        })
        .await
    });
    while f.auth.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    assert_eq!(f.bridge.close().await.unwrap_err().code, "desktop_busy");
    f.gate.notify_one();
    assert_eq!(
        load.await.unwrap().unwrap_err().code,
        "model_load_interrupted"
    );
    tokio::time::timeout(Duration::from_secs(1), async {
        while f.disconnected.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(matches!(
        f.bridge.snapshot().await.unwrap().connection,
        desktop_bridge::ConnectionState::Connected
    ));
    f.bridge.close_ui_only().await.unwrap();
}

#[tokio::test]
async fn absent_frontend_consumption_disconnects_stream_and_sends_owned_cancel() {
    let f = Fixture::new(Mode::LargeValid).await;
    let id = f.start();
    // Deliberately never call chat_next until the producer's real 10-second
    // deadline has fired. Wait for the observable cancel, not a sleep oracle.
    tokio::time::timeout(Duration::from_secs(15), async {
        while f.cancels.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    let events = f.terminal(id).await;
    let bytes: usize = events
        .iter()
        .map(|event| match event {
            ChatEvent::Delta { text } => text.len(),
            _ => 0,
        })
        .sum();
    assert!(bytes > 0 && bytes <= 64 * 1024);
    assert!(
        matches!(events.last(), Some(ChatEvent::Failed { code, .. }) if code == "slow_consumer")
    );
    assert_eq!(f.chats.load(Ordering::SeqCst), 1);
    assert_eq!(f.cancels.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn close_disconnects_owned_load_preparation_and_waits_for_registry_release() {
    let f = Fixture::new(Mode::PendingPreparation).await;
    let bridge = f.bridge.clone();
    let load = tokio::spawn(async move {
        bridge
            .load_model(desktop_bridge::LoadModelRequest {
                model_id: "a".into(),
                context_size: 2048,
                threads: 2,
                batch_size: 128,
            })
            .await
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        while f.auth.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    f.bridge.close().await.unwrap();
    assert_eq!(
        load.await.unwrap().unwrap_err().code,
        "model_load_interrupted"
    );
    assert_eq!(f.disconnected.load(Ordering::SeqCst), 1);
    assert_eq!(f.cancels.load(Ordering::SeqCst), 0);
    assert_eq!(f.chats.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn close_during_chat_preparation_drops_stream_and_waits_for_registry_release() {
    let f = Fixture::new(Mode::PendingChatPreparation).await;
    let id = f.start();
    tokio::time::timeout(Duration::from_secs(1), async {
        while f.chats.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    f.bridge.close().await.unwrap();
    assert_eq!(f.disconnected.load(Ordering::SeqCst), 1);
    assert!(f.bridge.chat_next(id).await.unwrap().terminal);
    assert_eq!(f.chats.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn manual_test_does_not_hide_evidence_errors_in_successful_http_responses() {
    for code in [
        "validation_record_unavailable",
        "validation_record_read_failed",
        "validation_record_write_failed",
        "validation_engine_unavailable",
        "validation_scope_unavailable",
        "validation_scope_changed",
    ] {
        let fixture = Fixture::new(Mode::ProbeEvidenceError(code)).await;
        let error = fixture
            .bridge
            .model_test(desktop_bridge::LoadModelRequest {
                model_id: "a".into(),
                context_size: 2048,
                threads: 2,
                batch_size: 128,
            })
            .await
            .unwrap_err();
        assert_eq!(error.code, code);
        assert_eq!(fixture.auth.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.chats.load(Ordering::SeqCst), 0);
    }
    let fixture = Fixture::new(Mode::Valid).await;
    let result = fixture
        .bridge
        .model_test(desktop_bridge::LoadModelRequest {
            model_id: "a".into(),
            context_size: 2048,
            threads: 2,
            batch_size: 128,
        })
        .await
        .unwrap();
    assert_eq!(
        result.state,
        model_store::local_validation::ValidationState::Passed
    );
    assert!(result.load_success && result.generation_pass && result.error_code.is_none());
}

#[tokio::test]
async fn corrupt_configuration_does_not_prevent_proved_stop_or_get_rewritten() {
    let mut f = Fixture::new(Mode::Valid).await;
    let root = f._temp.path().join("private");
    let token = std::fs::read(root.join("secrets/api-token")).unwrap();
    std::fs::write(root.join("config.toml"), b"damaged configuration").unwrap();
    let bridge = f.bridge.clone();
    let stopping = tokio::spawn(async move { bridge.stop().await });
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while f.auth.load(Ordering::SeqCst) == 0 {
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let lock = f._lock.take().unwrap();
    lock.remove_own(Discovery::read(&root).unwrap().instance_id)
        .unwrap();
    drop(lock);
    assert!(stopping.await.unwrap().unwrap().stopped);
    assert_eq!(
        std::fs::read(root.join("config.toml")).unwrap(),
        b"damaged configuration"
    );
    assert_eq!(
        std::fs::read(root.join("secrets/api-token")).unwrap(),
        token
    );
}
#[tokio::test]
async fn corrupt_configuration_never_allows_stop_without_server_proof_or_valid_token() {
    let f = Fixture::new(Mode::BadProof).await;
    let root = f._temp.path().join("private");
    std::fs::write(root.join("config.toml"), b"damaged configuration").unwrap();
    let error = f.bridge.stop().await.unwrap_err();
    assert_ne!(error.code, "configuration_invalid");
    assert_eq!(f.auth.load(Ordering::SeqCst), 0);
    assert!(InstanceLock::try_acquire(&root).unwrap().is_none());
    std::fs::write(root.join("secrets/api-token"), b"invalid token").unwrap();
    assert_eq!(
        f.bridge.stop().await.unwrap_err().code,
        "credentials_unavailable"
    );
    assert_eq!(f.auth.load(Ordering::SeqCst), 0);
    assert_eq!(
        std::fs::read(root.join("config.toml")).unwrap(),
        b"damaged configuration"
    );
}

fn load_request(id: Uuid) -> desktop_bridge::ModelLoadStartRequest {
    desktop_bridge::ModelLoadStartRequest {
        operation_id: id,
        load: desktop_bridge::LoadModelRequest {
            model_id: "a".into(),
            context_size: 2048,
            threads: 2,
            batch_size: 128,
        },
    }
}
#[tokio::test]
async fn held_or_lost_start_reply_keeps_owned_identity_and_stop_remains_prompt() {
    for mode in [Mode::HeldLoadStart, Mode::LostLoadStart] {
        let f = Fixture::new(mode).await;
        let id = Uuid::new_v4();
        let handle = f.bridge.model_load_start(load_request(id)).unwrap();
        assert_eq!(handle.operation_id, id);
        tokio::time::timeout(Duration::from_secs(2), async {
            while f.chats.load(Ordering::SeqCst) == 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            f.bridge
                .model_load_cancel(Uuid::new_v4())
                .await
                .unwrap_err()
                .code,
            "request_not_owned"
        );
        assert!(f.bridge.model_load_cancel(id).await.unwrap().stopping);
        let result = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Ok(result) = f.bridge.model_load_next(id).await
                    && result.terminal
                {
                    return result;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(result.status, "cancelled");
        assert_eq!(f.chats.load(Ordering::SeqCst), 1); // Never re-POST/reload.
        assert!(!result.error.unwrap().message.contains("private"));
        assert!(!f.bridge.model_load_cancel(id).await.unwrap().stopping);
        f.bridge.close_ui_only().await.unwrap();
    }
}
#[tokio::test]
async fn contradictory_terminal_never_releases_work_and_can_recover_by_owned_stop() {
    let f = Fixture::new(Mode::InvalidLoadState).await;
    let id = Uuid::new_v4();
    f.bridge.model_load_start(load_request(id)).unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if f.bridge.model_load_next(id).await.is_err() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        f.bridge
            .model_load_start(load_request(Uuid::new_v4()))
            .unwrap_err()
            .code,
        "desktop_busy"
    );
    f.bridge.model_load_cancel(id).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(result) = f.bridge.model_load_next(id).await
                && result.terminal
            {
                assert_eq!(result.status, "cancelled");
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    f.bridge.close_ui_only().await.unwrap();
}
#[test]
fn scoped_start_dtos_flatten_existing_fields_and_reject_unknowns() {
    let id = Uuid::new_v4();
    let legacy =
        json!({"operation_id":id,"model_id":"a","context_size":2048,"threads":2,"batch_size":128});
    assert!(serde_json::from_value::<desktop_bridge::ModelLoadStartRequest>(legacy).is_ok());
    let profile = json!({"operation_id":id,"model_id":"a","load_overrides":{"threads":2}});
    assert!(
        serde_json::from_value::<desktop_bridge::ModelLoadProfileStartRequest>(profile.clone())
            .is_ok()
    );
    let mut unknown = profile;
    unknown["path"] = json!("arbitrary");
    assert!(
        serde_json::from_value::<desktop_bridge::ModelLoadProfileStartRequest>(unknown).is_err()
    );
}
