//! Runtime scheduling contract, independent of any native backend or transport.
use crate::{
    ErrorCode, FinishReason, GenerationOptions, LoadOptions, Message, RuntimeError, Usage,
};
use serde::{Deserialize, Deserializer, Serialize};
use std::{fmt, path::PathBuf, str::FromStr, time::Duration};

#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ModelId(String);
impl ModelId {
    pub fn new(value: impl Into<String>) -> Result<Self, RuntimeError> {
        let value = value.into();
        let valid = (1..=64).contains(&value.len())
            && (value.as_bytes()[0].is_ascii_lowercase() || value.as_bytes()[0].is_ascii_digit())
            && value
                .bytes()
                .all(|b| (b.is_ascii_lowercase() || b.is_ascii_digit()) || b"._-".contains(&b));
        if !valid {
            return Err(RuntimeError::invalid(
                "model_id must match [a-z0-9][a-z0-9._-]{0,63}",
            ));
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl fmt::Display for ModelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl FromStr for ModelId {
    type Err = RuntimeError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}
impl TryFrom<&str> for ModelId {
    type Error = RuntimeError;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}
impl<'de> Deserialize<'de> for ModelId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Self::new(String::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RequestId(uuid::Uuid);
impl RequestId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}
impl Default for RequestId {
    fn default() -> Self {
        Self::new()
    }
}
impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl FromStr for RequestId {
    type Err = RuntimeError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        uuid::Uuid::parse_str(value)
            .map(Self)
            .map_err(|_| RuntimeError::invalid("request_id must be a UUID"))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedModel {
    pub id: ModelId,
    pub path: PathBuf,
    pub context_limit: u32,
    pub default_context: u32,
    pub validated: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationRequest {
    pub request_id: RequestId,
    pub model: ModelId,
    pub messages: Vec<Message>,
    pub options: GenerationOptions,
}
impl GenerationRequest {
    pub fn validate(&self) -> Result<(), RuntimeError> {
        crate::validate_messages(&self.messages)?;
        self.options.validate()
    }
}

#[derive(Clone, Debug)]
pub struct RuntimeConfig {
    pub max_queued_jobs: usize,
    pub queue_timeout: Duration,
    pub load_timeout: Duration,
    pub execution_timeout: Duration,
    pub idle_unload: Duration,
    pub slow_consumer_timeout: Duration,
    pub load_options: LoadOptions,
}
impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            max_queued_jobs: 8,
            queue_timeout: Duration::from_secs(120),
            load_timeout: Duration::from_secs(300),
            execution_timeout: Duration::from_secs(300),
            idle_unload: Duration::from_secs(300),
            slow_consumer_timeout: Duration::from_secs(10),
            load_options: LoadOptions::default(),
        }
    }
}
impl RuntimeConfig {
    pub fn android() -> Self {
        Self {
            max_queued_jobs: 1,
            idle_unload: Duration::from_secs(60),
            load_options: LoadOptions {
                context_size: 2048,
                ..LoadOptions::default()
            },
            ..Self::default()
        }
    }
    pub fn validate(&self) -> Result<(), RuntimeError> {
        self.load_options.validate()?;
        if self.max_queued_jobs > 8
            || [
                self.queue_timeout,
                self.load_timeout,
                self.execution_timeout,
                self.idle_unload,
                self.slow_consumer_timeout,
            ]
            .contains(&Duration::ZERO)
        {
            return Err(RuntimeError::new(
                ErrorCode::InvalidArgument,
                "queue capacity must be <=8 and deadlines must be positive",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelState {
    Unloaded,
    Loading,
    Ready,
    Generating,
    Unloading,
    Faulted,
}
#[derive(Clone, Debug)]
pub struct RuntimeStatus {
    pub state: ModelState,
    pub selected_model: Option<ModelId>,
    pub load_options: Option<LoadOptions>,
    pub active_request: Option<RequestId>,
    pub queued_jobs: usize,
    pub stopping: bool,
    /// A registry transaction owns an actor-granted exclusive reservation.
    pub registry_busy: bool,
    pub last_error: Option<RuntimeError>,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RequestTimings {
    pub queue_ms: u64,
    pub load_ms: u64,
    pub execution_ms: u64,
}
#[derive(Clone, Debug, PartialEq)]
pub struct RequestEvent {
    pub request_id: RequestId,
    pub seq: u64,
    pub kind: RequestEventKind,
}
#[derive(Clone, Debug, PartialEq)]
pub enum RequestEventKind {
    Accepted,
    Queued,
    Loading,
    Started {
        prompt_tokens: u32,
    },
    TextDelta(String),
    Completed {
        usage: Usage,
        finish_reason: FinishReason,
        timings: RequestTimings,
    },
    Cancelled {
        reason: ErrorCode,
        usage: Usage,
        timings: RequestTimings,
    },
    Failed {
        error: RuntimeError,
        usage: Usage,
        timings: RequestTimings,
    },
}
impl RequestEventKind {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed { .. } | Self::Cancelled { .. } | Self::Failed { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn model_ids_validate_exact_ascii_path_free_grammar() {
        for good in ["a", "0", "qa.small-v1_0", "con"] {
            assert!(ModelId::new(good).is_ok());
        }
        for bad in ["", ".", "..", "/x", "x/y", "x\\y", "A", "a:b", "你", "a\0b"] {
            assert!(ModelId::new(bad).is_err(), "{bad:?}");
        }
        assert!(ModelId::new("a".repeat(64)).is_ok());
        assert!(ModelId::new("a".repeat(65)).is_err());
    }
    #[test]
    fn ids_and_configuration_are_checked() {
        let id = RequestId::new();
        assert_eq!(id.to_string().parse::<RequestId>().unwrap(), id);
        assert!("not-uuid".parse::<RequestId>().is_err());
        let mut config = RuntimeConfig {
            max_queued_jobs: 9,
            ..RuntimeConfig::default()
        };
        assert!(config.validate().is_err());
        config.max_queued_jobs = 8;
        config.load_timeout = Duration::ZERO;
        assert!(config.validate().is_err());
        assert_eq!(RuntimeConfig::android().max_queued_jobs, 1);
    }
}
