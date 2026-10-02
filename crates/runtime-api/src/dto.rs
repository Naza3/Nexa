use crate::{Config, errors::ApiError};
use model_store::ModelManifest;
use runtime_types::{GenerationRequest, LoadOptions, ModelId, RequestId};
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

pub struct ValidatedChat {
    pub request: GenerationRequest,
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
        "tools",
        "response_format",
        "top_logprobs",
        "function_call",
        "functions",
        "modalities",
        "audio",
        "prediction",
        "store",
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
        .get("tool_choice")
        .is_some_and(|value| value.as_str() != Some("none"))
    {
        return Err(ApiError::unsupported("tool_choice"));
    }
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
    let model: ModelId = decoded(
        root.get("model")
            .cloned()
            .ok_or_else(|| ApiError::invalid("model", "Model ID is required."))?,
        "model",
    )?;
    let messages = root
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::invalid("messages", "An array of text messages is required."))?;
    if messages.is_empty() || messages.len() > 128 {
        return Err(ApiError::invalid("messages", "Expected 1..=128 messages."));
    }
    for (index, message) in messages.iter().enumerate() {
        let prefix = format!("messages.{index}");
        let message = object(message, &prefix)?;
        fields(message, &["role", "content"], &prefix)?;
        if message
            .get("content")
            .is_some_and(|value| !value.is_string())
        {
            return Err(ApiError::unsupported(&format!("{prefix}.content")));
        }
        if message
            .get("role")
            .and_then(Value::as_str)
            .is_some_and(|role| !["system", "user", "assistant"].contains(&role))
        {
            return Err(ApiError::unsupported(&format!("{prefix}.role")));
        }
    }
    let messages: Vec<runtime_types::Message> =
        decoded(Value::Array(messages.clone()), "messages")?;
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
    runtime_types::validate_messages(&messages).map_err(|_| ApiError::invalid("messages", "Messages must alternate user/assistant after an optional first system and end with user."))?;
    Ok(ValidatedChat {
        request: GenerationRequest {
            request_id,
            model,
            messages,
            options,
        },
        stream,
        include_usage,
    })
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
}
impl ImportModelRequest {
    pub fn parse(bytes: &[u8]) -> Result<Self, ApiError> {
        let value = parse_json(bytes)?;
        fields(
            object(&value, "body")?,
            &["id", "file", "display_name", "expected_sha256"],
            "",
        )?;
        let request: Self = decoded(value, "body")?;
        let file = request.file.to_string_lossy();
        if !request.file.is_absolute()
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
        let defaults = config.load_options();
        let options = LoadOptions {
            context_size: self.context_size.unwrap_or(defaults.context_size),
            threads: self.threads.unwrap_or(defaults.threads),
            batch_size: self.batch_size.unwrap_or(defaults.batch_size),
        };
        options.validate().map_err(|_| {
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
    pub storage: model_store::ModelStorage,
    pub availability_error: Option<String>,
    pub id: ModelId,
    pub display_name: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub architecture: String,
    pub quantization: String,
    pub validated: bool,
    pub available: bool,
    pub context_size: Option<u32>,
}
impl From<ModelManifest> for ModelSummary {
    fn from(model: ModelManifest) -> Self {
        Self {
            storage: model.storage,
            availability_error: None,
            id: model.id,
            display_name: model.display_name,
            size_bytes: model.size_bytes,
            sha256: model.sha256,
            architecture: model.architecture,
            quantization: model.quantization,
            validated: model.validated,
            available: model.validated && model.capabilities.chat,
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
            r#", "tools":[]"#,
            r#", "response_format":{}"#,
            r#", "frequency_penalty":0.1"#,
            r#", "max_tokens":1,"max_completion_tokens":1"#,
            r#", "stream_options":{"include_usage":true}"#,
        ] {
            assert!(chat(extra).is_err(), "{extra}");
        }
    }
    #[test]
    fn import_cannot_inject_internal_manifest_or_network_source() {
        for body in [
            r#"{"id":"a","file":"https://example.com/model"}"#,
            r#"{"id":"a","file":"relative.gguf"}"#,
            r#"{"id":"a","file":"/tmp/a","validated":true}"#,
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
