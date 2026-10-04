import { invoke, isTauri } from "@tauri-apps/api/core";
import type { DesktopApi, SafeError } from "./types";

export class DesktopError extends Error {
  readonly code: string;
  constructor(code: string, message: string) {
    super(message);
    this.code = code;
  }
}
export function safeError(error: unknown): SafeError {
  if (
    error &&
    typeof error === "object" &&
    "code" in error &&
    "message" in error &&
    typeof error.code === "string" &&
    /^[a-z0-9_]{1,80}$/.test(error.code) &&
    typeof error.message === "string"
  ) {
    return { code: error.code, message: error.message.slice(0, 500) };
  }
  return {
    code: "desktop_unavailable",
    message: "无法完成桌面操作，请检查运行服务后重试。未自动重放请求。",
  };
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
  catalog: () => call("model_catalog"),
  discoverDirectory: () => call("model_directory_discover"),
  downloadStart: (catalog_id, auto_test) => call("model_download_start", { catalog_id, ...(auto_test === undefined ? {} : { auto_test }) }),
  downloadNext: (operation_id) => call("model_download_next", { operation_id }),
  downloadCancel: (operation_id) => call("model_download_cancel", { operation_id }),
  snapshot: () => call("desktop_snapshot"),
  start: (initialize_if_missing) =>
    call("runtime_start", { initialize_if_missing }),
  pickDirectory: () => call("model_directory_pick"),
  applyDirectory: (selection_id) =>
    call("model_directory_apply", { selection_id }),
  scanModels: () => call("models_scan"),
  reconcileModels: () => call("models_reconcile"),
  libraryNext: (operation_id) => call("model_library_next", { operation_id }),
  libraryCancel: (operation_id) =>
    call("model_library_cancel", { operation_id }),
  modelsPage: (after, generation) => call("models_page", { after, generation }),
  loadModel: (model_id, options) =>
    call("model_load", { model_id, ...options }),
  testModel: (model_id, options) =>
    call("model_test", { model_id, ...options }),
  unloadModel: () => call("model_unload"),
  chatStart: (request) => call("chat_start", request),
  chatNext: (request_id) => call("chat_next", { request_id }),
  chatCancel: (request_id) => call("chat_cancel", { request_id }),
  saveSettings: (settings) => call("settings_save", { settings }),
  saveIdle: (idle_unload_seconds) =>
    call("runtime_idle_save", { idle_unload_seconds }),
  copyToken: () => call("token_copy"),
  stop: () => call("runtime_stop"),
  close: () => call("desktop_close"),
};
