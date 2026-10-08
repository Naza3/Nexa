//! Local-only, session-scoped inference observations. No request or response text.
use runtime_types::{PerformanceRecord, PerformanceStatus};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PerformanceSnapshot {
    pub instance_id: uuid::Uuid,
    pub capacity: usize,
    pub records: Vec<PerformanceRecord>,
}

impl PerformanceSnapshot {
    /// Validate the bounded wire response before passing it into the webview.
    pub fn is_valid(&self) -> bool {
        const SAFE: u64 = 9_007_199_254_740_991;
        !self.instance_id.is_nil()
            && (1..=200).contains(&self.capacity)
            && self.records.len() <= self.capacity
            && self
                .records
                .windows(2)
                .all(|pair| pair[0].sequence > pair[1].sequence)
            && self.records.iter().all(|record| {
                (1..=SAFE).contains(&record.sequence)
                    && record.accepted_at_unix_ms <= SAFE
                    && record.max_output_tokens > 0
                    && record.usage.completion_tokens <= record.max_output_tokens
                    && record
                        .timings
                        .queue_ms
                        .checked_add(record.timings.load_ms)
                        .and_then(|sum| sum.checked_add(record.timings.execution_ms))
                        .is_some_and(|sum| sum <= SAFE)
                    && match record.status {
                        PerformanceStatus::Completed => {
                            record.error_code.is_none()
                                && record.finish_reason.is_some()
                                && record.performance.is_none_or(|performance| {
                                    performance.timings.is_valid()
                                        && performance.load_options.validate().is_ok()
                                        && record.usage.prompt_tokens > 0
                                })
                        }
                        PerformanceStatus::Cancelled | PerformanceStatus::Failed => {
                            record.performance.is_none()
                                && record.finish_reason.is_none()
                                && record.error_code.is_some()
                        }
                    }
            })
    }
}
