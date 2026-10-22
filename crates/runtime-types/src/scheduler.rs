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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedModel {
    pub id: ModelId,
    pub path: PathBuf,
    /// Integrity-checked companion asset belonging to this logical model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projector_path: Option<PathBuf>,
    pub context_limit: u32,
    pub default_context: u32,
    /// Store-authorized controlled attempt after integrity checks. This does
    /// not assert historical validation, native compatibility or available RAM.
    pub loadable: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationRequest {
    pub request_id: RequestId,
    pub model: ModelId,
    pub messages: Vec<Message>,
    pub options: GenerationOptions,
    #[serde(default)]
    pub tools: crate::ToolConfig,
}
impl GenerationRequest {
    pub fn uses_tools(&self) -> bool {
        self.tools.is_active(&self.messages)
    }
    pub fn validate(&self) -> Result<(), RuntimeError> {
        self.tools.validate_input(&self.messages)?;
        self.options.validate()
    }
}

#[derive(Clone, Debug)]
pub struct RuntimeConfig {
    pub max_queued_jobs: usize,
    pub queue_timeout: Duration,
    pub load_timeout: Duration,
    pub execution_timeout: Duration,
    /// Retain the positive idle budget even when automatic unloading is disabled.
    pub idle_unload: Duration,
    pub idle_unload_enabled: bool,
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
            idle_unload_enabled: true,
            slow_consumer_timeout: Duration::from_secs(10),
            load_options: LoadOptions::default(),
        }
    }
}
impl RuntimeConfig {
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
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestTimings {
    pub queue_ms: u64,
    pub load_ms: u64,
    pub execution_ms: u64,
}
/// Engine phase measurements, in microseconds. Decode excludes synchronous output
/// delivery; output_callback_us records that delivery separately. Token counts retain
/// native Usage semantics (including the first sampled token and EOG).
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InferenceTimings {
    pub prepare_us: u64,
    pub prefill_us: u64,
    pub decode_us: u64,
    pub output_callback_us: u64,
}
impl InferenceTimings {
    /// Bound both fields and their sum for exact transport through JavaScript.
    pub fn is_valid(self) -> bool {
        [
            self.prepare_us,
            self.prefill_us,
            self.decode_us,
            self.output_callback_us,
        ]
        .into_iter()
        .try_fold(0_u64, u64::checked_add)
        .is_some_and(|sum| sum <= 9_007_199_254_740_991)
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestPerformance {
    pub timings: InferenceTimings,
    pub load_options: LoadOptions,
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
    ToolCallDelta(crate::ToolCallDelta),
    Completed {
        usage: Usage,
        finish_reason: FinishReason,
        timings: RequestTimings,
        performance: Option<Box<RequestPerformance>>,
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
        for max_queued_jobs in [0, 1, 8] {
            let desktop = RuntimeConfig {
                max_queued_jobs,
                ..RuntimeConfig::default()
            };
            assert!(desktop.validate().is_ok());
        }
        let mut disabled = RuntimeConfig {
            idle_unload_enabled: false,
            ..RuntimeConfig::default()
        };
        assert!(disabled.validate().is_ok());
        disabled.idle_unload = Duration::ZERO;
        assert!(disabled.validate().is_err());
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum PerformanceStatus {
    Completed,
    Cancelled,
    Failed,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum PerformanceModality {
    Text,
    Image,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerformanceRecord {
    pub sequence: u64,
    pub request_id: RequestId,
    pub model_id: ModelId,
    pub modality: PerformanceModality,
    pub status: PerformanceStatus,
    pub accepted_at_unix_ms: u64,
    pub max_output_tokens: u32,
    pub usage: Usage,
    pub timings: RequestTimings,
    pub performance: Option<RequestPerformance>,
    pub error_code: Option<ErrorCode>,
    pub finish_reason: Option<FinishReason>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerformanceHistory {
    pub capacity: usize,
    pub records: Vec<PerformanceRecord>,
}
