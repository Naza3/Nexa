use crate::{Config, errors::ApiError};
use model_store::ModelManifest;
use runtime_types::{GenerationOptions, ImageInput, LoadOptions, Message, ModelId, RequestId};
use serde::{
    Deserialize, Serialize,
    de::{self, MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Value};
use std::{fmt, path::PathBuf};

/// serde_json::Value normally replaces duplicate object fields. This visitor
/// rejects them recursively BEFORE conversion to any typed request.
struct UniqueJson(Value);
impl<'de> Deserialize<'de> for UniqueJson {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct UniqueVisitor;
        impl<'de> Visitor<'de> for UniqueVisitor {
            type Value = UniqueJson;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a JSON value without duplicate fields")
            }
            fn visit_bool<E: de::Error>(self, value: bool) -> Result<Self::Value, E> {
                Ok(UniqueJson(Value::Bool(value)))
            }
            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(UniqueJson(value.into()))
            }
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(UniqueJson(value.into()))
            }
            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Self::Value, E> {
                serde_json::Number::from_f64(value)
                    .map(|n| UniqueJson(Value::Number(n)))
                    .ok_or_else(|| E::custom("non-finite JSON number"))
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(UniqueJson(Value::String(value.into())))
            }
            fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
                Ok(UniqueJson(Value::String(value)))
            }
            fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(UniqueJson(Value::Null))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(UniqueJson(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(UniqueJson(value)) = sequence.next_element()? {
                    values.push(value);
                }
                Ok(UniqueJson(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut entries: A) -> Result<Self::Value, A::Error> {
                let mut values = Map::new();
                while let Some(key) = entries.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(de::Error::custom("duplicate JSON field"));
                    }
                    let UniqueJson(value) = entries.next_value()?;
                    values.insert(key, value);
                }
                Ok(UniqueJson(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(UniqueVisitor)
    }
}
pub fn parse_json(bytes: &[u8]) -> Result<Value, ApiError> {
    serde_json::from_slice::<UniqueJson>(bytes)
        .map(|value| value.0)
        .map_err(|_| {
            ApiError::invalid(
                "body",
                "Malformed JSON, duplicate fields, or excessive nesting.",
            )
        })
}
fn object<'a>(value: &'a Value, param: &str) -> Result<&'a Map<String, Value>, ApiError> {
    value
        .as_object()
        .ok_or_else(|| ApiError::invalid(param, "Expected a JSON object."))
}
fn fields(value: &Map<String, Value>, allowed: &[&str], prefix: &str) -> Result<(), ApiError> {
    for key in value.keys() {
        if !allowed.contains(&key.as_str()) {
            let key: String = key.chars().take(128).collect();
            let param = if prefix.is_empty() {
                key
            } else {
                format!("{prefix}.{key}")
            };
            return Err(ApiError::invalid(&param, "Unknown request field."));
        }
    }
    Ok(())
}
fn decoded<T: de::DeserializeOwned>(value: Value, param: &str) -> Result<T, ApiError> {
    serde_json::from_value(value)
        .map_err(|_| ApiError::invalid(param, "Invalid field value or type."))
}
fn optional<T: de::DeserializeOwned>(
    value: &Map<String, Value>,
    name: &str,
) -> Result<Option<T>, ApiError> {
    value
        .get(name)
        .map(|value| decoded(value.clone(), name))
        .transpose()
}

