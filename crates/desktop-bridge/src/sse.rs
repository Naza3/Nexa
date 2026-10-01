//! Incremental, strictly bounded SSE decoder for the frozen runtime protocol.
use crate::{BridgeError, ChatEvent, Result, Usage};
use serde_json::Value;
use uuid::Uuid;
pub(crate) const MAX_DECODE: usize = 64 * 1024;
pub(crate) const MAX_EVENT: usize = 32 * 1024;
pub(crate) struct Decoder {
    bytes: Vec<u8>,
    id: String,
    model: String,
    started: bool,
    finish: Option<String>,
    usage: Option<Usage>,
    done: bool,
}
impl Decoder {
    pub fn new(id: Uuid, model: String) -> Self {
        Self {
            bytes: Vec::new(),
            id: format!("chatcmpl-{id}"),
            model,
            started: false,
            finish: None,
            usage: None,
            done: false,
        }
    }
    pub fn push(&mut self, bytes: &[u8]) -> Result<()> {
        if self.done || bytes.len() > MAX_DECODE.saturating_sub(self.bytes.len()) {
            return Err(invalid());
        }
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }
    pub fn next(&mut self) -> Result<Option<ChatEvent>> {
        loop {
            let delimiter = self
                .bytes
                .windows(2)
                .position(|s| s == b"\n\n")
                .map(|p| (p, 2));
            let crlf = self
                .bytes
                .windows(4)
                .position(|s| s == b"\r\n\r\n")
                .map(|p| (p, 4));
            let found = match (delimiter, crlf) {
                (Some(a), Some(b)) => Some(if a.0 < b.0 { a } else { b }),
                (a, b) => a.or(b),
            };
            let Some((end, width)) = found else {
                if self.bytes.len() > MAX_EVENT {
                    return Err(invalid());
                }
                return Ok(None);
            };
            if end > MAX_EVENT {
                return Err(invalid());
            }
            let frame: Vec<u8> = self.bytes.drain(..end + width).collect();
            let frame = std::str::from_utf8(&frame[..end]).map_err(|_| invalid())?;
            let mut data = None;
            for line in frame.lines() {
                if line.starts_with(':') {
                    continue;
                }
                let Some(value) = line.strip_prefix("data:") else {
                    return Err(invalid());
                };
                if data
                    .replace(value.strip_prefix(' ').unwrap_or(value))
                    .is_some()
                {
                    return Err(invalid());
                }
            }
            let Some(data) = data else {
                continue;
            };
            if data == "[DONE]" {
                if self.done
                    || !self.started
                    || self.finish.is_none()
                    || self.usage.is_none()
                    || !self.bytes.is_empty()
                {
                    return Err(invalid());
                }
                self.done = true;
                return Ok(Some(ChatEvent::Completed {
                    finish_reason: self.finish.take().unwrap(),
                    usage: self.usage.take().unwrap(),
                }));
            }
            let value = runtime_api::dto::parse_json(data.as_bytes()).map_err(|_| invalid())?;
            if let Some(error) = value.get("error") {
                let e = BridgeError::api(error.get("code").and_then(Value::as_str));
                return Ok(Some(
                    if matches!(e.code.as_str(), "request_cancelled" | "consumer_stopped") {
                        ChatEvent::Cancelled
                    } else {
                        ChatEvent::Failed {
                            code: e.code,
                            message: e.message,
                        }
                    },
                ));
            }
            let root = value.as_object().ok_or_else(invalid)?;
            if root.keys().any(|k| {
                !["id", "object", "created", "model", "choices", "usage"].contains(&k.as_str())
            }) || value["id"].as_str() != Some(&self.id)
                || value["model"].as_str() != Some(&self.model)
                || value["object"] != "chat.completion.chunk"
                || value["created"].as_u64().is_none()
            {
                return Err(invalid());
            }
            let choices = value["choices"].as_array().ok_or_else(invalid)?;
            if let Some(usage) = value.get("usage").filter(|v| !v.is_null()) {
                if !choices.is_empty() || self.finish.is_none() || self.usage.is_some() {
                    return Err(invalid());
                }
                let u: Usage = serde_json::from_value(usage.clone()).map_err(|_| invalid())?;
                if u.total_tokens != u.prompt_tokens as u64 + u.completion_tokens as u64 {
                    return Err(invalid());
                }
                self.usage = Some(u);
                continue;
            }
            if choices.len() != 1 || self.finish.is_some() {
                return Err(invalid());
            }
            let choice = choices[0].as_object().ok_or_else(invalid)?;
            if choice
                .keys()
                .any(|k| !["index", "delta", "finish_reason"].contains(&k.as_str()))
                || choices[0]["index"] != 0
            {
                return Err(invalid());
            }
            let delta = choices[0]["delta"].as_object().ok_or_else(invalid)?;
            if delta
                .keys()
                .any(|k| !["role", "content"].contains(&k.as_str()))
            {
                return Err(invalid());
            }
            if let Some(finish) = choices[0].get("finish_reason").filter(|v| !v.is_null()) {
                let finish = finish
                    .as_str()
                    .filter(|f| ["stop", "length"].contains(f))
                    .ok_or_else(invalid)?;
                if !self.started || !delta.is_empty() {
                    return Err(invalid());
                }
                self.finish = Some(finish.into());
                continue;
            }
            if let Some(role) = delta.get("role") {
                if self.started || role != "assistant" || delta.len() != 1 {
                    return Err(invalid());
                }
                self.started = true;
                return Ok(Some(ChatEvent::Started));
            }
            if !self.started {
                return Err(invalid());
            }
            let text = delta
                .get("content")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .ok_or_else(invalid)?;
            return Ok(Some(ChatEvent::Delta { text: text.into() }));
        }
    }
}
fn invalid() -> BridgeError {
    BridgeError::new("stream_invalid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn chunk(id: Uuid, choice: Value) -> String {
        format!(
            "data: {}\n\n",
            json!({"id":format!("chatcmpl-{id}"),"model":"a","object":"chat.completion.chunk","created":1,"choices":[choice]})
        )
    }
    #[test]
    fn chunks_utf8_and_valid_finish_usage_done() {
        let id = Uuid::new_v4();
        let mut d = Decoder::new(id, "a".into());
        let start = chunk(
            id,
            json!({"index":0,"delta":{"role":"assistant"},"finish_reason":null}),
        );
        for b in start.bytes() {
            d.push(&[b]).unwrap();
        }
        assert_eq!(d.next().unwrap(), Some(ChatEvent::Started));
        let text = chunk(
            id,
            json!({"index":0,"delta":{"content":"你好😀"},"finish_reason":null}),
        );
        for b in text.bytes() {
            d.push(&[b]).unwrap();
        }
        assert_eq!(
            d.next().unwrap(),
            Some(ChatEvent::Delta {
                text: "你好😀".into()
            })
        );
        d.push(chunk(id, json!({"index":0,"delta":{},"finish_reason":"stop"})).as_bytes())
            .unwrap();
        assert!(d.next().unwrap().is_none());
        d.push(format!("data: {}\n\n",json!({"id":format!("chatcmpl-{id}"),"model":"a","object":"chat.completion.chunk","created":1,"choices":[],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}})).as_bytes()).unwrap();
        assert!(d.next().unwrap().is_none());
        d.push(b"data: [DONE]\n\n").unwrap();
        assert!(matches!(
            d.next().unwrap(),
            Some(ChatEvent::Completed { .. })
        ));
    }
    #[test]
    fn malformed_duplicate_invalid_utf8_and_oversized_are_rejected() {
        for bytes in [
            b"data: [DONE]\n\n".to_vec(),
            b"data: {\"id\":1,\"id\":2}\n\n".to_vec(),
            b"data: \xff\n\n".to_vec(),
            vec![b'x'; MAX_EVENT + 1],
            b"event: surprise\n\n".to_vec(),
        ] {
            let mut d = Decoder::new(Uuid::new_v4(), "a".into());
            d.push(&bytes).unwrap();
            assert!(d.next().is_err());
        }
        assert!(
            Decoder::new(Uuid::new_v4(), "a".into())
                .push(&vec![b'x'; MAX_DECODE + 1])
                .is_err()
        );
    }
}
