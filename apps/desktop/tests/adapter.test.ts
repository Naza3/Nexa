import { beforeEach, describe, expect, it, vi } from "vitest";
const { invoke, isTauri } = vi.hoisted(() => ({
  invoke: vi.fn(),
  isTauri: vi.fn(() => true),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke, isTauri }));
import { nativeApi, safeError } from "../src/adapter";
import { DEFAULT_SETTINGS } from "../src/controller";
import { preferencesOnly } from "../src/runtimeSettingsValues";
beforeEach(() => {
  isTauri.mockReturnValue(true);
  invoke.mockResolvedValue({});
});
describe("native-only adapter", () => {
  it("uses frozen snake_case requests and fixed commands only", async () => {
    await nativeApi.performanceGet?.();
    expect(invoke).toHaveBeenLastCalledWith("performance_get", undefined);
    await nativeApi.closeAcknowledge?.("close-id");
    expect(invoke).toHaveBeenLastCalledWith("desktop_close_ack", { request: { id: "close-id" } });
    await nativeApi.workbenchGet?.();
    expect(invoke).toHaveBeenLastCalledWith("workbench_get", undefined);
    await nativeApi.ocrHistoryList?.();
    expect(invoke).toHaveBeenLastCalledWith("ocr_history_list", undefined);
    await nativeApi.ocrHistoryGet?.("record-id");
    expect(invoke).toHaveBeenLastCalledWith("ocr_history_get", { request: { id: "record-id" } });
    await nativeApi.ocrHistoryDelete?.("record-id");
    expect(invoke).toHaveBeenLastCalledWith("ocr_history_delete", { request: { id: "record-id" } });
    const historyRequest = { mode: "create" as const, id: "record-id", image_name: "image.png", model_id: "model", status: "completed" as const, finish_reason: "stop" as const, error_code: null, incomplete: false, markdown: "# output", performance: null };
    await nativeApi.ocrHistorySave?.(historyRequest);
    expect(invoke).toHaveBeenLastCalledWith("ocr_history_save", { request: historyRequest });
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
    const settings = preferencesOnly(DEFAULT_SETTINGS);
    await nativeApi.saveSettings(DEFAULT_SETTINGS);
    expect(invoke).toHaveBeenLastCalledWith("settings_save", {
      request: { settings },
    });
    await nativeApi.saveIdle(DEFAULT_SETTINGS.idle_unload_seconds);
    expect(invoke).toHaveBeenLastCalledWith("runtime_idle_save", {
      request: { idle_unload_seconds: 300 },
    });
    await nativeApi.saveIdle(900, false);
    expect(invoke).toHaveBeenLastCalledWith("runtime_idle_save", { request: { idle_unload_seconds: 900, idle_unload_enabled: false } });
    await nativeApi.saveVerificationTimeout(7200);
    expect(invoke).toHaveBeenLastCalledWith("runtime_verification_save", { request: { model_verification_timeout_seconds: 7200 } });
    const lan_api = { enabled: true, listen: "192.168.1.20:18081", allowed_cidrs: ["192.168.1.30/32"] };
    await nativeApi.lanAddresses();
    expect(invoke).toHaveBeenLastCalledWith("runtime_lan_addresses", undefined);
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
    await nativeApi.loadModelStart!("owned-load", "registered-model", { context_size: 2048, threads: 2, batch_size: 128 });
    expect(invoke).toHaveBeenLastCalledWith("model_load_start", { request: { operation_id: "owned-load", model_id: "registered-model", context_size: 2048, threads: 2, batch_size: 128 } });
    await nativeApi.loadModelProfileStart!("owned-load", "registered-model");
    expect(invoke).toHaveBeenLastCalledWith("model_load_profile_start", { request: { operation_id: "owned-load", model_id: "registered-model" } });
    await nativeApi.loadModelProfileStart!("owned-load", "registered-model", { threads: 3 });
    expect(invoke).toHaveBeenLastCalledWith("model_load_profile_start", { request: { operation_id: "owned-load", model_id: "registered-model", load_overrides: { threads: 3 } } });
    await nativeApi.modelLoadNext!("owned-load");
    expect(invoke).toHaveBeenLastCalledWith("model_load_next", { request: { operation_id: "owned-load" } });
    await nativeApi.modelLoadCancel!("owned-load");
    expect(invoke).toHaveBeenLastCalledWith("model_load_cancel", { request: { operation_id: "owned-load" } });
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
    await nativeApi.unregisterModel("registered-model", "generation");
    expect(invoke).toHaveBeenLastCalledWith("model_unregister", { request: { model_id: "registered-model", generation: "generation" } });
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

describe("canonical native configuration commands", () => {
  it("uses the eight frozen commands without adding inference fields to UI preferences or ordinary load", async () => {
    const revision = `sha256:${"a".repeat(64)}`;
    await nativeApi.initialize!(); expect(invoke).toHaveBeenLastCalledWith("runtime_initialize", undefined);
    await nativeApi.configurationGet!(); expect(invoke).toHaveBeenLastCalledWith("configuration_get", undefined);
    await nativeApi.configurationModelGet!("model-a"); expect(invoke).toHaveBeenLastCalledWith("configuration_model_get", { request: { model_id: "model-a" } });
    const update = { expected_revision: revision, update: { kind: "model_profile" as const, model_id: "model-a", load_overrides: { context_size: 2048, threads: null, batch_size: null } } };
    await nativeApi.configurationSave!(update); expect(invoke).toHaveBeenLastCalledWith("configuration_save", { request: update });
    const migration = { expected_revision: revision, expected_preferences_revision: null, choice: "api" as const, custom: null };
    await nativeApi.configurationMigrate!(migration); expect(invoke).toHaveBeenLastCalledWith("configuration_migrate", { request: migration });
    await nativeApi.loadModelProfile!("model-a"); expect(invoke).toHaveBeenLastCalledWith("model_load_profile", { request: { model_id: "model-a" } });
    await nativeApi.loadModelProfile!("model-a", { threads: 2 }); expect(invoke).toHaveBeenLastCalledWith("model_load_profile", { request: { model_id: "model-a", load_overrides: { threads: 2 } } });
    await nativeApi.uiPreferencesGet!(); expect(invoke).toHaveBeenLastCalledWith("ui_preferences_get", undefined);
    const ui = { expected_revision: revision, preferences: { download_source: "modelscope" as const, close_runtime_on_exit: false } };
    await nativeApi.uiPreferencesSave!(ui); expect(invoke).toHaveBeenLastCalledWith("ui_preferences_save", { request: ui });
  });
});
