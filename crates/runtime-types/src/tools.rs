//! Bounded tool protocol data. Schemas are preserved metadata, not constrained
//! decoding instructions; the caller remains responsible for tool execution.
use crate::{ErrorCode, MAX_MESSAGE_BYTES, Message, Role, RuntimeError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

pub const MAX_TOOLS: usize = 64;
pub const MAX_TOOL_CALLS: usize = 16;
pub const MAX_HISTORY_TOOL_CALLS: usize = 128;
pub const MAX_TOOL_NAME_BYTES: usize = 64;
pub const MAX_TOOL_ID_BYTES: usize = 128;
pub const MAX_TOOL_SCHEMA_BYTES: usize = 64 * 1024;
pub const MAX_TOOL_DEFINITION_BYTES: usize = 512 * 1024;
pub const MAX_TOOL_DESCRIPTION_BYTES: usize = 16 * 1024;
pub const MAX_TOOL_ARGUMENT_BYTES: usize = 16 * 1024;
pub const MAX_TOOL_OUTPUT_BYTES: usize = 64 * 1024;
pub const MAX_TOOL_RAW_BYTES: usize = MAX_TOOL_OUTPUT_BYTES;
pub const MAX_TOOL_CONTENT_BYTES: usize = 8 * 1024;
pub const MAX_TOOL_RESULT_BYTES: usize = 256 * 1024;
pub const MAX_TOOL_JSON_DEPTH: usize = 32;
pub const MAX_TOOL_JSON_NODES: usize = 8192;

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolDefinition {
    pub name: String,
    pub description: Option<String>,
    pub parameters: Value,
}
impl fmt::Debug for ToolDefinition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolDefinition")
            .field("name_bytes", &self.name.len())
            .field(
                "description_bytes",
                &self.description.as_ref().map_or(0, String::len),
            )
            .finish_non_exhaustive()
    }
}
#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "mode",
    content = "name",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ToolChoice {
    #[default]
    None,
    Auto,
    Required,
    Function(String),
}
impl fmt::Debug for ToolChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::None => "None",
            Self::Auto => "Auto",
            Self::Required => "Required",
            Self::Function(_) => "Function",
        })
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolConfig {
    pub definitions: Vec<ToolDefinition>,
    pub choice: ToolChoice,
    pub parallel_tool_calls: bool,
}
impl Default for ToolConfig {
    fn default() -> Self {
        Self {
            definitions: Vec::new(),
            choice: ToolChoice::None,
            parallel_tool_calls: true,
        }
    }
}
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}
impl fmt::Debug for ToolCall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolCall")
            .field("id_bytes", &self.id.len())
            .field("name_bytes", &self.name.len())
            .field("arguments_bytes", &self.arguments.len())
            .finish()
    }
}
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ToolCallDelta {
    Start {
        index: u32,
        id: String,
        name: String,
    },
    Arguments {
        index: u32,
        arguments: String,
    },
}
impl ToolCallDelta {
    pub fn validate_piece(&self) -> Result<(), RuntimeError> {
        let valid = match self {
            Self::Start { index, id, name } => {
                (*index as usize) < MAX_TOOL_CALLS && valid_tool_id(id) && valid_tool_name(name)
            }
            Self::Arguments { index, arguments } => {
                (*index as usize) < MAX_TOOL_CALLS
                    && !arguments.is_empty()
                    && arguments.len() <= 4096
            }
        };
        if valid {
            Ok(())
        } else {
            Err(RuntimeError::new(
                ErrorCode::NativeProtocol,
                "invalid tool delta",
            ))
        }
    }
    pub fn payload_bytes(&self) -> usize {
        match self {
            Self::Start { id, name, .. } => id.len() + name.len(),
            Self::Arguments { arguments, .. } => arguments.len(),
        }
    }
}
impl fmt::Debug for ToolCallDelta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Start { index, id, name } => f
                .debug_struct("ToolCallStart")
                .field("index", index)
                .field("id_bytes", &id.len())
                .field("name_bytes", &name.len())
                .finish(),
            Self::Arguments { index, arguments } => f
                .debug_struct("ToolArguments")
                .field("index", index)
                .field("argument_bytes", &arguments.len())
                .finish(),
        }
    }
}

