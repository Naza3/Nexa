import { describe, expect, it, vi } from "vitest";
import { DesktopController, DEFAULT_SETTINGS } from "../src/controller";
import { createPreviewApi } from "../src/preview";
import { preferencesOnly, validateIdleSeconds, validateVerificationSeconds } from "../src/runtimeSettingsValues";
import type { DesktopApi, Snapshot } from "../src/types";
import { deferred, makeApi, snapshot } from "./fixtures";

const stopped = (): Snapshot => ({ ...snapshot(), connection: "stopped", runtime: null });
async function setup(current = stopped(), overrides: Partial<DesktopApi> = {}) {
  const api = makeApi({ snapshot: vi.fn(async () => current), ...overrides });
  const controller = new DesktopController(api);
  await controller.refresh();
  return { api, controller };
}

describe("runtime setting contracts", () => {
  it.each([1, 300, 86400])("accepts idle seconds boundary %s", (seconds) => expect(validateIdleSeconds(seconds)).toBeNull());
  it.each([30, 300, 7200])("accepts verification seconds boundary %s", (seconds) => expect(validateVerificationSeconds(seconds)).toBeNull());
  it.each([0, -1, 86401, 1.5, NaN, Infinity])("rejects invalid idle seconds %s", (seconds) => expect(validateIdleSeconds(seconds)).toMatch(/1–86400/));
  it.each([0, -1, 29, 7201, 30.5, NaN, Infinity])("rejects invalid verification seconds %s", (seconds) => expect(validateVerificationSeconds(seconds)).toMatch(/30–7200/));
  it("defaults to enabled and 300 seconds and whitelists preference fields", () => {
    expect(DEFAULT_SETTINGS).toMatchObject({ idle_unload_enabled: true, idle_unload_seconds: 300, model_verification_timeout_seconds: 300 });
    const preferences = preferencesOnly(DEFAULT_SETTINGS);
    expect(preferences).not.toHaveProperty("idle_unload_enabled");
    expect(preferences).not.toHaveProperty("idle_unload_seconds");
    expect(preferences).not.toHaveProperty("model_verification_timeout_seconds");
  });
  it("preserves disabled and timeout when only preferences are saved", async () => {
    const current = stopped();
    current.settings = { ...current.settings, idle_unload_enabled: false, model_verification_timeout_seconds: 900 };
    const { api, controller } = await setup(current, { saveSettings: vi.fn(async () => current) });
    await controller.saveSettings(current.settings);
    expect(api.saveSettings).toHaveBeenCalledExactlyOnceWith(preferencesOnly(current.settings));
    expect(controller.getSnapshot().snapshot?.settings).toMatchObject({ idle_unload_enabled: false, model_verification_timeout_seconds: 900 });
    expect(api.saveIdle).not.toHaveBeenCalled(); expect(api.saveVerificationTimeout).not.toHaveBeenCalled();
  });
  it.each(["idle", "verification"] as const)("rejects %s saves while running without stopping service", async (kind) => {
    const { controller, api } = await setup(snapshot());
    if (kind === "idle") await controller.saveIdle(600, false); else await controller.saveVerificationTimeout(600);
    expect(controller.getSnapshot().error?.code).toBe("runtime_running");
    expect(api.saveIdle).not.toHaveBeenCalled(); expect(api.saveVerificationTimeout).not.toHaveBeenCalled();
    expect(api.stop).not.toHaveBeenCalled(); expect(api.start).not.toHaveBeenCalled();
  });
  it("does not initialize for either setting", async () => {
    const { controller, api } = await setup({ ...stopped(), initialized: false });
    await controller.saveIdle(600, false); expect(controller.getSnapshot().error?.code).toBe("not_initialized");
    await controller.saveVerificationTimeout(600); expect(controller.getSnapshot().error?.code).toBe("not_initialized");
    expect(api.saveIdle).not.toHaveBeenCalled(); expect(api.saveVerificationTimeout).not.toHaveBeenCalled(); expect(api.start).not.toHaveBeenCalled();
  });
  it("allows legacy seconds-only save but refuses new capabilities on an old DTO", async () => {
    const old = stopped(); delete old.settings.idle_unload_enabled; delete old.settings.model_verification_timeout_seconds;
    const { api, controller } = await setup(old, { saveIdle: vi.fn(async () => old) });
    await controller.saveIdle(600);
    expect(api.saveIdle).toHaveBeenCalledExactlyOnceWith(600);
    await controller.saveIdle(600, false);
    expect(controller.getSnapshot().error?.code).toBe("idle_settings_unavailable");
    await controller.saveVerificationTimeout(600);
    expect(controller.getSnapshot().error?.code).toBe("verification_settings_unavailable");
    expect(api.saveIdle).toHaveBeenCalledTimes(1); expect(api.saveVerificationTimeout).not.toHaveBeenCalled();
  });
  it.each([29, 7201, NaN, 300.5])("validates %s before sending verification", async (value) => {
    const { api, controller } = await setup(); await controller.saveVerificationTimeout(value);
    expect(controller.getSnapshot().error?.code).toBe("invalid_settings"); expect(api.saveVerificationTimeout).not.toHaveBeenCalled();
  });
  it.each([0, 86401, NaN, 300.5])("validates %s even with idle disabled", async (value) => {
    const { api, controller } = await setup(); await controller.saveIdle(value, false);
    expect(controller.getSnapshot().error?.code).toBe("invalid_settings"); expect(api.saveIdle).not.toHaveBeenCalled();
  });
  it("disables idle with a retained interval and independent timeout", async () => {
    const saved = stopped(); saved.settings = { ...saved.settings, idle_unload_enabled: false, idle_unload_seconds: 900, model_verification_timeout_seconds: 1200 };
    const { api, controller } = await setup(stopped(), { saveIdle: vi.fn(async () => saved) });
    await controller.saveIdle(900, false);
    expect(api.saveIdle).toHaveBeenCalledExactlyOnceWith(900, false);
    expect(controller.getSnapshot().snapshot?.settings).toEqual(saved.settings);
    expect(controller.getSnapshot().notice).toMatch(/下次显式启动/);
    expect(api.saveVerificationTimeout).not.toHaveBeenCalled(); expect(api.start).not.toHaveBeenCalled();
  });
  it("saves timeout independently without changing disabled idle or LAN", async () => {
    const saved = stopped(); saved.settings = { ...saved.settings, idle_unload_enabled: false, model_verification_timeout_seconds: 7200 };
    saved.lan_api = { enabled: true, listen: "192.168.1.20:18081", allowed_cidrs: ["192.168.1.30/32"] };
    const { api, controller } = await setup(stopped(), { saveVerificationTimeout: vi.fn(async () => saved) });
    await controller.saveVerificationTimeout(7200);
    expect(api.saveVerificationTimeout).toHaveBeenCalledExactlyOnceWith(7200);
    expect(controller.getSnapshot().snapshot).toEqual(saved);
    expect(api.saveIdle).not.toHaveBeenCalled(); expect(api.saveLanSettings).not.toHaveBeenCalled();
    expect(controller.getSnapshot().notice).toMatch(/后续新校验/);
  });
  it("deduplicates and exposes conflicting saves, blocks pick and close until resolution", async () => {
    const pending = deferred<Snapshot>();
    const { api, controller } = await setup(stopped(), { saveVerificationTimeout: vi.fn(() => pending.promise) });
    const saving = controller.saveVerificationTimeout(1200);
    await controller.saveVerificationTimeout(1200); await controller.saveIdle(900, false);
    await controller.saveLanSettings({ enabled: false, listen: null, allowed_cidrs: [] });
    await controller.pickModels(); await controller.close();
    expect(controller.getSnapshot().error?.code).toBe("operation_in_progress");
    expect(api.saveVerificationTimeout).toHaveBeenCalledTimes(1); expect(api.saveIdle).not.toHaveBeenCalled();
    expect(api.saveLanSettings).not.toHaveBeenCalled(); expect(api.pickModels).not.toHaveBeenCalled(); expect(api.close).not.toHaveBeenCalled();
    const saved = stopped(); saved.settings.model_verification_timeout_seconds = 1200;
    pending.resolve(saved); await saving;
    expect(controller.getSnapshot().snapshot?.settings.model_verification_timeout_seconds).toBe(1200);
    expect(controller.getSnapshot().error).toBeNull(); expect(controller.getSnapshot().operation).toBeNull();
    expect(controller.getSnapshot().notice).toMatch(/模型文件校验超时已保存/);
  });
  it("rejects settings during a pending LAN save without mixing payloads", async () => {
    const pending = deferred<Snapshot>();
    const { api, controller } = await setup(stopped(), { saveLanSettings: vi.fn(() => pending.promise) });
    const lan = { enabled: false, listen: null, allowed_cidrs: [] };
    const saving = controller.saveLanSettings(lan);
    await controller.saveVerificationTimeout(1200); await controller.saveIdle(900, false);
    expect(controller.getSnapshot().error?.code).toBe("operation_in_progress");
    expect(api.saveLanSettings).toHaveBeenCalledExactlyOnceWith(lan);
    expect(api.saveVerificationTimeout).not.toHaveBeenCalled(); expect(api.saveIdle).not.toHaveBeenCalled();
    pending.resolve(stopped()); await saving;
    expect(controller.getSnapshot().error).toBeNull(); expect(controller.getSnapshot().notice).toMatch(/局域网 API 已配置为关闭/);
  });
  it("rejects settings during a pending model picker and allows cancellation normally", async () => {
    const picker = deferred<null>();
    const { api, controller } = await setup(stopped(), { pickModels: vi.fn(() => picker.promise) });
    const selecting = controller.pickModels();
    await controller.saveVerificationTimeout(600); await controller.saveIdle(600, false);
    expect(controller.getSnapshot().error?.code).toBe("operation_in_progress");
    expect(api.saveIdle).not.toHaveBeenCalled(); expect(api.saveVerificationTimeout).not.toHaveBeenCalled();
    picker.resolve(null); await selecting; expect(controller.getSnapshot().model_selection).toBeNull();
  });
  it.each(["success", "failure"] as const)("ignores late %s after UI unmount and reads persisted state on remount", async (outcome) => {
    const pending = deferred<Snapshot>();
    const { api, controller } = await setup(stopped(), { saveIdle: vi.fn(() => pending.promise) });
    const unmount = controller.mount();
    const saving = controller.saveIdle(900, false); unmount();
    const saved = stopped(); saved.settings = { ...saved.settings, idle_unload_seconds: 900, idle_unload_enabled: false };
    if (outcome === "success") pending.resolve(saved); else pending.reject({ code: "settings_save_failed", message: "保存失败" });
    await saving;
    expect(controller.getSnapshot().snapshot?.settings.idle_unload_enabled).toBe(true);
    expect(controller.getSnapshot().notice).toBeNull(); expect(controller.getSnapshot().error).toBeNull();
    vi.mocked(api.snapshot).mockResolvedValue(saved);
    const remount = controller.mount(); await controller.refresh(); remount();
    expect(controller.getSnapshot().snapshot?.settings.idle_unload_enabled).toBe(false);
  });
  it.each(["runtime_running", "settings_durability_unconfirmed", "settings_save_failed"])("shows %s without false saved success or overwriting the last snapshot", async (code) => {
    const current = stopped();
    const { api, controller } = await setup(current, { saveVerificationTimeout: vi.fn().mockRejectedValue({ code, message: "保存未完成" }) });
    await controller.saveVerificationTimeout(600);
    expect(controller.getSnapshot().error?.code).toBe(code); expect(controller.getSnapshot().notice).toBeNull();
    expect(controller.getSnapshot().snapshot).toEqual(current); expect(api.saveVerificationTimeout).toHaveBeenCalledTimes(1);
    if (code === "settings_durability_unconfirmed") expect(controller.getSnapshot().error?.message).toMatch(/不代表已回滚/);
  });
});

