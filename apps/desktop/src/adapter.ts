import { invoke, isTauri } from "@tauri-apps/api/core";
import type { DesktopApi, SafeError } from "./types";
import { presentError } from "./errorPresentation";
import { preferencesOnly } from "./runtimeSettingsValues";

export class DesktopError extends Error {
  readonly code: string;
  constructor(code: string, message: string) {
    super(message);
    this.code = code;
  }
}
export function safeError(error: unknown): SafeError {
  return presentError(error, error instanceof DesktopError);
}
async function call<T>(command: string, request?: unknown): Promise<T> {
  if (!isTauri())
    throw new DesktopError(
      "desktop_required",
      "当前页面未连接原生桌面。请在 Nexa 桌面应用中打开；浏览器开发预览须显式启用。",
    );
  try {
    return await invoke<T>(
      command,
      request === undefined ? undefined : { request },
    );
  } catch (error) {
    const safe = safeError(error);
    throw new DesktopError(safe.code, safe.message);
  }
}
export const nativeApi: DesktopApi = {
  closeAcknowledge: (id) => call("desktop_close_ack", { id }),
  workbenchGet: () => call("workbench_get"),
  autostartGet: () => call("autostart_get"),
  autostartSet: (enabled) => call("autostart_set", { enabled }),
  workbenchSave: (request) => call("workbench_save", request),
  ocrHistoryList: () => call("ocr_history_list"),
  ocrHistoryGet: (id) => call("ocr_history_get", { id }),
  ocrHistorySave: (request) => call("ocr_history_save", request),
  ocrHistoryDelete: (id) => call("ocr_history_delete", { id }),
  performanceGet: () => call("performance_get"),
  initialize: () => call("runtime_initialize"),
  configurationGet: () => call("configuration_get"),
  configurationModelGet: (model_id) => call("configuration_model_get", { model_id }),
  configurationSave: (request) => call("configuration_save", request),
  configurationMigrate: (request) => call("configuration_migrate", request),
  loadModelStart: (operation_id, model_id, options) => call("model_load_start", { operation_id, model_id, ...options }),
  loadModelProfileStart: (operation_id, model_id, load_overrides) => call("model_load_profile_start", { operation_id, model_id, ...(load_overrides ? { load_overrides } : {}) }),
  modelLoadNext: (operation_id) => call("model_load_next", { operation_id }),
  modelLoadCancel: (operation_id) => call("model_load_cancel", { operation_id }),
  loadModelProfile: (model_id, load_overrides) => call("model_load_profile", { model_id, ...(load_overrides ? { load_overrides } : {}) }),
  uiPreferencesGet: () => call("ui_preferences_get"),
  uiPreferencesSave: (request) => call("ui_preferences_save", request),
  catalog: () => call("model_catalog"),
  discoverDirectory: () => call("model_directory_discover"),
  downloadStart: (catalog_id, auto_test) => call("model_download_start", { catalog_id, ...(auto_test === undefined ? {} : { auto_test }) }),
  downloadNext: (operation_id) => call("model_download_next", { operation_id }),
  downloadCancel: (operation_id) => call("model_download_cancel", { operation_id }),
  snapshot: () => call("desktop_snapshot"),
  start: (initialize_if_missing) =>
    call("runtime_start", { initialize_if_missing }),
  pickDirectory: () => call("model_directory_pick"),
  pickModels: () => call("models_pick"),
  discardModelSelection: (selection_id) => call("models_selection_discard", { selection_id }),
  addModels: (selection_id, auto_test) => call("models_add", { selection_id, ...(auto_test === undefined ? {} : { auto_test }) }),
  configureDirectory: (selection_id) => call("model_directory_configure", { selection_id }),
  applyDirectory: (selection_id) =>
    call("model_directory_apply", { selection_id }),
  scanModels: () => call("models_scan"),
  reconcileModels: () => call("models_reconcile"),
  libraryNext: (operation_id) => call("model_library_next", { operation_id }),
  libraryCancel: (operation_id) =>
    call("model_library_cancel", { operation_id }),
  modelsPage: (after, generation) => call("models_page", { after, generation }),
  unregisterModel: (model_id, generation) => call("model_unregister", { model_id, generation }),
  loadModel: (model_id, options) =>
    call("model_load", { model_id, ...options }),
  testModel: (model_id, options) =>
    call("model_test", { model_id, ...options }),
  unloadModel: () => call("model_unload"),
  ocrStart: (request) => call("ocr_start", request),
  saveOcrMarkdown: (text) => call("ocr_save_markdown", { text }),
  pickModelPair: () => call("models_pair_pick"),
  importModelPair: (selection_id, model_id) => call("models_pair_import", { selection_id, model_id }),
  chatStart: (request) => call("chat_start", request),
  chatNext: (request_id) => call("chat_next", { request_id }),
  chatCancel: (request_id) => call("chat_cancel", { request_id }),
  saveSettings: (settings) => call("settings_save", { settings: preferencesOnly(settings) }),
  saveIdle: (idle_unload_seconds, idle_unload_enabled) =>
    call("runtime_idle_save", { idle_unload_seconds, ...(idle_unload_enabled === undefined ? {} : { idle_unload_enabled }) }),
  saveVerificationTimeout: (model_verification_timeout_seconds) =>
    call("runtime_verification_save", { model_verification_timeout_seconds }),
  copyToken: () => call("token_copy"),
  lanAddresses: () => call("runtime_lan_addresses"),
  saveLanSettings: (lan_api) => call("runtime_lan_save", { lan_api }),
  copyLanToken: () => call("lan_token_copy"),
  stop: () => call("runtime_stop"),
  close: () => call("desktop_close"),
};
