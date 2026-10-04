use crate::{
    diagnostics,
    directory_selection::{AdmissionError, DirectorySelection, PickedDirectory},
    layout,
    selection::{PickedModel, Selection},
};
use desktop_bridge::{BridgeError, DesktopBridge, dto::*};
use rfd::{MessageButtons, MessageDialogResult, MessageLevel};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tauri::{Manager, State, WebviewWindow};
use uuid::Uuid;

type Result<T> = std::result::Result<T, BridgeError>;
struct Shell {
    bridge: Arc<DesktopBridge>,
    data_dir: PathBuf,
    package_root: PathBuf,
    selection: Mutex<Option<Selection>>,
    directory_selection: Mutex<Option<DirectorySelection>>,
    picking: AtomicBool,
    closing: AtomicBool,
    closed: AtomicBool,
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
            "clipboard_unavailable" => "无法写入系统剪贴板，请稍后重试。",
            "token_unavailable" => "令牌文件未初始化或安全校验失败。",
            "selection_expired" => "所选项目已过期，请重新选择。",
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
        .set_title("选择现有 GGUF 目录（扫描不移动，下载会保存到此目录）")
        .pick_folder()
        .await;
    guard(&window, &state)?;
    let Some(picked) = picked else {
        return Ok(None);
    };
    desktop_bridge::validate_model_directory_path(picked.path())?;
    let (selection, dto) =
        DirectorySelection::new(picked.path(), &state.package_root).map_err(error)?;
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
    state.bridge.save_idle(request.idle_unload_seconds).await
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
async fn desktop_close(
    window: WebviewWindow,
    state: State<'_, Arc<Shell>>,
    app: tauri::AppHandle,
) -> Result<()> {
    // Repeated closes merge; the native close button and this command share this
    // exact state machine. No shell-side Child handle or kill-on-drop is used.
    if window.label() != "main" || !window.url().is_ok_and(|url| local_url(&url)) {
        return Err(error("unauthorized_window"));
    }
    close(app, Arc::clone(&state)).await;
    Ok(())
}
async fn close(app: tauri::AppHandle, state: Arc<Shell>) {
    if state.closing.swap(true, Ordering::AcqRel) {
        return;
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
        directory_selection: Mutex::new(None),
        picking: AtomicBool::new(false),
        closing: AtomicBool::new(false),
        closed: AtomicBool::new(false),
    });
    let app = tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            desktop_snapshot,
            runtime_start,
            model_pick,
            model_import,
            model_directory_pick,
            model_directory_apply,
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
            chat_start,
            chat_next,
            chat_cancel,
            settings_save,
            runtime_idle_save,
            runtime_lan_save,
            lan_token_copy,
            token_copy,
            runtime_stop,
            desktop_close
        ])
        .setup(|app| {
            tauri::WebviewWindowBuilder::from_config(app, &app.config().app.windows[0])?
                .on_navigation(local_url)
                .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
                .build()?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let app = window.app_handle().clone();
                let state = Arc::clone(&app.state::<Arc<Shell>>());
                tauri::async_runtime::spawn(close(app, state));
            }
        })
        .build(tauri::generate_context!());
    match app {
        Ok(app) => app.run(|app, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                let state = Arc::clone(&app.state::<Arc<Shell>>());
                if !state.closed.load(Ordering::Acquire) {
                    api.prevent_exit();
                    tauri::async_runtime::spawn(close(app.clone(), state));
                }
            }
        }),
        Err(_) => failure(
            "原生窗口创建失败。请确认 WebView2 Evergreen 可正常运行；本次未自动安装组件或更改系统权限。",
        ),
    }
}