describe("preview settings parity", () => {
  it("retains disabled and unrelated fields across all independent saves", async () => {
    window.history.replaceState({}, "", "?scenario=stopped");
    const api = createPreviewApi();
    window.history.replaceState({}, "", "/");
    await api.saveIdle(900, false);
    await api.saveVerificationTimeout(1200);
    await api.saveSettings({ ...DEFAULT_SETTINGS, threads: 4 });
    const saved = await api.saveIdle(600);
    expect(saved.settings).toMatchObject({ idle_unload_enabled: false, idle_unload_seconds: 600, model_verification_timeout_seconds: 1200, threads: 4 });
    await expect(api.saveIdle(0, false)).rejects.toMatchObject({ code: "invalid_settings" });
    await expect(api.saveVerificationTimeout(7201)).rejects.toMatchObject({ code: "invalid_settings" });
  });
  it("refuses edits in a running or uninitialized preview", async () => {
    window.history.replaceState({}, "", "/");
    let api = createPreviewApi();
    await expect(api.saveIdle(900, false)).rejects.toMatchObject({ code: "runtime_running" });
    await expect(api.saveVerificationTimeout(900)).rejects.toMatchObject({ code: "runtime_running" });
    window.history.replaceState({}, "", "?scenario=initial"); api = createPreviewApi(); window.history.replaceState({}, "", "/");
    await expect(api.saveIdle(900, false)).rejects.toMatchObject({ code: "not_initialized" });
    await expect(api.saveVerificationTimeout(900)).rejects.toMatchObject({ code: "not_initialized" });
  });
});
