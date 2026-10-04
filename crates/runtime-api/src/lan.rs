//! Optional inference-only trusted-LAN boundary. No browser or proxy authority,
//! no management credentials, and no TLS/public-network claim.
use crate::{
    errors::ApiError,
    security::{BodyLimit, PeerEndpoints},
    token::SecretToken,
};
use axum::{
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use runtime_types::RuntimeError;
use serde::{Deserialize, Serialize};
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};

pub const MAX_ALLOWED_CIDRS: usize = 16;
pub const BODY_READ_TIMEOUT: Duration = Duration::from_secs(10);

/// Never contains credentials. Disabled configurations may be empty; retained
/// values still receive the same validation as an enabled configuration.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct LanApiConfig {
    pub enabled: bool,
    pub listen: Option<SocketAddr>,
    pub allowed_cidrs: Vec<String>,
}
#[derive(Clone, Debug)]
pub(crate) struct Ipv4Cidr {
    network: u32,
    mask: u32,
}
impl Ipv4Cidr {
    fn parse(text: &str) -> Result<Self, RuntimeError> {
        let (address, prefix) = text.split_once('/').ok_or_else(invalid_config)?;
        let address: Ipv4Addr = address.parse().map_err(|_| invalid_config())?;
        let prefix: u32 = prefix.parse().map_err(|_| invalid_config())?;
        if !(24..=32).contains(&prefix) || !address.is_private() {
            return Err(invalid_config());
        }
        let mask = u32::MAX << (32 - prefix);
        let network = u32::from(address);
        if network & mask != network || text != format!("{address}/{prefix}") {
            return Err(invalid_config());
        }
        Ok(Self { network, mask })
    }
    fn contains(&self, address: Ipv4Addr) -> bool {
        u32::from(address) & self.mask == self.network
    }
    fn overlaps(&self, other: &Self) -> bool {
        self.network & other.mask == other.network || other.network & self.mask == self.network
    }
}
fn invalid_config() -> RuntimeError {
    RuntimeError::invalid(
        "LAN requires a concrete RFC1918 IPv4 listener with port 1..=65535 and 1..=16 canonical, non-overlapping private IPv4 CIDRs with prefixes /24..=/32",
    )
}
impl LanApiConfig {
    pub fn validate(&self) -> Result<(), RuntimeError> {
        if self.listen.is_some_and(|listen| {
            !matches!(listen.ip(), IpAddr::V4(ip) if ip.is_private()) || listen.port() == 0
        }) || self.allowed_cidrs.len() > MAX_ALLOWED_CIDRS
            || (self.enabled && (self.listen.is_none() || self.allowed_cidrs.is_empty()))
        {
            return Err(invalid_config());
        }
        let mut ranges: Vec<Ipv4Cidr> = Vec::new();
        for text in &self.allowed_cidrs {
            let range = Ipv4Cidr::parse(text)?;
            if ranges.iter().any(|previous| previous.overlaps(&range)) {
                return Err(invalid_config());
            }
            ranges.push(range);
        }
        Ok(())
    }
    pub(crate) fn policy(&self) -> Result<LanAccessPolicy, RuntimeError> {
        self.validate()?;
        if !self.enabled {
            return Err(invalid_config());
        }
        Ok(LanAccessPolicy {
            listen: self.listen.ok_or_else(invalid_config)?,
            ranges: self
                .allowed_cidrs
                .iter()
                .map(|text| Ipv4Cidr::parse(text))
                .collect::<Result<_, _>>()?,
        })
    }
}
#[derive(Clone, Debug)]
pub(crate) struct LanAccessPolicy {
    pub(crate) listen: SocketAddr,
    ranges: Vec<Ipv4Cidr>,
}
impl LanAccessPolicy {
    pub(crate) fn allows(&self, endpoints: PeerEndpoints) -> bool {
        endpoints.server == self.listen
            && match endpoints.client.ip() {
                IpAddr::V4(ip) if ip.is_private() => {
                    self.ranges.iter().any(|range| range.contains(ip))
                }
                _ => false,
            }
    }
}
pub struct LanSecurityContext {
    token: SecretToken,
    policy: LanAccessPolicy,
}
impl LanSecurityContext {
    pub fn new(token: SecretToken, config: &LanApiConfig) -> Result<Self, RuntimeError> {
        Ok(Self {
            token,
            policy: config.policy()?,
        })
    }
}
fn reject() -> ApiError {
    ApiError::new(
        StatusCode::FORBIDDEN,
        "forbidden",
        "The LAN request authority, peer, or origin is not permitted.",
        None,
    )
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
fn validate(context: &LanSecurityContext, request: &Request) -> Result<(), ApiError> {
    let endpoints = request
        .extensions()
        .get::<PeerEndpoints>()
        .copied()
        .ok_or_else(reject)?;
    if !context.policy.allows(endpoints) {
        return Err(reject());
    }
    let host = unique(request.headers(), "host")?
        .and_then(|value| value.to_str().ok())
        .ok_or_else(reject)?;
    if host != context.policy.listen.to_string()
        || request
            .uri()
            .authority()
            .is_some_and(|authority| authority.as_str() != host)
        || request
            .uri()
            .scheme()
            .is_some_and(|scheme| scheme.as_str() != "http")
    {
        return Err(reject());
    }
    // Native server-to-server clients only. Proxy headers never reinterpret the
    // actual socket peer; reject them rather than silently imply proxy support.
    if request.headers().keys().any(|name| {
        matches!(
            name.as_str(),
            "origin" | "forwarded" | "x-real-ip" | "x-nexa-server-challenge" | "x-request-id"
        ) || name.as_str().starts_with("x-forwarded-")
    }) {
        return Err(reject());
    }
    if !unique(request.headers(), "authorization")?
        .is_some_and(|value| context.token.matches_authorization(value.as_bytes()))
    {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "invalid_api_key",
            "A valid LAN Bearer API token is required.",
            None,
        ));
    }
    // Do not let Axum's GET handler implicitly enable HEAD. Unknown management
    // paths retain an authenticated 404 and never reach the local router.
    if (request.uri().path() == "/v1/models" && request.method() != Method::GET)
        || (request.uri().path() == "/v1/chat/completions" && request.method() != Method::POST)
    {
        return Err(ApiError::new(
            StatusCode::METHOD_NOT_ALLOWED,
            "method_not_allowed",
            "Method not allowed for this route.",
            None,
        ));
    }
    if !matches!(request.uri().path(), "/v1/models" | "/v1/chat/completions") {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "Route not found.",
            None,
        ));
    }
    if let Some(length) = unique(request.headers(), "content-length")? {
        length
            .to_str()
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or_else(|| ApiError::invalid("body", "Invalid Content-Length."))?;
    }
    Ok(())
}
fn early_rejection(error: ApiError) -> Response {
    let mut response = error.into_response();
    response
        .headers_mut()
        .insert(header::CONNECTION, HeaderValue::from_static("close"));
    response
}
pub async fn enforce(
    State(context): State<Arc<LanSecurityContext>>,
    request: Request,
    next: Next,
) -> Response {
    let Some(BodyLimit(limit)) = request.extensions().get::<BodyLimit>().copied() else {
        return early_rejection(reject());
    };
    if limit == 0 || limit > crate::config::MAX_BODY_BYTES {
        return early_rejection(reject());
    }
    if let Err(error) = validate(&context, &request) {
        return early_rejection(error);
    }
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
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|length| length > limit as u64)
    {
        return too_large();
    }
    let (parts, body) = request.into_parts();
    let bytes = match tokio::time::timeout(BODY_READ_TIMEOUT, to_bytes(body, limit)).await {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(_)) => return too_large(),
        Err(_) => {
            return early_rejection(ApiError::new(
                StatusCode::REQUEST_TIMEOUT,
                "request_timeout",
                "The request body read deadline was exceeded.",
                None,
            ));
        }
    };
    let mut response = next
        .run(Request::from_parts(parts, Body::from(bytes)))
        .await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> LanApiConfig {
        LanApiConfig {
            enabled: true,
            listen: Some("192.168.10.2:18081".parse().unwrap()),
            allowed_cidrs: vec!["192.168.10.0/24".into()],
        }
    }
    #[test]
    fn config_defaults_roundtrip_and_reject_unsafe_addresses_and_ranges() {
        assert_eq!(
            crate::Config::from_toml("").unwrap().lan_api,
            LanApiConfig::default()
        );
        assert!(LanApiConfig::default().validate().is_ok());
        assert!(config().validate().is_ok());
        for address in [
            "0.0.0.0:18081",
            "127.0.0.1:18081",
            "8.8.8.8:18081",
            "169.254.1.2:18081",
            "100.64.0.2:18081",
            "224.0.0.1:18081",
            "[::1]:18081",
            "[::ffff:192.168.1.2]:18081",
            "192.168.1.2:0",
        ] {
            let mut c = config();
            c.listen = Some(address.parse().unwrap());
            assert!(c.validate().is_err(), "{address}");
            c.enabled = false;
            assert!(c.validate().is_err());
        }
        for cidrs in [
            vec![],
            vec!["0.0.0.0/0"],
            vec!["10.0.0.0/8"],
            vec!["10.0.0.0/23"],
            vec!["192.168.10.1/24"],
            vec!["192.168.10.0/33"],
            vec!["192.168.10.0/024"],
            vec!["192.168.10.2"],
            vec!["8.8.8.8/32"],
            vec!["127.0.0.1/32"],
            vec!["::1/128"],
            vec!["192.168.10.0/24", "192.168.10.2/32"],
            vec!["192.168.10.2/32", "192.168.10.0/24"],
            vec!["192.168.10.0/24", "192.168.10.0/24"],
        ] {
            let mut c = config();
            c.allowed_cidrs = cidrs.iter().map(|s| (*s).into()).collect();
            assert!(c.validate().is_err(), "{cidrs:?}");
        }
        let mut c = config();
        c.allowed_cidrs = (0..17).map(|n| format!("10.0.{n}.0/24")).collect();
        assert!(c.validate().is_err());
        c.allowed_cidrs.pop();
        assert!(c.validate().is_ok());
        c.listen = None;
        assert!(c.validate().is_err());
        let whole = crate::Config {
            lan_api: config(),
            ..Default::default()
        };
        assert_eq!(
            crate::Config::from_toml(&whole.to_toml().unwrap())
                .unwrap()
                .lan_api,
            config()
        );
        for text in [
            "[lan_api]\nunknown=true",
            "[lan_api]\nenabled=true\nenabled=false",
            "[lan_api]\nallowed_cidrs=['10.0.0.0/8']",
        ] {
            assert!(crate::Config::from_toml(text).is_err());
        }
    }
    fn request(context: &LanSecurityContext) -> Request {
        let mut r = Request::builder()
            .uri("/v1/models")
            .header("host", "192.168.10.2:18081")
            .header("authorization", context.token.bearer_header_value())
            .body(Body::empty())
            .unwrap();
        r.extensions_mut().insert(PeerEndpoints {
            client: "192.168.10.3:25000".parse().unwrap(),
            server: context.policy.listen,
        });
        r
    }
    #[test]
    fn socket_authority_and_independent_bearer_cannot_be_spoofed() {
        let c = LanSecurityContext::new(SecretToken::generate().unwrap(), &config()).unwrap();
        assert!(validate(&c, &request(&c)).is_ok());
        for peer in [
            "192.168.11.3:25000",
            "127.0.0.1:25000",
            "8.8.8.8:25000",
            "[::ffff:192.168.10.3]:25000",
            "[::ffff:127.0.0.1]:25000",
            "[::ffff:8.8.8.8]:25000",
        ] {
            let mut r = request(&c);
            r.extensions_mut().insert(PeerEndpoints {
                client: peer.parse().unwrap(),
                server: c.policy.listen,
            });
            assert!(validate(&c, &r).is_err(), "{peer}");
        }
        let mut r = request(&c);
        r.extensions_mut().insert(PeerEndpoints {
            client: "192.168.10.3:25000".parse().unwrap(),
            server: "192.168.10.2:18082".parse().unwrap(),
        });
        assert!(validate(&c, &r).is_err());
        for host in [
            "localhost:18081",
            "192.168.10.2",
            "192.168.10.2:18082",
            "attacker.test:18081",
            "192.168.10.2:18081.attacker",
        ] {
            let mut r = request(&c);
            r.headers_mut()
                .insert("host", HeaderValue::from_str(host).unwrap());
            assert!(validate(&c, &r).is_err());
        }
        for name in [
            "origin",
            "forwarded",
            "x-forwarded-for",
            "x-forwarded-host",
            "x-real-ip",
            "x-nexa-server-challenge",
            "x-request-id",
        ] {
            let mut r = request(&c);
            r.headers_mut()
                .insert(name, HeaderValue::from_static("anything"));
            assert!(validate(&c, &r).is_err(), "{name}");
        }
        for name in ["host", "authorization", "content-length"] {
            let mut r = request(&c);
            r.headers_mut().append(name, HeaderValue::from_static("1"));
            r.headers_mut().append(name, HeaderValue::from_static("1"));
            assert!(validate(&c, &r).is_err());
        }
        let mut r = request(&c);
        *r.uri_mut() = "http://attacker.test/v1/models".parse().unwrap();
        assert!(validate(&c, &r).is_err());
        let mut r = request(&c);
        *r.uri_mut() = "https://192.168.10.2:18081/v1/models".parse().unwrap();
        assert!(validate(&c, &r).is_err());
        let mut r = request(&c);
        r.headers_mut().remove("authorization");
        r.headers_mut()
            .insert("cookie", c.token.bearer_header_value());
        *r.uri_mut() = "/v1/models?api_key=ignored".parse().unwrap();
        assert_eq!(
            validate(&c, &r).unwrap_err().status,
            StatusCode::UNAUTHORIZED
        );
        let mut r = request(&c);
        let local = SecretToken::generate().unwrap();
        r.headers_mut()
            .insert("authorization", local.bearer_header_value());
        assert_eq!(
            validate(&c, &r).unwrap_err().status,
            StatusCode::UNAUTHORIZED
        );
        for (method, path, code) in [
            ("HEAD", "/v1/models", 405),
            ("OPTIONS", "/v1/models", 405),
            ("GET", "/v1/chat/completions", 405),
            ("POST", "/runtime/shutdown", 404),
            ("GET", "/healthz", 404),
            ("GET", "/v1/models/", 404),
            ("GET", "/%72untime/status", 404),
        ] {
            let mut r = request(&c);
            *r.method_mut() = method.parse().unwrap();
            *r.uri_mut() = path.parse().unwrap();
            assert_eq!(validate(&c, &r).unwrap_err().status.as_u16(), code);
        }
    }
}
