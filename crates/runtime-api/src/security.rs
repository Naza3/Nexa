//! Reject ambient browser authority and proxy-supplied connection identities.
use crate::{
    errors::ApiError,
    proof::{self, ProofContext},
    token::SecretToken,
};
use axum::{
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::{net::SocketAddr, sync::Arc};
#[derive(Clone, Copy, Debug)]
pub struct BodyLimit(pub usize);
fn local_read_limit(request: &Request, text_limit: usize) -> usize {
    if request.method() == Method::POST && request.uri().path() == "/v1/chat/completions" {
        crate::config::MAX_IMAGE_BODY_BYTES
    } else {
        text_limit
    }
}
#[derive(Clone, Copy, Debug)]
pub struct PeerEndpoints {
    pub client: SocketAddr,
    pub server: SocketAddr,
}
pub struct SecurityContext {
    pub(crate) token: SecretToken,
    instance_id: uuid::Uuid,
    listen: SocketAddr,
    trusted_origins: Vec<String>,
}
impl SecurityContext {
    pub(crate) fn instance_id(&self) -> uuid::Uuid {
        self.instance_id
    }
    pub fn new(
        token: SecretToken,
        instance_id: uuid::Uuid,
        listen: SocketAddr,
        trusted_origins: Vec<String>,
    ) -> Result<Self, ApiError> {
        if !proof::normalize_endpoint(listen).ip().is_loopback()
            || listen.port() == 0
            || trusted_origins.iter().any(|s| !valid_origin(s))
        {
            return Err(ApiError::invalid(
                "api",
                "A concrete loopback listener and exact HTTP(S) origins are required.",
            ));
        }
        Ok(Self {
            token,
            instance_id,
            listen,
            trusted_origins,
        })
    }
}
fn valid_origin(value: &str) -> bool {
    let Ok(uri) = value.parse::<axum::http::Uri>() else {
        return false;
    };
    let Some(authority) = uri.authority() else {
        return false;
    };
    let Some(scheme) = uri.scheme_str() else {
        return false;
    };
    if !matches!(scheme, "http" | "https")
        || value != format!("{scheme}://{authority}")
        || value.contains(['*', '@'])
    {
        return false;
    }
    let raw = authority.as_str();
    let explicit_port = if raw.starts_with('[') {
        let Some(end) = raw.find(']') else {
            return false;
        };
        let suffix = &raw[end + 1..];
        if suffix.is_empty() {
            None
        } else {
            let Some(port) = suffix.strip_prefix(':') else {
                return false;
            };
            Some(port)
        }
    } else {
        raw.split_once(':').map(|(_, port)| port)
    };
    if explicit_port.is_some_and(|port| {
        port.is_empty()
            || !port.bytes().all(|b| b.is_ascii_digit())
            || port.parse::<u16>().ok().is_none_or(|port| port == 0)
    }) {
        return false;
    }
    let host = authority.host();
    if host.is_empty() || host == "null" || authority.port().is_some_and(|p| p.as_u16() == 0) {
        return false;
    }
    if host.starts_with('[') {
        return host
            .strip_prefix('[')
            .and_then(|h| h.strip_suffix(']'))
            .is_some_and(|h| h.parse::<std::net::Ipv6Addr>().is_ok());
    }
    host.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-')
    })
}
fn unique<'a>(headers: &'a HeaderMap, name: &str) -> Result<Option<&'a HeaderValue>, ApiError> {
    let mut all = headers.get_all(name).iter();
    let first = all.next();
    if all.next().is_some() {
        return Err(ApiError::invalid(
            "headers",
            "Duplicate security-sensitive headers are not permitted.",
        ));
    }
    Ok(first)
}
fn reject() -> ApiError {
    ApiError::new(
        StatusCode::FORBIDDEN,
        "forbidden",
        "The local request authority or origin is not permitted.",
        None,
    )
}
fn validate(
    context: &SecurityContext,
    request: &Request,
) -> Result<Option<ProofContext>, ApiError> {
    let endpoints = request
        .extensions()
        .get::<PeerEndpoints>()
        .ok_or_else(reject)?;
    let client = proof::normalize_endpoint(endpoints.client);
    let server = proof::normalize_endpoint(endpoints.server);
    if !client.ip().is_loopback() || server != proof::normalize_endpoint(context.listen) {
        return Err(reject());
    }
    let host = unique(request.headers(), "host")?
        .and_then(|h| h.to_str().ok())
        .ok_or_else(reject)?;
    // Only exact numeric socket authority is accepted; no DNS, suffix tests,
    // untrusted Forwarded, default-port elision, commas, or userinfo.
    if host != context.listen.to_string() {
        return Err(reject());
    }
    if request
        .uri()
        .authority()
        .is_some_and(|authority| authority.as_str() != host)
        || request.uri().scheme().is_some_and(|s| s.as_str() != "http")
    {
        return Err(reject());
    }
    if let Some(origin) = unique(request.headers(), "origin")? {
        let origin = origin.to_str().map_err(|_| reject())?;
        if !context
            .trusted_origins
            .iter()
            .any(|trusted| trusted == origin)
        {
            return Err(reject());
        }
    }
    let authorization = unique(request.headers(), "authorization")?;
    if request.uri().path() != "/healthz"
        && !authorization.is_some_and(|a| context.token.matches_authorization(a.as_bytes()))
    {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "invalid_api_key",
            "A valid Bearer API token is required.",
            None,
        ));
    }
    // A declared size is checked before collecting, parsing JSON, or submitting.
    if let Some(length) = unique(request.headers(), "content-length")? {
        let length = length
            .to_str()
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .ok_or_else(|| ApiError::invalid("body", "Invalid Content-Length."))?;
        if length > local_read_limit(request, crate::config::MAX_BODY_BYTES) as u64 {
            return Err(ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "request_too_large",
                "The request body exceeds the configured limit.",
                None,
            ));
        }
    }
    if let Some(challenge) = unique(request.headers(), "x-nexa-server-challenge")? {
        if request.method() != Method::GET || request.uri().path() != "/healthz" {
            return Err(ApiError::invalid(
                "headers",
                "Server challenges are only accepted on GET /healthz.",
            ));
        }
        let nonce = challenge
            .to_str()
            .ok()
            .and_then(proof::decode_hex::<32>)
            .ok_or_else(|| ApiError::invalid("headers", "Invalid server challenge."))?;
        return Ok(Some(ProofContext {
            instance_id: *context.instance_id.as_bytes(),
            nonce,
            client,
            server,
        }));
    }
    Ok(None)
}
/// Early rejection leaves request bytes unread. Signal that this connection is
/// closing; transport performs bounded post-response lingering close so an eager
/// uploader cannot routinely lose the final response to an immediate TCP reset.
fn early_rejection(error: ApiError) -> Response {
    let mut response = error.into_response();
    response
        .headers_mut()
        .insert(header::CONNECTION, HeaderValue::from_static("close"));
    response
}
pub async fn enforce(
    State(context): State<Arc<SecurityContext>>,
    request: Request,
    next: Next,
) -> Response {
    let Some(BodyLimit(limit)) = request.extensions().get::<BodyLimit>().copied() else {
        return early_rejection(reject());
    };
    if limit == 0 || limit > crate::config::MAX_BODY_BYTES {
        return early_rejection(reject());
    }
    let limit = local_read_limit(&request, limit);
    let too_large = || {
        early_rejection(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "request_too_large",
            "The request body exceeds the configured limit.",
            None,
        ))
    };
    if request
        .headers()
        .get("content-length")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.parse::<u64>().ok())
        .is_some_and(|length| length > limit as u64)
    {
        return too_large();
    }
    let proof = match validate(&context, &request) {
        Ok(value) => value,
        Err(error) => return early_rejection(error),
    };
    // This common boundary also covers GET, unknown routes and chunked bodies.
    // No JSON parser, storage operation or scheduler submission precedes it.
    let (parts, body) = request.into_parts();
    let bytes = match to_bytes(body, limit).await {
        Ok(bytes) => bytes,
        Err(_) => return too_large(),
    };
    let request = Request::from_parts(parts, Body::from(bytes));
    let mut response = next.run(request).await;
    if let Some(proof) = proof
        && response.status() == StatusCode::OK
    {
        response.headers_mut().insert(
            "x-nexa-instance-id",
            HeaderValue::from_str(&context.instance_id.to_string()).unwrap(),
        );
        response.headers_mut().insert(
            "x-nexa-server-proof",
            HeaderValue::from_str(&proof::encode_hex(&proof::create_server_proof(
                &context.token,
                &proof,
            )))
            .unwrap(),
        );
        response
            .headers_mut()
            .insert("x-nexa-protocol-version", HeaderValue::from_static("1"));
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    response
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_local_post_chat_receives_the_image_envelope_budget() {
        let c = context();
        let mut r = request(&c);
        r.headers_mut()
            .insert("content-length", HeaderValue::from_static("1048577"));
        assert_eq!(
            validate(&c, &r).unwrap_err().status,
            StatusCode::PAYLOAD_TOO_LARGE
        );
        *r.uri_mut() = "/v1/chat/completions".parse().unwrap();
        assert!(validate(&c, &r).is_err());
        *r.method_mut() = axum::http::Method::POST;
        assert!(validate(&c, &r).is_ok());
        r.headers_mut()
            .insert("content-length", HeaderValue::from_static("8388609"));
        assert_eq!(
            validate(&c, &r).unwrap_err().status,
            StatusCode::PAYLOAD_TOO_LARGE
        );
        r.headers_mut().remove("authorization");
        assert_eq!(
            validate(&c, &r).unwrap_err().status,
            StatusCode::UNAUTHORIZED
        );
    }
    fn context() -> SecurityContext {
        SecurityContext::new(
            SecretToken::generate().unwrap(),
            uuid::Uuid::new_v4(),
            "127.0.0.1:18080".parse().unwrap(),
            vec![],
        )
        .unwrap()
    }
    fn request(context: &SecurityContext) -> Request {
        let mut r = Request::builder()
            .uri("/runtime/status")
            .header("host", "127.0.0.1:18080")
            .header("authorization", context.token.bearer_header_value())
            .body(axum::body::Body::empty())
            .unwrap();
        r.extensions_mut().insert(PeerEndpoints {
            client: "127.0.0.1:33000".parse().unwrap(),
            server: context.listen,
        });
        r
    }
    #[test]
    fn lan_credential_never_authenticates_local_management_or_server_proof() {
        let c = context();
        let lan = SecretToken::generate().unwrap();
        let mut r = request(&c);
        r.headers_mut()
            .insert("authorization", lan.bearer_header_value());
        assert_eq!(
            validate(&c, &r).unwrap_err().status,
            StatusCode::UNAUTHORIZED
        );
        // Local health proof remains anonymous by design, but its HMAC cannot
        // be authenticated with the independent LAN key.
        *r.uri_mut() = "/healthz".parse().unwrap();
        r.headers_mut().insert(
            "x-nexa-server-challenge",
            HeaderValue::from_str(&"00".repeat(32)).unwrap(),
        );
        let challenge = validate(&c, &r).unwrap().unwrap();
        let proof = proof::create_server_proof(&c.token, &challenge);
        assert!(proof::verify_server_proof(&c.token, &challenge, &proof));
        assert!(!proof::verify_server_proof(&lan, &challenge, &proof));
    }
    #[test]
    fn trusted_origins_are_exact_valid_authorities() {
        for good in [
            "http://localhost:1420",
            "https://app.example",
            "http://[::1]:8080",
        ] {
            assert!(valid_origin(good), "{good}");
        }
        for bad in [
            "null",
            "http://*",
            "http://*.example",
            "http://host:99999",
            "http://host:0",
            "http://host/path",
            "http://host/",
            "http://host?x",
            "http://user@host",
            "http://-bad",
        ] {
            assert!(!valid_origin(bad), "{bad}");
        }
    }
    #[test]
    fn exact_host_real_peer_origin_and_unique_bearer() {
        let c = context();
        assert!(validate(&c, &request(&c)).is_ok());
        for host in [
            "localhost:18080",
            "127.0.0.1",
            "127.0.0.1:18081",
            "127.0.0.1:18080.attacker",
            "evil:18080",
        ] {
            let mut r = request(&c);
            r.headers_mut()
                .insert("host", HeaderValue::from_str(host).unwrap());
            assert!(validate(&c, &r).is_err());
        }
        for h in ["host", "authorization"] {
            let mut r = request(&c);
            r.headers_mut().append(h, HeaderValue::from_static("other"));
            assert!(validate(&c, &r).is_err());
        }
        let mut r = request(&c);
        r.headers_mut()
            .insert("origin", HeaderValue::from_static("http://localhost"));
        assert!(validate(&c, &r).is_err());
        let mut r = request(&c);
        r.extensions_mut().insert(PeerEndpoints {
            client: "10.0.0.1:33000".parse().unwrap(),
            server: c.listen,
        });
        r.headers_mut()
            .insert("forwarded", HeaderValue::from_static("for=127.0.0.1"));
        assert!(validate(&c, &r).is_err());
        let mut r = request(&c);
        *r.uri_mut() = "http://evil:18080/runtime/status".parse().unwrap();
        assert!(validate(&c, &r).is_err());
        let mut r = request(&c);
        r.headers_mut().remove("host");
        assert!(validate(&c, &r).is_err());
        let mut r = request(&c);
        r.headers_mut().remove("authorization");
        r.headers_mut()
            .insert("cookie", c.token.bearer_header_value());
        *r.uri_mut() = "/runtime/status?api_key=ignored".parse().unwrap();
        assert_eq!(
            validate(&c, &r).unwrap_err().status,
            StatusCode::UNAUTHORIZED
        );
        let mut r = request(&c);
        r.headers_mut().remove("authorization");
        r.headers_mut().insert(
            "forwarded",
            HeaderValue::from_static("for=127.0.0.1;host=127.0.0.1:18080"),
        );
        assert_eq!(
            validate(&c, &r).unwrap_err().status,
            StatusCode::UNAUTHORIZED
        );
        let mut r = request(&c);
        r.headers_mut()
            .insert("content-length", HeaderValue::from_static("1048577"));
        assert_eq!(
            validate(&c, &r).unwrap_err().status,
            StatusCode::PAYLOAD_TOO_LARGE
        );
    }
}
