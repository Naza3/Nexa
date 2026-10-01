//! Platform-independent types for the native inference boundary.
//!
//! This crate deliberately contains no HTTP types, native pointers, or scheduling
//! implementation. Public wire DTOs will be introduced with their owning layers.

use serde::{Deserialize, Serialize};
use std::fmt;

mod scheduler;
pub use scheduler::*;

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_MESSAGES: usize = 128;
pub const MAX_STOPS: usize = 4;
pub const MAX_STOP_BYTES: usize = 128;
pub const MAX_OUTPUT_TOKENS: u32 = 4096;
/// Mirrors the maximum API request size, before template expansion.
pub const MAX_MESSAGE_BYTES: usize = 1_048_576;
/// llama.cpp uses this value to request a fresh random seed.
pub const RANDOM_SEED: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
}

impl Role {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }
}

/// Debug intentionally omits request text.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn new(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
        }
    }
}

impl fmt::Debug for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Message")
            .field("role", &self.role)
            .field("content_bytes", &self.content.len())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoadOptions {
    pub context_size: u32,
    pub threads: u32,
    pub batch_size: u32,
}

impl Default for LoadOptions {
    fn default() -> Self {
        Self {
            context_size: 4096,
            threads: 4,
            batch_size: 512,
        }
    }
}

