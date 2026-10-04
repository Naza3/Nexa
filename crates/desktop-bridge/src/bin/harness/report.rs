//! Closed, bounded acceptance diagnostic protocol. Never serialize an error's
//! Display/Debug, request content, credentials, file paths, or subprocess text.
use desktop_bridge::{BridgeError, StartupDiagnostics};
use serde::{Deserialize, Serialize};

pub const MAX_REPORT_BYTES: usize = 4096;
pub const FAILURE_KEYS: &[&str] = &[
    "schema_version",
    "kind",
    "success",
    "stage",
    "code",
    "bridge_code",
    "os_error",
    "runtime_exit_code",
    "child_stage",
    "child_exit_code",
    "cleanup",
];
pub const CLEANUP_KEYS: &[&str] = &[
    "status",
    "code",
    "bridge_code",
    "os_error",
    "instance_lock",
    "discovery",
    "temporary_data_retained",
];

pub const STAGES: &[&str] = &[
    "arguments",
    "create_private_directory",
    "initialize_token",
    "write_config",
    "construct_bridge",
    "launch_initial_child",
    "start_after_child_exit",
    "read_initial_instance",
    "attach_existing",
    "close_attached_window",
    "import_model",
    "list_models",
    "reject_running_idle_change",
    "load_model",
    "verify_local_validation",
    "verify_offline_inventory",
    "first_chat_start",
    "first_chat_consume",
    "cancel_chat_start",
    "cancel_chat_consume",
    "wait_ready",
    "repeat_chat_start",
    "repeat_chat_consume",
    "close_window",
    "verify_default_close",
    "launch_keep_child",
    "verify_process_exit",
    "unload_model",
    "reload_model",
    "launch_stop_child",
    "save_close_preference",
    "repeated_close",
    "verify_instance_released",
    "save_stopped_idle",
    "restart_runtime",
    "stop_runtime",
    "external_apply",
    "external_rescan",
    "external_start",
    "external_list",
    "external_direct_chat",
    "external_cancel_prepare",
    "external_unload",
    "external_stop",
    "external_preexisting_writer",
    "external_source_changed",
    "external_final_stop",
    "cleanup_stop",
    "remove_temporary_data",
    "child_arguments",
    "child_construct_bridge",
    "child_start",
    "child_save_preferences",
    "child_close",
];
pub const CODES: &[&str] = &[
    "bridge_error",
    "io_error",
    "invalid_arguments",
    "configuration_error",
    "assertion_failed",
    "timeout",
    "child_spawn_failed",
    "child_failed",
    "child_report_invalid",
    "unexpected_failure",
    "cleanup_unconfirmed",
];
pub const BRIDGE_CODES: &[&str] = &[
    "configuration_invalid",
    "configuration_unavailable",
    "connection_failed",
    "consumer_busy",
    "credentials_unavailable",
    "data_directory_unavailable",
    "desktop_busy",
    "desktop_closing",
    "execution_timeout",
    "history_limit",
    "import_cleanup_unconfirmed",
    "import_interrupted",
    "instance_unavailable",
    "invalid_model_source",
    "invalid_request",
    "not_initialized",
    "packaged_runtime_missing",
    "request_cancelled",
    "request_cleanup_unconfirmed",
    "request_not_owned",
    "response_invalid",
    "response_limit",
    "runtime_running",
    "runtime_start_failed",
    "runtime_stop_unconfirmed",
    "settings_durability_unconfirmed",
    "settings_invalid",
    "settings_write_failed",
    "slow_consumer",
    "stream_invalid",
    "unsafe_file",
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
    "consumer_stopped",
    "queue_full",
    "queue_timeout",
    "load_timeout",
    "runtime_faulted",
    "worker_lost",
    "runtime_shutdown",
    "model_load_failed",
    "insufficient_storage",
    "internal_error",
    "response_too_large",
    "import_committed_durability_unconfirmed",
    "api_error",
    "invalid_manifest",
    "invalid_argument",
    "model_load_interrupted",
    "model_library_unsupported",
    "model_directory_required",
    "model_directory_unavailable",
    "model_directory_unsupported",
    "model_library_limit",
    "model_library_changed",
    "model_list_changed",
    "model_scan_timeout",
    "model_scan_cancelled",
    "model_file_changed",
    "model_file_unavailable",
    "model_file_in_use",
    "model_library_write_failed",
    "unrecognized",
];
pub const CLEANUP_STATUSES: &[&str] = &["not_needed", "confirmed", "unconfirmed"];
pub const LOCK_STATES: &[&str] = &["free", "held", "unavailable", "not_checked"];
pub const DISCOVERY_STATES: &[&str] = &["absent", "present", "unavailable", "not_checked"];

