import { beforeEach, describe, expect, it, vi } from "vitest";
const { invoke, isTauri } = vi.hoisted(() => ({
  invoke: vi.fn(),
  isTauri: vi.fn(() => true),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke, isTauri }));
import { nativeApi, safeError } from "../src/adapter";
import { DEFAULT_SETTINGS } from "../src/controller";
beforeEach(() => {
  isTauri.mockReturnValue(true);
  invoke.mockResolvedValue({});
});
describe("native-only adapter", () => {
  it("uses frozen snake_case requests and fixed commands only", async () => {
    await nativeApi.catalog();
    expect(invoke).toHaveBeenLastCalledWith("model_catalog", undefined);
    await nativeApi.discoverDirectory();
    expect(invoke).toHaveBeenLastCalledWith("model_directory_discover", undefined);
    await nativeApi.downloadStart("fixed-candidate");
    expect(invoke).toHaveBeenLastCalledWith("model_download_start", { request: { catalog_id: "fixed-candidate" } });
    await nativeApi.downloadStart("fixed-candidate", true);
    expect(invoke).toHaveBeenLastCalledWith("model_download_start", { request: { catalog_id: "fixed-candidate", auto_test: true } });
    await nativeApi.downloadNext("download-id");
    expect(invoke).toHaveBeenLastCalledWith("model_download_next", { request: { operation_id: "download-id" } });
    await nativeApi.downloadCancel("download-id");
    expect(invoke).toHaveBeenLastCalledWith("model_download_cancel", { request: { operation_id: "download-id" } });
    await nativeApi.start(true);
    expect(invoke).toHaveBeenLastCalledWith("runtime_start", {
      request: { initialize_if_missing: true },
    });
    await nativeApi.chatCancel("id");
    expect(invoke).toHaveBeenLastCalledWith("chat_cancel", {
      request: { request_id: "id" },
    });
    const { idle_unload_seconds, ...settings } = DEFAULT_SETTINGS;
    await nativeApi.saveSettings(settings);
    expect(invoke).toHaveBeenLastCalledWith("settings_save", {
      request: { settings },
    });
    await nativeApi.saveIdle(idle_unload_seconds);
    expect(invoke).toHaveBeenLastCalledWith("runtime_idle_save", {
      request: { idle_unload_seconds: 300 },
    });
    const lan_api = { enabled: true, listen: "192.168.1.20:18081", allowed_cidrs: ["192.168.1.30/32"] };
    await nativeApi.saveLanSettings(lan_api);
    expect(invoke).toHaveBeenLastCalledWith("runtime_lan_save", { request: { lan_api } });
    await nativeApi.copyLanToken();
    expect(invoke).toHaveBeenLastCalledWith("lan_token_copy", undefined);
    await nativeApi.copyToken();
    expect(invoke).toHaveBeenLastCalledWith("token_copy", undefined);
    await nativeApi.snapshot();
    expect(invoke).toHaveBeenLastCalledWith("desktop_snapshot", undefined);
    await nativeApi.pickDirectory();
    expect(invoke).toHaveBeenLastCalledWith("model_directory_pick", undefined);
    await nativeApi.configureDirectory("selection-only");
    expect(invoke).toHaveBeenLastCalledWith("model_directory_configure", { request: { selection_id: "selection-only" } });
    await nativeApi.applyDirectory("selection-only");
    expect(invoke).toHaveBeenLastCalledWith("model_directory_apply", {
      request: { selection_id: "selection-only" },
    });
    await nativeApi.pickModels();
    expect(invoke).toHaveBeenLastCalledWith("models_pick", undefined);
    await nativeApi.addModels("opaque-selection", false);
    expect(invoke).toHaveBeenLastCalledWith("models_add", { request: { selection_id: "opaque-selection", auto_test: false } });
    await nativeApi.discardModelSelection("opaque-selection");
    expect(invoke).toHaveBeenLastCalledWith("models_selection_discard", { request: { selection_id: "opaque-selection" } });
    await nativeApi.scanModels();
    expect(invoke).toHaveBeenLastCalledWith("models_scan", undefined);
    await nativeApi.reconcileModels();
    expect(invoke).toHaveBeenLastCalledWith("models_reconcile", undefined);
    await nativeApi.testModel("registered-model", { context_size: 2048, threads: 2, batch_size: 128 });
    expect(invoke).toHaveBeenLastCalledWith("model_test", { request: { model_id: "registered-model", context_size: 2048, threads: 2, batch_size: 128 } });
    await nativeApi.libraryNext("operation-only");
    expect(invoke).toHaveBeenLastCalledWith("model_library_next", {
      request: { operation_id: "operation-only" },
    });
    await nativeApi.libraryCancel("operation-only");
    expect(invoke).toHaveBeenLastCalledWith("model_library_cancel", {
      request: { operation_id: "operation-only" },
    });
    await nativeApi.modelsPage(null, null);
    expect(invoke).toHaveBeenLastCalledWith("models_page", {
      request: { after: null, generation: null },
    });
    await nativeApi.modelsPage("cursor", "version");
    expect(invoke).toHaveBeenLastCalledWith("models_page", {
      request: { after: "cursor", generation: "version" },
    });
  });
  it("never falls back to mock when native desktop is missing", async () => {
    isTauri.mockReturnValue(false);
    await expect(nativeApi.snapshot()).rejects.toMatchObject({
      code: "desktop_required",
    });
    expect(invoke).not.toHaveBeenCalled();
  });
  it("never includes unknown raw errors or credential-like strings in visible diagnostics", () => {
    expect(safeError(new Error("Bearer secret /private/user/path"))).toEqual({
      code: "desktop_unavailable",
      message: expect.not.stringContaining("secret"),
    });
    expect(safeError("raw sensitive failure").message).not.toContain(
      "sensitive",
    );
  });
});
