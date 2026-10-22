//! Chat mapping retains the original affine core output lease until the socket
//! accepts its Bytes (or drops the connection). Acceptance is NOT a TCP ACK.
use crate::{ApiState, config::MAX_NONSTREAM_RESPONSE_BYTES, dto::parse_chat, errors::ApiError};
use axum::{
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use hyper::body::{Body as HttpBody, Frame};
use runtime_core::{DisconnectHandle, EventLease, EventReceiver, TextPermit};
use runtime_types::{RequestEventKind, RequestId, RuntimeError, Usage};
use serde::Serialize;
use std::{
    convert::Infallible,
    pin::Pin,
    task::{Context, Poll},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::mpsc;

const RETAINED_CHARGE: usize = 96 * 1024;
const TRANSIT_CHARGE: usize = 120 * 1024;
const MAX_ENCODED_DELTA: usize = 25 * 1024;
const SUFFIX_RESERVE: usize = 256;
struct EventPump {
    events: mpsc::Receiver<EventLease>,
    disconnect: DisconnectHandle,
}
impl EventPump {
    fn new(receiver: EventReceiver) -> Self {
        let disconnect = receiver.disconnect_handle();
        let (tx, events) = mpsc::channel(1);
        // Dropping a JoinHandle does not stop blocking work. The independently
        // owned disconnect handle wakes recv_leased; dropping rx wakes send.
        tokio::task::spawn_blocking(move || {
            while let Some(event) = receiver.recv_leased() {
                if tx.blocking_send(event).is_err() {
                    break;
                }
            }
        });
        Self { events, disconnect }
    }
    async fn next(&mut self) -> Option<EventLease> {
        self.events.recv().await
    }
}
impl Drop for EventPump {
    fn drop(&mut self) {
        self.disconnect.disconnect();
    }
}
struct EncodedEvent {
    bytes: Vec<u8>,
    _lease: Option<EventLease>,
    _retained: Option<TextPermit>,
}
impl AsRef<[u8]> for EncodedEvent {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}
fn owned(bytes: Vec<u8>, lease: Option<EventLease>, retained: Option<TextPermit>) -> Bytes {
    Bytes::from_owner(EncodedEvent {
        bytes,
        _lease: lease,
        _retained: retained,
    })
}
#[derive(Clone)]
struct Metadata {
    id: String,
    created: u64,
    model: String,
}
#[derive(Serialize)]
struct Chunk<'a> {
    id: &'a str,
    object: &'static str,
    created: u64,
    model: &'a str,
    choices: Vec<Choice<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<UsageDto>,
}
#[derive(Serialize)]
struct Choice<'a> {
    index: u8,
    delta: Delta<'a>,
    finish_reason: Option<&'a str>,
}
#[derive(Default, Serialize)]
struct Delta<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<[ToolDeltaDto<'a>; 1]>,
}
#[derive(Serialize)]
struct ToolDeltaDto<'a> {
    index: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<&'a str>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    kind: Option<&'static str>,
    function: FunctionDeltaDto<'a>,
}
#[derive(Serialize)]
struct FunctionDeltaDto<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    arguments: &'a str,
}
#[derive(Clone, Copy, Serialize)]
struct UsageDto {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u64,
}
impl From<Usage> for UsageDto {
    fn from(value: Usage) -> Self {
        Self {
            prompt_tokens: value.prompt_tokens,
            completion_tokens: value.completion_tokens,
            total_tokens: value.total_tokens(),
        }
    }
}
fn encode_chunk(
    metadata: &Metadata,
    role: bool,
    text: Option<&str>,
    finish: Option<&str>,
    usage: Option<Usage>,
) -> Vec<u8> {
    let choices = if usage.is_some() {
        vec![]
    } else {
        vec![Choice {
            index: 0,
            delta: Delta {
                role: role.then_some("assistant"),
                content: text,
                tool_calls: None,
            },
            finish_reason: finish,
        }]
    };
    let chunk = Chunk {
        id: &metadata.id,
        object: "chat.completion.chunk",
        created: metadata.created,
        model: &metadata.model,
        choices,
        usage: usage.map(Into::into),
    };
    let mut bytes = Vec::with_capacity(text.map_or(512, |text| text.len() * 6 + 1024));
    bytes.extend_from_slice(b"data: ");
    serde_json::to_writer(&mut bytes, &chunk).expect("serialize primitive response");
    bytes.extend_from_slice(b"\n\n");
    bytes
}
fn encode_tool_chunk(
    metadata: &Metadata,
    delta: &runtime_types::ToolCallDelta,
) -> Result<Vec<u8>, ApiError> {
    use runtime_types::ToolCallDelta;
    delta.validate_piece().map_err(|_| ApiError::internal())?;
    let tool = match delta {
        ToolCallDelta::Start { index, id, name } => ToolDeltaDto {
            index: *index,
            id: Some(id),
            kind: Some("function"),
            function: FunctionDeltaDto {
                name: Some(name),
                arguments: "",
            },
        },
        ToolCallDelta::Arguments { index, arguments } => ToolDeltaDto {
            index: *index,
            id: None,
            kind: None,
            function: FunctionDeltaDto {
                name: None,
                arguments,
            },
        },
    };
    let chunk = Chunk {
        id: &metadata.id,
        object: "chat.completion.chunk",
        created: metadata.created,
        model: &metadata.model,
        choices: vec![Choice {
            index: 0,
            delta: Delta {
                role: None,
                content: None,
                tool_calls: Some([tool]),
            },
            finish_reason: None,
        }],
        usage: None,
    };
    let mut bytes = Vec::with_capacity(delta.payload_bytes() * 6 + 1024);
    bytes.extend_from_slice(b"data: ");
    serde_json::to_writer(&mut bytes, &chunk).expect("serialize primitive tool delta");
    bytes.extend_from_slice(b"\n\n");
    if bytes.len() > MAX_ENCODED_DELTA + 1024 {
        return Err(ApiError::internal());
    }
    Ok(bytes)
}
fn encode_error(error: &ApiError) -> Vec<u8> {
    let mut bytes = b"data: ".to_vec();
    serde_json::to_writer(&mut bytes, error).expect("serialize error");
    bytes.extend_from_slice(b"\n\n");
    bytes
}
fn event_error(event: &RequestEventKind, started: bool) -> Option<ApiError> {
    match event {
        RequestEventKind::Failed { error, .. } => Some(if started {
            ApiError::from_generation(error.clone())
        } else {
            error.clone().into()
        }),
        RequestEventKind::Cancelled { reason, .. } => {
            Some(RuntimeError::new(*reason, "cancelled").into())
        }
        _ => None,
    }
}
enum Tail {
    None,
    Usage(Usage),
    Done,
    Closed,
}
struct SseBody {
    pump: EventPump,
    metadata: Metadata,
    role: bool,
    include_usage: bool,
    tail: Tail,
}
impl HttpBody for SseBody {
    type Data = Bytes;
    type Error = Infallible;
    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        let this = self.get_mut();
        let bytes = if this.role {
            this.role = false;
            owned(
                encode_chunk(&this.metadata, true, None, None, None),
                None,
                None,
            )
        } else {
            match std::mem::replace(&mut this.tail, Tail::None) {
                Tail::Usage(usage) => {
                    this.tail = Tail::Done;
                    owned(
                        encode_chunk(&this.metadata, false, None, None, Some(usage)),
                        None,
                        None,
                    )
                }
                Tail::Done => {
                    this.tail = Tail::Closed;
                    Bytes::from_static(b"data: [DONE]\n\n")
                }
                Tail::Closed => {
                    this.tail = Tail::Closed;
                    return Poll::Ready(None);
                }
                Tail::None => {
                    let event = match this.pump.events.poll_recv(cx) {
                        Poll::Pending => return Poll::Pending,
                        Poll::Ready(Some(event)) => event,
                        Poll::Ready(None) => {
                            this.tail = Tail::Closed;
                            return Poll::Ready(Some(Ok(Frame::data(owned(
                                encode_error(&ApiError::internal()),
                                None,
                                None,
                            )))));
                        }
                    };
                    if let Some(error) = event_error(&event.kind, true) {
                        this.tail = Tail::Closed;
                        owned(encode_error(&error), Some(event), None)
                    } else {
                        match &event.kind {
                            RequestEventKind::TextDelta(text) => {
                                // The IPC credit conservatively covers source text,
                                // encoding scratch and this final encoded buffer.
                                if event.charged_bytes() < TRANSIT_CHARGE {
                                    this.tail = Tail::Closed;
                                    this.pump.disconnect.disconnect();
                                    owned(encode_error(&ApiError::internal()), Some(event), None)
                                } else {
                                    let bytes =
                                        encode_chunk(&this.metadata, false, Some(text), None, None);
                                    debug_assert!(bytes.len() <= MAX_ENCODED_DELTA + 1024);
                                    owned(bytes, Some(event), None)
                                }
                            }
                            RequestEventKind::ToolCallDelta(delta) => {
                                let encoded = if event.charged_bytes() < TRANSIT_CHARGE {
                                    Err(ApiError::internal())
                                } else {
                                    encode_tool_chunk(&this.metadata, delta)
                                };
                                match encoded {
                                    Ok(bytes) => owned(bytes, Some(event), None),
                                    Err(error) => {
                                        this.tail = Tail::Closed;
                                        this.pump.disconnect.disconnect();
                                        owned(encode_error(&error), Some(event), None)
                                    }
                                }
                            }
                            RequestEventKind::Completed {
                                usage,
                                finish_reason,
                                ..
                            } => {
                                this.tail = if this.include_usage {
                                    Tail::Usage(*usage)
                                } else {
                                    Tail::Done
                                };
                                owned(
                                    encode_chunk(
                                        &this.metadata,
                                        false,
                                        None,
                                        Some(finish_reason.as_str()),
                                        None,
                                    ),
                                    Some(event),
                                    None,
                                )
                            }
                            _ => {
                                this.tail = Tail::Closed;
                                this.pump.disconnect.disconnect();
                                owned(encode_error(&ApiError::internal()), Some(event), None)
                            }
                        }
                    }
                }
            }
        };
        Poll::Ready(Some(Ok(Frame::data(bytes))))
    }
    fn is_end_stream(&self) -> bool {
        matches!(self.tail, Tail::Closed)
    }
}
fn json_prefix(meta: &Metadata) -> Vec<u8> {
    let mut buffer = Vec::new();
    buffer.extend_from_slice(b"{\"id\":");
    serde_json::to_writer(&mut buffer, &meta.id).unwrap();
    buffer.extend_from_slice(b",\"object\":\"chat.completion\",\"created\":");
    serde_json::to_writer(&mut buffer, &meta.created).unwrap();
    buffer.extend_from_slice(b",\"model\":");
    serde_json::to_writer(&mut buffer, &meta.model).unwrap();
    buffer.extend_from_slice(
        b",\"choices\":[{\"index\":0,\"message\":{\"role\":\"assistant\",\"content\":\"",
    );
    buffer
}
fn append_json_content(out: &mut Vec<u8>, text: &str) -> bool {
    let encoded_len = text
        .bytes()
        .map(|byte| match byte {
            b'"' | b'\\' | b'\n' | b'\r' | b'\t' | 8 | 12 => 2,
            0..=31 => 6,
            _ => 1,
        })
        .sum::<usize>();
    if encoded_len > MAX_ENCODED_DELTA
        || out.len() + encoded_len + SUFFIX_RESERVE > MAX_NONSTREAM_RESPONSE_BYTES
    {
        return false;
    }
    const HEX: &[u8] = b"0123456789abcdef";
    for byte in text.bytes() {
        match byte {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\t' => out.extend_from_slice(b"\\t"),
            8 => out.extend_from_slice(b"\\b"),
            12 => out.extend_from_slice(b"\\f"),
            0..=31 => out.extend_from_slice(&[
                b'\\',
                b'u',
                b'0',
                b'0',
                HEX[(byte >> 4) as usize],
                HEX[(byte & 15) as usize],
            ]),
            _ => out.push(byte),
        }
    }
    true
}
/// Writes the one bounded response directly, retaining no second full text or
/// argument aggregate. The validated engine emits content before tool headers.
async fn nonstream(mut pump: EventPump, meta: Metadata) -> Result<Response, ApiError> {
    let prefix = json_prefix(&meta);
    let mut buffer: Option<Vec<u8>> = None;
    let mut retained = None;
    let mut has_text = false;
    let mut call_count = 0_u32;
    while let Some(event) = pump.next().await {
        if let Some(error) = event_error(&event.kind, true) {
            return Err(error);
        }
        let payload = matches!(
            &event.kind,
            RequestEventKind::TextDelta(_) | RequestEventKind::ToolCallDelta(_)
        );
        if payload && event.charged_bytes() < TRANSIT_CHARGE {
            return Err(ApiError::internal());
        }
        if payload && buffer.is_none() {
            let mut out = Vec::with_capacity(MAX_NONSTREAM_RESPONSE_BYTES);
            out.extend_from_slice(&prefix);
            buffer = Some(out);
        }
        match &event.kind {
            RequestEventKind::TextDelta(text) => {
                if call_count != 0 {
                    return Err(ApiError::internal());
                }
                let out = buffer.as_mut().expect("payload buffer initialized");
                if !append_json_content(out, text) {
                    pump.disconnect.disconnect();
                    return Err(ApiError::response_too_large());
                }
                has_text = true;
            }
            RequestEventKind::ToolCallDelta(delta) => {
                use runtime_types::ToolCallDelta;
                delta.validate_piece().map_err(|_| ApiError::internal())?;
                let out = buffer.as_mut().expect("payload buffer initialized");
                match delta {
                    ToolCallDelta::Start { index, id, name } => {
                        if *index != call_count {
                            return Err(ApiError::internal());
                        }
                        // Header fields have their own small finite bounds. Check
                        // before serialization, preserving room for the final tail.
                        if out.len() + 1024 + SUFFIX_RESERVE > MAX_NONSTREAM_RESPONSE_BYTES {
                            return Err(ApiError::response_too_large());
                        }
                        if call_count == 0 {
                            if has_text {
                                out.extend_from_slice(b"\",\"tool_calls\":[");
                            } else {
                                // Prefix ends in content:"; replace only that fixed
                                // opening quote, never any model-generated content.
                                out.pop();
                                out.extend_from_slice(b"null,\"tool_calls\":[");
                            }
                        } else {
                            out.extend_from_slice(b"\"}},");
                        }
                        out.extend_from_slice(b"{\"id\":");
                        serde_json::to_writer(&mut *out, id).expect("serialize bounded ID");
                        out.extend_from_slice(b",\"type\":\"function\",\"function\":{\"name\":");
                        serde_json::to_writer(&mut *out, name).expect("serialize bounded name");
                        out.extend_from_slice(b",\"arguments\":\"");
                        call_count += 1;
                    }
                    ToolCallDelta::Arguments { index, arguments } => {
                        if call_count == 0 || *index != call_count - 1 {
                            return Err(ApiError::internal());
                        }
                        if !append_json_content(out, arguments) {
                            pump.disconnect.disconnect();
                            return Err(ApiError::response_too_large());
                        }
                    }
                }
            }
            RequestEventKind::Completed {
                usage,
                finish_reason,
                ..
            } => {
                if (*finish_reason == runtime_types::FinishReason::ToolCalls) != (call_count > 0) {
                    return Err(ApiError::internal());
                }
                let mut out = buffer.unwrap_or(prefix);
                let message_tail = if call_count == 0 { "\"}" } else { "\"}}]}" };
                let suffix = format!(
                    "{message_tail},\"finish_reason\":\"{}\"}}],\"usage\":{{\"prompt_tokens\":{},\"completion_tokens\":{},\"total_tokens\":{}}}}}",
                    finish_reason.as_str(),
                    usage.prompt_tokens,
                    usage.completion_tokens,
                    usage.total_tokens()
                );
                if out.len() + suffix.len() > MAX_NONSTREAM_RESPONSE_BYTES {
                    return Err(ApiError::response_too_large());
                }
                out.extend_from_slice(suffix.as_bytes());
                let mut response = Response::new(Body::from(owned(out, None, retained)));
                response.headers_mut().insert(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                );
                return Ok(response);
            }
            _ => return Err(ApiError::internal()),
        }
        if payload && retained.is_none() {
            retained = Some(
                event
                    .retain_permit(RETAINED_CHARGE)
                    .map_err(|_| ApiError::internal())?,
            );
        }
    }
    Err(ApiError::internal())
}
fn one_header<'a>(headers: &'a HeaderMap, name: &str) -> Result<Option<&'a HeaderValue>, ApiError> {
    let mut values = headers.get_all(name).iter();
    let first = values.next();
    if values.next().is_some() {
        return Err(ApiError::invalid(
            name,
            "Duplicate headers are not permitted.",
        ));
    }
    Ok(first)
}
pub async fn chat(State(state): State<ApiState>, request: Request) -> Response {
    chat_response(state, request, false).await
}
pub async fn lan_chat(State(state): State<ApiState>, request: Request) -> Response {
    chat_response(state, request, true).await
}
async fn chat_response(state: ApiState, request: Request, loaded_only: bool) -> Response {
    let id = (|| match one_header(request.headers(), "x-request-id")? {
        Some(value) => {
            let value = value
                .to_str()
                .map_err(|_| ApiError::invalid("X-Request-ID", "Expected a canonical UUID."))?;
            let id = value.parse::<RequestId>().map_err(ApiError::from)?;
            if id.to_string() != value {
                return Err(ApiError::invalid(
                    "X-Request-ID",
                    "Expected a canonical UUID.",
                ));
            }
            Ok(id)
        }
        None => Ok(RequestId::new()),
    })();
    let id = match id {
        Ok(id) => id,
        Err(error) => return error.into_response(),
    };
    let mut response = match chat_request(state, request, id, loaded_only).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    };
    response.headers_mut().insert(
        "x-request-id",
        HeaderValue::from_str(&id.to_string()).unwrap(),
    );
    response
}
async fn chat_request(
    state: ApiState,
    request: Request,
    id: RequestId,
    loaded_only: bool,
) -> Result<Response, ApiError> {
    let content_type = one_header(request.headers(), "content-type")?
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if content_type.split(';').next().map(str::trim) != Some("application/json") {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "Content-Type must be application/json.",
            Some("body"),
        ));
    }
    let too_large = || {
        ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "request_too_large",
            "The request body exceeds the configured limit.",
            None,
        )
    };
    let read_limit = if loaded_only {
        state.config.api.max_body_bytes
    } else {
        crate::config::MAX_IMAGE_BODY_BYTES
    };
    if one_header(request.headers(), "content-length")?
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .is_some_and(|v| v > read_limit as u64)
    {
        return Err(too_large());
    }
    let bytes = to_bytes(request.into_body(), read_limit)
        .await
        .map_err(|_| too_large())?;
    let defaults = state.active_config()?;
    // Only the image envelope gets the larger local read budget. Oversized
    // ordinary text retains its existing 413 response before field validation.
    if bytes.len() > state.config.api.max_body_bytes {
        let image_envelope = crate::dto::parse_json(&bytes).ok().is_some_and(|value| {
            value
                .get("messages")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|messages| {
                    messages.iter().any(|message| {
                        message
                            .get("content")
                            .and_then(serde_json::Value::as_array)
                            .is_some_and(|parts| {
                                parts.iter().any(|part| {
                                    part.get("type").and_then(serde_json::Value::as_str)
                                        == Some("image_url")
                                })
                            })
                    })
                })
        });
        if loaded_only || !image_envelope {
            return Err(too_large());
        }
    }
    let validated = parse_chat(&bytes, id, &defaults)?;
    let has_image = validated
        .messages
        .iter()
        .any(|message| message.image.is_some());
    if loaded_only && has_image {
        return Err(ApiError::unsupported("messages.content.image_url"));
    }
    if !has_image && bytes.len() > state.config.api.max_body_bytes {
        return Err(too_large());
    }
    drop(bytes);
    let (model, events) = if let Some(model) = validated.model {
        let request = runtime_types::GenerationRequest {
            request_id: validated.request_id,
            model: model.clone(),
            messages: validated.messages,
            options: validated.options,
            tools: validated.tools,
        };
        let events = if loaded_only {
            state.submit_loaded(request).await?
        } else {
            state.submit(request).await?
        };
        (model, events)
    } else {
        state
            .submit_current_chat(
                validated.request_id,
                validated.messages,
                validated.options,
                validated.tools,
            )
            .await?
    };
    let meta = Metadata {
        id: format!("chatcmpl-{id}"),
        created: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        model: model.to_string(),
    };
    let mut pump = EventPump::new(events);
    loop {
        let event = pump.next().await.ok_or_else(ApiError::internal)?;
        if let Some(error) = event_error(&event.kind, false) {
            return Err(error);
        }
        match event.kind {
            RequestEventKind::Accepted | RequestEventKind::Queued | RequestEventKind::Loading => {
                continue;
            }
            RequestEventKind::Started { .. } => break,
            _ => return Err(ApiError::internal()),
        }
    }
    let response = if validated.stream {
        let mut response = Response::new(Body::new(SseBody {
            pump,
            metadata: meta,
            role: true,
            include_usage: validated.include_usage,
            tail: Tail::None,
        }));
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        );
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        response
            .headers_mut()
            .insert("x-accel-buffering", HeaderValue::from_static("no"));
        response
    } else {
        nonstream(pump, meta).await?
    };
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capped_direct_escaping_matches_json_without_scratch_or_reallocation() {
        let text = (0u8..=127).map(char::from).collect::<String>() + "你好🙂";
        let mut buffer = Vec::with_capacity(MAX_NONSTREAM_RESPONSE_BYTES);
        let pointer = buffer.as_ptr();
        assert!(append_json_content(&mut buffer, &text));
        let expected = serde_json::to_vec(&text).unwrap();
        assert_eq!(buffer, &expected[1..expected.len() - 1]);
        assert_eq!(pointer, buffer.as_ptr());
        buffer.resize(MAX_NONSTREAM_RESPONSE_BYTES - SUFFIX_RESERVE - 5, b'x');
        assert!(!append_json_content(&mut buffer, "\0"));
        assert_eq!(
            buffer.len(),
            MAX_NONSTREAM_RESPONSE_BYTES - SUFFIX_RESERVE - 5
        );
    }
}
