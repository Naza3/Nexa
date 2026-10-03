//! Deliberately bounded, typed WebView-facing values. No credential or source path DTO.
use runtime_types::{LoadOptions, Message, ModelId};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DesktopSettings {
    pub context_size: u32,
    pub threads: u32,
    pub batch_size: u32,
    pub max_output_tokens: u32,
    pub idle_unload_seconds: u64,
    pub close_runtime_on_exit: bool,
}
impl Default for DesktopSettings {
    fn default() -> Self {
        Self {
            context_size: 2048,
            threads: 2,
            batch_size: 128,
            max_output_tokens: 512,
            idle_unload_seconds: 300,
            close_runtime_on_exit: false,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DesktopPreferences {
    pub context_size: u32,
    pub threads: u32,
    pub batch_size: u32,
    pub max_output_tokens: u32,
    pub close_runtime_on_exit: bool,
}
impl Default for DesktopPreferences {
    fn default() -> Self {
        DesktopSettings::default().into()
    }
}
impl From<DesktopSettings> for DesktopPreferences {
    fn from(s: DesktopSettings) -> Self {
        Self {
            context_size: s.context_size,
            threads: s.threads,
            batch_size: s.batch_size,
            max_output_tokens: s.max_output_tokens,
            close_runtime_on_exit: s.close_runtime_on_exit,
        }
    }
}
impl DesktopPreferences {
    pub fn with_idle(self, idle_unload_seconds: u64) -> DesktopSettings {
        DesktopSettings {
            context_size: self.context_size,
            threads: self.threads,
            batch_size: self.batch_size,
            max_output_tokens: self.max_output_tokens,
            close_runtime_on_exit: self.close_runtime_on_exit,
            idle_unload_seconds,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    Stopped,
    Connecting,
    Connected,
    Error,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DesktopSnapshot {
    pub initialized: bool,
    pub connection: ConnectionState,
    pub api_address: Option<String>,
    pub runtime: Option<RuntimeStatus>,
    pub settings: DesktopSettings,
    pub model_directory: ModelDirectorySnapshot,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeState {
    Unloaded,
    Loading,
    Ready,
    Generating,
    Unloading,
    Faulted,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeStatus {
    pub state: RuntimeState,
    pub selected_model: Option<ModelId>,
    #[serde(default)]
    pub selected_model_display_name: Option<String>,
    #[serde(default)]
    pub model_library: Option<ModelLibraryObservation>,
    pub load_options: Option<LoadOptions>,
    pub active_request: Option<Uuid>,
    pub queued_jobs: usize,
    pub stopping: bool,
    pub registry_busy: bool,
    pub configured_backend: String,
    pub backend: Option<String>,
    pub backend_observation: String,
    pub last_error: Option<crate::BridgeError>,
    pub threads_source: Option<String>,
    pub available_parallelism: Option<u32>,
    pub threads_exceed_available_parallelism: Option<bool>,
    pub worker: WorkerStatus,
    pub memory: MemoryStatus,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkerStatus {
    pub pid: Option<u32>,
    pub sessions_started: Option<u64>,
    pub sessions_reaped: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemoryStatus {
    pub api_private_bytes: Option<u64>,
    pub worker_private_bytes: Option<u64>,
    pub gpu_bytes: Option<u64>,
    pub observation: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelSummary {
    #[serde(default)]
    pub compatibility: runtime_types::ModelCompatibility,
    #[serde(default)]
    pub storage: model_store::ModelStorage,
    #[serde(default)]
    pub availability_error: Option<String>,
    pub id: ModelId,
    pub display_name: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub architecture: String,
    pub quantization: String,
    pub validated: bool,
    #[serde(default)]
    pub loadable: bool,
    pub available: bool,
    #[serde(default)]
    pub context_limit: Option<u32>,
    pub context_size: Option<u32>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelsPage {
    pub generation: Uuid,
    pub data: Vec<ModelSummary>,
    pub next_after: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoadModelRequest {
    pub model_id: String,
    pub context_size: u32,
    pub threads: u32,
    pub batch_size: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatStartRequest {
    pub model_id: String,
    pub messages: Vec<Message>,
    pub max_output_tokens: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatEvent {
    Started,
    Delta { text: String },
    Completed { finish_reason: String, usage: Usage },
    Cancelled,
    Failed { code: String, message: String },
}
impl ChatEvent {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed { .. } | Self::Cancelled | Self::Failed { .. }
        )
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestHandle {
    pub request_id: Uuid,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatBatch {
    pub request_id: Uuid,
    pub events: Vec<ChatEvent>,
    pub terminal: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Stopping {
    pub request_id: Uuid,
    pub status: &'static str,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Stopped {
    pub stopped: bool,
}

/// Read-only path display. Native selection IDs, never this string, authorize writes.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelDirectoryInfo {
    pub directory_id: Uuid,
    pub display_path: String,
    pub library_generation: Uuid,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelDirectoryState {
    Default,
    Ready,
    Stopped,
    Stale,
    Missing,
    Unavailable,
    Unsupported,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelDirectorySnapshot {
    pub configured: Option<ModelDirectoryInfo>,
    pub effective: Option<ModelDirectoryInfo>,
    pub state: ModelDirectoryState,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LibraryOperationHandle {
    pub operation_id: Uuid,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LibraryOperationStatus {
    Running,
    Completed,
    Cancelled,
    Failed,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LibraryOperationPhase {
    Checking,
    Enumerating,
    Verifying,
    Committing,
    Finished,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LibraryOperationResult {
    pub library_generation: Uuid,
    pub directory_id: Uuid,
    pub registered_files: usize,
    pub available_files: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LibraryOperationState {
    pub operation_id: Uuid,
    pub status: LibraryOperationStatus,
    pub phase: LibraryOperationPhase,
    pub examined_entries: usize,
    pub candidate_files: usize,
    pub verified_files: usize,
    pub failed_file_name: Option<String>,
    pub terminal: bool,
    pub result: Option<LibraryOperationResult>,
    pub error: Option<crate::BridgeError>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LibraryStopping {
    pub operation_id: Uuid,
    pub status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelLibraryObservation {
    pub supported: bool,
    pub directory: Option<ModelDirectoryInfo>,
}

#[cfg(test)]
mod compatibility_tests {
    use super::*;
    use runtime_types::ModelCompatibility;
    use serde_json::json;

    #[test]
    fn api_compatibility_details_roundtrip_without_reinterpreting_source_failures() {
        for compatibility in [
            "admitted",
            "architecture_unsupported",
            "quantization_unvalidated",
            "template_unvalidated",
            "context_unvalidated",
            "artifact_unvalidated",
            "unvalidated",
        ] {
            let value = json!({
                "id":"fixture", "display_name":"中文 模型", "size_bytes":64,
                "sha256":"0".repeat(64), "architecture":"qwen3", "quantization":"Q8_0",
                "validated":false, "loadable":true, "available":false, "context_size":null, "context_limit":40960,
                "storage":"external", "availability_error":"model_file_changed",
                "compatibility":compatibility
            });
            let summary: ModelSummary = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(summary).unwrap(), value);
            let mut legacy = value;
            legacy.as_object_mut().unwrap().remove("compatibility");
            let summary: ModelSummary = serde_json::from_value(legacy.clone()).unwrap();
            assert_eq!(summary.compatibility, ModelCompatibility::Unknown);
            legacy["compatibility"] = json!("future_status");
            let summary: ModelSummary = serde_json::from_value(legacy).unwrap();
            assert_eq!(summary.compatibility, ModelCompatibility::Unknown);
            assert_eq!(
                summary.availability_error.as_deref(),
                Some("model_file_changed")
            );
        }
    }
}