fn parse_tools(root: &Map<String, Value>) -> Result<runtime_types::ToolConfig, ApiError> {
    use runtime_types::{MAX_TOOLS, ToolChoice, ToolConfig, ToolDefinition};
    let mut definitions = Vec::new();
    if let Some(value) = root.get("tools") {
        let values = value.as_array().ok_or_else(|| {
            ApiError::invalid("tools", "Expected an array of function definitions.")
        })?;
        if values.len() > MAX_TOOLS {
            return Err(ApiError::invalid("tools", "Too many tool definitions."));
        }
        for (index, value) in values.iter().enumerate() {
            let prefix = format!("tools.{index}");
            let tool = object(value, &prefix)?;
            fields(tool, &["type", "function"], &prefix)?;
            if tool.get("type").and_then(Value::as_str) != Some("function") {
                return Err(ApiError::unsupported(&format!("{prefix}.type")));
            }
            let prefix = format!("{prefix}.function");
            let function = object(tool.get("function").unwrap_or(&Value::Null), &prefix)?;
            fields(
                function,
                &["name", "description", "parameters", "strict"],
                &prefix,
            )?;
            if function
                .get("strict")
                .is_some_and(|value| value.as_bool() != Some(false))
            {
                return Err(ApiError::unsupported(&format!("{prefix}.strict")));
            }
            let name = function
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    ApiError::invalid(&format!("{prefix}.name"), "Expected a function name.")
                })?
                .to_owned();
            let description = function
                .get("description")
                .map(|value| decoded::<String>(value.clone(), &format!("{prefix}.description")))
                .transpose()?;
            let parameters = function
                .get("parameters")
                .filter(|v| v.is_object())
                .ok_or_else(|| {
                    ApiError::invalid(
                        &format!("{prefix}.parameters"),
                        "Expected JSON Schema object metadata.",
                    )
                })?
                .clone();
            definitions.push(ToolDefinition {
                name,
                description,
                parameters,
            });
        }
    }
    let choice = match root.get("tool_choice") {
        None => {
            if definitions.is_empty() {
                ToolChoice::None
            } else {
                ToolChoice::Auto
            }
        }
        Some(Value::String(value)) => match value.as_str() {
            "none" => ToolChoice::None,
            "auto" => ToolChoice::Auto,
            "required" => ToolChoice::Required,
            _ => return Err(ApiError::unsupported("tool_choice")),
        },
        Some(value) => {
            let choice = object(value, "tool_choice")?;
            fields(choice, &["type", "function"], "tool_choice")?;
            if choice.get("type").and_then(Value::as_str) != Some("function") {
                return Err(ApiError::unsupported("tool_choice.type"));
            }
            let function = object(
                choice.get("function").unwrap_or(&Value::Null),
                "tool_choice.function",
            )?;
            fields(function, &["name"], "tool_choice.function")?;
            ToolChoice::Function(
                function
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ApiError::invalid("tool_choice.function.name", "Expected a function name.")
                    })?
                    .to_owned(),
            )
        }
    };
    let tools = ToolConfig {
        definitions,
        choice,
        parallel_tool_calls: optional(root, "parallel_tool_calls")?.unwrap_or(true),
    };
    tools.validate().map_err(|_| {
        ApiError::invalid(
            "tools",
            "Invalid or oversized tool definitions or tool choice.",
        )
    })?;
    Ok(tools)
}
fn parse_history_calls(
    value: Option<&Value>,
    prefix: &str,
) -> Result<Vec<runtime_types::ToolCall>, ApiError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let prefix = format!("{prefix}.tool_calls");
    let calls = value
        .as_array()
        .ok_or_else(|| ApiError::invalid(&prefix, "Expected an array of tool calls."))?;
    if calls.is_empty() || calls.len() > runtime_types::MAX_TOOL_CALLS {
        return Err(ApiError::invalid(
            &prefix,
            "Expected a bounded nonempty call array.",
        ));
    }
    let mut result = Vec::with_capacity(calls.len());
    for (index, value) in calls.iter().enumerate() {
        let prefix = format!("{prefix}.{index}");
        let call = object(value, &prefix)?;
        fields(call, &["id", "type", "function"], &prefix)?;
        if call.get("type").and_then(Value::as_str) != Some("function") {
            return Err(ApiError::unsupported(&format!("{prefix}.type")));
        }
        let id = call
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| ApiError::invalid(&format!("{prefix}.id"), "Call ID is required."))?
            .to_owned();
        let function = object(call.get("function").unwrap_or(&Value::Null), &prefix)?;
        fields(
            function,
            &["name", "arguments"],
            &format!("{prefix}.function"),
        )?;
        let name = function
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| ApiError::invalid(&prefix, "Function name is required."))?
            .to_owned();
        let arguments = function
            .get("arguments")
            .and_then(Value::as_str)
            .ok_or_else(|| ApiError::invalid(&prefix, "Function arguments must be a JSON string."))?
            .to_owned();
        result.push(runtime_types::ToolCall {
            id,
            name,
            arguments,
        });
    }
    Ok(result)
}

