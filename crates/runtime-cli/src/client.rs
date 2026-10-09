//! Authenticated local HTTP/1 connection. Authentication is released only after
//! an endpoint-bound server proof on this exact TCP connection. No proxies,
//! redirects, pooling, reconnects, or request replay are possible here.
use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::{
    HeaderMap, Method, Request, Response,
    body::{Body, Frame, Incoming, SizeHint},
    client::conn::http1::SendRequest,
};
use hyper_util::rt::TokioIo;
use runtime_api::{
    configuration::ConfigurationFailureReason,
    proof::{ProofContext, decode_hex, encode_hex, verify_server_proof},
    token::SecretToken,
};
use std::{
    collections::VecDeque,
    convert::Infallible,
    fmt,
    net::SocketAddr,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{net::TcpStream, task::JoinHandle, time::timeout};
use uuid::Uuid;

const PROOF_TIMEOUT: Duration = Duration::from_secs(5);
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(750);
/// These local operations can verify a registered external payload before the
/// native load/generation starts. This is only a client response-header budget.
pub fn may_verify_model(method: &Method, path: &str) -> bool {
    *method == Method::POST
        && matches!(
            path,
            "/runtime/load"
                | "/runtime/load-and-test"
                | "/runtime/load-if-unloaded"
                | "/v1/chat/completions"
        )
}
fn request_timeout(
    method: &Method,
    path: &str,
    verification: Option<Duration>,
) -> Result<Duration> {
    match verification {
        None => Ok(REQUEST_TIMEOUT),
        Some(budget)
            if may_verify_model(method, path)
                && (Duration::from_secs(model_store::library::MIN_VERIFICATION_TIMEOUT_SECONDS)
                    ..=Duration::from_secs(
                        model_store::library::MAX_VERIFICATION_TIMEOUT_SECONDS,
                    ))
                    .contains(&budget) =>
        {
            Ok(REQUEST_TIMEOUT + budget)
        }
        Some(_) => Err(ClientError::Connection(
            "invalid model verification wait budget",
        )),
    }
}

#[derive(Debug)]
pub enum ClientError {
    Connection(&'static str),
    Transport {
        stage: &'static str,
        error_canceled: bool,
        error_closed: bool,
        error_incomplete_message: bool,
        error_parse: bool,
        error_body_write_aborted: bool,
        error_timeout: bool,
        io_kind: Option<&'static str>,
        sender_ready: bool,
        sender_closed: bool,
    },
    Api {
        status: u16,
        code: Option<String>,
        param: Option<String>,
        reason: Option<ConfigurationFailureReason>,
    },
}
impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connection(message) => f.write_str(message),
            Self::Transport { .. } => {
                f.write_str("verified connection could not accept the request; it was not retried")
            }
            Self::Api {
                status,
                code,
                param,
                ..
            } => {
                write!(f, "local API returned HTTP {status}")?;
                if let Some(code) = code {
                    write!(f, ": {code}")?;
                }
                if let Some(param) = param {
                    write!(f, " ({param})")?;
                }
                Ok(())
            }
        }
    }
}
impl std::error::Error for ClientError {}
type Result<T> = std::result::Result<T, ClientError>;
fn io_error_kind(error: &(dyn std::error::Error + 'static)) -> Option<&'static str> {
    let mut current = Some(error);
    for _ in 0..16 {
        let candidate = current?;
        if let Some(io) = candidate.downcast_ref::<std::io::Error>() {
            return Some(match io.kind() {
                std::io::ErrorKind::ConnectionReset => "connection_reset",
                std::io::ErrorKind::ConnectionAborted => "connection_aborted",
                std::io::ErrorKind::BrokenPipe => "broken_pipe",
                std::io::ErrorKind::UnexpectedEof => "unexpected_eof",
                std::io::ErrorKind::TimedOut => "timed_out",
                std::io::ErrorKind::WouldBlock => "would_block",
                _ => "other",
            });
        }
        current = candidate.source();
    }
    None
}

pub struct RequestBody {
    parts: VecDeque<Bytes>,
    remaining: usize,
    fixed: bool,
}
impl RequestBody {
    pub fn fixed(bytes: impl Into<Bytes>) -> Self {
        let bytes = bytes.into();
        Self {
            remaining: bytes.len(),
            parts: VecDeque::from([bytes]),
            fixed: true,
        }
    }
    pub fn chunked(parts: Vec<Bytes>) -> Self {
        Self {
            remaining: parts.iter().map(Bytes::len).sum(),
            parts: parts.into(),
            fixed: false,
        }
    }
}
impl Body for RequestBody {
    type Data = Bytes;
    type Error = Infallible;
    fn poll_frame(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<std::result::Result<Frame<Bytes>, Infallible>>> {
        Poll::Ready(self.parts.pop_front().map(|part| {
            self.remaining -= part.len();
            Ok(Frame::data(part))
        }))
    }
    fn is_end_stream(&self) -> bool {
        self.parts.is_empty()
    }
    fn size_hint(&self) -> SizeHint {
        if self.fixed {
            SizeHint::with_exact(self.remaining as u64)
        } else {
            SizeHint::default()
        }
    }
}

pub struct VerifiedConnection {
    instance_id: Uuid,
    sender: SendRequest<RequestBody>,
    driver: JoinHandle<()>,
    token: SecretToken,
    authority: String,
}
impl Drop for VerifiedConnection {
    fn drop(&mut self) {
        self.driver.abort();
    }
}
impl VerifiedConnection {
    pub async fn connect(address: SocketAddr, instance: Uuid, token: SecretToken) -> Result<Self> {
        if !address.ip().is_loopback() || address.port() == 0 {
            return Err(ClientError::Connection(
                "discovery address must be a concrete loopback endpoint",
            ));
        }
        timeout(PROOF_TIMEOUT, Self::prove(address, instance, token))
            .await
            .map_err(|_| ClientError::Connection("server proof timed out"))?
    }
    async fn prove(address: SocketAddr, instance: Uuid, token: SecretToken) -> Result<Self> {
        let socket = TcpStream::connect(address)
            .await
            .map_err(|_| ClientError::Connection("cannot connect to discovered instance"))?;
        let local = socket
            .local_addr()
            .map_err(|_| ClientError::Connection("cannot identify client endpoint"))?;
        let peer = socket
            .peer_addr()
            .map_err(|_| ClientError::Connection("cannot identify server endpoint"))?;
        if peer != address {
            return Err(ClientError::Connection(
                "connected endpoint differs from discovery",
            ));
        }
        let (mut sender, connection) = hyper::client::conn::http1::Builder::new()
            .max_headers(64)
            .max_buf_size(16 * 1024)
            .handshake::<_, RequestBody>(TokioIo::new(socket))
            .await
            .map_err(|_| ClientError::Connection("HTTP connection failed"))?;
        let driver = tokio::spawn(async move {
            let _ = connection.await;
        });
        // The guard closes the connection on every failed proof, including timeout.
        let mut guard = DriverGuard(Some(driver));
        let mut nonce = [0u8; 32];
        getrandom::fill(&mut nonce)
            .map_err(|_| ClientError::Connection("secure random source unavailable"))?;
        let request = Request::builder()
            .method(Method::GET)
            .uri("/healthz")
            .header("host", address.to_string())
            .header("X-Nexa-Server-Challenge", encode_hex(&nonce))
            .body(RequestBody::fixed(Bytes::new()))
            .map_err(|_| ClientError::Connection("cannot construct proof request"))?;
        // The outer proof deadline includes dispatch readiness on this socket.
        sender
            .ready()
            .await
            .map_err(|_| ClientError::Connection("proof connection closed before readiness"))?;
        let response = sender
            .send_request(request)
            .await
            .map_err(|_| ClientError::Connection("server proof connection failed"))?;
        if response.status() != 200 {
            return Err(ClientError::Connection("server proof requires HTTP 200"));
        }
        let headers = response.headers();
        let returned_instance = single(headers, "x-nexa-instance-id")?;
        if returned_instance != instance.hyphenated().to_string() {
            return Err(ClientError::Connection(
                "server instance identity differs from discovery",
            ));
        }
        if single(headers, "x-nexa-protocol-version")?
            != runtime_types::PROTOCOL_VERSION.to_string()
        {
            return Err(ClientError::Connection("server protocol version mismatch"));
        }
        if single(headers, "cache-control")? != "no-store" {
            return Err(ClientError::Connection("proof response must not be cached"));
        }
        let proof = decode_hex::<32>(single(headers, "x-nexa-server-proof")?)
            .ok_or(ClientError::Connection("malformed server proof"))?;
        let context = ProofContext {
            instance_id: *instance.as_bytes(),
            nonce,
            client: local,
            server: peer,
        };
        if !verify_server_proof(&token, &context, &proof) {
            return Err(ClientError::Connection(
                "server proof authentication failed",
            ));
        }
        collect_bounded(response.into_body(), 4096, PROOF_TIMEOUT).await?;
        // All observations above came from this sender's one socket.
        Ok(Self {
            instance_id: instance,
            sender,
            driver: guard.0.take().unwrap(),
            token,
            authority: address.to_string(),
        })
    }
    /// Identity authenticated on this exact connection, not a later discovery read.
    pub fn instance_id(&self) -> Uuid {
        self.instance_id
    }
    pub async fn request(
        &mut self,
        method: Method,
        path: &str,
        body: RequestBody,
        extra: HeaderMap,
    ) -> Result<Response<Incoming>> {
        self.request_inner(method, path, body, extra, None).await
    }
    /// Available only after the same-connection proof. Callers supply a budget
    /// from validated local configuration, only for registered external models.
    pub async fn request_with_verification(
        &mut self,
        method: Method,
        path: &str,
        body: RequestBody,
        extra: HeaderMap,
        budget: Duration,
    ) -> Result<Response<Incoming>> {
        self.request_inner(method, path, body, extra, Some(budget))
            .await
    }
    async fn request_inner(
        &mut self,
        method: Method,
        path: &str,
        body: RequestBody,
        extra: HeaderMap,
        verification: Option<Duration>,
    ) -> Result<Response<Incoming>> {
        let response_timeout = request_timeout(&method, path, verification)?;
        self.request_with_timeout(method, path, body, extra, response_timeout)
            .await
    }
    /// Submit chat once on this proved connection, using the running service's
    /// effective wait budget, including preparation before SSE headers.
    pub async fn request_chat_with_timeout(
        &mut self,
        body: RequestBody,
        extra: HeaderMap,
        budget: Duration,
    ) -> Result<Response<Incoming>> {
        self.request_with_timeout(Method::POST, "/v1/chat/completions", body, extra, budget)
            .await
    }
    async fn request_with_timeout(
        &mut self,
        method: Method,
        path: &str,
        body: RequestBody,
        extra: HeaderMap,
        response_timeout: Duration,
    ) -> Result<Response<Incoming>> {
        let deadline = tokio::time::Instant::now()
            .checked_add(response_timeout)
            .filter(|_| !response_timeout.is_zero())
            .ok_or(ClientError::Connection("invalid local request wait budget"))?;
        if !path.starts_with('/')
            || path.starts_with("//")
            || extra.contains_key("authorization")
            || extra.contains_key("proxy-authorization")
        {
            return Err(ClientError::Connection("unsafe local request"));
        }
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("host", &self.authority)
            .header("content-type", "application/json")
            .body(body)
            .map_err(|_| ClientError::Connection("invalid local request"))?;
        request.headers_mut().extend(extra);
        request
            .headers_mut()
            .insert("authorization", self.token.bearer_header_value());
        // Hyper's send_request is not a readiness wait: when its one buffered
        // allowance was consumed by the proof, a temporarily unready dispatcher
        // rejects immediately with is_canceled even if the socket remains open.
        // Readiness and exactly one send share the original total deadline.
        tokio::time::timeout_at(deadline, async {
            if let Err(error) = self.sender.ready().await {
                return Err(ClientError::Transport {
                    stage: "sender_ready",
                    error_canceled: error.is_canceled(),
                    error_closed: error.is_closed(),
                    error_incomplete_message: error.is_incomplete_message(),
                    error_parse: error.is_parse(),
                    error_body_write_aborted: error.is_body_write_aborted(),
                    error_timeout: error.is_timeout(),
                    io_kind: io_error_kind(&error),
                    sender_ready: self.sender.is_ready(),
                    sender_closed: self.sender.is_closed(),
                });
            }
            let sender_ready = self.sender.is_ready();
            let sender_closed = self.sender.is_closed();
            self.sender
                .send_request(request)
                .await
                .map_err(|error| ClientError::Transport {
                    stage: "send_request",
                    error_canceled: error.is_canceled(),
                    error_closed: error.is_closed(),
                    error_incomplete_message: error.is_incomplete_message(),
                    error_parse: error.is_parse(),
                    error_body_write_aborted: error.is_body_write_aborted(),
                    error_timeout: error.is_timeout(),
                    io_kind: io_error_kind(&error),
                    sender_ready,
                    sender_closed,
                })
        })
        .await
        .map_err(|_| ClientError::Connection("local request timed out; it was not retried"))?
    }
    pub async fn json(
        &mut self,
        method: Method,
        path: &str,
        value: Option<&serde_json::Value>,
    ) -> Result<serde_json::Value> {
        self.json_inner(method, path, value, None).await
    }
    pub async fn json_with_verification(
        &mut self,
        method: Method,
        path: &str,
        value: Option<&serde_json::Value>,
        budget: Duration,
    ) -> Result<serde_json::Value> {
        self.json_inner(method, path, value, Some(budget)).await
    }
    async fn json_inner(
        &mut self,
        method: Method,
        path: &str,
        value: Option<&serde_json::Value>,
        verification: Option<Duration>,
    ) -> Result<serde_json::Value> {
        let body = value
            .map(serde_json::to_vec)
            .transpose()
            .map_err(|_| ClientError::Connection("cannot encode request"))?
            .unwrap_or_default();
        let response = self
            .request_inner(
                method,
                path,
                RequestBody::fixed(body),
                HeaderMap::new(),
                verification,
            )
            .await?;
        let status = response.status();
        let bytes = collect_bounded(response.into_body(), 1024 * 1024, REQUEST_TIMEOUT).await?;
        if !status.is_success() {
            return Err(api_error(status.as_u16(), &bytes));
        }
        serde_json::from_slice(&bytes)
            .map_err(|_| ClientError::Connection("invalid API JSON response"))
    }
}
/// Keep arbitrary HTTP messages out of diagnostics. Reasons are a fixed local
/// catalog, validated together with the top-level code; old peers omit them.
fn api_error(status: u16, bytes: &[u8]) -> ClientError {
    let value = serde_json::from_slice::<serde_json::Value>(bytes).ok();
    let detail = value.as_ref().and_then(|v| v.get("error"));
    let safe = |name: &str| {
        detail?
            .get(name)?
            .as_str()
            .filter(|s| {
                !s.is_empty()
                    && s.len() <= 128
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
            })
            .map(str::to_owned)
    };
    let code = safe("code");
    let reason = code
        .as_deref()
        .zip(
            detail
                .and_then(|d| d.get("reason"))
                .and_then(|r| r.as_str()),
        )
        .and_then(|(code, reason)| ConfigurationFailureReason::from_reason_code(code, reason));
    ClientError::Api {
        status,
        code,
        param: safe("param"),
        reason,
    }
}
struct DriverGuard(Option<JoinHandle<()>>);
impl Drop for DriverGuard {
    fn drop(&mut self) {
        if let Some(driver) = self.0.take() {
            driver.abort();
        }
    }
}
fn single<'a>(headers: &'a HeaderMap, name: &str) -> Result<&'a str> {
    let mut values = headers.get_all(name).iter();
    let value = values
        .next()
        .ok_or(ClientError::Connection("missing server proof header"))?;
    if values.next().is_some() {
        return Err(ClientError::Connection("duplicate server proof header"));
    }
    value
        .to_str()
        .map_err(|_| ClientError::Connection("non-ASCII server proof header"))
}
pub async fn collect_bounded(
    body: Incoming,
    maximum: usize,
    deadline: Duration,
) -> Result<Vec<u8>> {
    timeout(deadline, async move {
        let mut body = body;
        let mut bytes = Vec::new();
        while let Some(frame) = body.frame().await {
            let frame =
                frame.map_err(|_| ClientError::Connection("response body connection failed"))?;
            if let Ok(data) = frame.into_data() {
                if data.len() > maximum.saturating_sub(bytes.len()) {
                    return Err(ClientError::Connection("response exceeded byte limit"));
                }
                bytes.extend_from_slice(&data);
            }
        }
        Ok(bytes)
    })
    .await
    .map_err(|_| ClientError::Connection("response body timed out"))?
}

/// Read a local discovery record and private token, then authenticate exactly
/// that instance. Suitable for independent API acceptance tools.
pub async fn connect_data_dir(
    root: &std::path::Path,
) -> std::result::Result<VerifiedConnection, Box<dyn std::error::Error + Send + Sync>> {
    let record = crate::instance::Discovery::read(root)?;
    Ok(VerifiedConnection::connect(
        record.listen,
        record.instance_id,
        runtime_api::token::load_private_token(root)?,
    )
    .await?)
}

#[cfg(test)]
mod readiness_tests {
    use super::*;
    use http_body_util::Full;
    use std::sync::{Arc, Mutex};
    #[test]
    fn configuration_reasons_round_trip_without_forwarding_remote_messages() {
        use ConfigurationFailureReason::*;
        for (code, reason) in [
            ("configuration_unavailable", Missing),
            ("configuration_unavailable", AccessDenied),
            ("configuration_unavailable", PathSecurity),
            ("configuration_unavailable", Io),
            ("configuration_invalid", Oversized),
            ("configuration_invalid", InvalidUtf8),
            ("configuration_invalid", InvalidToml),
            ("configuration_invalid", SchemaInvalid),
            ("configuration_busy", LockContended),
        ] {
            let source = runtime_api::configuration::ConfigurationError {
                code,
                param: None,
                reason: Some(reason),
            };
            let api: runtime_api::ApiError = source.into();
            let mut payload = serde_json::to_value(&api).unwrap();
            payload["error"]["message"] =
                serde_json::json!("private_sentinel_secret /private/path/config.toml");
            let client = api_error(api.status.as_u16(), &serde_json::to_vec(&payload).unwrap());
            match &client {
                ClientError::Api {
                    code: actual,
                    reason: actual_reason,
                    ..
                } => {
                    assert_eq!(actual.as_deref(), Some(code));
                    assert_eq!(*actual_reason, Some(reason));
                }
                _ => panic!("expected API failure"),
            }
            assert!(!format!("{client:?} {client}").contains("private_sentinel_secret"));
            assert!(!format!("{client:?} {client}").contains("/private/path"));
        }
    }
    #[test]
    fn optional_configuration_reason_rejects_unknown_malformed_and_mismatched_values() {
        for payload in [
            serde_json::json!({"error":{"code":"configuration_invalid"}}),
            serde_json::json!({"error":{"code":"configuration_invalid","reason":null}}),
            serde_json::json!({"error":{"code":"configuration_invalid","reason":42}}),
            serde_json::json!({"error":{"code":"configuration_invalid","reason":{ "private_sentinel_secret":true }}}),
            serde_json::json!({"error":{"code":"configuration_invalid","reason":["configuration_invalid_toml"]}}),
            serde_json::json!({"error":{"code":"configuration_invalid","reason":"private_sentinel_secret"}}),
            serde_json::json!({"error":{"code":"configuration_invalid","reason":"configuration_missing"}}),
            serde_json::json!({"error":{"code":"configuration_unavailable","reason":"configuration_lock_contended"}}),
            serde_json::json!({"error":{"code":"runtime_busy","reason":"configuration_lock_contended"}}),
            serde_json::json!({"error":{"reason":"configuration_missing"}}),
        ] {
            let failure = api_error(500, &serde_json::to_vec(&payload).unwrap());
            assert!(matches!(&failure, ClientError::Api { reason: None, .. }));
            assert!(!format!("{failure:?} {failure}").contains("private_sentinel_secret"));
        }
        assert!(matches!(
            api_error(500, b"not json private_sentinel_secret"),
            ClientError::Api {
                code: None,
                param: None,
                reason: None,
                ..
            }
        ));
    }
    #[test]
    fn ordinary_api_errors_keep_legacy_shape_and_safe_param() {
        let source = runtime_api::ApiError::invalid("update.runtime", "private_sentinel_secret");
        let payload = serde_json::to_value(&source).unwrap();
        assert!(payload["error"].get("reason").is_none());
        let failure = api_error(400, &serde_json::to_vec(&payload).unwrap());
        assert!(
            matches!(&failure, ClientError::Api { status: 400, code: Some(code), param: Some(param), reason: None }
            if code == "invalid_request" && param == "update.runtime")
        );
        assert!(!format!("{failure:?} {failure}").contains("private_sentinel_secret"));
        let old = api_error(400, br#"{"error":{"code":"configuration_invalid","param":"update.runtime","message":"private_sentinel_secret"}}"#);
        assert!(matches!(old, ClientError::Api { reason: None, .. }));
    }
    #[test]
    fn verification_wait_is_route_method_and_budget_bounded_without_extending_defaults() {
        for path in [
            "/runtime/load",
            "/runtime/load-and-test",
            "/runtime/load-if-unloaded",
            "/v1/chat/completions",
        ] {
            assert_eq!(
                request_timeout(&Method::POST, path, None).unwrap(),
                REQUEST_TIMEOUT
            );
            for seconds in [30, 300, 7200] {
                assert_eq!(
                    request_timeout(&Method::POST, path, Some(Duration::from_secs(seconds)))
                        .unwrap(),
                    Duration::from_secs(750 + seconds)
                );
            }
            for seconds in [0, 29, 7201, u64::MAX] {
                assert!(
                    request_timeout(&Method::POST, path, Some(Duration::from_secs(seconds)))
                        .is_err()
                );
            }
            assert!(request_timeout(&Method::GET, path, Some(Duration::from_secs(300))).is_err());
        }
        for path in [
            "/runtime/model-test",
            "/runtime/status",
            "/runtime/unload",
            "/runtime/shutdown",
            "/runtime/models/import",
            "/v1/models",
            "/runtime/load?extra=true",
            "http://192.168.1.2/runtime/load",
        ] {
            assert_eq!(
                request_timeout(&Method::POST, path, None).unwrap(),
                REQUEST_TIMEOUT
            );
            assert!(request_timeout(&Method::POST, path, Some(Duration::from_secs(300))).is_err());
        }
    }
    #[tokio::test]
    async fn request_waits_for_same_connection_readiness() {
        // Private test construction models an already-proved connection. There
        // is no production bypass, alternate worker, reconnect, or retry hook.
        let (client_io, server_io) = tokio::io::duplex(4096);
        let (mut sender, connection) =
            hyper::client::conn::http1::handshake(TokioIo::new(client_io))
                .await
                .unwrap();
        // Hyper permits one buffered request before its driver is first polled.
        // Consuming that allowance gives a deterministic not-ready state.
        let first = sender.send_request(
            Request::builder()
                .uri("/proof-fixture")
                .body(RequestBody::fixed(Bytes::new()))
                .unwrap(),
        );
        assert!(!sender.is_ready());
        assert!(!sender.is_closed());
        let (gate_tx, gate_rx) = tokio::sync::oneshot::channel();
        let driver = tokio::spawn(async move {
            let _ = gate_rx.await;
            let _ = connection.await;
        });
        let seen = Arc::new(Mutex::new(Vec::new()));
        let observed = seen.clone();
        let server = tokio::spawn(async move {
            let service = hyper::service::service_fn(move |request: Request<Incoming>| {
                let observed = observed.clone();
                async move {
                    observed
                        .lock()
                        .unwrap()
                        .push(request.uri().path().to_owned());
                    Ok::<_, Infallible>(Response::new(Full::new(Bytes::from_static(b"{}"))))
                }
            });
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(server_io), service)
                .await;
        });
        let first = tokio::spawn(async move {
            let response = first.await.unwrap();
            collect_bounded(response.into_body(), 4096, Duration::from_secs(1))
                .await
                .unwrap();
        });
        let mut client = VerifiedConnection {
            instance_id: Uuid::new_v4(),
            sender,
            driver,
            token: SecretToken::generate().unwrap(),
            authority: "127.0.0.1:1".into(),
        };
        let request = client.request(
            Method::POST,
            "/exactly-once",
            RequestBody::fixed(Bytes::new()),
            HeaderMap::new(),
        );
        tokio::pin!(request);
        let premature = timeout(Duration::from_millis(20), &mut request).await;
        assert!(
            premature.is_err(),
            "request must wait while the same driver is paused; got {premature:?}"
        );
        gate_tx.send(()).unwrap();
        first.await.unwrap();
        let response = timeout(Duration::from_secs(1), &mut request)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(response.status(), 200);
        collect_bounded(response.into_body(), 4096, Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(&*seen.lock().unwrap(), &["/proof-fixture", "/exactly-once"]);
        server.abort();
    }
}
