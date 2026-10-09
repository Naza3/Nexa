import type { SafeError } from "./types";

// External messages are not trusted merely because they have a plausible code.
// Keep diagnostics bounded and stable; never include raw paths, URLs, credentials,
// prompts or generated text in error presentation or copied support details.
const messages: Readonly<Record<string, string>> = {
  model_load_failed: "模型加载失败。请先确认任务结束，检查模型文件、可用内存和加载参数，再手动重试；不能仅凭此错误认定为内存不足。",
  deadline_exceeded: "本次任务超过等待期限。请先重新确认原任务和服务状态，不要直接重复提交；已有输出可能不完整。",
  model_download_outcome_unknown: "下载结果未确认，文件可能已经保存。请先检查模型目录和列表，勿直接重复下载。",
  runtime_loopback_bind_failed: "本机 API 监听失败。请检查设置的回环地址和端口是否被占用，修正后手动启动。",
  runtime_security_invalid: "本机 API 安全配置无效。请检查配置并使用同一完整安装包，未启动服务；不要删除凭据或绕过安全检查。",
  "persistence_busy": "正在保存识别历史或窗口设置，请稍后重试关闭。",
  "ocr_history_io": "识别历史无法读写。当前识别正文仍保留在窗口中。",
  "ocr_history_busy": "识别历史正被另一个窗口修改，请稍后重试。",
  "ocr_history_corrupt": "识别历史文件损坏或版本不受支持，已保留原文件。",
  "ocr_history_invalid": "识别记录格式不正确，未保存。",
  "ocr_history_limit": "识别历史超过存储大小限制，未改动已有记录。",
  "ocr_history_durability_unconfirmed": "识别记录可能已保存，但磁盘写入未确认，请刷新历史后核对。",
  "ocr_history_not_found": "这条识别记录已删除或被最近100条限制淘汰。",
  "ocr_history_conflict": "同一识别记录的内容不一致，未覆盖已有记录。",
  "ocr_history_closing": "窗口正在关闭，识别记录未保存。",
  "workbench_io": "窗口偏好无法读写，请检查本机数据目录。",
  "workbench_busy": "另一个窗口正在保存偏好，请稍后重试。",
  "workbench_corrupt": "窗口偏好文件损坏或版本不受支持，已保留原文件。",
  "workbench_invalid": "参数或提示词超出允许范围，窗口偏好未保存。",
  "workbench_limit": "窗口偏好超过存储大小限制，未改动已有设置。",
  "workbench_conflict": "窗口偏好已被其他窗口修改，请重新读取后比较。",
  "workbench_durability_unconfirmed": "窗口偏好可能已保存，但磁盘写入未确认，请重新读取核对。",
  "workbench_closing": "窗口正在关闭，偏好未保存。",
  "performance_unsupported": "当前运行服务不支持性能记录，请使用新版运行时并重启服务。",
  "model_unregister_loaded": "请先卸载此模型，再从模型库移除。",
  "model_unregister_durability_unconfirmed": "操作可能已经提交，但最终结果或持久化尚未确认。请先刷新模型列表或已保存设置，核对后再操作，勿直接重复提交。",
  "model_list_changed": "模型库已变化或达到限制。请刷新模型列表，重新核对要操作的模型。",
  "configuration_conflict": "配置已被其他窗口修改。草稿保留，请重新读取并核对后再保存；未自动重试。",
  "configuration_migration_required": "请先在设置中确认旧配置迁移来源。",
  "configuration_restart_required": "已保存配置与当前运行实例不一致，请先显式停止并重新启动服务。",
  "configuration_revision_required": "此旧写入接口不支持配置版本检查。请使用新版设置页面。",
  "configuration_busy": "另一个配置事务正在进行，请稍后重试。",
  "configuration_invalid": "配置或模型档案超出有效范围，未保存。",
  "model_profile_invalid": "参数或请求超出支持范围。请检查字段提示和模型限制，修改后再提交。",
  "configuration_durability_unconfirmed": "配置可能已发布，但持久化尚未确认。请重新读取核对，不要直接重复保存。",
  "validation_record_unavailable": "本机测试记录无法保存，未确认本次验证通过。已加载模型仍可继续使用，请检查数据目录后重试测试。",
  "validation_record_write_failed": "本机测试记录无法保存，未确认本次验证通过。已加载模型仍可继续使用，请检查数据目录后重试测试。",
  "validation_record_read_failed": "本机测试记录无法读取，不能判定为未测试或已通过。原记录已保留，请检查数据目录和记录文件。",
  "validation_record_invalid": "本机测试记录无法读取，不能判定为未测试或已通过。原记录已保留，请检查数据目录和记录文件。",
  "validation_engine_unavailable": "无法读取当前运行时及推理进程的验证身份，未确认本次验证通过。请检查完整安装包后重试。",
  "validation_scope_unavailable": "无法读取模型或数据目录的本机验证条件，未确认本次验证通过。请检查模型和数据目录后重试。",
  "validation_scope_changed": "模型、运行时或验证条件在测试期间发生变化，本次结果不再有效。请重新加载并测试。",
  "model_download_engine_unavailable": "已验证的下载组件尚未就绪。请使用包含受控下载组件的完整安装包，或手动下载后扫描模型目录。",
  "model_download_network_failed": "当前下载源连接、响应或重定向失败。请检查网络或在设置中显式选择其他来源，再手动重试；未发布模型文件。",
  "model_download_http_failed": "当前下载源连接、响应或重定向失败。请检查网络或在设置中显式选择其他来源，再手动重试；未发布模型文件。",
  "model_download_redirect_rejected": "当前下载源连接、响应或重定向失败。请检查网络或在设置中显式选择其他来源，再手动重试；未发布模型文件。",
  "lan_settings_invalid": "LAN 设置需具体私有 IPv4 地址和 1–65535 端口；启用时需 1–16 条 /24–/32 规范私网 CIDR，不能重复、重叠或带主机位。",
  "lan_token_unavailable": "请先显式启用 LAN 并成功启动服务，再复制独立 LAN 密钥。现有密钥仍需通过安全文件校验。",
  "lan_address_discovery_failed": "无法读取本机 IPv4 网卡地址，可刷新重试或手动填写。",
  "lan_address_discovery_busy": "上次本机地址读取尚未结束，请稍后刷新或手动填写。",
  "lan_address_discovery_timeout": "读取本机网卡地址超时，可稍后刷新或手动填写。",
  "lan_address_discovery_invalid": "系统返回的网卡信息无法安全读取，可刷新重试或手动填写。",
  "lan_address_discovery_limit": "本机网卡信息超过读取上限，请手动填写 IPv4 地址。",
  "not_initialized": "请先明确初始化本机配置。初始化不会启动服务。",
  "response_invalid": "运行服务响应不完整或与当前版本不兼容。请重新确认原任务和服务状态，已有输出可能不完整，不会自动重放。",
  "execution_timeout": "推理执行超时，已生成内容可能不完整。可减少输入，或停止服务后在设置中调大推理执行超时，再启动服务并手动重试。",
  "executor_cleanup_unconfirmed": "服务或任务清理尚未确认。请保留窗口并重新检查当前状态，勿直接启动或重放任务。",
  "model_load_result_unavailable": "本次操作已经结束，但结果已过期。请刷新模型状态后重试。",
  "request_cancelled": "本次加载或基础测试已停止。",
  "model_load_interrupted": "本次加载结果暂时无法确认，正在重新读取；请勿重复加载。",
  "desktop_unavailable": "无法完成桌面操作，请先检查运行服务和当前任务状态。未自动重放请求；排查时请提供诊断码。",
  "api_error": "无法完成桌面操作，请先检查运行服务和当前任务状态。未自动重放请求；排查时请提供诊断码。",
  "internal_error": "无法完成桌面操作，请先检查运行服务和当前任务状态。未自动重放请求；排查时请提供诊断码。",
  "runtime_start_failed": "运行服务未能启动。请检查完整安装包及本机服务状态，再显式启动；不会自动替换已有实例。",
  "runtime_stop_unconfirmed": "服务或任务清理尚未确认。请保留窗口并重新检查当前状态，勿直接启动或重放任务。",
  "import_cleanup_unconfirmed": "服务或任务清理尚未确认。请保留窗口并重新检查当前状态，勿直接启动或重放任务。",
  "connection_failed": "无法确认本机服务连接或身份。请重新检查服务，确认使用同一完整安装包；不要删除凭据或启动重复实例。",
  "runtime_unavailable": "无法确认本机服务连接或身份。请重新检查服务，确认使用同一完整安装包；不要删除凭据或启动重复实例。",
  "runtime_not_running": "运行服务尚未启动，请检查服务状态后显式启动。",
  "runtime_running": "请先停止运行服务，再保存此设置；停止可能影响其他客户端。",
  "model_not_loaded": "当前没有可用的已加载模型。请在模型页显式加载模型，等待就绪后再手动提交。",
  "model_not_ready": "当前没有可用的已加载模型。请在模型页显式加载模型，等待就绪后再手动提交。",
  "worker_lost": "推理进程异常，本次结果未完成。请先检查服务状态，清理确认后显式重新加载模型；已有部分输出保留，不会自动重放。",
  "worker_failed": "推理进程异常，本次结果未完成。请先检查服务状态，清理确认后显式重新加载模型；已有部分输出保留，不会自动重放。",
  "runtime_faulted": "推理进程异常，本次结果未完成。请先检查服务状态，清理确认后显式重新加载模型；已有部分输出保留，不会自动重放。",
  "native_failure": "推理进程异常，本次结果未完成。请先检查服务状态，清理确认后显式重新加载模型；已有部分输出保留，不会自动重放。",
  "native_protocol_error": "推理进程异常，本次结果未完成。请先检查服务状态，清理确认后显式重新加载模型；已有部分输出保留，不会自动重放。",
  "load_timeout": "模型加载超时。请先确认加载任务已结束，再检查文件和加载参数后手动重试；不能据此判定为内存不足。",
  "queue_timeout": "当前任务或队列繁忙。请等待任务结束或确认停止后再手动操作；不会自动排队或中断其他客户端。",
  "queue_full": "当前任务或队列繁忙。请等待任务结束或确认停止后再手动操作；不会自动排队或中断其他客户端。",
  "runtime_busy": "当前任务或队列繁忙。请等待任务结束或确认停止后再手动操作；不会自动排队或中断其他客户端。",
  "desktop_busy": "当前任务或队列繁忙。请等待任务结束或确认停止后再手动操作；不会自动排队或中断其他客户端。",
  "operation_in_progress": "当前任务或队列繁忙。请等待任务结束或确认停止后再手动操作；不会自动排队或中断其他客户端。",
  "consumer_busy": "当前任务或队列繁忙。请等待任务结束或确认停止后再手动操作；不会自动排队或中断其他客户端。",
  "runtime_shutdown": "服务或窗口正在关闭，请等待清理结束并核对状态后再操作。",
  "desktop_closing": "服务或窗口正在关闭，请等待清理结束并核对状态后再操作。",
  "context_length_exceeded": "输入、模板或输出预算超过上下文限制。请减少输入和输出预算，或调整模型支持的上下文配置后重新加载。",
  "context_too_large": "输入、模板或输出预算超过上下文限制。请减少输入和输出预算，或调整模型支持的上下文配置后重新加载。",
  "history_limit": "会话或输出达到安全限制，已有内容可能不完整。请先确认任务结束，再减少输入或清理会话后手动重试。",
  "response_limit": "会话或输出达到安全限制，已有内容可能不完整。请先确认任务结束，再减少输入或清理会话后手动重试。",
  "response_too_large": "会话或输出达到安全限制，已有内容可能不完整。请先确认任务结束，再减少输入或清理会话后手动重试。",
  "slow_consumer": "会话或输出达到安全限制，已有内容可能不完整。请先确认任务结束，再减少输入或清理会话后手动重试。",
  "stream_invalid": "运行服务响应不完整或与当前版本不兼容。请重新确认原任务和服务状态，已有输出可能不完整，不会自动重放。",
  "invalid_stream": "运行服务响应不完整或与当前版本不兼容。请重新确认原任务和服务状态，已有输出可能不完整，不会自动重放。",
  "request_not_owned": "无法确认本窗口原请求。请检查任务和服务状态，不要直接重新提交。",
  "request_not_found": "无法确认本窗口原请求。请检查任务和服务状态，不要直接重新提交。",
  "import_interrupted": "操作可能已经提交，但最终结果或持久化尚未确认。请先刷新模型列表或已保存设置，核对后再操作，勿直接重复提交。",
  "import_committed_durability_unconfirmed": "操作可能已经提交，但最终结果或持久化尚未确认。请先刷新模型列表或已保存设置，核对后再操作，勿直接重复提交。",
  "settings_durability_unconfirmed": "操作可能已经提交，但最终结果或持久化尚未确认。请先刷新模型列表或已保存设置，核对后再操作，勿直接重复提交。",
  "unsupported_model": "此模型未通过当前引擎兼容性检查。请核对 GGUF 文件、架构和配套文件；不保证所有模型均可加载。",
  "unsupported_chat_template": "模型内嵌聊天模板缺失或不支持当前请求。请使用兼容模板的模型，不会静默替换模板。",
  "invalid_manifest": "模型文件或登记信息无法核验。请确认原文件仍可读取且未修改，再刷新模型库或重新添加；已有文件不会自动删除。",
  "model_file_changed": "模型文件或登记信息无法核验。请确认原文件仍可读取且未修改，再刷新模型库或重新添加；已有文件不会自动删除。",
  "model_file_unavailable": "模型文件或登记信息无法核验。请确认原文件仍可读取且未修改，再刷新模型库或重新添加；已有文件不会自动删除。",
  "model_not_found": "找不到所选模型或资源。请刷新列表后重新选择。",
  "not_found": "找不到所选模型或资源。请刷新列表后重新选择。",
  "model_conflict": "所选模型当前正在使用或与驻留状态冲突。请先核对当前任务，空闲后显式卸载再操作。",
  "model_file_in_use": "模型目录或文件暂不可用。请检查本地磁盘、文件访问权限及其他占用，再刷新状态；不会自动修改权限。",
  "model_scan_no_usable_files": "所选文件未通过模型检查，原目录和索引已保留。请查看逐文件诊断，选择有效 GGUF 后重新添加。",
  "settings_invalid": "参数或请求超出支持范围。请检查字段提示和模型限制，修改后再提交。",
  "invalid_request": "参数或请求超出支持范围。请检查字段提示和模型限制，修改后再提交。",
  "invalid_argument": "参数或请求超出支持范围。请检查字段提示和模型限制，修改后再提交。",
  "unsupported_parameter": "参数或请求超出支持范围。请检查字段提示和模型限制，修改后再提交。",
  "packaged_runtime_missing": "安装包中的运行服务或推理组件缺失或校验失败。请使用同一版本的完整安装包修复，不要绕过完整性校验。",
  "settings_write_failed": "文件或配置无法保存。请检查可用磁盘空间及应用数据目录可写性，保留当前输入，核对已保存状态后再操作。",
  "configuration_write_failed": "文件或配置无法保存。请检查可用磁盘空间及应用数据目录可写性，保留当前输入，核对已保存状态后再操作。",
  "model_library_write_failed": "文件或配置无法保存。请检查可用磁盘空间及应用数据目录可写性，保留当前输入，核对已保存状态后再操作。",
  "insufficient_storage": "文件或配置无法保存。请检查可用磁盘空间及应用数据目录可写性，保留当前输入，核对已保存状态后再操作。",
  "model_download_active": "模型正在下载。请等待完成或确认取消后，再修改模型、目录或设置。",
  "model_download_cleanup_unconfirmed": "下载临时文件清理尚未确认。请保留窗口，重新确认下载状态后再关闭。",
  "model_download_cancelled": "下载已取消，未发布新的模型文件。",
  "model_download_timeout": "下载超时。请检查网络和所选下载源，确认当前任务结束后手动重试。",
  "model_download_identity_mismatch": "下载内容与固定大小或 SHA256 不一致，未发布模型文件。请确认下载源后手动重试，不要跳过校验。",
  "model_download_size_mismatch": "下载内容与固定大小或 SHA256 不一致，未发布模型文件。请确认下载源后手动重试，不要跳过校验。",
  "already_exists": "同名文件已经存在，未覆盖原文件。请先核对已有文件，必要时选择它进行登记。",
  "model_directory_required": "请先选择并保存模型目录，再手动执行下载或扫描。已有文件可通过添加模型登记。",
  "model_directory_unavailable": "模型目录或文件暂不可用。请检查本地磁盘、文件访问权限及其他占用，再刷新状态；不会自动修改权限。",
  "model_directory_unsupported": "模型目录或文件暂不可用。请检查本地磁盘、文件访问权限及其他占用，再刷新状态；不会自动修改权限。",
  "model_library_changed": "模型库已变化或达到限制。请刷新模型列表，重新核对要操作的模型。",
  "model_library_limit": "模型库或扫描文件数量达到安全上限。请缩小本次范围后重试，先刷新核对现有索引。",
  "model_library_unsupported": "当前服务不支持此模型管理接口。请显式停止服务并使用匹配的完整安装包重新启动。",
  "model_scan_timeout": "模型扫描已超时或取消。请先核对扫描结果和模型列表，再手动重试。",
  "model_scan_cancelled": "模型扫描已超时或取消。请先核对扫描结果和模型列表，再手动重试。",
  "configuration_unavailable": "当前后台无法提供统一配置。请检查版本；必要时显式停止并重新启动匹配版本，不会自动替换实例。",
  "invalid_download_error": "下载未完成，请提供诊断码和当前进度以便排查。",
  "model_download_write_failed": "下载文件无法写入，请检查目标磁盘空间和目录可写性，核对已保存文件后再手动操作。",
  "clipboard_unavailable": "无法写入剪贴板，请手动复制页面显示的内容。",
  "ocr_image_invalid": "图片准备失败，请重新选择 PNG 或 JPEG 图片并检查尺寸。"
};
// Exact static causes emitted by the bridge; translated values also survive repeated normalization.
const configurationCauses: Readonly<Record<string, string>> = {
  "A required configuration file or directory is missing. Check the existing data directory before initializing or restoring it.": "必需的配置文件或目录缺失。请先检查原有数据目录，再决定初始化或恢复；不会自动重置配置。",
  "The operating system denied access to configuration storage. Check account access and file permissions without weakening its protection.": "操作系统拒绝访问配置存储。请检查当前账户与文件权限，勿降低安全保护。",
  "Configuration storage failed path or ownership protection checks. Use an owned regular file and secure directories; do not bypass these checks.": "配置路径或文件所有权未通过安全检查。请使用本人所有的普通文件和安全目录，不要绕过检查。",
  "Configuration storage could not be accessed. Check storage availability and system diagnostics before retrying.": "无法访问配置存储。请检查磁盘是否可用及系统诊断，再手动重试。",
  "The configuration document exceeds its supported size limit. Review it before retrying; it has not been reset.": "配置文件超过支持的大小限制。请检查文件内容后再试，原文件未被重置。",
  "The configuration file is not valid UTF-8. Correct its encoding while preserving the existing settings.": "配置文件不是有效 UTF-8。请保留原有设置并修正文件编码。",
  "The configuration file contains invalid TOML syntax. Correct the document before retrying.": "配置文件 TOML 语法无效。请修正语法后重新读取，原文件未被重置。",
  "Configuration fields or values are unsupported. Check the schema and allowed values before retrying.": "配置字段或值不受支持。请核对配置版本、字段与允许范围后重新读取。",
  "Another process is changing configuration. Wait for it to finish, then refresh before retrying.": "另一个进程正在修改配置。请等待完成后先重新读取，再决定是否保存。",
  "必需的配置文件或目录缺失。请先检查原有数据目录，再决定初始化或恢复；不会自动重置配置。": "必需的配置文件或目录缺失。请先检查原有数据目录，再决定初始化或恢复；不会自动重置配置。",
  "操作系统拒绝访问配置存储。请检查当前账户与文件权限，勿降低安全保护。": "操作系统拒绝访问配置存储。请检查当前账户与文件权限，勿降低安全保护。",
  "配置路径或文件所有权未通过安全检查。请使用本人所有的普通文件和安全目录，不要绕过检查。": "配置路径或文件所有权未通过安全检查。请使用本人所有的普通文件和安全目录，不要绕过检查。",
  "无法访问配置存储。请检查磁盘是否可用及系统诊断，再手动重试。": "无法访问配置存储。请检查磁盘是否可用及系统诊断，再手动重试。",
  "配置文件超过支持的大小限制。请检查文件内容后再试，原文件未被重置。": "配置文件超过支持的大小限制。请检查文件内容后再试，原文件未被重置。",
  "配置文件不是有效 UTF-8。请保留原有设置并修正文件编码。": "配置文件不是有效 UTF-8。请保留原有设置并修正文件编码。",
  "配置文件 TOML 语法无效。请修正语法后重新读取，原文件未被重置。": "配置文件 TOML 语法无效。请修正语法后重新读取，原文件未被重置。",
  "配置字段或值不受支持。请核对配置版本、字段与允许范围后重新读取。": "配置字段或值不受支持。请核对配置版本、字段与允许范围后重新读取。",
  "另一个进程正在修改配置。请等待完成后先重新读取，再决定是否保存。": "另一个进程正在修改配置。请等待完成后先重新读取，再决定是否保存。"
};
const fallback = messages.desktop_unavailable;
export function errorCode(error: unknown, fallbackCode = "desktop_unavailable"): string {
  const code = error && typeof error === "object" && "code" in error ? error.code : null;
  return typeof code === "string" && /^[a-z0-9_]{1,96}$/.test(code) ? code : fallbackCode;
}
function safeLocalMessage(message: unknown): message is string {
  return typeof message === "string" && !!message.trim() && new TextEncoder().encode(message).byteLength <= 2000 &&
    !Array.from(message).some((c) => c.charCodeAt(0) < 32 || c.charCodeAt(0) >= 127 && c.charCodeAt(0) <= 159) &&
    !/[\u202a-\u202e\u2066-\u2069<>\\]|:\/\/|www\.|(?:^|\s)\/(?:[^\s/]+\/)|\b(?:bearer|authorization|cookie|password|secret|signature|credential)\b/i.test(message);
}
export function presentError(error: unknown, trustedLocal = false, fallbackCode = "desktop_unavailable"): SafeError {
  const code = errorCode(error, fallbackCode);
  const raw = error && typeof error === "object" && "message" in error ? error.message : null;
  let message = Object.hasOwn(messages, code) ? messages[code] : fallback;
  if (trustedLocal && safeLocalMessage(raw)) message = raw;
  // The bridge intentionally exposes only a numeric OS failure and fixed field labels.
  // Preserve these support details without propagating any surrounding native text.
  if (!trustedLocal && typeof raw === "string") {
    if (code === "runtime_start_failed") {
      const os = raw.match(/(?:\(OS error |（OS 错误 )(-?\d{1,10})[)）]$/);
      if (os) message += `（OS 错误 ${os[1]}）`;
    }
    if (code.startsWith("model_download_") || code === "already_exists") {
      const exit = raw.match(/（下载进程退出码 (\d{1,10})）。未切换下载源，未发布模型文件。(?: 临时文件清理未确认，请勿自动重试。)?$/);
      if (exit && Number(exit[1]) <= 4294967295) {
        const reasons: Record<number, string> = { 2: "下载源等待超时", 3: "下载源未找到指定文件", 4: "下载源未找到指定文件", 6: "下载源网络连接或传输失败", 8: "下载源不支持本次续传，有限全量恢复后仍未完成", 9: "模型目录可用空间不足", 13: "下载目标已存在，未覆盖", 14: "本任务临时文件操作失败", 15: "本任务临时文件操作失败", 16: "本任务临时文件操作失败", 17: "本任务临时文件操作失败", 18: "本任务临时文件操作失败", 19: "下载源域名解析失败", 22: "下载源 HTTP 响应不符合要求", 23: "下载源重定向次数超过限制", 24: "下载源要求认证，本下载器不使用账户凭据", 29: "下载源服务暂不可用", 32: "下载字节不符合固定 SHA256" };
        message = `${reasons[Number(exit[1])] ?? "下载进程未成功完成"}（下载进程退出码 ${exit[1]}）。未切换下载源，未发布模型文件。`;
      }
    }
    if (raw.endsWith("临时文件清理未确认，请勿自动重试。")) message += " 临时文件清理未确认，请勿自动重试。";
    if (code.startsWith("configuration_") || code === "model_profile_invalid") {
      const field = raw.match(/（字段：(保存版本|旧桌面偏好版本|配置组|模型运行档案|模型上下文上限|全局加载默认值|全局默认上下文与模型上限|请求默认值|运行策略|空闲释放时间|本机监听|局域网设置)）$/);
      const cause = field ? raw.slice(0, field.index) : raw;
      if (["configuration_unavailable", "configuration_invalid", "configuration_busy"].includes(code) && Object.hasOwn(configurationCauses, cause)) message = configurationCauses[cause];
      if (field) message += `（字段：${field[1]}）`;
    }
  }
  return { code, message };
}
export function errorText(error: unknown): string {
  const safe = presentError(error);
  return `${safe.message}（${safe.code}）`;
}
