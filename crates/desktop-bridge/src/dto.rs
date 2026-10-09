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
    #[serde(default = "default_idle_enabled")]
    pub idle_unload_enabled: bool,
    #[serde(default = "default_verification_seconds")]
    pub model_verification_timeout_seconds: u64,
    pub close_runtime_on_exit: bool,
    #[serde(default)]
    pub download_source: DownloadSource,
}
fn default_idle_enabled() -> bool {
    true
}
fn default_verification_seconds() -> u64 {
    model_store::library::SCAN_TIMEOUT.as_secs()
}
impl Default for DesktopSettings {
    fn default() -> Self {
        Self {
            context_size: 2048,
            threads: 2,
            batch_size: 128,
            max_output_tokens: 512,
            idle_unload_seconds: 300,
            idle_unload_enabled: true,
            model_verification_timeout_seconds: default_verification_seconds(),
            close_runtime_on_exit: false,
            download_source: DownloadSource::default(),
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
    #[serde(default)]
    pub download_source: DownloadSource,
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
            download_source: s.download_source,
        }
    }
}
impl DesktopPreferences {
    pub fn with_idle(self, idle_unload_seconds: u64) -> DesktopSettings {
        let mut settings = self.with_runtime(&runtime_api::Config::default());
        settings.idle_unload_seconds = idle_unload_seconds;
        settings
    }
    pub fn with_runtime(self, config: &runtime_api::Config) -> DesktopSettings {
        DesktopSettings {
            context_size: self.context_size,
            threads: self.threads,
            batch_size: self.batch_size,
            max_output_tokens: self.max_output_tokens,
            close_runtime_on_exit: self.close_runtime_on_exit,
            download_source: self.download_source,
            idle_unload_seconds: config.runtime.idle_unload_seconds,
            idle_unload_enabled: config.runtime.idle_unload_enabled,
            model_verification_timeout_seconds: config.runtime.model_verification_timeout_seconds,
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
    #[serde(default)]
    pub configuration: Option<runtime_api::configuration::ConfigurationSnapshot>,
    #[serde(default)]
    pub configuration_error: Option<crate::BridgeError>,
    #[serde(default)]
    pub ui_preferences: Option<runtime_api::configuration::UiPreferencesSnapshot>,
    pub initialized: bool,
    pub connection: ConnectionState,
    pub api_address: Option<String>,
    pub runtime: Option<RuntimeStatus>,
    pub settings: DesktopSettings,
    #[serde(default)]
    pub lan_api: runtime_api::LanApiConfig,
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
    #[serde(default)]
    pub lan_api: Option<LanApiStatus>,
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
pub struct LanApiStatus {
    #[serde(default)]
    pub startup_error: Option<runtime_api::lan::LanStartupError>,
    pub enabled: bool,
    pub listen: Option<String>,
    pub running: bool,
}
/// An ephemeral local-interface observation, never a persisted LAN setting.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LanIpv4Address {
    pub interface_index: u32,
    pub interface_name: String,
    pub address: String,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LanAddressDiscoveryStatus {
    Available,
    Empty,
    Unsupported,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LanIpv4Addresses {
    pub status: LanAddressDiscoveryStatus,
    pub addresses: Vec<LanIpv4Address>,
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
    pub has_projector: bool,
    #[serde(default)]
    pub projector_size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_validation: Option<model_store::local_validation::LocalValidation>,
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
#[derive(Default, Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelsSource {
    Local,
    #[default]
    Runtime,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelsPage {
    #[serde(default)]
    pub source: ModelsSource,
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
pub struct OcrStartRequest {
    pub model_id: String,
    pub image_data_url: String,
    pub prompt: String,
    pub max_output_tokens: u32,
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
    /// Identity proved on the connection that submitted this request, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_instance_id: Option<Uuid>,
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
    Partial,
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
    Testing,
    Finished,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LibraryOperationResult {
    pub library_generation: Uuid,
    pub directory_id: Option<Uuid>,
    pub registered_files: usize,
    pub available_files: usize,
    #[serde(default)]
    pub rejected_files: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LibraryFileError {
    pub file_name: String,
    pub code: String,
    pub message: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AddedFileResult {
    #[serde(flatten)]
    pub registration: model_store::library::selected::SelectedResult,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_validation: Option<LocalValidation>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LibraryOperationState {
    #[serde(default)]
    pub load_phase: Option<String>,
    pub operation_id: Uuid,
    pub status: LibraryOperationStatus,
    pub phase: LibraryOperationPhase,
    pub examined_entries: usize,
    pub candidate_files: usize,
    pub verified_files: usize,
    pub failed_file_name: Option<String>,
    #[serde(default)]
    pub file_errors: Vec<LibraryFileError>,
    #[serde(default)]
    pub files: Vec<AddedFileResult>,
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
                "compatibility":compatibility, "has_projector":false, "projector_size_bytes":null
            });
            let summary: ModelSummary = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(summary).unwrap(), value);
            let mut legacy = value;
            legacy.as_object_mut().unwrap().remove("compatibility");
            legacy.as_object_mut().unwrap().remove("has_projector");
            legacy
                .as_object_mut()
                .unwrap()
                .remove("projector_size_bytes");
            let summary: ModelSummary = serde_json::from_value(legacy.clone()).unwrap();
            assert!(!summary.has_projector);
            assert_eq!(summary.projector_size_bytes, None);
            assert_eq!(summary.compatibility, ModelCompatibility::Unknown);
            legacy["compatibility"] = json!("future_status");
            let summary: ModelSummary = serde_json::from_value(legacy).unwrap();
            assert!(!summary.has_projector);
            assert_eq!(summary.projector_size_bytes, None);
            assert_eq!(summary.compatibility, ModelCompatibility::Unknown);
            assert_eq!(
                summary.availability_error.as_deref(),
                Some("model_file_changed")
            );
        }
    }
}

#[cfg(test)]
mod library_protocol_tests {
    use super::*;
    #[test]
    fn legacy_complete_defaults_and_worst_case_diagnostics_fit_full_response() {
        let old = serde_json::json!({
            "operation_id": Uuid::new_v4(), "status": "completed", "phase": "finished",
            "examined_entries": 0, "candidate_files": 0, "verified_files": 0,
            "failed_file_name": null, "terminal": true, "error": null,
            "result": {"library_generation": Uuid::new_v4(), "directory_id": Uuid::new_v4(), "registered_files": 0, "available_files": 0}
        });
        let mut state: LibraryOperationState = serde_json::from_value(old).unwrap();
        assert!(state.file_errors.is_empty());
        assert_eq!(state.result.as_ref().unwrap().rejected_files, 0);
        state.status = LibraryOperationStatus::Failed;
        state.result = None;
        state.examined_entries = 1025;
        state.candidate_files = 64;
        state.error = Some(crate::BridgeError::new("settings_durability_unconfirmed"));
        let longest = [
            "invalid_manifest",
            "unsupported_model",
            "unsupported_chat_template",
        ]
        .map(crate::BridgeError::new)
        .into_iter()
        .max_by_key(|e| e.message.len())
        .unwrap();
        state.file_errors = (0..64)
            .map(|index| LibraryFileError {
                file_name: format!("{}{:02}.gguf", "\u{1}".repeat(1017), index),
                code: longest.code.clone(),
                message: longest.message.clone(),
            })
            .collect();
        assert!(
            state
                .file_errors
                .iter()
                .all(|failure| failure.file_name.len() == 1024)
        );
        state.failed_file_name = Some(state.file_errors[0].file_name.clone());
        assert!(
            serde_json::to_vec(&state.file_errors).unwrap().len()
                <= model_store::library::MAX_SCAN_DIAGNOSTIC_BYTES
        );
        assert!(serde_json::to_vec(&state).unwrap().len() < 1024 * 1024);
    }
}

/// Curated download suggestions never grant model loading permission or validation.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DownloadSource {
    #[default]
    Modelscope,
    Huggingface,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelCatalog {
    pub entries: Vec<CatalogEntry>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogEntry {
    pub catalog_id: String,
    pub display_name: String,
    pub file_name: String,
    pub architecture: String,
    pub quantization: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub license: String,
    pub context_hint: u32,
    pub recommendation: String,
    pub sources: Vec<CatalogSource>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogSource {
    pub source: DownloadSource,
    pub repository: String,
    pub revision: String,
    pub url: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DownloadOperationHandle {
    pub operation_id: Uuid,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DownloadPhase {
    Connecting,
    Downloading,
    Verifying,
    Committing,
    Registering,
    Testing,
    Finished,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DownloadStatus {
    Running,
    Completed,
    Cancelled,
    Failed,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DownloadResult {
    #[serde(default)]
    pub registration_error: Option<crate::BridgeError>,
    #[serde(default)]
    pub local_validation: Option<model_store::local_validation::LocalValidation>,
    pub saved: bool,
    pub registered: bool,
    pub file_name: String,
    pub cleanup_warning: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DownloadOperationState {
    #[serde(default)]
    pub load_phase: Option<String>,
    pub operation_id: Uuid,
    pub catalog_id: String,
    pub source: DownloadSource,
    pub file_name: String,
    pub directory_id: Uuid,
    pub target_display_path: String,
    pub downloaded_bytes: u64,
    #[serde(default = "download_initial_attempt")]
    pub attempt: u8,
    pub total_bytes: u64,
    pub phase: DownloadPhase,
    pub status: DownloadStatus,
    pub terminal: bool,
    pub result: Option<DownloadResult>,
    pub error: Option<crate::BridgeError>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DownloadStopping {
    pub stopping: bool,
}

fn download_initial_attempt() -> u8 {
    1
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelsReconcile {
    pub status: String,
    pub operation_id: Option<Uuid>,
}
pub use model_store::local_validation::LocalValidation;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelLoadProfileRequest {
    pub model_id: String,
    #[serde(default)]
    pub load_overrides: TemporaryLoadOverrides,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TemporaryLoadOverrides {
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "nonnull_optional_u32"
    )]
    pub context_size: Option<u32>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "nonnull_optional_u32"
    )]
    pub threads: Option<u32>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "nonnull_optional_u32"
    )]
    pub batch_size: Option<u32>,
}
fn nonnull_optional_u32<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<u32>, D::Error> {
    u32::deserialize(d).map(Some)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelLoadOperationHandle {
    pub operation_id: Uuid,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelLoadOperationState {
    pub operation_id: Uuid,
    pub model_id: String,
    pub phase: String,
    pub status: String,
    pub terminal: bool,
    pub runtime: Option<RuntimeStatus>,
    pub local_validation: Option<LocalValidation>,
    pub error: Option<crate::BridgeError>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelLoadStopping {
    pub stopping: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelLoadStartRequest {
    pub operation_id: Uuid,
    #[serde(flatten)]
    pub load: LoadModelRequest,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelLoadProfileStartRequest {
    pub operation_id: Uuid,
    #[serde(flatten)]
    pub load: ModelLoadProfileRequest,
}

pub use runtime_api::dto::{UnregisterModelRequest, UnregisterModelResult};

#[cfg(test)]
mod lan_status_tests {
    use super::*;
    #[test]
    fn older_lan_status_defaults_to_no_startup_error() {
        let status: LanApiStatus =
            serde_json::from_str(r#"{"enabled":true,"listen":"192.168.1.2:18081","running":true}"#)
                .unwrap();
        assert!(status.startup_error.is_none());
    }
    #[test]
    fn future_startup_code_uses_bounded_fallback() {
        let status: LanApiStatus = serde_json::from_str(
            r#"{"enabled":true,"listen":null,"running":false,"startup_error":"future OS error"}"#,
        )
        .unwrap();
        assert_eq!(
            status.startup_error,
            Some(runtime_api::lan::LanStartupError::BindFailed)
        );
    }
    #[test]
    fn typed_lan_failure_roundtrips_without_affecting_saved_settings() {
        let status: LanApiStatus = serde_json::from_str(r#"{"enabled":true,"listen":"192.168.1.2:18081","running":false,"startup_error":"address_unavailable"}"#).unwrap();
        assert_eq!(
            status.startup_error,
            Some(runtime_api::lan::LanStartupError::AddressUnavailable)
        );
        assert!(status.enabled);
        assert!(!status.running);
    }
}