#[derive(Clone, Debug)]
pub struct ReapObservation {
    pub confirmed: bool,
    pub kill_failed: bool,
    pub os_error: Option<i32>,
}
#[derive(Clone, Debug)]
pub struct Fault {
    // Internal only. Never part of the external failure protocol.
    pub child_reap: Option<ReapObservation>,
    pub code: &'static str,
    pub bridge_code: Option<String>,
    pub os_error: Option<i32>,
    pub runtime_exit_code: Option<i32>,
    pub child_stage: Option<String>,
    pub child_exit_code: Option<i32>,
}
impl Fault {
    pub fn new(code: &'static str) -> Self {
        debug_assert!(CODES.contains(&code));
        Self {
            child_reap: None,
            code,
            bridge_code: None,
            os_error: None,
            runtime_exit_code: None,
            child_stage: None,
            child_exit_code: None,
        }
    }
    pub fn bridge(error: BridgeError) -> Self {
        let code = if BRIDGE_CODES.contains(&error.code.as_str()) {
            error.code
        } else {
            "unrecognized".into()
        };
        Self {
            bridge_code: Some(code),
            ..Self::new("bridge_error")
        }
    }
    pub fn startup(error: BridgeError, diagnostic: StartupDiagnostics) -> Self {
        // A rejected concurrent/closing call did not own a start attempt. Do
        // not associate the preceding attempt's observations with that error.
        if error.code != "runtime_start_failed" {
            return Self::bridge(error);
        }
        Self {
            os_error: diagnostic.os_error,
            runtime_exit_code: diagnostic.process_exit_code,
            ..Self::bridge(error)
        }
    }
    pub fn io(error: std::io::Error) -> Self {
        Self {
            os_error: error.raw_os_error(),
            ..Self::new("io_error")
        }
    }
}
impl From<BridgeError> for Fault {
    fn from(error: BridgeError) -> Self {
        Self::bridge(error)
    }
}
impl From<std::io::Error> for Fault {
    fn from(error: std::io::Error) -> Self {
        Self::io(error)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cleanup {
    pub status: String,
    pub code: Option<String>,
    pub bridge_code: Option<String>,
    pub os_error: Option<i32>,
    pub instance_lock: String,
    pub discovery: String,
    pub temporary_data_retained: bool,
}
impl Cleanup {
    pub fn not_needed(retained: bool) -> Self {
        Self {
            status: "not_needed".into(),
            code: None,
            bridge_code: None,
            os_error: None,
            instance_lock: "not_checked".into(),
            discovery: "not_checked".into(),
            temporary_data_retained: retained,
        }
    }
    fn validate(&self) -> bool {
        CLEANUP_STATUSES.contains(&self.status.as_str())
            && self.code.as_deref().is_none_or(|s| CODES.contains(&s))
            && self
                .bridge_code
                .as_deref()
                .is_none_or(|s| BRIDGE_CODES.contains(&s))
            && LOCK_STATES.contains(&self.instance_lock.as_str())
            && DISCOVERY_STATES.contains(&self.discovery.as_str())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FailureReport {
    pub schema_version: u32,
    pub kind: String,
    pub success: bool,
    pub stage: String,
    pub code: String,
    pub bridge_code: Option<String>,
    pub os_error: Option<i32>,
    pub runtime_exit_code: Option<i32>,
    pub child_stage: Option<String>,
    pub child_exit_code: Option<i32>,
    pub cleanup: Cleanup,
}
impl FailureReport {
    pub fn new(stage: &str, fault: Fault, cleanup: Cleanup) -> Self {
        Self {
            schema_version: 1,
            kind: "nexa-desktop-bridge-acceptance".into(),
            success: false,
            stage: stage.into(),
            code: fault.code.into(),
            bridge_code: fault.bridge_code,
            os_error: fault.os_error,
            runtime_exit_code: fault.runtime_exit_code,
            child_stage: fault.child_stage,
            child_exit_code: fault.child_exit_code,
            cleanup,
        }
    }
    pub fn validate(&self) -> bool {
        self.schema_version == 1
            && self.kind == "nexa-desktop-bridge-acceptance"
            && !self.success
            && STAGES.contains(&self.stage.as_str())
            && CODES.contains(&self.code.as_str())
            && self
                .bridge_code
                .as_deref()
                .is_none_or(|s| BRIDGE_CODES.contains(&s))
            && self
                .child_stage
                .as_deref()
                .is_none_or(|s| STAGES.contains(&s))
            && self.cleanup.validate()
    }
    pub fn parse(bytes: &[u8]) -> Result<Self, Fault> {
        if bytes.len() > MAX_REPORT_BYTES {
            return Err(Fault::new("child_report_invalid"));
        }
        // Runtime's parser rejects duplicate fields recursively before typed
        // deserialization rejects unknown fields. Neither parser prints input.
        let value =
            runtime_api::dto::parse_json(bytes).map_err(|_| Fault::new("child_report_invalid"))?;
        let keys_match = |value: &serde_json::Value, keys: &[&str]| {
            value.as_object().is_some_and(|object| {
                object.len() == keys.len() && object.keys().all(|key| keys.contains(&key.as_str()))
            })
        };
        if !keys_match(&value, FAILURE_KEYS)
            || !value
                .get("cleanup")
                .is_some_and(|cleanup| keys_match(cleanup, CLEANUP_KEYS))
        {
            return Err(Fault::new("child_report_invalid"));
        }
        let report: Self =
            serde_json::from_value(value).map_err(|_| Fault::new("child_report_invalid"))?;
        if !report.validate() {
            return Err(Fault::new("child_report_invalid"));
        }
        Ok(report)
    }
    pub fn into_child_fault(self, exit: Option<i32>) -> Fault {
        let code = CODES
            .iter()
            .copied()
            .find(|code| *code == self.code)
            .unwrap_or("child_report_invalid");
        Fault {
            child_reap: None,
            code,
            bridge_code: self.bridge_code,
            os_error: self.os_error,
            runtime_exit_code: self.runtime_exit_code,
            child_stage: Some(self.stage),
            child_exit_code: exit,
        }
    }
}
pub const CHILD_SUCCESS: &str =
    "{\"schema_version\":1,\"kind\":\"nexa-desktop-bridge-child\",\"success\":true}";

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn report_never_copies_message_and_preserves_numeric_zero() {
        let fault = Fault::startup(
            BridgeError {
                code: "runtime_start_failed".into(),
                message: "SECRET /full/private/path prompt".into(),
            },
            StartupDiagnostics {
                os_error: Some(5),
                process_exit_code: Some(0),
            },
        );
        let report = FailureReport::new("child_start", fault, Cleanup::not_needed(true));
        let bytes = serde_json::to_vec(&report).unwrap();
        assert!(bytes.len() < MAX_REPORT_BYTES);
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(!text.contains("SECRET"));
        assert!(!text.contains("private"));
        let decoded = FailureReport::parse(&bytes).unwrap();
        assert_eq!(decoded.os_error, Some(5));
        assert_eq!(decoded.runtime_exit_code, Some(0));
    }
    #[test]
    fn unknown_fields_codes_duplicate_keys_and_oversized_reports_fail_closed() {
        let report = FailureReport::new(
            "arguments",
            Fault::new("invalid_arguments"),
            Cleanup::not_needed(false),
        );
        let original = serde_json::to_value(&report).unwrap();
        for (key, value) in [
            ("message", serde_json::json!("SECRET")),
            ("stage", serde_json::json!("/private/path")),
            ("bridge_code", serde_json::json!("secret")),
            ("os_error", serde_json::json!(2147483648u64)),
        ] {
            let mut v = original.clone();
            v[key] = value;
            assert!(FailureReport::parse(&serde_json::to_vec(&v).unwrap()).is_err());
        }
        let mut missing = original.clone();
        missing.as_object_mut().unwrap().remove("os_error");
        assert!(FailureReport::parse(&serde_json::to_vec(&missing).unwrap()).is_err());
        assert!(FailureReport::parse(&vec![b' '; MAX_REPORT_BYTES + 1]).is_err());
        assert!(FailureReport::parse(br#"{"success":false,"success":true}"#).is_err());
    }
    #[test]
    fn cleanup_does_not_overwrite_original_failure() {
        let mut cleanup = Cleanup::not_needed(true);
        cleanup.status = "unconfirmed".into();
        cleanup.code = Some("bridge_error".into());
        cleanup.bridge_code = Some("connection_failed".into());
        let report = FailureReport::new(
            "child_start",
            Fault::startup(
                BridgeError {
                    code: "runtime_start_failed".into(),
                    message: String::new(),
                },
                StartupDiagnostics {
                    os_error: Some(5),
                    process_exit_code: None,
                },
            ),
            cleanup,
        );
        assert_eq!(report.stage, "child_start");
        assert_eq!(report.bridge_code.as_deref(), Some("runtime_start_failed"));
        assert_eq!(report.os_error, Some(5));
        assert_eq!(
            report.cleanup.bridge_code.as_deref(),
            Some("connection_failed")
        );
    }
}

pub const PROBE_KEYS: &[&str] = &[
    "schema_version",
    "kind",
    "strategy",
    "success",
    "code",
    "os_error",
    "spawn_os_error",
    "child_exit_code",
    "signal_state",
    "signal_os_error",
    "parent_in_job",
    "parent_job_os_error",
    "child_in_job",
    "child_job_os_error",
    "cleanup_confirmed",
];
pub const LAUNCH_STRATEGIES: &[&str] = &["breakaway", "inherit_job"];
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchStrategy {
    Breakaway,
    InheritJob,
}
impl LaunchStrategy {
    pub fn parse(value: &std::ffi::OsStr) -> Option<Self> {
        match value.to_str()? {
            "breakaway" => Some(Self::Breakaway),
            "inherit_job" => Some(Self::InheritJob),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Breakaway => "breakaway",
            Self::InheritJob => "inherit_job",
        }
    }
    #[cfg(windows)]
    pub fn creation_flags(self) -> u32 {
        use windows_sys::Win32::System::Threading::{
            CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS,
        };
        let flags = CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS;
        match self {
            Self::Breakaway => flags | CREATE_BREAKAWAY_FROM_JOB,
            Self::InheritJob => flags,
        }
    }
}
pub const PROBE_CODES: &[&str] = &[
    "observed_pending",
    "observed_signal",
    "signal_registration_failed",
    "spawn_failed",
    "child_failed",
    "timeout",
    "probe_io_failed",
    "invalid_report",
    "unsupported_platform",
];
pub const SIGNAL_STATES: &[&str] = &["pending", "received", "error", "not_observed"];
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchProbeReport {
    pub schema_version: u32,
    pub kind: String,
    pub strategy: String,
    pub success: bool,
    pub code: String,
    pub os_error: Option<i32>,
    pub spawn_os_error: Option<i32>,
    pub child_exit_code: Option<i32>,
    pub signal_state: String,
    pub signal_os_error: Option<i32>,
    pub parent_in_job: Option<bool>,
    pub parent_job_os_error: Option<i32>,
    pub child_in_job: Option<bool>,
    pub child_job_os_error: Option<i32>,
    pub cleanup_confirmed: bool,
}
impl LaunchProbeReport {
    pub fn empty(code: &str, strategy: LaunchStrategy) -> Self {
        Self {
            schema_version: 2,
            kind: "nexa-desktop-launch-probe".into(),
            strategy: strategy.as_str().into(),
            success: false,
            code: code.into(),
            os_error: None,
            spawn_os_error: None,
            child_exit_code: None,
            signal_state: "not_observed".into(),
            signal_os_error: None,
            parent_in_job: None,
            parent_job_os_error: None,
            child_in_job: None,
            child_job_os_error: None,
            cleanup_confirmed: false,
        }
    }
    pub fn pending_confirmed(&self) -> bool {
        self.code == "observed_pending"
            && self.signal_state == "pending"
            && self.signal_os_error.is_none()
            && self.spawn_os_error.is_none()
            && self.os_error.is_none()
            && self.child_exit_code == Some(0)
            && self.parent_in_job.is_some()
            && self.parent_job_os_error.is_none()
            && self.child_in_job.is_some()
            && self.child_job_os_error.is_none()
            && self.cleanup_confirmed
    }
    pub fn validate(&self) -> bool {
        self.schema_version == 2
            && self.kind == "nexa-desktop-launch-probe"
            && LAUNCH_STRATEGIES.contains(&self.strategy.as_str())
            && PROBE_CODES.contains(&self.code.as_str())
            && SIGNAL_STATES.contains(&self.signal_state.as_str())
            && !(self.parent_in_job.is_some() && self.parent_job_os_error.is_some())
            && !(self.child_in_job.is_some() && self.child_job_os_error.is_some())
            && (!self.success || self.pending_confirmed())
    }
    pub fn encode(&self) -> Option<Vec<u8>> {
        if !self.validate() {
            return None;
        }
        let value = serde_json::to_value(self).ok()?;
        let object = value.as_object()?;
        if object.len() != PROBE_KEYS.len()
            || !object.keys().all(|key| PROBE_KEYS.contains(&key.as_str()))
        {
            return None;
        }
        let bytes = serde_json::to_vec(&value).ok()?;
        (bytes.len() <= MAX_REPORT_BYTES).then_some(bytes)
    }
}
