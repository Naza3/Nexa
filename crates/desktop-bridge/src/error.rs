use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BridgeError {
    pub code: String,
    pub message: String,
}
impl BridgeError {
    pub(crate) fn new(code: &str) -> Self {
        let message = match code {
            "configuration_conflict" => "配置已被其他窗口或程序更改。请重读并比较草稿后再保存。",
            "configuration_migration_required" => {
                "请先在设置中比较并确认旧桌面/API默认值，升级配置后再使用模型档案。"
            }
            "configuration_restart_required" => {
                "磁盘配置与运行服务不同。请停服并重启后再主动加载或保存。"
            }
            "configuration_revision_required" => {
                "此旧写入接口不支持配置版本检查。请使用新版设置页面。"
            }
            "configuration_busy" => "另一个配置事务正在进行，请稍后重试。",
            "configuration_invalid" | "model_profile_invalid" => {
                "配置或模型档案超出有效范围，未保存。"
            }
            "configuration_durability_unconfirmed" => {
                "配置可能已保存，但磁盘持久化未确认。请先重新读取，不要直接重试。"
            }

            "validation_record_unavailable" | "validation_record_write_failed" => {
                "本机测试记录无法保存，未确认本次验证通过。已加载模型仍可继续使用，请检查数据目录后重试测试。"
            }
            "validation_record_read_failed" | "validation_record_invalid" => {
                "本机测试记录无法读取，不能判定为未测试或已通过。原记录已保留，请检查数据目录和记录文件。"
            }
            "validation_engine_unavailable" => {
                "无法读取当前运行时及推理进程的验证身份，未确认本次验证通过。请检查完整安装包后重试。"
            }
            "validation_scope_unavailable" => {
                "无法读取模型或数据目录的本机验证条件，未确认本次验证通过。请检查模型和数据目录后重试。"
            }
            "validation_scope_changed" => {
                "模型、运行时或验证条件在测试期间发生变化，本次结果不再有效。请重新加载并测试。"
            }
            "model_download_engine_unavailable" => {
                "已验证的下载组件尚未就绪。请使用包含受控下载组件的完整安装包，或手动下载后扫描模型目录。"
            }
            "model_download_active" => {
                "A download is active. Cancel it or wait before changing models, directories or settings."
            }
            "model_download_cleanup_unconfirmed" => {
                "Download cleanup has not completed. Keep the window open and retry closing."
            }
            "model_download_cancelled" => "The download was cancelled before publication.",
            "model_download_timeout" => {
                "The download deadline was reached. Retry explicitly when the source is available."
            }
            "model_download_identity_mismatch" | "model_download_size_mismatch" => {
                "The downloaded bytes do not match the pinned size and SHA256. No model was published."
            }
            "model_download_network_failed"
            | "model_download_http_failed"
            | "model_download_redirect_rejected" => {
                "当前下载源连接、响应或重定向失败。未切换下载源，未发布模型文件。"
            }
            "already_exists" => {
                "A file with this name already exists. It was not changed or overwritten."
            }
            "desktop_busy" => "This window already has an operation in progress.",
            "desktop_closing" => "This window is closing.",
            "lan_settings_invalid" => {
                "LAN 设置需具体私有 IPv4 地址和 1–65535 端口；启用时需 1–16 条 /24–/32 规范私网 CIDR，不能重复、重叠或带主机位。"
            }
            "lan_token_unavailable" => {
                "请先显式启用 LAN 并成功启动服务，再复制独立 LAN 密钥。现有密钥仍需通过安全文件校验。"
            }
            "lan_address_discovery_failed" => "无法读取本机 IPv4 网卡地址，可刷新重试或手动填写。",
            "lan_address_discovery_busy" => "上次本机地址读取尚未结束，请稍后刷新或手动填写。",
            "lan_address_discovery_timeout" => "读取本机网卡地址超时，可稍后刷新或手动填写。",
            "lan_address_discovery_invalid" => {
                "系统返回的网卡信息无法安全读取，可刷新重试或手动填写。"
            }
            "lan_address_discovery_limit" => "本机网卡信息超过读取上限，请手动填写 IPv4 地址。",
            "runtime_running" => "Stop the runtime before applying this runtime setting.",
            "not_initialized" => "请先明确初始化本机配置。初始化不会启动服务。",
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
            "model_load_interrupted" => {
                "This window's model request was disconnected. Preparation is cancelling; native loading, if already admitted, must finish cleanup."
            }
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
            "unsupported_model" => {
                "This model cannot use the current engine/text adapter. Only protected single-file GGUF models are supported; split files are rejected."
            }
            "unsupported_chat_template" => {
                "The embedded template is missing or cannot preserve this plain-text conversation. No replacement template is used."
            }
            "invalid_manifest" => "The selected GGUF structure or model registration is invalid.",
            "model_scan_no_usable_files" => {
                "Every GGUF candidate was rejected. The previous directory and index were preserved."
            }
            "settings_invalid" | "invalid_request" | "invalid_argument" => {
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
    fn with_configuration_param(mut self, param: Option<&str>) -> Self {
        let label = match param {
            Some("expected_revision") => Some("保存版本"),
            Some("expected_preferences_revision") => Some("旧桌面偏好版本"),
            Some("update") => Some("配置组"),
            Some("update.load_overrides") => Some("模型运行档案"),
            Some("update.load_overrides.context_size") => Some("模型上下文上限"),
            Some("update.global_defaults") => Some("全局加载默认值"),
            Some("update.global_defaults.context_size") => Some("全局默认上下文与模型上限"),
            Some("update.request_defaults") => Some("请求默认值"),
            Some("update.runtime") => Some("运行策略"),
            Some("update.runtime.idle_unload_seconds") => Some("空闲释放时间"),
            Some("update.local_api") => Some("本机监听"),
            Some("update.lan_api") => Some("局域网设置"),
            _ => None,
        };
        if let Some(label) = label {
            self.message.push_str(&format!("（字段：{label}）"));
        }
        self
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
            "not_found",
            "configuration_conflict",
            "configuration_migration_required",
            "configuration_restart_required",
            "configuration_revision_required",
            "configuration_busy",
            "configuration_invalid",
            "model_profile_invalid",
            "configuration_unavailable",
            "configuration_write_failed",
            "configuration_durability_unconfirmed",
            "runtime_running",
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
            runtime_cli::client::ClientError::Api { code, param, .. } => {
                Self::api(code.as_deref()).with_configuration_param(param.as_deref())
            }
            _ => Self::new("connection_failed"),
        }
    }
}
impl From<runtime_api::configuration::ConfigurationError> for BridgeError {
    fn from(e: runtime_api::configuration::ConfigurationError) -> Self {
        Self::new(e.code).with_configuration_param(e.param)
    }
}
pub type Result<T> = std::result::Result<T, BridgeError>;

/// Only typed sidecar outcomes cross this boundary; never echo console output,
/// URLs, response bodies, signed queries, executable paths or OS error strings.
pub(crate) fn download_error(error: download_engine::DownloadError) -> BridgeError {
    use download_engine::DownloadError as E;
    match error {
        E::InvalidSpec => BridgeError::new("model_catalog_invalid"),
        E::InvalidOptions | E::UnsupportedPlatform | E::SpawnFailed => {
            BridgeError::new("model_download_engine_unavailable")
        }
        E::Cancelled => BridgeError::new("model_download_cancelled"),
        E::Timeout => BridgeError::new("model_download_timeout"),
        E::CleanupUnconfirmed => BridgeError::new("model_download_cleanup_unconfirmed"),
        E::PipeFailed => BridgeError {
            code: "model_download_network_failed".into(),
            message: "下载进程输出读取失败。未切换下载源，未发布模型文件。".into(),
        },
        E::SidecarExit { exit_code, .. } => {
            let (code, reason) = match exit_code {
                2 => ("model_download_timeout", "下载源等待超时"),
                3 | 4 => ("model_download_http_failed", "下载源未找到指定文件"),
                6 => ("model_download_network_failed", "下载源网络连接或传输失败"),
                8 => (
                    "model_download_http_failed",
                    "下载源不支持本次续传，有限全量恢复后仍未完成",
                ),
                9 => ("model_download_write_failed", "模型目录可用空间不足"),
                13 => ("already_exists", "下载目标已存在，未覆盖"),
                14..=18 => ("model_download_write_failed", "本任务临时文件操作失败"),
                19 => ("model_download_network_failed", "下载源域名解析失败"),
                22 => ("model_download_http_failed", "下载源 HTTP 响应不符合要求"),
                23 => (
                    "model_download_redirect_rejected",
                    "下载源重定向次数超过限制",
                ),
                24 => (
                    "model_download_http_failed",
                    "下载源要求认证，本下载器不使用账户凭据",
                ),
                29 => ("model_download_http_failed", "下载源服务暂不可用"),
                32 => (
                    "model_download_identity_mismatch",
                    "下载字节不符合固定 SHA256",
                ),
                _ => ("model_download_network_failed", "下载进程未成功完成"),
            };
            BridgeError {
                code: code.into(),
                message: format!(
                    "{reason}（下载进程退出码 {exit_code}）。未切换下载源，未发布模型文件。"
                ),
            }
        }
    }
}
#[cfg(test)]
mod download_diagnostic_tests {
    use super::*;
    #[test]
    fn sidecar_diagnostics_use_only_known_numeric_facts() {
        for exit_code in [1, 2, 3, 6, 8, 9, 13, 17, 19, 22, 23, 24, 29, 32, u32::MAX] {
            let error = download_error(download_engine::DownloadError::SidecarExit {
                exit_code,
                error_code: Some(999),
            });
            assert!(error.message.contains(&format!("退出码 {exit_code}")));
            assert!(!error.message.contains("999"));
            assert!(!error.message.contains("https://"));
            assert!(error.message.len() < 400);
        }
    }
}
