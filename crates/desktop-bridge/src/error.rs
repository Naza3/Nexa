use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BridgeError {
    pub code: String,
    pub message: String,
}
impl BridgeError {
    pub(crate) fn new(code: &str) -> Self {
        let message = match code {
            "persistence_busy" => "正在保存识别历史或窗口设置，请稍后重试关闭。",
            "ocr_history_io" => "识别历史无法读写。当前识别正文仍保留在窗口中。",
            "ocr_history_busy" => "识别历史正被另一个窗口修改，请稍后重试。",
            "ocr_history_corrupt" => "识别历史文件损坏或版本不受支持，已保留原文件。",
            "ocr_history_invalid" => "识别记录格式不正确，未保存。",
            "ocr_history_limit" => "识别历史超过存储大小限制，未改动已有记录。",
            "ocr_history_durability_unconfirmed" => {
                "识别记录可能已保存，但磁盘写入未确认，请刷新历史后核对。"
            }
            "ocr_history_not_found" => "这条识别记录已删除或被最近100条限制淘汰。",
            "ocr_history_conflict" => "同一识别记录的内容不一致，未覆盖已有记录。",
            "ocr_history_closing" => "窗口正在关闭，识别记录未保存。",
            "workbench_io" => "窗口偏好无法读写，请检查本机数据目录。",
            "workbench_busy" => "另一个窗口正在保存偏好，请稍后重试。",
            "workbench_corrupt" => "窗口偏好文件损坏或版本不受支持，已保留原文件。",
            "workbench_invalid" => "参数或提示词超出允许范围，窗口偏好未保存。",
            "workbench_limit" => "窗口偏好超过存储大小限制，未改动已有设置。",
            "workbench_conflict" => "窗口偏好已被其他窗口修改，请重新读取后比较。",
            "workbench_durability_unconfirmed" => {
                "窗口偏好可能已保存，但磁盘写入未确认，请重新读取核对。"
            }
            "workbench_closing" => "窗口正在关闭，偏好未保存。",
            "performance_unsupported" => "当前运行服务不支持性能记录，请使用新版运行时并重启服务。",
            "model_unregister_loaded" => "请先卸载此模型，再从模型库移除。",
            "model_unregister_durability_unconfirmed" => {
                "移除登记可能已完成，但磁盘持久化未确认。请刷新模型库后再操作。"
            }
            "model_list_changed" => "模型库已变化，请刷新并重新确认要移除的模型。",
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
            "configuration_unavailable" => {
                "无法安全读取本机配置。请检查数据目录、配置文件及访问权限，原配置未被替换。"
            }
            "configuration_write_failed" => {
                "配置保存失败。请检查数据目录的可用空间和写入权限，再重新读取核对。"
            }
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
            "model_download_outcome_unknown" => {
                "下载操作结果暂时无法确认，文件可能已经发布，不会自动重试。请先刷新模型目录和模型库并核对，再决定是否手动重试。"
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
            "runtime_loopback_bind_failed" => {
                "本机 API 监听失败。请检查设置中的回环地址和端口是否可用或被占用，修正后手动启动服务。"
            }
            "runtime_security_invalid" => {
                "服务安全配置无效。请检查本机及局域网安全设置，并使用同一完整安装包重新启动。"
            }
            "model_not_loaded" => "当前没有可用于推理的模型。请先加载模型，再手动发送请求。",
            "model_not_found" => "找不到已登记模型。请刷新模型库并重新选择，不会自动换用其他模型。",
            "model_conflict" => {
                "请求模型与当前已加载模型不一致。请等待当前任务结束，再明确切换模型。"
            }
            "runtime_busy" | "queue_full" => {
                "服务正在处理任务或等待队列已满。请等待已有任务结束，再手动重试。"
            }
            "queue_timeout" => "请求等待队列超时，尚未开始推理。请等待服务空闲后手动重试。",
            "load_timeout" => "模型加载超时。请刷新模型状态，确认清理完成后再手动加载。",
            "worker_lost" | "runtime_faulted" => {
                "推理进程异常结束，当前任务未完成。请刷新服务状态，确认清理完成后显式重新加载模型；已输出内容不会自动重放。"
            }
            "runtime_shutdown" => {
                "服务已停止，当前任务未完成。请重新启动服务并加载模型后再手动操作。"
            }
            "model_load_failed" => {
                "模型加载失败。请检查模型是否受支持、文件是否完整及可用内存，刷新状态后再手动加载。"
            }
            "insufficient_storage" => {
                "模型存储空间不足，操作未完成。请释放空间并刷新模型库后再操作。"
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
            "response_invalid" => {
                "运行服务响应不完整或与当前桌面版本不兼容。请刷新状态；仍失败时，请显式停止服务并使用同一完整安装包重新启动。"
            }
            "execution_timeout" => {
                "推理执行超时，已生成内容可能不完整。可减少输入，或停止服务后在设置中调大推理执行超时，再启动服务并手动重试。"
            }
            "slow_consumer" => {
                "The window stopped consuming output. The partial reply is incomplete."
            }
            "stream_invalid" => {
                "The stream was malformed or ended before a verified completion. The partial reply is incomplete."
            }
            "consumer_busy" => "Only one pending output read is allowed.",
            "executor_cleanup_unconfirmed" => {
                "推理进程或资源清理未确认。请检查服务状态；不能继续加载，也不能把它视为已停止。"
            }
            "model_load_result_unavailable" => {
                "本次操作已经结束，但结果已过期。请刷新模型状态后重试。"
            }
            "request_cancelled" => "本次加载或基础测试已停止。",
            "request_not_owned" => "This request does not belong to this window.",
            "model_load_interrupted" => "本次加载结果暂时无法确认，正在重新读取；请勿重复加载。",
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
            "model_not_loaded",
            "request_not_found",
            "model_conflict",
            "model_unregister_loaded",
            "model_unregister_durability_unconfirmed",
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
            "executor_cleanup_unconfirmed",
            "native_failure",
            "native_protocol_error",
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
            runtime_cli::client::ClientError::Api {
                code,
                param,
                reason,
                ..
            } => {
                let mut error = Self::api(code.as_deref());
                let configuration_code = match error.code.as_str() {
                    "configuration_unavailable" => Some("configuration_unavailable"),
                    "configuration_invalid" => Some("configuration_invalid"),
                    "configuration_busy" => Some("configuration_busy"),
                    _ => None,
                };
                if let (Some(code), Some(reason)) = (configuration_code, reason) {
                    let source = runtime_api::configuration::ConfigurationError {
                        code,
                        param: None,
                        reason: Some(reason),
                    };
                    if runtime_api::configuration::ConfigurationFailureReason::from_reason_code(
                        code,
                        source.reason_code().unwrap(),
                    )
                    .is_some()
                    {
                        error.message = source.safe_message().into();
                    }
                }
                error.with_configuration_param(param.as_deref())
            }
            _ => Self::new("connection_failed"),
        }
    }
}
impl From<runtime_api::configuration::ConfigurationError> for BridgeError {
    fn from(e: runtime_api::configuration::ConfigurationError) -> Self {
        let mut error = Self::new(e.code);
        if e.reason_code().is_some() {
            error.message = e.safe_message().into();
        }
        error.with_configuration_param(e.param)
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
    #[test]
    fn model_not_loaded_survives_api_allowlist_and_unknown_codes_are_redacted() {
        let error = BridgeError::api(Some("model_not_loaded"));
        assert_eq!(error.code, "model_not_loaded");
        assert!(error.message.contains("加载模型"));
        let error = BridgeError::api(Some("private-token-or-path"));
        assert_eq!(error.code, "api_error");
        assert!(!error.message.contains("private"));
    }
    #[test]
    fn configuration_reason_survives_without_exposing_parser_or_path_text() {
        use runtime_api::configuration::{ConfigurationError, ConfigurationFailureReason};
        let source = ConfigurationError {
            code: "configuration_invalid",
            param: None,
            reason: Some(ConfigurationFailureReason::InvalidUtf8),
        };
        let expected = source.safe_message();
        let error = BridgeError::from(source);
        assert_eq!(error.code, "configuration_invalid");
        assert_eq!(error.message, expected);
    }
    #[test]
    fn online_configuration_reason_is_fixed_and_must_match_primary_code() {
        use runtime_api::configuration::{ConfigurationError, ConfigurationFailureReason};
        let error = BridgeError::from(runtime_cli::client::ClientError::Api {
            status: 400,
            code: Some("configuration_invalid".into()),
            param: None,
            reason: Some(ConfigurationFailureReason::InvalidUtf8),
        });
        let expected = ConfigurationError {
            code: "configuration_invalid",
            param: None,
            reason: Some(ConfigurationFailureReason::InvalidUtf8),
        };
        assert_eq!(error.message, expected.safe_message());
        for reason in [None, Some(ConfigurationFailureReason::Missing)] {
            let error = BridgeError::from(runtime_cli::client::ClientError::Api {
                status: 400,
                code: Some("configuration_invalid".into()),
                param: None,
                reason,
            });
            assert_eq!(error, BridgeError::new("configuration_invalid"));
        }
    }
}