impl LoadOptions {
    pub fn validate(self) -> Result<(), RuntimeError> {
        if !(32..=131_072).contains(&self.context_size) {
            return Err(RuntimeError::invalid("context_size must be in 32..=131072"));
        }
        if !(1..=256).contains(&self.threads) {
            return Err(RuntimeError::invalid("threads must be in 1..=256"));
        }
        if self.batch_size == 0 || self.batch_size > self.context_size.min(4096) {
            return Err(RuntimeError::invalid(
                "batch_size must be in 1..=min(context_size, 4096)",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationOptions {
    pub max_tokens: u32,
    pub temperature: f32,
    pub top_p: f32,
    /// RANDOM_SEED requests random sampling. Other values are best-effort seeds,
    /// reproducible only with the same model, backend, toolchain, and hardware.
    pub seed: u32,
    pub stops: Vec<String>,
}

impl Default for GenerationOptions {
    fn default() -> Self {
        Self {
            max_tokens: 512,
            temperature: 0.7,
            top_p: 0.9,
            seed: RANDOM_SEED,
            stops: Vec::new(),
        }
    }
}

impl GenerationOptions {
    pub fn validate(&self) -> Result<(), RuntimeError> {
        if !(1..=MAX_OUTPUT_TOKENS).contains(&self.max_tokens) {
            return Err(RuntimeError::invalid("max_tokens must be in 1..=4096"));
        }
        if !self.temperature.is_finite() || !(0.0..=2.0).contains(&self.temperature) {
            return Err(RuntimeError::invalid(
                "temperature must be finite and in 0..=2",
            ));
        }
        if !self.top_p.is_finite() || !(0.0 < self.top_p && self.top_p <= 1.0) {
            return Err(RuntimeError::invalid("top_p must be finite and in (0, 1]"));
        }
        if self.stops.len() > MAX_STOPS {
            return Err(RuntimeError::invalid(
                "at most four stop strings are supported",
            ));
        }
        if self
            .stops
            .iter()
            .any(|stop| stop.is_empty() || stop.len() > MAX_STOP_BYTES)
        {
            return Err(RuntimeError::invalid(
                "stop strings must contain 1..=128 UTF-8 bytes",
            ));
        }
        Ok(())
    }
}

/// Checks the v0.1 conversation contract without truncating or modifying input.
pub fn validate_messages(messages: &[Message]) -> Result<(), RuntimeError> {
    if messages.is_empty() || messages.len() > MAX_MESSAGES {
        return Err(RuntimeError::invalid(
            "messages must contain 1..=128 entries",
        ));
    }
    let mut expected = Role::User;
    let mut total_bytes = 0usize;
    for (index, message) in messages.iter().enumerate() {
        total_bytes = total_bytes
            .checked_add(message.content.len())
            .ok_or_else(|| RuntimeError::invalid("message content length overflow"))?;
        if total_bytes > MAX_MESSAGE_BYTES {
            return Err(RuntimeError::invalid(
                "message contents exceed the 1 MiB input bound",
            ));
        }
        if index == 0 && message.role == Role::System {
            continue;
        }
        if message.role != expected {
            return Err(RuntimeError::invalid(
                "messages must alternate user/assistant after an optional initial system",
            ));
        }
        expected = if expected == Role::User {
            Role::Assistant
        } else {
            Role::User
        };
    }
    if messages
        .last()
        .is_none_or(|message| message.role != Role::User)
    {
        return Err(RuntimeError::invalid(
            "the final message must have role user",
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    /// Includes the complete template and special tokens.
    pub prompt_tokens: u32,
    /// Includes sampled terminal/stop tokens, even when not emitted as text.
    pub completion_tokens: u32,
}

impl Usage {
    pub const fn total_tokens(self) -> u64 {
        self.prompt_tokens as u64 + self.completion_tokens as u64
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    Length,
}

impl FinishReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stop => "stop",
            Self::Length => "length",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidArgument,
    RequestCancelled,
    UnsupportedModel,
    UnsupportedChatTemplate,
    ContextLengthExceeded,
    NativeFailure,
    WrongThread,
    ConsumerStopped,
    #[serde(rename = "native_protocol_error")]
    NativeProtocol,
    ModelNotFound,
    InvalidManifest,
    Io,
    IntegrityFailure,
    AlreadyExists,
    InsufficientSpace,
    ModelConflict,
    RuntimeBusy,
    RuntimeFaulted,
    RuntimeShutdown,
    QueueFull,
    DuplicateRequestId,
    RequestNotFound,
    QueueTimeout,
    LoadTimeout,
    ExecutionTimeout,
    SlowConsumer,
    ExecutorUnavailable,
    ExecutorCleanupUnconfirmed,
}

impl ErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidArgument => "invalid_argument",
            Self::RequestCancelled => "request_cancelled",
            Self::UnsupportedModel => "unsupported_model",
            Self::UnsupportedChatTemplate => "unsupported_chat_template",
            Self::ContextLengthExceeded => "context_length_exceeded",
            Self::NativeFailure => "native_failure",
            Self::WrongThread => "wrong_thread",
            Self::ConsumerStopped => "consumer_stopped",
            Self::NativeProtocol => "native_protocol_error",
            Self::ModelNotFound => "model_not_found",
            Self::InvalidManifest => "invalid_manifest",
            Self::Io => "io",
            Self::IntegrityFailure => "integrity_failure",
            Self::AlreadyExists => "already_exists",
            Self::InsufficientSpace => "insufficient_space",
            Self::ModelConflict => "model_conflict",
            Self::RuntimeBusy => "runtime_busy",
            Self::RuntimeFaulted => "runtime_faulted",
            Self::RuntimeShutdown => "runtime_shutdown",
            Self::QueueFull => "queue_full",
            Self::DuplicateRequestId => "duplicate_request_id",
            Self::RequestNotFound => "request_not_found",
            Self::QueueTimeout => "queue_timeout",
            Self::LoadTimeout => "load_timeout",
            Self::ExecutionTimeout => "execution_timeout",
            Self::SlowConsumer => "slow_consumer",
            Self::ExecutorUnavailable => "executor_unavailable",
            Self::ExecutorCleanupUnconfirmed => "executor_cleanup_unconfirmed",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeError {
    pub code: ErrorCode,
    /// Diagnostic text for the trusted host; do not expose native paths over HTTP.
    pub message: String,
}

impl RuntimeError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidArgument, message)
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}
impl std::error::Error for RuntimeError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_conversations_keep_unicode_and_nul() {
        let messages = [
            Message::new(Role::System, "摘要"),
            Message::new(Role::User, "你好\0🙂"),
            Message::new(Role::Assistant, "你好"),
            Message::new(Role::User, "继续"),
        ];
        assert!(validate_messages(&messages).is_ok());
        assert!(!format!("{:?}", messages[1]).contains("你好"));
    }

    #[test]
    fn invalid_order_is_rejected() {
        for roles in [
            vec![],
            vec![Role::System],
            vec![Role::Assistant],
            vec![Role::User, Role::Assistant],
            vec![Role::User, Role::User],
            vec![Role::User, Role::System, Role::User],
        ] {
            let messages: Vec<_> = roles
                .into_iter()
                .map(|role| Message::new(role, "x"))
                .collect();
            assert!(validate_messages(&messages).is_err());
        }
    }

    #[test]
    fn message_limits_are_bounded() {
        assert!(validate_messages(&vec![Message::new(Role::User, "x"); MAX_MESSAGES + 1]).is_err());
        assert!(
            validate_messages(&[Message::new(Role::User, "x".repeat(MAX_MESSAGE_BYTES + 1))])
                .is_err()
        );
    }

    #[test]
    fn sampling_rejects_nan_infinity_and_out_of_range() {
        for temperature in [f32::NAN, f32::INFINITY, -0.1, 2.1] {
            assert!(
                GenerationOptions {
                    temperature,
                    ..Default::default()
                }
                .validate()
                .is_err()
            );
        }
        for top_p in [f32::NAN, f32::NEG_INFINITY, 0.0, 1.1] {
            assert!(
                GenerationOptions {
                    top_p,
                    ..Default::default()
                }
                .validate()
                .is_err()
            );
        }
        for max_tokens in [0, MAX_OUTPUT_TOKENS + 1] {
            assert!(
                GenerationOptions {
                    max_tokens,
                    ..Default::default()
                }
                .validate()
                .is_err()
            );
        }
        assert!(
            GenerationOptions {
                temperature: 0.0,
                top_p: 1.0,
                ..Default::default()
            }
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn stop_limits_measure_utf8_bytes() {
        assert!(
            GenerationOptions {
                stops: vec!["🙂".repeat(32)],
                ..Default::default()
            }
            .validate()
            .is_ok()
        );
        for stops in [
            vec![String::new()],
            vec!["🙂".repeat(33)],
            vec!["a".into(); 5],
        ] {
            assert!(
                GenerationOptions {
                    stops,
                    ..Default::default()
                }
                .validate()
                .is_err()
            );
        }
    }

    #[test]
    fn load_options_and_usage_do_not_overflow() {
        assert!(LoadOptions::default().validate().is_ok());
        assert!(
            LoadOptions {
                threads: 0,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            LoadOptions {
                batch_size: 4097,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert_eq!(
            Usage {
                prompt_tokens: u32::MAX,
                completion_tokens: u32::MAX
            }
            .total_tokens(),
            8_589_934_590
        );
    }
}
