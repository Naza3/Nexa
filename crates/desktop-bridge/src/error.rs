use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BridgeError {
    pub code: String,
    pub message: String,
}
impl BridgeError {
    pub(crate) fn new(code: &str) -> Self {
        let message = match code {
            "desktop_busy" => "This window already has an operation in progress.",
            "desktop_closing" => "This window is closing.",
            "runtime_running" => "Stop the runtime before applying this runtime setting.",
            "not_initialized" => "Initialize and start the runtime explicitly first.",
            "runtime_not_running" => "Start the runtime first.",
            "connection_failed" => {
                "The existing instance could not be securely verified. It was not replaced."
            }
            "runtime_start_failed" => {
                "The packaged runtime did not become available. No business request was replayed."
            }
            "runtime_stop_unconfirmed" => {
                "Runtime cleanup could not be confirmed. The window remains open."
            }
            "context_length_exceeded" => {
                "The context or token budget exceeds the loaded model. Clear or reduce the conversation."
            }
            "history_limit" => {
                "The conversation limit was reached. Explicitly clear or reduce it before sending again."
            }
            "response_limit" => {
                "The reply exceeded the output limit. The partial reply is incomplete."
            }
            "slow_consumer" => {
                "The window stopped consuming output. The partial reply is incomplete."
            }
            "stream_invalid" => {
                "The stream was malformed or ended before a verified completion. The partial reply is incomplete."
            }
            "consumer_busy" => "Only one pending output read is allowed.",
            "request_not_owned" => "This request does not belong to this window.",
            "import_interrupted" => {
                "The import connection was closed. Refresh the model list before retrying; a completed copy may already exist."
            }
            "import_cleanup_unconfirmed" => {
                "The registry is still busy after closing this window's import. Keep the window open and retry closing."
            }
            "import_committed_durability_unconfirmed" => {
                "The import may already be committed. Refresh the model list before any retry."
            }
            "settings_durability_unconfirmed" => {
                "The setting was published but disk durability was not confirmed. Refresh before retrying."
            }
            "settings_invalid" | "invalid_request" => {
                "The supplied settings or request are invalid."
            }
            "packaged_runtime_missing" => {
                "The fixed packaged runtime or worker is missing or invalid."
            }
            "settings_write_failed" => "The settings could not be saved.",
            _ => "The operation could not be completed safely. Refresh the status before retrying.",
        };
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
    pub(crate) fn spawn(error: &std::io::Error) -> Self {
        let mut safe = Self::new("runtime_start_failed");
        if let Some(code) = error.raw_os_error() {
            // A numeric OS code distinguishes denied Job breakaway from a
            // missing binary without exposing an executable or user path.
            safe.message.push_str(&format!(" (OS error {code})"));
        }
        safe
    }
    pub(crate) fn api(code: Option<&str>) -> Self {
        const KNOWN: &[&str] = &[
            "invalid_request",
            "unsupported_parameter",
            "unsupported_model",
            "unsupported_chat_template",
            "context_length_exceeded",
            "model_not_found",
            "request_not_found",
            "model_conflict",
            "runtime_busy",
            "already_exists",
            "duplicate_request_id",
            "request_cancelled",
            "consumer_stopped",
            "slow_consumer",
            "queue_full",
            "queue_timeout",
            "load_timeout",
            "execution_timeout",
            "runtime_faulted",
            "worker_lost",
            "runtime_shutdown",
            "model_load_failed",
            "insufficient_storage",
            "internal_error",
            "response_too_large",
            "import_committed_durability_unconfirmed",
        ];
        Self::new(code.filter(|s| KNOWN.contains(s)).unwrap_or("api_error"))
    }
}
impl std::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for BridgeError {}
impl From<runtime_cli::client::ClientError> for BridgeError {
    fn from(e: runtime_cli::client::ClientError) -> Self {
        match e {
            runtime_cli::client::ClientError::Api { code, .. } => Self::api(code.as_deref()),
            _ => Self::new("connection_failed"),
        }
    }
}
pub type Result<T> = std::result::Result<T, BridgeError>;