pub fn valid_tool_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_TOOL_NAME_BYTES
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}
pub fn valid_tool_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_TOOL_ID_BYTES
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}
fn invalid() -> RuntimeError {
    RuntimeError::invalid("invalid or oversized tool protocol data")
}
fn add(total: &mut usize, count: usize) -> Result<(), RuntimeError> {
    *total = total
        .checked_add(count)
        .filter(|sum| *sum <= MAX_MESSAGE_BYTES)
        .ok_or_else(invalid)?;
    Ok(())
}
fn validate_json_bound(value: &Value, depth: usize, nodes: &mut usize) -> Result<(), RuntimeError> {
    *nodes += 1;
    if depth > MAX_TOOL_JSON_DEPTH || *nodes > MAX_TOOL_JSON_NODES {
        return Err(invalid());
    }
    match value {
        Value::Array(values) => {
            for value in values {
                validate_json_bound(value, depth + 1, nodes)?;
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                validate_json_bound(value, depth + 1, nodes)?;
            }
        }
        _ => {}
    }
    Ok(())
}

// Reject duplicate keys before Value can silently replace them. The parser's
// own recursion bound applies during allocation; tighter protocol bounds follow.
#[derive(Debug)]
struct UniqueJson(Value);
impl<'de> Deserialize<'de> for UniqueJson {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::{self, MapAccess, SeqAccess, Visitor};
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = UniqueJson;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("unique JSON")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(UniqueJson(v.into()))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(UniqueJson(v.into()))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(UniqueJson(v.into()))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
                serde_json::Number::from_f64(v)
                    .map(|n| UniqueJson(Value::Number(n)))
                    .ok_or_else(|| E::custom("number"))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(UniqueJson(v.into()))
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(UniqueJson(v.into()))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(UniqueJson(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(UniqueJson(v)) = seq.next_element()? {
                    values.push(v);
                }
                Ok(UniqueJson(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut values = serde_json::Map::new();
                while let Some(k) = map.next_key::<String>()? {
                    if values.contains_key(&k) {
                        return Err(de::Error::custom("duplicate key"));
                    }
                    let UniqueJson(v) = map.next_value()?;
                    values.insert(k, v);
                }
                Ok(UniqueJson(Value::Object(values)))
            }
        }
        d.deserialize_any(V)
    }
}
/// Exact JSON object, without repairs, duplicate keys, trailing data or excess
/// depth. JSON Schema semantics are intentionally not evaluated here.
pub fn validate_tool_arguments(arguments: &str) -> Result<(), RuntimeError> {
    if arguments.len() > MAX_TOOL_ARGUMENT_BYTES {
        return Err(invalid());
    }
    let UniqueJson(value) = serde_json::from_str(arguments).map_err(|_| invalid())?;
    if !value.is_object() {
        return Err(invalid());
    }
    validate_json_bound(&value, 0, &mut 0)
}
impl ToolConfig {
    pub fn is_active(&self, messages: &[Message]) -> bool {
        !self.definitions.is_empty()
            || self.choice != ToolChoice::None
            || messages
                .iter()
                .any(|m| m.role == Role::Tool || !m.tool_calls.is_empty())
    }
    pub fn validate(&self) -> Result<(), RuntimeError> {
        if self.definitions.len() > MAX_TOOLS {
            return Err(invalid());
        }
        let mut names = BTreeSet::new();
        let mut total = 0usize;
        for tool in &self.definitions {
            if !valid_tool_name(&tool.name)
                || !names.insert(&tool.name)
                || tool
                    .description
                    .as_ref()
                    .is_some_and(|s| s.len() > MAX_TOOL_DESCRIPTION_BYTES)
                || !tool.parameters.is_object()
            {
                return Err(invalid());
            }
            validate_json_bound(&tool.parameters, 0, &mut 0)?;
            let schema_bytes = serde_json::to_vec(&tool.parameters)
                .map_err(|_| invalid())?
                .len();
            if schema_bytes > MAX_TOOL_SCHEMA_BYTES {
                return Err(invalid());
            }
            add(
                &mut total,
                tool.name.len() + tool.description.as_ref().map_or(0, String::len) + schema_bytes,
            )?;
            if total > MAX_TOOL_DEFINITION_BYTES {
                return Err(invalid());
            }
        }
        match &self.choice {
            ToolChoice::Required if self.definitions.is_empty() => Err(invalid()),
            ToolChoice::Function(name) if !names.contains(name) => Err(invalid()),
            _ => Ok(()),
        }
    }
    /// Policy validation only: schemas remain metadata unless a future explicit
    /// constrained-decoding capability is introduced.
    pub fn validate_output(&self, calls: &[ToolCall]) -> Result<(), RuntimeError> {
        let fail = || {
            RuntimeError::new(
                ErrorCode::InvalidToolOutput,
                "model returned invalid tool calls",
            )
        };
        if calls.len() > MAX_TOOL_CALLS || (!self.parallel_tool_calls && calls.len() > 1) {
            return Err(fail());
        }
        match &self.choice {
            ToolChoice::None if !calls.is_empty() => return Err(fail()),
            ToolChoice::Required | ToolChoice::Function(_) if calls.is_empty() => {
                return Err(fail());
            }
            _ => {}
        }
        let mut ids = BTreeSet::new();
        let mut bytes = 0usize;
        for call in calls {
            if !valid_tool_id(&call.id)
                || !ids.insert(&call.id)
                || !self.definitions.iter().any(|t| t.name == call.name)
                || matches!(&self.choice, ToolChoice::Function(name) if *name != call.name)
            {
                return Err(fail());
            }
            validate_tool_arguments(&call.arguments).map_err(|_| fail())?;
            bytes = bytes
                .checked_add(call.id.len() + call.name.len() + call.arguments.len())
                .ok_or_else(fail)?;
            if bytes > MAX_TOOL_OUTPUT_BYTES {
                return Err(RuntimeError::new(
                    ErrorCode::ToolOutputLimitExceeded,
                    "tool output exceeds the byte limit",
                ));
            }
        }
        Ok(())
    }
    pub fn validate_input(&self, messages: &[Message]) -> Result<(), RuntimeError> {
        self.validate()?;
        crate::validate_messages(messages)?;
        if self.is_active(messages) && messages.iter().any(|m| m.image.is_some()) {
            return Err(invalid());
        }
        let mut bytes = if self.definitions.is_empty() {
            0
        } else {
            serde_json::to_vec(&self.definitions)
                .map_err(|_| invalid())?
                .len()
        };
        for message in messages {
            add(&mut bytes, message.content.as_ref().map_or(0, String::len))?;
            for call in &message.tool_calls {
                add(
                    &mut bytes,
                    call.id.len() + call.name.len() + call.arguments.len(),
                )?;
            }
            add(
                &mut bytes,
                message.tool_call_id.as_ref().map_or(0, String::len),
            )?;
        }
        Ok(())
    }
}
/// Validate history without looking up old calls in the *current* tool catalog.
/// A caller may retire tools between turns; that does not erase their results.
pub(crate) fn validate_tool_history(messages: &[Message]) -> Result<(), RuntimeError> {
    let mut pending = BTreeMap::new();
    let mut ids = BTreeSet::new();
    let mut after_results = false;
    let mut expect_user = true;
    let mut total_calls = 0;
    let mut bytes = 0usize;
    for (index, message) in messages.iter().enumerate() {
        add(&mut bytes, message.content.as_ref().map_or(0, String::len))?;
        if message.image.is_some() {
            return Err(invalid());
        }
        if message.role != Role::Assistant && !message.tool_calls.is_empty() {
            return Err(invalid());
        }
        if message.role != Role::Tool && message.tool_call_id.is_some() {
            return Err(invalid());
        }
        if (message.role != Role::Assistant || message.tool_calls.is_empty())
            && message.content.is_none()
        {
            return Err(invalid());
        }
        match message.role {
            Role::System if index == 0 => continue,
            Role::System => return Err(invalid()),
            Role::User => {
                if !pending.is_empty() || (!expect_user && !after_results) {
                    return Err(invalid());
                }
                expect_user = false;
                after_results = false;
            }
            Role::Assistant => {
                if !pending.is_empty() || (expect_user && !after_results) || index == 0 {
                    return Err(invalid());
                }
                if message.tool_calls.len() > MAX_TOOL_CALLS {
                    return Err(invalid());
                }
                for call in &message.tool_calls {
                    total_calls += 1;
                    if total_calls > MAX_HISTORY_TOOL_CALLS
                        || !valid_tool_id(&call.id)
                        || !valid_tool_name(&call.name)
                        || !ids.insert(call.id.as_str())
                    {
                        return Err(invalid());
                    }
                    validate_tool_arguments(&call.arguments)?;
                    add(
                        &mut bytes,
                        call.id.len() + call.name.len() + call.arguments.len(),
                    )?;
                    pending.insert(call.id.as_str(), call.name.as_str());
                }
                expect_user = true;
                after_results = false;
            }
            Role::Tool => {
                let id = message
                    .tool_call_id
                    .as_deref()
                    .filter(|id| valid_tool_id(id))
                    .ok_or_else(invalid)?;
                if pending.remove(id).is_none()
                    || message
                        .content
                        .as_ref()
                        .is_none_or(|s| s.len() > MAX_TOOL_RESULT_BYTES)
                {
                    return Err(invalid());
                }
                add(&mut bytes, id.len())?;
                after_results = pending.is_empty();
            }
        }
    }
    if !pending.is_empty()
        || messages
            .last()
            .is_none_or(|m| m.role != Role::User && !(m.role == Role::Tool && after_results))
    {
        return Err(invalid());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> ToolConfig {
        ToolConfig {
            definitions: vec![ToolDefinition {
                name: "Read".into(),
                description: Some("private description".into()),
                parameters: serde_json::json!({"type":"object","properties":{"tasks":{"type":"array","items":{"anyOf":[{"const":"pending"},{"type":"null"}]}}},"required":[]}),
            }],
            choice: ToolChoice::Auto,
            parallel_tool_calls: true,
        }
    }
    fn call() -> ToolCall {
        ToolCall {
            id: "call_123_0".into(),
            name: "Read".into(),
            arguments: "{\"path\":\"private path\"}".into(),
        }
    }
    #[test]
    fn schema_metadata_preserved_without_scalar_subset_or_strict_claim() {
        let tools = config();
        assert!(tools.validate().is_ok());
        let encoded = serde_json::to_vec(&tools).unwrap();
        let decoded: ToolConfig = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, tools);
        assert!(!format!("{tools:?}").contains("private description"));
        assert!(!format!("{:?}", call()).contains("private path"));
    }
    #[test]
    fn exact_object_arguments_fail_closed() {
        for bad in [
            "",
            "{",
            "{\"a\":1,\"a\":2}",
            "{\"x\":{\"y\":1,\"y\":2}}",
            "{}{}",
            "[]",
            "null",
            "{\"n\":1e999}",
        ] {
            assert!(validate_tool_arguments(bad).is_err(), "{bad}");
        }
        for good in ["{}", " {\"x\":[1,null,true,{\"中文\":\"🙂\"}]} \n"] {
            assert!(validate_tool_arguments(good).is_ok());
        }
    }
    #[test]
    fn selection_and_parallel_count_are_enforced() {
        let mut tools = config();
        let one = call();
        assert!(tools.validate_output(std::slice::from_ref(&one)).is_ok());
        tools.choice = ToolChoice::None;
        assert!(tools.validate_output(std::slice::from_ref(&one)).is_err());
        tools.choice = ToolChoice::Required;
        assert!(tools.validate_output(&[]).is_err());
        tools.choice = ToolChoice::Function("unknown".into());
        assert!(tools.validate().is_err());
        assert!(tools.validate_output(std::slice::from_ref(&one)).is_err());
        tools.choice = ToolChoice::Auto;
        tools.parallel_tool_calls = false;
        let mut two = one.clone();
        two.id = "other".into();
        assert!(tools.validate_output(&[one, two]).is_err());
    }
    #[test]
    fn complete_history_and_retired_tools_keep_null_content_and_ids() {
        let user = Message::new(Role::User, "read");
        let mut assistant = Message::new(Role::Assistant, "");
        assistant.content = None;
        assistant.tool_calls = vec![call()];
        let mut result = Message::new(Role::Tool, "done");
        result.tool_call_id = Some(call().id);
        let history = vec![user.clone(), assistant.clone(), result.clone()];
        assert!(ToolConfig::default().validate_input(&history).is_ok());
        for bad in [
            vec![user.clone(), assistant.clone()],
            vec![user.clone(), result.clone()],
            vec![user.clone(), assistant.clone(), result.clone(), result],
            vec![user, assistant, Message::new(Role::User, "interrupt")],
        ] {
            assert!(crate::validate_messages(&bad).is_err());
        }
    }
    #[test]
    fn per_call_and_catalog_limits_are_explicit() {
        let mut tools = config();
        tools.definitions = vec![tools.definitions[0].clone(); MAX_TOOLS + 1];
        assert!(tools.validate().is_err());
        assert!(
            validate_tool_arguments(&format!(
                "{{\"x\":\"{}\"}}",
                "x".repeat(MAX_TOOL_ARGUMENT_BYTES)
            ))
            .is_err()
        );
        assert!(
            ToolCallDelta::Arguments {
                index: 0,
                arguments: "x".repeat(4097)
            }
            .validate_piece()
            .is_err()
        );
    }
}
