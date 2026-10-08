use crate::{
    close_request::CloseRequestGate,
    diagnostics,
    directory_selection::{AdmissionError, DirectorySelection, PickedDirectory},
    file_selection::{FileSelection, PickedFile, PickedFiles},
    layout,
    selection::{PickedModel, Selection},
};
use desktop_bridge::{
    BridgeError, ConfigurationMigrateRequest, ConfigurationSaveRequest, ConfigurationSnapshot,
    DesktopBridge, ModelConfiguration, OcrHistoryEntry, OcrHistoryList, OcrHistorySaveRequest,
    PerformanceSnapshot, UiPreferencesSaveRequest, UiPreferencesSnapshot,
    WorkbenchPreferencesSaveRequest, WorkbenchPreferencesSnapshot, dto::*,
};
use rfd::{MessageButtons, MessageDialogResult, MessageLevel};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
use tauri::{
    Manager, State, WebviewWindow,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use uuid::Uuid;

type Result<T> = std::result::Result<T, BridgeError>;
struct Shell {
    bridge: Arc<DesktopBridge>,
    data_dir: PathBuf,
    package_root: PathBuf,
    selection: Mutex<Option<Selection>>,
    pair_selection: Mutex<Option<FileSelection<(PathBuf, desktop_bridge::SelectedFile)>>>,
    file_selection: Mutex<Option<FileSelection<desktop_bridge::SelectedFile>>>,
    directory_selection: Mutex<Option<DirectorySelection>>,
    picking: AtomicBool,
    closing: AtomicBool,
    closed: AtomicBool,
    window_close_pending: AtomicBool,
    exit_generation: AtomicU64,
    close_request: CloseRequestGate,
}
fn error(code: &str) -> BridgeError {
    if diagnostics::PACKAGE_ERROR_CODES.contains(&code) {
        return BridgeError {
            code: diagnostics::safe_code(code).into(),
            message: diagnostics::package_operation_message(code),
        };
    }
    BridgeError {
        code: code.into(),
        message: match code {
            "desktop_closing" => "窗口正在关闭，请等待清理完成。",
            "desktop_busy" => "操作正在进行，请等待。",
            "autostart_path_too_long" => "当前 Nexa 路径过长，超过 Windows 启动项命令的 260 字符限制。请将完整 Nexa 目录移到更短路径，重新打开后再启用开机启动。",
            "clipboard_unavailable" => "无法写入系统剪贴板，请稍后重试。",
            "token_unavailable" => "令牌文件未初始化或安全校验失败。",
            "selection_expired" => "所选项目已过期，请重新选择。",
            "selected_file_not_gguf" => "请选择扩展名为 .gguf 的模型文件。",
            "selected_file_name_invalid" => "所选文件名无法识别，请重新选择有效的 GGUF 文件。",
            "selected_pair_same_file" => {
                "主模型与视觉投影不能是同一个文件，请选择配套的 mmproj 文件。"
            }
            "model_library_limit" => {
                "所选文件超出数量或大小限制：最多64个、单个16 GiB、每批32 GiB。"
            }
            "model_file_in_use" => "所选文件正在被其他程序写入或占用，请关闭占用程序后重新选择。",
            "model_file_changed" => "所选文件或所在目录已经变化，请重新选择。",
            "model_file_unavailable" => "所选文件当前不可访问，请检查文件后重新选择。",
            "model_directory_unavailable" => "所选模型目录当前不可访问，请检查后重新选择。",
            "model_directory_unsupported" => {
                "请选择支持的本地普通目录，不使用网络、设备、链接或重解析路径。"
            }
            "model_directory_inside_package_unsupported" => {
                "程序包内只能选程序根、model 或 models 目录；也可选择程序包外目录。"
            }
            "unauthorized_window" => "此窗口无权调用本机操作。",
            _ => "操作无法安全完成，请刷新状态后重试。",
        }
        .into(),
    }
}
fn pair_selection_error(code: &str) -> BridgeError {
    // These codes also occur during package validation. Describe the selected
    // OCR input here without changing the separate package diagnostic messages.
    let message = match code {
        "selected_path_invalid" => "请选择本机磁盘中的 GGUF 文件，不使用网络盘、设备或特殊路径。",
        "selected_path_indirect" => "所选文件或目录是链接或重解析路径，请从本机普通目录重新选择。",
        "selected_file_unavailable" => {
            "所选文件无法读取，请确认下载已完成且文件仍在原位置，然后重新选择。"
        }
        "selected_file_invalid" => "请选择文件，不能选择目录。",
        _ => return error(code),
    };
    BridgeError {
        code: code.into(),
        message: message.into(),
    }
}
fn local_url(url: &tauri::Url) -> bool {
    matches!(
        (url.scheme(), url.host_str(), url.port()),
        ("tauri", Some("localhost"), None) | ("http", Some("tauri.localhost"), None)
    ) || (cfg!(debug_assertions)
        && matches!(
            (url.scheme(), url.host_str(), url.port()),
            ("http", Some("127.0.0.1"), Some(1420))
        ))
}
fn guard(window: &WebviewWindow, shell: &Shell) -> Result<()> {
    if window.label() != "main" || !window.url().is_ok_and(|url| local_url(&url)) {
        return Err(error("unauthorized_window"));
    }
    if shell.closing.load(Ordering::Acquire) {
        return Err(error("desktop_closing"));
    }
    Ok(())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StartRequest {
    initialize_if_missing: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageRequest {
    after: Option<String>,
    generation: Option<Uuid>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DirectoryRequest {
    selection_id: Uuid,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DownloadRequest {
    catalog_id: String,
    #[serde(default)]
    auto_test: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LibraryRequest {
    operation_id: Uuid,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddRequest {
    selection_id: Uuid,
    #[serde(default)]
    auto_test: bool,
}
#[derive(Serialize)]
struct Discarded {
    discarded: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportRequest {
    selection_id: Uuid,
    model_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestId {
    request_id: Uuid,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsRequest {
    settings: DesktopPreferences,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IdleRequest {
    idle_unload_seconds: u64,
    idle_unload_enabled: Option<bool>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VerificationRequest {
    model_verification_timeout_seconds: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LanRequest {
    lan_api: runtime_api::LanApiConfig,
}
#[derive(Serialize)]
struct Copied {
    copied: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigurationModelRequest {
    model_id: String,
}
#[tauri::command]
async fn runtime_initialize(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
) -> Result<DesktopSnapshot> {
    guard(&window, &state)?;
    state.bridge.initialize().await
}
#[tauri::command]
async fn performance_get(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
) -> Result<PerformanceSnapshot> {
    guard(&window, &state)?;
    state.bridge.performance_get().await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HistoryIdRequest {
    id: Uuid,
}
#[tauri::command]
async fn ocr_history_list(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
) -> Result<OcrHistoryList> {
    guard(&window, &state)?;
    state.bridge.ocr_history_list().await
}
#[tauri::command]
async fn ocr_history_get(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: HistoryIdRequest,
) -> Result<OcrHistoryEntry> {
    guard(&window, &state)?;
    state.bridge.ocr_history_get(request.id).await
}
#[tauri::command]
async fn ocr_history_save(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: OcrHistorySaveRequest,
) -> Result<OcrHistoryList> {
    guard(&window, &state)?;
    state.bridge.ocr_history_save(request).await
}
#[tauri::command]
async fn ocr_history_delete(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: HistoryIdRequest,
) -> Result<OcrHistoryList> {
    guard(&window, &state)?;
    state.bridge.ocr_history_delete(request.id).await
}
#[tauri::command]
async fn workbench_get(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
) -> Result<WorkbenchPreferencesSnapshot> {
    guard(&window, &state)?;
    state.bridge.workbench_get().await
}
#[tauri::command]
async fn autostart_get(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
) -> Result<crate::autostart::Snapshot> {
    guard(&window, &state)?;
    crate::autostart::get().map_err(error)
}
#[tauri::command]
async fn autostart_set(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: crate::autostart::SetRequest,
) -> Result<crate::autostart::Snapshot> {
    guard(&window, &state)?;
    crate::autostart::set(request.enabled).map_err(error)
}
#[tauri::command]
async fn workbench_save(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: WorkbenchPreferencesSaveRequest,
) -> Result<WorkbenchPreferencesSnapshot> {
    guard(&window, &state)?;
    state.bridge.workbench_save(request).await
}
#[tauri::command]
async fn configuration_get(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
) -> Result<ConfigurationSnapshot> {
    guard(&window, &state)?;
    state.bridge.configuration_get().await
}
#[tauri::command]
async fn configuration_model_get(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: ConfigurationModelRequest,
) -> Result<ModelConfiguration> {
    guard(&window, &state)?;
    state.bridge.configuration_model_get(request.model_id).await
}
#[tauri::command]
async fn configuration_save(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: ConfigurationSaveRequest,
) -> Result<ConfigurationSnapshot> {
    guard(&window, &state)?;
    state.bridge.configuration_save(request).await
}
#[tauri::command]
async fn configuration_migrate(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: ConfigurationMigrateRequest,
) -> Result<ConfigurationSnapshot> {
    guard(&window, &state)?;
    state.bridge.configuration_migrate(request).await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelLoadOperationRequest {
    operation_id: Uuid,
}
#[tauri::command]
async fn model_load_start(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: ModelLoadStartRequest,
) -> Result<ModelLoadOperationHandle> {
    guard(&window, &state)?;
    state.bridge.model_load_start(request)
}
#[tauri::command]
async fn model_load_profile_start(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: ModelLoadProfileStartRequest,
) -> Result<ModelLoadOperationHandle> {
    guard(&window, &state)?;
    state.bridge.model_load_profile_start(request)
}
#[tauri::command]
async fn model_load_next(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: ModelLoadOperationRequest,
) -> Result<ModelLoadOperationState> {
    guard(&window, &state)?;
    state.bridge.model_load_next(request.operation_id).await
}
#[tauri::command]
async fn model_load_cancel(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: ModelLoadOperationRequest,
) -> Result<ModelLoadStopping> {
    guard(&window, &state)?;
    state.bridge.model_load_cancel(request.operation_id).await
}
#[tauri::command]
async fn model_load_profile(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: ModelLoadProfileRequest,
) -> Result<RuntimeStatus> {
    guard(&window, &state)?;
    state.bridge.model_load_profile(request).await
}
#[tauri::command]
async fn ui_preferences_get(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
) -> Result<UiPreferencesSnapshot> {
    guard(&window, &state)?;
    state.bridge.ui_preferences_get()
}
#[tauri::command]
async fn ui_preferences_save(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: UiPreferencesSaveRequest,
) -> Result<UiPreferencesSnapshot> {
    guard(&window, &state)?;
    state.bridge.ui_preferences_save(request).await
}

#[tauri::command]
async fn desktop_snapshot(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
) -> Result<DesktopSnapshot> {
    guard(&window, &state)?;
    state.bridge.snapshot().await
}
#[tauri::command]
async fn runtime_start(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: StartRequest,
) -> Result<DesktopSnapshot> {
    guard(&window, &state)?;
    state.bridge.start(request.initialize_if_missing).await
}
#[tauri::command]
async fn model_pick(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
) -> Result<Option<PickedModel>> {
    guard(&window, &state)?;
    if state.picking.swap(true, Ordering::AcqRel) {
        return Err(error("desktop_busy"));
    }
    struct Picking<'a>(&'a AtomicBool);
    impl Drop for Picking<'_> {
        fn drop(&mut self) {
            self.0.store(false, Ordering::Release);
        }
    }
    let _picking = Picking(&state.picking);
    let picked = rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .set_title("选择本地 GGUF 模型（导入会复制文件）")
        .add_filter("GGUF 模型", &["gguf"])
        .pick_file()
        .await;
    guard(&window, &state)?;
    let Some(picked) = picked else {
        return Ok(None);
    };
    let (selection, dto) = Selection::new(picked.path()).map_err(error)?;
    *state.selection.lock().map_err(|_| error("desktop_busy"))? = Some(selection);
    Ok(Some(dto))
}
#[tauri::command]
async fn models_pick(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
) -> Result<Option<PickedFiles>> {
    guard(&window, &state)?;
    if state.picking.swap(true, Ordering::AcqRel) {
        return Err(error("desktop_busy"));
    }
    struct Picking<'a>(&'a AtomicBool);
    impl Drop for Picking<'_> {
        fn drop(&mut self) {
            self.0.store(false, Ordering::Release);
        }
    }
    let _picking = Picking(&state.picking);
    *state
        .file_selection
        .lock()
        .map_err(|_| error("desktop_busy"))? = None;
    let picked = rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .set_title("添加本地 GGUF 模型（零复制，可多选）")
        .add_filter("GGUF 模型", &["gguf"])
        .pick_files()
        .await;
    guard(&window, &state)?;
    let Some(picked) = picked else {
        return Ok(None);
    };
    if picked.is_empty() || picked.len() > 64 {
        return Err(error("model_library_limit"));
    }
    let files = picked
        .iter()
        .map(|file| {
            desktop_bridge::SelectedFile::open(file.path())
                .map_err(|cause| error(cause.code.as_str()))
        })
        .collect::<Result<Vec<_>>>()?;
    desktop_bridge::validate_selection(&files).map_err(|cause| error(cause.code.as_str()))?;
    let summaries = files
        .iter()
        .enumerate()
        .map(|(selection_index, file)| PickedFile {
            selection_index,
            file_name: file.file_name().to_owned(),
            size_bytes: file.size_bytes(),
        })
        .collect();
    let (selection, dto) = FileSelection::new(files, summaries);
    *state
        .file_selection
        .lock()
        .map_err(|_| error("desktop_busy"))? = Some(selection);
    Ok(Some(dto))
}
#[tauri::command]
async fn models_selection_discard(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: DirectoryRequest,
) -> Result<Discarded> {
    guard(&window, &state)?;
    let mut slot = state
        .file_selection
        .lock()
        .map_err(|_| error("desktop_busy"))?;
    Ok(Discarded {
        discarded: FileSelection::discard(&mut slot, request.selection_id),
    })
}
#[tauri::command]
async fn models_pair_pick(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
) -> Result<Option<PickedFiles>> {
    guard(&window, &state)?;
    if state.picking.swap(true, Ordering::AcqRel) {
        return Err(error("desktop_busy"));
    }
    struct Picking<'a>(&'a AtomicBool);
    impl Drop for Picking<'_> {
        fn drop(&mut self) {
            self.0.store(false, Ordering::Release);
        }
    }
    let _picking = Picking(&state.picking);
    *state
        .pair_selection
        .lock()
        .map_err(|_| error("desktop_busy"))? = None;
    let mut files = Vec::new();
    let mut summaries = Vec::new();
    let mut previous_canonical = None;
    for title in [
        "选择 OCR 主模型 GGUF（将复制两个文件）",
        "选择配套视觉投影 GGUF（mmproj）",
    ] {
        let picked = rfd::AsyncFileDialog::new()
            .set_parent(&window)
            .set_title(title)
            .add_filter("GGUF", &["gguf"])
            .pick_file()
            .await;
        guard(&window, &state)?;
        let Some(picked) = picked else {
            return Ok(None);
        };
        let file = crate::selection::pair_file(picked.path(), previous_canonical.as_deref())
            .map_err(pair_selection_error)?;
        let lease =
            desktop_bridge::SelectedFile::open(&file.source).map_err(|e| error(e.code.as_str()))?;
        summaries.push(PickedFile {
            selection_index: files.len(),
            file_name: lease.file_name().to_owned(),
            size_bytes: lease.size_bytes(),
        });
        previous_canonical = Some(file.canonical);
        files.push((file.source, lease));
    }
    let (selection, dto) = FileSelection::new(files, summaries);
    *state
        .pair_selection
        .lock()
        .map_err(|_| error("desktop_busy"))? = Some(selection);
    Ok(Some(dto))
}
#[tauri::command]
async fn models_pair_import(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: ImportRequest,
) -> Result<ModelSummary> {
    guard(&window, &state)?;
    let files = {
        let mut slot = state
            .pair_selection
            .lock()
            .map_err(|_| error("desktop_busy"))?;
        FileSelection::consume_pair(&mut slot, request.selection_id).map_err(error)?
    };
    // Keep both native read/directory leases alive until publication or failure.
    let result = state
        .bridge
        .import_model_pair(files[0].0.clone(), files[1].0.clone(), request.model_id)
        .await;
    drop(files);
    result
}
#[tauri::command]
async fn models_add(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: AddRequest,
) -> Result<LibraryOperationHandle> {
    guard(&window, &state)?;
    let mut slot = state
        .file_selection
        .lock()
        .map_err(|_| error("desktop_busy"))?;
    let selection = FileSelection::get(&mut slot, request.selection_id).map_err(error)?;
    let handle = state
        .bridge
        .models_add(&mut selection.files, request.auto_test)?;
    *slot = None;
    Ok(handle)
}
#[tauri::command]
async fn model_import(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: ImportRequest,
) -> Result<ModelSummary> {
    guard(&window, &state)?;
    let path = Selection::consume(
        &mut *state.selection.lock().map_err(|_| error("desktop_busy"))?,
        request.selection_id,
    )
    .map_err(error)?;
    state.bridge.import_model(path, request.model_id).await
}
#[tauri::command]
async fn model_directory_pick(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
) -> Result<Option<PickedDirectory>> {
    guard(&window, &state)?;
    if state.picking.swap(true, Ordering::AcqRel) {
        return Err(error("desktop_busy"));
    }
    struct Picking<'a>(&'a AtomicBool);
    impl Drop for Picking<'_> {
        fn drop(&mut self) {
            self.0.store(false, Ordering::Release);
        }
    }
    let _picking = Picking(&state.picking);
    let picked = rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .set_title("选择默认下载目录（只保存位置，不扫描模型）")
        .pick_folder()
        .await;
    guard(&window, &state)?;
    let Some(picked) = picked else {
        return Ok(None);
    };
    desktop_bridge::validate_model_directory_path(picked.path())?;
    let (selection, dto) =
        DirectorySelection::new_location(picked.path(), &state.package_root).map_err(error)?;
    *state
        .directory_selection
        .lock()
        .map_err(|_| error("desktop_busy"))? = Some(selection);
    Ok(Some(dto))
}
#[tauri::command]
async fn model_directory_apply(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: DirectoryRequest,
) -> Result<LibraryOperationHandle> {
    guard(&window, &state)?;
    let mut selection = state
        .directory_selection
        .lock()
        .map_err(|_| error("desktop_busy"))?;
    if state.bridge.library_active().is_some() {
        return Err(error("desktop_busy"));
    }
    DirectorySelection::admit(
        &mut selection,
        request.selection_id,
        &state.package_root,
        |path| {
            desktop_bridge::validate_model_directory_path(&path)?;
            state.bridge.directory_apply(path)
        },
    )
    .map_err(|failure| match failure {
        AdmissionError::Selection(code) => error(code),
        AdmissionError::Rejected(error) => error,
    })
}
#[tauri::command]
async fn model_directory_configure(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: DirectoryRequest,
) -> Result<LibraryOperationHandle> {
    guard(&window, &state)?;
    let mut selection = state
        .directory_selection
        .lock()
        .map_err(|_| error("desktop_busy"))?;
    if state.bridge.library_active().is_some() {
        return Err(error("desktop_busy"));
    }
    DirectorySelection::admit_location(
        &mut selection,
        request.selection_id,
        &state.package_root,
        |path| {
            desktop_bridge::validate_model_directory_path(&path)?;
            state.bridge.directory_configure(path)
        },
    )
    .map_err(|failure| match failure {
        AdmissionError::Selection(code) => error(code),
        AdmissionError::Rejected(error) => error,
    })
}
#[tauri::command]
async fn model_directory_discover(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
) -> Result<Option<LibraryOperationHandle>> {
    guard(&window, &state)?;
    state.bridge.directory_discover()
}
#[tauri::command]
async fn model_catalog(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
) -> Result<ModelCatalog> {
    guard(&window, &state)?;
    state.bridge.model_catalog()
}
#[tauri::command]
async fn model_download_start(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: DownloadRequest,
) -> Result<DownloadOperationHandle> {
    guard(&window, &state)?;
    state
        .bridge
        .download_start_with_options(request.catalog_id, request.auto_test)
}
#[tauri::command]
async fn model_download_next(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: LibraryRequest,
) -> Result<DownloadOperationState> {
    guard(&window, &state)?;
    state.bridge.download_next(request.operation_id).await
}
#[tauri::command]
async fn model_download_cancel(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: LibraryRequest,
) -> Result<DownloadStopping> {
    guard(&window, &state)?;
    state.bridge.download_cancel(request.operation_id).await
}
#[tauri::command]
async fn models_scan(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
) -> Result<LibraryOperationHandle> {
    guard(&window, &state)?;
    state.bridge.models_scan()
}
#[tauri::command]
async fn models_reconcile(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
) -> Result<ModelsReconcile> {
    guard(&window, &state)?;
    state.bridge.models_reconcile()
}
#[tauri::command]
async fn model_test(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: LoadModelRequest,
) -> Result<LocalValidation> {
    guard(&window, &state)?;
    state.bridge.model_test(request).await
}
#[tauri::command]
async fn model_library_next(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: LibraryRequest,
) -> Result<LibraryOperationState> {
    guard(&window, &state)?;
    state.bridge.library_next(request.operation_id).await
}
#[tauri::command]
async fn model_library_cancel(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: LibraryRequest,
) -> Result<LibraryStopping> {
    guard(&window, &state)?;
    state.bridge.library_cancel(request.operation_id).await
}
#[tauri::command]
async fn models_page(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: PageRequest,
) -> Result<ModelsPage> {
    guard(&window, &state)?;
    state
        .bridge
        .models_page(request.after, request.generation)
        .await
}
#[tauri::command]
async fn model_load(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: LoadModelRequest,
) -> Result<RuntimeStatus> {
    guard(&window, &state)?;
    state.bridge.load_model(request).await
}
#[tauri::command]
async fn model_unregister(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: UnregisterModelRequest,
) -> Result<UnregisterModelResult> {
    guard(&window, &state)?;
    state.bridge.unregister_model(request).await
}
#[tauri::command]
async fn model_unload(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
) -> Result<RuntimeStatus> {
    guard(&window, &state)?;
    state.bridge.unload_model().await
}
#[tauri::command]
async fn chat_start(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: ChatStartRequest,
) -> Result<RequestHandle> {
    guard(&window, &state)?;
    state.bridge.chat_start(request)
}
#[tauri::command]
async fn ocr_start(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: OcrStartRequest,
) -> Result<RequestHandle> {
    guard(&window, &state)?;
    state.bridge.ocr_start(request)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MarkdownRequest {
    text: String,
}
#[derive(Serialize)]
struct SavedMarkdown {
    saved: bool,
}
#[tauri::command]
async fn ocr_save_markdown(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: MarkdownRequest,
) -> Result<SavedMarkdown> {
    guard(&window, &state)?;
    if request.text.len() > 256 * 1024 {
        return Err(error("invalid_request"));
    }
    let file = rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .set_title("保存 OCR 原文")
        .set_file_name("ocr.md")
        .add_filter("Markdown", &["md"])
        .save_file()
        .await;
    guard(&window, &state)?;
    if let Some(file) = file {
        file.write(request.text.as_bytes())
            .await
            .map_err(|_| error("file_save_failed"))?;
        Ok(SavedMarkdown { saved: true })
    } else {
        Ok(SavedMarkdown { saved: false })
    }
}
#[tauri::command]
async fn chat_next(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: RequestId,
) -> Result<ChatBatch> {
    guard(&window, &state)?;
    state.bridge.chat_next(request.request_id).await
}
#[tauri::command]
async fn chat_cancel(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: RequestId,
) -> Result<Stopping> {
    guard(&window, &state)?;
    state.bridge.chat_cancel(request.request_id).await
}
#[tauri::command]
async fn settings_save(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: SettingsRequest,
) -> Result<DesktopSnapshot> {
    guard(&window, &state)?;
    state.bridge.settings_save(request.settings).await
}
#[tauri::command]
async fn runtime_idle_save(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: IdleRequest,
) -> Result<DesktopSnapshot> {
    guard(&window, &state)?;
    state
        .bridge
        .save_idle_policy(request.idle_unload_seconds, request.idle_unload_enabled)
        .await
}
#[tauri::command]
async fn runtime_verification_save(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: VerificationRequest,
) -> Result<DesktopSnapshot> {
    guard(&window, &state)?;
    state
        .bridge
        .save_verification_timeout(request.model_verification_timeout_seconds)
        .await
}
#[tauri::command]
async fn runtime_lan_save(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: LanRequest,
) -> Result<DesktopSnapshot> {
    guard(&window, &state)?;
    state.bridge.save_lan(request.lan_api).await
}
#[tauri::command]
async fn runtime_lan_addresses(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
) -> Result<LanIpv4Addresses> {
    guard(&window, &state)?;
    state.bridge.lan_addresses().await
}
#[tauri::command]
async fn lan_token_copy(window: WebviewWindow, state: State<'_, Arc<Shell>>) -> Result<Copied> {
    guard(&window, &state)?;
    let token = state.bridge.lan_token_for_copy().await?;
    let header = token.bearer_header_value();
    let value = header
        .to_str()
        .ok()
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or_else(|| error("token_unavailable"))?;
    clipboard_win::set_clipboard_string(value).map_err(|_| error("clipboard_unavailable"))?;
    Ok(Copied { copied: true })
}
#[tauri::command]
async fn token_copy(window: WebviewWindow, state: State<'_, Arc<Shell>>) -> Result<Copied> {
    guard(&window, &state)?;
    // The secret and Authorization value never cross IPC, appear in errors, or
    // enter application logging. There is deliberately no clipboard-read command.
    let token = runtime_api::token::load_private_token(&state.data_dir)
        .map_err(|_| error("token_unavailable"))?;
    let header = token.bearer_header_value();
    let value = header
        .to_str()
        .ok()
        .and_then(|h| h.strip_prefix("Bearer "))
        .ok_or_else(|| error("token_unavailable"))?;
    clipboard_win::set_clipboard_string(value).map_err(|_| error("clipboard_unavailable"))?;
    Ok(Copied { copied: true })
}
#[tauri::command]
async fn runtime_stop(window: WebviewWindow, state: State<'_, Arc<Shell>>) -> Result<Stopped> {
    guard(&window, &state)?;
    state.bridge.stop().await
}
#[tauri::command]
async fn desktop_close_ack(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    request: HistoryIdRequest,
) -> Result<bool> {
    guard(&window, &state)?;
    Ok(state.close_request.acknowledge(request.id))
}
#[tauri::command]
async fn desktop_close(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    app: tauri::AppHandle,
) -> Result<()> {
    // The UI flushes pending preferences/history before this command. Repeated
    // closes merge; no shell-side Child handle or kill-on-drop is used.
    if window.label() != "main" || !window.url().is_ok_and(|url| local_url(&url)) {
        return Err(error("unauthorized_window"));
    }
    close(app, Arc::clone(&state)).await;
    Ok(())
}
async fn request_close(app: tauri::AppHandle, state: Arc<Shell>) {
    // Invalidate any title-bar preference read still in flight. An explicit
    // exit must not be hidden by that older request after restoring the UI.
    state.exit_generation.fetch_add(1, Ordering::AcqRel);
    if state.closing.load(Ordering::Acquire) {
        return;
    }
    let Some(mut id) = state.close_request.begin() else {
        return;
    };
    // Explicit exit always restores the UI so its save/discard confirmation is
    // visible. It never passes through the title-bar hide-to-tray path.
    show_main(&app);
    loop {
        // Only dispatch a fixed lifecycle event to our bundled main UI. UUID
        // formatting contains no script text supplied by the webview or files.
        let dispatched = app.get_webview_window("main").is_some_and(|window| {
            window.url().is_ok_and(|url| local_url(&url))
                && window.eval(format!(
                    "window.dispatchEvent(new CustomEvent('nexa-close-requested', {{detail: {{id: '{id}'}}}}));"
                )).is_ok()
        });
        if dispatched {
            let _ = tauri::async_runtime::spawn_blocking(|| {
                std::thread::sleep(std::time::Duration::from_secs(5));
            })
            .await;
        }
        if state.closing.load(Ordering::Acquire) || !state.close_request.expire(id) {
            return;
        }
        // A successful eval only queues JavaScript. Never silently discard
        // pending edits if the UI was not ready or has stopped responding.
        let choice = rfd::AsyncMessageDialog::new()
            .set_title("Nexa：界面尚未确认保存")
            .set_description("界面没有响应关闭请求，无法确认刚修改的设置和识别结果是否已保存。\n重试会再次请求保存；仍要关闭可能丢失尚未保存的内容。")
            .set_level(MessageLevel::Warning)
            .set_buttons(MessageButtons::YesNoCancelCustom("重试".into(), "仍要关闭".into(), "保留窗口".into()))
            .show().await;
        if !state.close_request.is_pending(id) || state.closing.load(Ordering::Acquire) {
            return;
        }
        match choice {
            MessageDialogResult::Custom(label) if label == "重试" => {
                state.close_request.clear();
                let Some(next) = state.close_request.begin() else {
                    return;
                };
                id = next;
                continue;
            }
            MessageDialogResult::Custom(label) if label == "仍要关闭" => {
                state.close_request.clear();
                close(app, state).await;
            }
            _ => {
                state.close_request.clear();
            }
        }
        return;
    }
}
fn show_main(app: &tauri::AppHandle) {
    let state = app.state::<Arc<Shell>>();
    // Invalidate queued hide callbacks before restoring, including a tray
    // click while a title-bar close is still reading the saved preference.
    state.exit_generation.fetch_add(1, Ordering::AcqRel);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

async fn hide_main_if_current(
    app: &tauri::AppHandle,
    state: &Arc<Shell>,
    exit_generation: u64,
) -> Option<bool> {
    let (sender, mut receiver) = tauri::async_runtime::channel(1);
    let on_main = app.clone();
    let state = Arc::clone(state);
    // Tauri executes window commands immediately on the main thread, but
    // queues calls from workers. Check freshness and hide in the same main-
    // thread callback so an older queued hide cannot undo a newer tray restore.
    if app
        .run_on_main_thread(move || {
            let hidden = if state.closing.load(Ordering::Acquire)
                || state.exit_generation.load(Ordering::Acquire) != exit_generation
            {
                None
            } else {
                Some(
                    on_main.tray_by_id("nexa-main-tray").is_some()
                        && on_main
                            .get_webview_window("main")
                            .is_some_and(|window| window.hide().is_ok()),
                )
            };
            let _ = sender.try_send(hidden);
        })
        .is_err()
    {
        return Some(false);
    }
    receiver.recv().await.unwrap_or(Some(false))
}

fn create_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "nexa-tray-show", "显示 Nexa", true, None::<&str>)?;
    let exit = MenuItem::with_id(app, "nexa-tray-exit", "退出 Nexa", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &exit])?;
    let Some(icon) = app.default_window_icon().cloned() else {
        return Err(tauri::Error::AssetNotFound("tray icon".into()));
    };
    TrayIconBuilder::with_id("nexa-main-tray")
        .icon(icon)
        .tooltip("Nexa")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "nexa-tray-show" => show_main(app),
            "nexa-tray-exit" => {
                let state = Arc::clone(&app.state::<Arc<Shell>>());
                tauri::async_runtime::spawn(request_close(app.clone(), state));
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                show_main(tray.app_handle());
            }
        })
        .build(app)?;
    // Tauri's resource table retains the successfully registered tray icon.
    Ok(())
}

async fn request_window_close(app: tauri::AppHandle, state: Arc<Shell>, exit_generation: u64) {
    if state.closing.load(Ordering::Acquire)
        || state.window_close_pending.swap(true, Ordering::AcqRel)
    {
        return;
    }
    // Read the committed CAS-protected preference each time. Uncommitted UI
    // drafts and another process's stale preference never decide hide behavior.
    let preference = state.bridge.workbench_get().await;
    if state.closing.load(Ordering::Acquire)
        || state.exit_generation.load(Ordering::Acquire) != exit_generation
    {
        state.window_close_pending.store(false, Ordering::Release);
        return;
    }
    match preference {
        Ok(snapshot) if !snapshot.preferences.close_to_tray => {
            state.window_close_pending.store(false, Ordering::Release);
            request_close(app, state).await;
            return;
        }
        Ok(_) => {
            let hidden = hide_main_if_current(&app, &state, exit_generation).await;
            if hidden == Some(false) {
                show_main(&app);
                rfd::AsyncMessageDialog::new()
                    .set_title("Nexa：无法关闭到托盘")
                    .set_description(
                        "托盘入口不可用，窗口已保留。请使用界面中的退出按钮，或重启 Nexa 后重试。",
                    )
                    .set_level(MessageLevel::Warning)
                    .show()
                    .await;
            }
        }
        Err(_) => {
            show_main(&app);
            rfd::AsyncMessageDialog::new()
                .set_title("Nexa：无法读取关闭偏好")
                .set_description("工作区设置读取失败，窗口已保留。请在设置中检查保存状态后重试。")
                .set_level(MessageLevel::Warning)
                .show()
                .await;
        }
    }
    state.window_close_pending.store(false, Ordering::Release);
}

async fn close(app: tauri::AppHandle, state: Arc<Shell>) {
    if state.closing.swap(true, Ordering::AcqRel) {
        return;
    }
    state.close_request.clear();
    if let Ok(mut slot) = state.file_selection.lock() {
        *slot = None;
    }
    if let Ok(mut slot) = state.pair_selection.lock() {
        *slot = None;
    }
    loop {
        match state.bridge.close().await {
            Ok(()) => break,
            Err(_) => {
                let choice = rfd::AsyncMessageDialog::new().set_title("Nexa：尚未确认关闭")
                    .set_description("无法确认任务或 runtime 清理完成，尚未标记为已停止。\n重试：重新执行关闭。仅关闭界面：取消本窗口任务并保留 runtime。保留窗口：返回界面检查状态。")
                    .set_level(MessageLevel::Warning)
                    .set_buttons(MessageButtons::YesNoCancelCustom("重试".into(), "仅关闭界面".into(), "保留窗口".into())).show().await;
                match choice {
                    MessageDialogResult::Custom(label) if label == "重试" => continue,
                    MessageDialogResult::Custom(label) if label == "仅关闭界面" => {
                        if state.bridge.close_ui_only().await.is_ok() {
                            break;
                        }
                        rfd::AsyncMessageDialog::new().set_title("Nexa：任务清理未确认")
                            .set_description("本窗口任务清理仍未确认。窗口已保留，请等待或重试；没有强制结束 runtime。")
                            .set_level(MessageLevel::Warning).show().await;
                    }
                    _ => {}
                }
                state.closing.store(false, Ordering::Release);
                return;
            }
        }
    }
    state.closed.store(true, Ordering::Release);
    app.exit(0);
}
fn failure(message: &str) {
    rfd::MessageDialog::new()
        .set_title("Nexa 无法启动")
        .set_level(MessageLevel::Error)
        .set_description(message)
        .show();
}
pub fn run() {
    let version = tauri::webview_version()
        .ok()
        .filter(|v| !v.trim().is_empty());
    let layout = std::env::current_exe()
        .map_err(|_| "current_executable_unavailable")
        .and_then(|exe| layout::validate(&exe));
    // Diagnostic mode does not create a WebView, change user data, or launch the
    // runtime. CI reads only these controlled observations from an extracted ZIP.
    if std::env::args_os()
        .skip(1)
        .eq([std::ffi::OsString::from("--diagnose")])
    {
        println!(
            "{}",
            serde_json::to_string(&diagnostics::StartupDiagnostic::new(
                &layout,
                version.as_deref()
            ))
            .expect("fixed startup diagnostic is serializable")
        );
        std::process::exit(if layout.is_ok() && version.is_some() {
            0
        } else {
            1
        });
    }
    if std::env::args_os().len() != 1 {
        failure("启动参数无效，请直接打开 nexa-desktop.exe。");
        return;
    }
    if version.is_none() {
        failure(
            "未检测到可用的 Microsoft Edge WebView2 Evergreen Runtime。\n请自行打开微软官方页面，选择 Evergreen Standalone Installer 的 x64 版本：\nhttps://developer.microsoft.com/microsoft-edge/webview2/\n安装后重新启动 Nexa。本程序不会自动下载、安装或更改系统权限。",
        );
        return;
    }
    let layout = match layout {
        Ok(layout) => layout,
        Err(code) => {
            failure(&diagnostics::package_message(code));
            return;
        }
    };
    let Ok(data_dir) = desktop_bridge::default_data_dir() else {
        failure("无法确定受控的本地 Nexa 数据目录。");
        return;
    };
    let validation_root = layout.package_root.clone();
    let download_root = layout.package_root.clone();
    let download_commit = layout.project_commit.clone();
    let bridge = match DesktopBridge::new(data_dir.clone(), layout.runtime_executable) {
        Ok(bridge) => Arc::new(
            bridge
                .with_download_sidecar_verifier(move || {
                    crate::download_component::verify(&download_root, &download_commit)
                })
                .with_default_model_directory(layout.package_root.join("models"))
                .with_directory_validator(move |path| {
                    desktop_bridge::validate_model_directory_path(path)?;
                    crate::directory_selection::preflight_directory(path, &validation_root)
                        .map_err(error)
                }),
        ),
        Err(_) => {
            failure("本地配置或凭据未通过安全校验。未启动、覆盖或替换 runtime。");
            return;
        }
    };
    let state = Arc::new(Shell {
        bridge,
        data_dir,
        package_root: layout.package_root,
        selection: Mutex::new(None),
        pair_selection: Mutex::new(None),
        file_selection: Mutex::new(None),
        directory_selection: Mutex::new(None),
        picking: AtomicBool::new(false),
        closing: AtomicBool::new(false),
        closed: AtomicBool::new(false),
        window_close_pending: AtomicBool::new(false),
        exit_generation: AtomicU64::new(0),
        close_request: CloseRequestGate::default(),
    });
    // One shell-owned reaper, rather than one sleeping task per pick. It never
    // holds the Shell alive or delays runtime shutdown for the lease lifetime.
    let weak = Arc::downgrade(&state);
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
            let Some(state) = weak.upgrade() else {
                break;
            };
            if let Ok(mut slot) = state.file_selection.lock() {
                if state.closing.load(Ordering::Acquire) {
                    *slot = None;
                } else {
                    FileSelection::expire(&mut slot);
                }
            }
            if let Ok(mut slot) = state.pair_selection.lock() {
                if state.closing.load(Ordering::Acquire) {
                    *slot = None;
                } else {
                    FileSelection::expire(&mut slot);
                }
            }
            if state.closed.load(Ordering::Acquire) {
                break;
            }
        }
    });
    let app = tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            desktop_snapshot,
            runtime_initialize,
            configuration_get,
            performance_get,
            ocr_history_list,
            ocr_history_get,
            ocr_history_save,
            ocr_history_delete,
            workbench_get,
            workbench_save,
            autostart_get,
            autostart_set,
            configuration_model_get,
            configuration_save,
            configuration_migrate,
            model_load_profile,
            model_load_start,
            model_load_profile_start,
            model_load_next,
            model_load_cancel,
            ui_preferences_get,
            ui_preferences_save,
            runtime_start,
            model_pick,
            models_pick,
            models_add,
            models_selection_discard,
            model_import,
            model_directory_pick,
            model_directory_apply,
            model_directory_configure,
            model_directory_discover,
            model_catalog,
            model_download_start,
            model_download_next,
            model_download_cancel,
            models_scan,
            models_reconcile,
            model_test,
            model_library_next,
            model_library_cancel,
            models_page,
            model_load,
            model_unload,
            model_unregister,
            models_pair_pick,
            models_pair_import,
            ocr_start,
            ocr_save_markdown,
            chat_start,
            chat_next,
            chat_cancel,
            settings_save,
            runtime_idle_save,
            runtime_verification_save,
            runtime_lan_save,
            runtime_lan_addresses,
            lan_token_copy,
            token_copy,
            runtime_stop,
            desktop_close_ack,
            desktop_close
        ])
        .setup(|app| {
            tauri::WebviewWindowBuilder::from_config(app, &app.config().app.windows[0])?
                .on_navigation(local_url)
                .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
                .build()?;
            if create_tray(app.handle()).is_err() {
                rfd::MessageDialog::new()
                    .set_title("Nexa：托盘不可用")
                    .set_description("托盘创建失败，窗口仍可使用。启用关闭到托盘时会保留窗口，避免失去操作入口。")
                    .set_level(MessageLevel::Warning).show();
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let app = window.app_handle().clone();
                let state = Arc::clone(&app.state::<Arc<Shell>>());
                // Capture at receipt, not when the async task first gets CPU:
                // a later tray restore must invalidate even an unstarted close.
                let exit_generation = state.exit_generation.load(Ordering::Acquire);
                tauri::async_runtime::spawn(request_window_close(app, state, exit_generation));
            }
        })
        .build(tauri::generate_context!());
    match app {
        Ok(app) => app.run(|app, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                let state = Arc::clone(&app.state::<Arc<Shell>>());
                if !state.closed.load(Ordering::Acquire) {
                    api.prevent_exit();
                    tauri::async_runtime::spawn(request_close(app.clone(), state));
                }
            }
        }),
        Err(_) => failure(
            "原生窗口创建失败。请确认 WebView2 Evergreen 可正常运行；本次未自动安装组件或更改系统权限。",
        ),
    }
}