pub struct ValidatedChat {
    pub request_id: RequestId,
    /// None requests atomic binding to an already loaded model in the actor.
    pub model: Option<ModelId>,
    pub messages: Vec<Message>,
    pub options: GenerationOptions,
    pub tools: runtime_types::ToolConfig,
    pub stream: bool,
    pub include_usage: bool,
}
pub fn parse_chat(
    bytes: &[u8],
    request_id: RequestId,
    config: &Config,
) -> Result<ValidatedChat, ApiError> {
    let value = parse_json(bytes)?;
    let root = object(&value, "body")?;
    for name in [
        "response_format",
        "top_logprobs",
        "function_call",
        "functions",
        "modalities",
        "audio",
        "prediction",
        "metadata",
        "reasoning_effort",
    ] {
        if root.contains_key(name) {
            return Err(ApiError::unsupported(name));
        }
    }
    fields(
        root,
        &[
            "model",
            "messages",
            "stream",
            "max_tokens",
            "max_completion_tokens",
            "temperature",
            "top_p",
            "seed",
            "stop",
            "n",
            "stream_options",
            "user",
            "frequency_penalty",
            "presence_penalty",
            "logprobs",
            "tool_choice",
            "tools",
            "parallel_tool_calls",
            "store",
        ],
        "",
    )?;
    for name in ["frequency_penalty", "presence_penalty"] {
        if root
            .get(name)
            .is_some_and(|value| value.as_f64() != Some(0.0))
        {
            return Err(ApiError::unsupported(name));
        }
    }
    if root
        .get("logprobs")
        .is_some_and(|value| value.as_bool() != Some(false))
    {
        return Err(ApiError::unsupported("logprobs"));
    }
    if root
        .get("store")
        .is_some_and(|value| value.as_bool() != Some(false))
    {
        return Err(ApiError::unsupported("store"));
    }
    let tools = parse_tools(root)?;
    if root.get("n").is_some_and(|value| value.as_u64() != Some(1)) {
        return Err(ApiError::unsupported("n"));
    }
    if root.contains_key("max_tokens") && root.contains_key("max_completion_tokens") {
        return Err(ApiError::invalid(
            "max_completion_tokens",
            "Supply only one output-token limit.",
        ));
    }
    if let Some(value) = root.get("user") {
        let user = value
            .as_str()
            .ok_or_else(|| ApiError::invalid("user", "Expected a string."))?;
        if user.chars().count() > 128 {
            return Err(ApiError::invalid(
                "user",
                "User identifier exceeds 128 characters.",
            ));
        }
    }
    let model = match root.get("model") {
        None => None,
        Some(Value::String(value)) if value.trim().is_empty() => None,
        Some(value) => Some(decoded::<ModelId>(value.clone(), "model")?),
    };
    let messages = root
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::invalid("messages", "An array of text messages is required."))?;
    if messages.is_empty() || messages.len() > 128 {
        return Err(ApiError::invalid("messages", "Expected 1..=128 messages."));
    }
    let mut decoded_messages = Vec::with_capacity(messages.len());
    for (index, message) in messages.iter().enumerate() {
        let prefix = format!("messages.{index}");
        let message = object(message, &prefix)?;
        fields(
            message,
            &["role", "content", "tool_calls", "tool_call_id"],
            &prefix,
        )?;
        if message
            .get("role")
            .and_then(Value::as_str)
            .is_some_and(|role| !["system", "user", "assistant", "tool"].contains(&role))
        {
            return Err(ApiError::unsupported(&format!("{prefix}.role")));
        }
        let role = decoded(
            message.get("role").cloned().unwrap_or(Value::Null),
            &format!("{prefix}.role"),
        )?;
        let tool_calls = parse_history_calls(message.get("tool_calls"), &prefix)?;
        let tool_call_id = message
            .get("tool_call_id")
            .map(|value| decoded::<String>(value.clone(), &format!("{prefix}.tool_call_id")))
            .transpose()?;
        let content = message.get("content").unwrap_or(&Value::Null);
        if let Some(text) = content.as_str() {
            decoded_messages.push(Message::new(role, text));
        } else if let Some(parts) = content.as_array() {
            if !parts
                .iter()
                .any(|part| part.get("type").and_then(Value::as_str) == Some("image_url"))
            {
                return Err(ApiError::unsupported(&format!("{prefix}.content")));
            }
            if parts.len() != 2 {
                return Err(ApiError::invalid(
                    &prefix,
                    "OCR requires exactly one image_url and one text part.",
                ));
            }
            let mut text = None;
            let mut image = None;
            for (part_index, part) in parts.iter().enumerate() {
                let part = object(part, &prefix)?;
                match part.get("type").and_then(Value::as_str) {
                    Some("text") if text.is_none() => {
                        fields(part, &["type", "text"], &prefix)?;
                        text = Some(part.get("text").and_then(Value::as_str).ok_or_else(|| {
                            ApiError::invalid(&prefix, "Text part must contain a string.")
                        })?);
                    }
                    Some("image_url") if image.is_none() => {
                        fields(part, &["type", "image_url"], &prefix)?;
                        let image_url =
                            object(part.get("image_url").unwrap_or(&Value::Null), &prefix)?;
                        fields(image_url, &["url"], &prefix)?;
                        let url =
                            image_url
                                .get("url")
                                .and_then(Value::as_str)
                                .ok_or_else(|| {
                                    ApiError::invalid(
                                        &prefix,
                                        "Image URL must be a PNG/JPEG data URL.",
                                    )
                                })?;
                        let mut input = ImageInput::from_data_url(url)
                            .map_err(|_| ApiError::invalid(&prefix, "Invalid image: require inline PNG/JPEG, <=4 MiB, <=8192 pixels per side and <=16 megapixels."))?;
                        input.after_text = part_index == 1;
                        image = Some(input);
                    }
                    _ => return Err(ApiError::unsupported(&format!("{prefix}.content"))),
                }
            }
            let mut decoded_message = Message::new(
                role,
                text.ok_or_else(|| ApiError::invalid(&prefix, "OCR text part is required."))?,
            );
            decoded_message.image = Some(
                image.ok_or_else(|| ApiError::invalid(&prefix, "OCR image part is required."))?,
            );
            decoded_messages.push(decoded_message);
        } else if content.is_null()
            && role == runtime_types::Role::Assistant
            && !tool_calls.is_empty()
        {
            let mut message = Message::new(role, "");
            message.content = None;
            decoded_messages.push(message);
        } else {
            return Err(ApiError::unsupported(&format!("{prefix}.content")));
        }
        // At least one content branch above has appended exactly one message.
        let decoded = decoded_messages.last_mut().expect("one decoded message");
        decoded.tool_calls = tool_calls;
        decoded.tool_call_id = tool_call_id;
    }
    let messages = decoded_messages;
    let stream = optional(root, "stream")?.unwrap_or(false);
    let include_usage = if let Some(value) = root.get("stream_options") {
        if !stream {
            return Err(ApiError::invalid(
                "stream_options",
                "stream_options requires stream=true.",
            ));
        }
        let options = object(value, "stream_options")?;
        fields(options, &["include_usage"], "stream_options")?;
        optional(options, "include_usage")?.unwrap_or(false)
    } else {
        false
    };
    let mut options = config.generation_options();
    if let Some(value) = optional(root, "max_tokens")?.or(optional(root, "max_completion_tokens")?)
    {
        options.max_tokens = value;
    }
    if let Some(value) = optional(root, "temperature")? {
        options.temperature = value;
    }
    if let Some(value) = optional(root, "top_p")? {
        options.top_p = value;
    }
    if let Some(value) = optional(root, "seed")? {
        options.seed = value;
    }
    if let Some(value) = root.get("stop") {
        options.stops = match value {
            Value::String(stop) => vec![stop.clone()],
            Value::Array(_) => decoded(value.clone(), "stop")?,
            _ => {
                return Err(ApiError::invalid(
                    "stop",
                    "Expected a string or array of strings.",
                ));
            }
        };
    }
    options.validate().map_err(|_| {
        ApiError::invalid(
            "parameters",
            "Invalid output-token limit, sampling value, or stop string.",
        )
    })?;
    tools.validate_input(&messages).map_err(|_| {
        ApiError::invalid(
            "messages",
            "Invalid message roles, tool-call relationships, JSON arguments, or input bounds.",
        )
    })?;
    Ok(ValidatedChat {
        request_id,
        model,
        messages,
        options,
        tools,
        stream,
        include_usage,
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportProjectorRequest {
    pub file: PathBuf,
    #[serde(default)]
    pub expected_sha256: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportModelRequest {
    pub id: ModelId,
    pub file: PathBuf,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub expected_sha256: Option<String>,
    #[serde(default)]
    pub projector: Option<ImportProjectorRequest>,
}
impl ImportModelRequest {
    pub fn parse(bytes: &[u8]) -> Result<Self, ApiError> {
        let value = parse_json(bytes)?;
        fields(
            object(&value, "body")?,
            &["id", "file", "display_name", "expected_sha256", "projector"],
            "",
        )?;
        let request: Self = decoded(value, "body")?;
        for selected in
            std::iter::once(&request.file).chain(request.projector.as_ref().map(|p| &p.file))
        {
            let file = selected.to_string_lossy();
            if !selected.is_absolute()
                || file.contains("://")
                || file.starts_with("\\\\")
                || file.starts_with("//")
                || file.contains('\0')
            {
                return Err(ApiError::invalid(
                    "file",
                    "Select an absolute local regular-file path; network and URL imports are unsupported.",
                ));
            }
        }
        Ok(request)
    }
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoadRequest {
    pub model: ModelId,
    pub backend: Option<String>,
    pub context_size: Option<u32>,
    pub gpu_layers: Option<u32>,
    pub threads: Option<u32>,
    pub batch_size: Option<u32>,
}
impl LoadRequest {
    pub fn parse(bytes: &[u8]) -> Result<Self, ApiError> {
        let value = parse_json(bytes)?;
        fields(
            object(&value, "body")?,
            &[
                "model",
                "backend",
                "context_size",
                "gpu_layers",
                "threads",
                "batch_size",
            ],
            "",
        )?;
        decoded(value, "body")
    }
    pub fn options(&self, config: &Config) -> Result<LoadOptions, ApiError> {
        if self.backend.as_deref().unwrap_or(&config.inference.backend) != "cpu" {
            return Err(ApiError::new(
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "backend_unavailable",
                "This build supports only the verified CPU backend.",
                Some("backend"),
            ));
        }
        if self.gpu_layers.unwrap_or(config.inference.gpu_layers) != 0 {
            return Err(ApiError::unsupported("gpu_layers"));
        }
        let options = crate::configuration::resolve_load_options(
            config,
            &self.model,
            crate::configuration::LoadOverrides {
                context_size: self.context_size,
                threads: self.threads,
                batch_size: self.batch_size,
            },
        )
        .map_err(|_| {
            ApiError::invalid(
                "load_options",
                "Invalid context size, thread count, or batch size.",
            )
        })?;
        Ok(options)
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct ModelSummary {
    pub compatibility: runtime_types::ModelCompatibility,
    pub storage: model_store::ModelStorage,
    pub availability_error: Option<String>,
    pub id: ModelId,
    pub display_name: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub architecture: String,
    pub quantization: String,
    pub validated: bool,
    /// Can attempt a controlled load; successful inference is not promised.
    pub loadable: bool,
    pub available: bool,
    pub context_limit: u32,
    pub context_size: Option<u32>,
    /// A paired asset is an attempt capability, not a validation label.
    pub has_projector: bool,
    pub projector_size_bytes: Option<u64>,
}
impl From<ModelManifest> for ModelSummary {
    fn from(model: ModelManifest) -> Self {
        let loadable = model.load_candidate();
        Self {
            has_projector: model.projector.is_some(),
            projector_size_bytes: model.projector.as_ref().map(|p| p.size_bytes),
            compatibility: model.compatibility(),
            storage: model.storage,
            availability_error: (!loadable)
                .then(|| runtime_types::ErrorCode::UnsupportedModel.as_str().into()),
            loadable,
            available: loadable,
            context_limit: model.context_limit.min(131_072),
            id: model.id,
            display_name: model.display_name,
            size_bytes: model.size_bytes,
            sha256: model.sha256,
            architecture: model.architecture,
            quantization: model.quantization,
            validated: model.validated,
            context_size: model.validation.map(|e| e.context_size),
        }
    }
}

impl ModelSummary {
    pub fn from_store(model: ModelManifest, store: &model_store::ModelStore) -> Self {
        let unavailable = store.external_availability(&model.id);
        let mut summary = Self::from(model);
        if let Some(code) = unavailable {
            summary.available = false;
            summary.availability_error = Some(code.as_str().into());
        }
        summary
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const PIXEL: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/lX8AAAAASUVORK5CYII=";
    fn ocr_body() -> Value {
        serde_json::json!({"model":"glm-ocr","messages":[{"role":"user","content":[
            {"type":"image_url","image_url":{"url":PIXEL}},
            {"type":"text","text":"Text Recognition:"}
        ]}],"max_tokens":2048,"temperature":0})
    }
    fn ocr_parse(body: &Value) -> Result<ValidatedChat, ApiError> {
        parse_chat(
            &serde_json::to_vec(body).unwrap(),
            RequestId::new(),
            &Config::default(),
        )
    }
    #[test]
    fn single_image_preserves_text_and_both_part_orders() {
        let mut body = ocr_body();
        for after_text in [false, true] {
            if after_text {
                body["messages"][0]["content"]
                    .as_array_mut()
                    .unwrap()
                    .reverse();
            }
            let parsed = ocr_parse(&body).unwrap();
            assert_eq!(parsed.messages.len(), 1);
            assert_eq!(
                parsed.messages[0].content.as_deref(),
                Some("Text Recognition:")
            );
            let image = parsed.messages[0].image.as_ref().unwrap();
            assert_eq!(image.after_text, after_text);
            assert_eq!(image.data_url(), PIXEL);
            assert_eq!(parsed.options.max_tokens, 2048);
        }
    }
    #[test]
    fn image_rejects_remote_files_ambiguous_parts_and_history() {
        for url in [
            "https://example.com/private.png",
            "file:///tmp/private.png",
            "data:image/gif;base64,AA==",
            "data:image/png;base64,!",
        ] {
            let mut body = ocr_body();
            body["messages"][0]["content"][0]["image_url"]["url"] = url.into();
            assert!(ocr_parse(&body).is_err());
        }
        for role in ["system", "assistant"] {
            let mut body = ocr_body();
            body["messages"][0]["role"] = role.into();
            assert!(ocr_parse(&body).is_err());
        }
        let mut body = ocr_body();
        body["messages"][0]["content"][1]["text"] = "  ".into();
        assert!(ocr_parse(&body).is_err());
        let mut body = ocr_body();
        body["messages"]
            .as_array_mut()
            .unwrap()
            .insert(0, serde_json::json!({"role":"system","content":"test"}));
        assert!(ocr_parse(&body).is_err());
        let mut body = ocr_body();
        body["messages"][0]["content"][1] = body["messages"][0]["content"][0].clone();
        assert!(ocr_parse(&body).is_err());
        let mut body = ocr_body();
        body["messages"][0]["content"][0]["image_url"]["detail"] = "auto".into();
        assert!(ocr_parse(&body).is_err());
    }
    #[test]
    fn paired_import_keeps_projector_local_and_rejects_manifest_injection() {
        let file = std::env::temp_dir().join("ocr-model.gguf");
        let projector = std::env::temp_dir().join("mmproj.gguf");
        let mut body = serde_json::json!({"id":"ocr","file":file,"projector":{"file":projector}});
        let parsed = ImportModelRequest::parse(&serde_json::to_vec(&body).unwrap()).unwrap();
        assert_eq!(parsed.projector.unwrap().file, projector);
        for source in [
            "relative.gguf",
            "https://example.com/mmproj.gguf",
            "//server/share/mmproj.gguf",
        ] {
            body["projector"]["file"] = source.into();
            assert!(ImportModelRequest::parse(&serde_json::to_vec(&body).unwrap()).is_err());
        }
        body["projector"]["file"] = serde_json::to_value(projector).unwrap();
        body["projector"]["validated"] = true.into();
        assert!(ImportModelRequest::parse(&serde_json::to_vec(&body).unwrap()).is_err());
    }
    fn chat(extra: &str) -> Result<ValidatedChat, ApiError> {
        parse_chat(
            format!(
                r#"{{"model":"qa-small","messages":[{{"role":"user","content":"你好"}}]{extra}}}"#
            )
            .as_bytes(),
            RequestId::new(),
            &Config::default(),
        )
    }
    #[test]
    fn rejects_duplicates_recursively_and_names_unknown_fields() {
        for body in [
            br#"{"a":1,"a":2}"#.as_slice(),
            br#"{"messages":[{"role":"user","role":"assistant"}]}"#,
        ] {
            assert!(parse_json(body).is_err());
        }
        assert_eq!(
            chat(r#", "surprise": true"#)
                .err()
                .unwrap()
                .error
                .param
                .as_deref(),
            Some("surprise")
        );
    }
    #[test]
    fn compatibility_noops_and_aliases_are_exact() {
        assert!(chat(r#", "n":1,"frequency_penalty":0,"presence_penalty":0.0,"logprobs":false,"tool_choice":"none","max_completion_tokens":1"#).is_ok());
        for extra in [
            r#", "n":2"#,
            r#", "response_format":{}"#,
            r#", "frequency_penalty":0.1"#,
            r#", "max_tokens":1,"max_completion_tokens":1"#,
            r#", "stream_options":{"include_usage":true}"#,
        ] {
            assert!(chat(extra).is_err(), "{extra}");
        }
    }
    #[test]
    fn current_model_selection_accepts_only_missing_or_blank_strings() {
        for model in [None, Some(""), Some(" \t\r\n"), Some("\u{2003}")] {
            let mut body = serde_json::json!({"messages":[{"role":"user","content":"hello"}]});
            if let Some(model) = model {
                body["model"] = Value::String(model.into());
            }
            let parsed = parse_chat(
                &serde_json::to_vec(&body).unwrap(),
                RequestId::new(),
                &Config::default(),
            )
            .unwrap();
            assert!(parsed.model.is_none(), "{model:?}");
        }
        assert_eq!(chat("").unwrap().model.unwrap().as_str(), "qa-small");
        for model in [
            Value::Null,
            serde_json::json!(0),
            serde_json::json!(false),
            serde_json::json!([]),
            serde_json::json!({}),
            serde_json::json!(" qa-small"),
            serde_json::json!("qa-small "),
            serde_json::json!("../qa-small"),
            serde_json::json!("UNKNOWN"),
        ] {
            let body =
                serde_json::json!({"model":model,"messages":[{"role":"user","content":"hello"}]});
            let error = parse_chat(
                &serde_json::to_vec(&body).unwrap(),
                RequestId::new(),
                &Config::default(),
            )
            .err()
            .unwrap();
            assert_eq!(error.error.code, "invalid_request", "{model}");
            assert_eq!(error.error.param.as_deref(), Some("model"));
        }
    }
    #[test]
    fn import_cannot_inject_internal_manifest_or_network_source() {
        for body in [
            r#"{"id":"a","file":"https://example.com/model"}"#,
            r#"{"id":"a","file":"relative.gguf"}"#,
            r#"{"id":"a","file":"/tmp/a","validated":true}"#,
            r#"{"id":"a","file":"/tmp/a","loadable":true}"#,
            r#"{"id":"a","file":"/tmp/a","path":"/other/file"}"#,
            r#"{"id":"a","file":"/tmp/a","context_limit":999999}"#,
            r#"{"id":"a","file":"/tmp/a","relative_file":"x"}"#,
        ] {
            assert!(ImportModelRequest::parse(body.as_bytes()).is_err());
        }
    }
    #[test]
    fn default_context_is_not_downgraded() {
        let load = LoadRequest::parse(br#"{"model":"a"}"#).unwrap();
        assert_eq!(load.options(&Config::default()).unwrap().context_size, 4096);
        assert!(
            LoadRequest::parse(
                br#"{"model":"a","context_size":2048,"threads":2,"batch_size":128}"#
            )
            .unwrap()
            .options(&Config::default())
            .is_ok()
        );
    }
}

/// Removes only an exact observed registration; unknown/deletion fields fail.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UnregisterModelRequest {
    pub model_id: ModelId,
    pub generation: uuid::Uuid,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UnregisterModelResult {
    pub model_id: ModelId,
    pub removed: bool,
    pub files_preserved: bool,
}
pub fn parse_unregister(bytes: &[u8]) -> Result<UnregisterModelRequest, ApiError> {
    let value = parse_json(bytes)?;
    let request: UnregisterModelRequest = decoded(value, "body")?;
    if request.generation.is_nil() {
        return Err(ApiError::invalid(
            "generation",
            "A current model list generation is required.",
        ));
    }
    Ok(request)
}

#[cfg(test)]
mod tool_tests {
    use super::*;
    use serde_json::json;
    fn body() -> Value {
        json!({"model":"fixture","messages":[{"role":"user","content":"make a list"}],"store":false,"tools":[{"type":"function","function":{"name":"TodoWrite","description":"Keep a checklist","strict":false,"parameters":{"type":"object","properties":{"todos":{"type":"array","items":{"type":"object","properties":{"content":{"type":"string","minLength":1},"status":{"anyOf":[{"const":"pending"},{"const":"completed"}]}}},"maxItems":50}},"required":["todos"]}}}]})
    }
    fn parse(value: &Value) -> Result<ValidatedChat, ApiError> {
        parse_chat(
            &serde_json::to_vec(value).unwrap(),
            RequestId::new(),
            &Config::default(),
        )
    }
    #[test]
    fn pi_style_schema_is_preserved_and_all_choice_modes_work() {
        let mut value = body();
        let schema = value["tools"][0]["function"]["parameters"].clone();
        for choice in [
            json!("none"),
            json!("auto"),
            json!("required"),
            json!({"type":"function","function":{"name":"TodoWrite"}}),
        ] {
            value["tool_choice"] = choice;
            let request = parse(&value).unwrap();
            assert_eq!(request.tools.definitions[0].parameters, schema);
            assert_eq!(request.messages[0].content.as_deref(), Some("make a list"));
        }
        value["parallel_tool_calls"] = json!(false);
        assert!(!parse(&value).unwrap().tools.parallel_tool_calls);
    }
    #[test]
    fn strict_and_store_true_are_never_accepted_as_noops() {
        for (pointer, replacement, param) in [
            (
                "/tools/0/function/strict",
                json!(true),
                "tools.0.function.strict",
            ),
            ("/store", json!(true), "store"),
        ] {
            let mut value = body();
            *value.pointer_mut(pointer).unwrap() = replacement;
            let error = parse(&value).err().unwrap();
            assert_eq!(error.error.code, "unsupported_parameter");
            assert_eq!(error.error.param.as_deref(), Some(param));
        }
        let mut value = body();
        value["tools"][0]["function"]["unexpected"] = json!(1);
        assert!(parse(&value).is_err());
    }
    #[test]
    fn tool_result_history_roundtrips_null_and_string_arguments() {
        let mut value = body();
        value["messages"].as_array_mut().unwrap().extend([
            json!({"role":"assistant","content":null,"tool_calls":[{"id":"call_one","type":"function","function":{"name":"TodoWrite","arguments":"{\"todos\":[]}"}}]}),
            json!({"role":"tool","tool_call_id":"call_one","content":"done"})
        ]);
        let request = parse(&value).unwrap();
        assert_eq!(request.messages[1].content, None);
        assert_eq!(
            request.messages[2].tool_call_id.as_deref(),
            Some("call_one")
        );
        value["messages"][1]
            .as_object_mut()
            .unwrap()
            .remove("content");
        assert!(parse(&value).is_ok());
        value["messages"][1]["tool_calls"][0]["function"]["arguments"] = json!({"todos":[]});
        assert!(parse(&value).is_err());
    }
    #[test]
    fn invalid_tool_history_never_reaches_scheduler() {
        let base = json!({"messages":[{"role":"user","content":"x"},{"role":"assistant","tool_calls":[{"id":"call_one","type":"function","function":{"name":"old","arguments":"{}"}}]},{"role":"tool","tool_call_id":"call_one","content":"done"}]});
        assert!(parse(&base).is_ok());
        for (pointer, replacement) in [
            ("/messages/2/tool_call_id", json!("unknown")),
            (
                "/messages/1/tool_calls/0/function/arguments",
                json!("{\"a\":1,\"a\":2}"),
            ),
            ("/messages/2/content", Value::Null),
        ] {
            let mut value = base.clone();
            *value.pointer_mut(pointer).unwrap() = replacement;
            assert!(parse(&value).is_err());
        }
        let mut value = base.clone();
        value["messages"].as_array_mut().unwrap().pop();
        assert!(parse(&value).is_err());
        let mut value = base;
        let result = value["messages"][2].clone();
        value["messages"].as_array_mut().unwrap().push(result);
        assert!(parse(&value).is_err());
    }
    #[test]
    fn empty_tools_store_false_and_none_keep_text_compatibility() {
        let value = json!({"messages":[{"role":"user","content":"hello"}],"tools":[],"tool_choice":"none","store":false,"n":1,"logprobs":false,"frequency_penalty":0,"presence_penalty":0});
        let request = parse(&value).unwrap();
        assert!(!request.tools.is_active(&request.messages));
        let mut value = value;
        value["tool_choice"] = json!("required");
        assert!(parse(&value).is_err());
    }
}

#[cfg(test)]
mod official_pi_fixtures {
    use super::*;
    // Captured by the official pi-ai 1.0.1 serializer with PI Desktop v0.17.0's
    // package patch. These verify request compatibility, not native generation.
    #[test]
    fn all_captured_pi_desktop_requests_preserve_schemas_and_history() {
        for (name, bytes) in [
            (
                "ordinary",
                include_bytes!("../../../tests/fixtures/pi-desktop/ordinary.json").as_slice(),
            ),
            (
                "core-tools",
                include_bytes!("../../../tests/fixtures/pi-desktop/core-tools.json").as_slice(),
            ),
            (
                "tool-first",
                include_bytes!("../../../tests/fixtures/pi-desktop/tool-first.json").as_slice(),
            ),
            (
                "tool-second",
                include_bytes!("../../../tests/fixtures/pi-desktop/tool-second.json").as_slice(),
            ),
            (
                "truncated",
                include_bytes!("../../../tests/fixtures/pi-desktop/truncated.json").as_slice(),
            ),
        ] {
            let value: Value = serde_json::from_slice(bytes).unwrap();
            let request = parse_chat(bytes, RequestId::new(), &Config::default())
                .unwrap_or_else(|error| panic!("{name}: {error:?}"));
            assert!(request.stream);
            assert!(request.include_usage);
            assert_eq!(request.options.max_tokens, 256);
            if let Some(tools) = value["tools"].as_array() {
                assert_eq!(request.tools.definitions.len(), tools.len());
                for (tool, wire) in request.tools.definitions.iter().zip(tools) {
                    assert_eq!(tool.name, wire["function"]["name"].as_str().unwrap());
                    assert_eq!(tool.parameters, wire["function"]["parameters"]);
                }
            }
            for (message, wire) in request
                .messages
                .iter()
                .zip(value["messages"].as_array().unwrap())
            {
                assert_eq!(message.content.as_deref(), wire["content"].as_str());
                if let Some(calls) = wire["tool_calls"].as_array() {
                    assert_eq!(message.tool_calls.len(), calls.len());
                    for (call, wire) in message.tool_calls.iter().zip(calls) {
                        assert_eq!(call.id, wire["id"].as_str().unwrap());
                        assert_eq!(
                            call.arguments,
                            wire["function"]["arguments"].as_str().unwrap()
                        );
                    }
                }
                assert_eq!(
                    message.tool_call_id.as_deref(),
                    wire["tool_call_id"].as_str()
                );
            }
        }
    }
}
