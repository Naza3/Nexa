import { openModelDetails, openGroup } from "./navigation";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import App from "../src/App";
import { DesktopController } from "../src/controller";
import { DraftStore, DRAFT_STORAGE_KEY } from "../src/unsavedDrafts";
import { ACTIVITY_STORAGE_KEY, persistActivitySummaries, readActivitySummaries } from "../src/activity";
import { configuredSnapshot, modelConfiguration, revision } from "./configurationFixtures";
import { deferred, makeApi, model } from "./fixtures";
import type { ConfigurationSnapshot, LocalValidation, Snapshot } from "../src/types";

describe("write acknowledgement and deferred recovery ownership", () => {
  it("invalidates a poll begun during a write and performs a distinct post-ack read", async () => {
    const before = configuredSnapshot(true), after = structuredClone(before); after.configuration!.revision = revision("b"); after.configuration!.saved.request_defaults.max_output_tokens = 999;
    const write = deferred<ConfigurationSnapshot>(), poll = deferred<Snapshot>(); const api = makeApi({ snapshot: vi.fn().mockResolvedValueOnce(before).mockReturnValueOnce(poll.promise).mockResolvedValue(after), configurationSave: vi.fn(() => write.promise) }); const controller = new DesktopController(api); await controller.refresh();
    const saving = controller.saveConfiguration({ expected_revision: revision(), update: { kind: "request_defaults", request_defaults: after.configuration!.saved.request_defaults } }); const polling = controller.refresh(); write.resolve(after.configuration!); await Promise.resolve(); await Promise.resolve(); expect(controller.getSnapshot().snapshot?.configuration?.revision).toBe(revision("b")); poll.resolve(before); await polling; expect(await saving).toBe(true); expect(api.snapshot).toHaveBeenCalledTimes(3); expect(controller.getSnapshot().snapshot?.configuration?.revision).toBe(revision("b"));
  });
  it("does not accept a configuration-get begun before an acknowledged save", async () => {
    const before = configuredSnapshot(true), after = structuredClone(before); after.configuration!.revision = revision("b"); const read = deferred<ConfigurationSnapshot>(); const api = makeApi({ snapshot: vi.fn().mockResolvedValueOnce(before).mockResolvedValue(after), configurationGet: vi.fn(() => read.promise), configurationSave: vi.fn(async () => after.configuration!) }); const controller = new DesktopController(api); await controller.refresh(); const pending = controller.refreshConfiguration(); await controller.saveConfiguration({ expected_revision: revision(), update: { kind: "request_defaults", request_defaults: after.configuration!.saved.request_defaults } }); read.resolve(before.configuration!); await pending; expect(controller.getSnapshot().snapshot?.configuration?.revision).toBe(revision("b"));
  });
  it("does not let clean or newer dirty form synchronization erase an unaccepted disk draft", () => {
    const source = { global_defaults: { context_size: 4096, threads: null, batch_size: 512 } }, draft = { global_defaults: { context_size: 8192, threads: null, batch_size: 512 } }; const first = new DraftStore(); first.set("global_defaults", { source, draft, revision: revision(), conflict: false }); const reopened = new DraftStore(); const current = { global_defaults: { context_size: 16384, threads: null, batch_size: 512 } };
    reopened.set("global_defaults", { source: current, draft: current, revision: revision("b"), conflict: false }); expect(new DraftStore().pending.get("global_defaults")?.draft).toEqual(draft);
    reopened.set("global_defaults", { source: current, draft: { global_defaults: { ...current.global_defaults, context_size: 32768 } }, revision: revision("b"), conflict: false }); expect(reopened.warning).toContain("尚未确认"); expect(new DraftStore().pending.get("global_defaults")?.draft).toEqual(draft);
  });
  it("retains the pending owner and disk draft when explicit discard fails", () => {
    const first = new DraftStore(); first.set("verification_policy", { source: 300, draft: 600, revision: revision(), conflict: false }); const reopened = new DraftStore(); vi.spyOn(Storage.prototype, "removeItem").mockImplementation(() => { throw new Error("blocked"); }); reopened.discardPending(); expect(reopened.pending.size).toBe(1); expect(localStorage.getItem(DRAFT_STORAGE_KEY)).toContain("600"); expect(reopened.warning).not.toBeNull();
  });
});

describe("activity and model identity boundaries", () => {
  it("can render and fetch a profile for the backend-valid model ID constructor", async () => {
    const value = configuredSnapshot(true); const api = makeApi({ snapshot: vi.fn(async () => value), modelsPage: vi.fn(async () => ({ data: [{ ...model, id: "constructor" }], generation: "g", next_after: null })), configurationModelGet: vi.fn(async () => ({ ...modelConfiguration(), model_id: "constructor", current_load_options: null })) }); const controller = new DesktopController(api); render(<App initialPage="models" controller={controller} />); await screen.findByRole("heading", { name: model.display_name }); expect(screen.queryByLabelText("本次模型测试")).not.toBeInTheDocument(); await openModelDetails(model.display_name); fireEvent.click(screen.getByText("运行档案与当前参数")); expect(await screen.findByText(/当前驻留参数：无/)).toBeVisible(); expect(api.configurationModelGet).toHaveBeenCalledWith("constructor");
  });
  it("labels a selected unloaded model as history in the auxiliary chat", async () => {
    const value = configuredSnapshot(); value.runtime!.state = "unloaded"; value.runtime!.selected_model_display_name = null; const controller = new DesktopController(makeApi({ snapshot: vi.fn(async () => value) })); render(<App initialPage="chat" controller={controller} />); await waitFor(() => expect(controller.getSnapshot().booting).toBe(false)); expect(screen.getByText(/上次选择：qwen/)).toBeVisible(); expect(screen.queryByText(/已加载模型（名称暂不可用）/)).not.toBeInTheDocument();
  });
  it("preserves unique legacy and modern activity identities through repeated reopenings", () => {
    const old = { id: "model:1", kind: "model" as const, status: "completed" as const, updated_at: 1, detail: "", label: "", error: null }; persistActivitySummaries([old]); const first = readActivitySummaries(); persistActivitySummaries([...first, { ...old, updated_at: 2 }]); const second = readActivitySummaries(); expect(new Set(second.map((value) => value.id)).size).toBe(2); persistActivitySummaries(second); expect(readActivitySummaries().map((value) => value.id)).toEqual(second.map((value) => value.id));
    const one = new DesktopController(makeApi()), two = new DesktopController(makeApi()); expect(one.getSnapshot().activity_session_id).not.toBe(two.getSnapshot().activity_session_id);
  });
  it("strictly rejects extra activity fields and oversized UTF-8 caches", () => {
    const entry = { id: "model:1", kind: "model", status: "completed", updated_at: 1, error_code: null }; for (const extra of ["secret", "汉".repeat(12000)]) { localStorage.setItem(ACTIVITY_STORAGE_KEY, JSON.stringify([{ ...entry, unexpected: extra }])); expect(readActivitySummaries()).toEqual([]); }
  });
  it("clears old unknown summaries and terminal sources without clearing a current native task", async () => {
    persistActivitySummaries([{ id: "model:old", kind: "model", status: "running", updated_at: 1, detail: "", label: "", error: null }]); const pending = deferred<LocalValidation>(); const api = makeApi({ testModel: vi.fn(() => pending.promise) }); const controller = new DesktopController(api); await controller.refresh(); const task = controller.testModel(model.id); controller.clearActivityHistory(); expect(controller.getSnapshot().activities).toHaveLength(1); expect(controller.getSnapshot().activities[0].id).not.toContain("previous:"); expect(api.chatCancel).not.toHaveBeenCalled(); pending.resolve({ state: "untested", load_success: false, generation_pass: false, checked_at_unix_ms: null, error_code: null }); await task; controller.clearActivityHistory(); await controller.refresh(); expect(controller.getSnapshot().activities).toEqual([]); expect(localStorage.getItem(ACTIVITY_STORAGE_KEY)).toBeNull();
  });
  it("keeps history visible when clearing persisted summaries is unconfirmed", async () => {
    persistActivitySummaries([{ id: "model:old", kind: "model", status: "completed", updated_at: 1, detail: "", label: "", error: null }]); const controller = new DesktopController(makeApi()); await controller.refresh(); vi.spyOn(Storage.prototype, "removeItem").mockImplementation(() => { throw new Error("blocked"); }); controller.clearActivityHistory(); expect(controller.getSnapshot().activities).toHaveLength(1); expect(controller.getSnapshot().activity_storage_warning).not.toBeNull();
  });
  it("exposes a confirmed clear action and an explicit storage warning", async () => {
    persistActivitySummaries([{ id: "model:old", kind: "model", status: "completed", updated_at: 1, detail: "", label: "", error: null }]); const controller = new DesktopController(makeApi()); render(<App initialPage="activity" controller={controller} />); await waitFor(() => expect(controller.getSnapshot().booting).toBe(false)); fireEvent.click(screen.getByRole("button", { name: "清除活动历史" })); expect(controller.getSnapshot().activities).toHaveLength(1); vi.spyOn(Storage.prototype, "removeItem").mockImplementation(() => { throw new Error("blocked"); }); fireEvent.click(screen.getByRole("button", { name: "确认清除活动历史" })); expect(screen.getByText(/活动摘要未能保存或清除/)).toBeVisible();
  });
});

describe("configuration-error explicit stop recovery", () => {
  it.each(["configuration_invalid", "configuration_unavailable", "settings_invalid"])("requires confirmation before recovering stop after %s, and preserves the invalid configuration", async (code) => {
    const api = makeApi({ snapshot: vi.fn(async () => { throw { code, message: "configuration cannot be read" }; }) }); const controller = new DesktopController(api); render(<App controller={controller} />); fireEvent.click(await screen.findByRole("button", { name: "检查并停止本机服务" })); expect(api.stop).not.toHaveBeenCalled(); expect(within(screen.getByRole("dialog")).getByText(/其他客户端/)).toBeVisible(); await act(async () => fireEvent.click(screen.getByRole("button", { name: "确认检查并停止" }))); await waitFor(() => expect(api.stop).toHaveBeenCalledTimes(1)); expect(controller.getSnapshot().snapshot).toBeNull(); expect(controller.getSnapshot().configuration_recovery_error?.code).toBe(code); expect(controller.getSnapshot().notice).toContain("配置仍不可读取"); expect(api.start).not.toHaveBeenCalled();
  });
  it("does not offer this exceptional stop for unrelated unknown transport or native errors", async () => {
    const api = makeApi({ snapshot: vi.fn(async () => { throw { code: "desktop_unavailable", message: "unknown connection" }; }) }); const controller = new DesktopController(api); render(<App controller={controller} />); await waitFor(() => expect(controller.getSnapshot().booting).toBe(false)); expect(screen.queryByRole("button", { name: "检查并停止本机服务" })).not.toBeInTheDocument(); await act(async () => controller.recoverStopService()); expect(api.stop).not.toHaveBeenCalled(); expect(api.start).not.toHaveBeenCalled();
  });
  it("does not manufacture success if cleanup is unconfirmed", async () => {
    const api = makeApi({ snapshot: vi.fn(async () => { throw { code: "configuration_invalid", message: "bad config" }; }), stop: vi.fn(async () => { throw { code: "runtime_stop_unconfirmed", message: "not cleaned" }; }) }); const controller = new DesktopController(api); await controller.refresh(); await controller.recoverStopService(); expect(controller.getSnapshot().notice).toBeNull(); expect(controller.getSnapshot().error?.code).toBe("runtime_stop_unconfirmed"); expect(controller.getSnapshot().configuration_recovery_error?.code).toBe("configuration_invalid"); expect(api.snapshot).toHaveBeenCalledTimes(1);
  });
  it("labels saved and running idle policies separately when a restart is pending", async () => {
    const value = configuredSnapshot(); value.configuration!.saved.runtime.idle_unload_seconds = 900; value.configuration!.pending_restart = true; value.settings.idle_unload_seconds = 300; const controller = new DesktopController(makeApi({ snapshot: vi.fn(async () => value) })); render(<App initialPage="api" controller={controller} />); await waitFor(() => expect(controller.getSnapshot().booting).toBe(false)); openGroup("调用规则与运行策略"); expect(screen.getByText(/已保存空闲策略：空闲 900 秒/)).toBeVisible(); expect(screen.getByText(/运行中空闲策略：空闲 300 秒/)).toBeVisible();
  });
});

describe("major action accountability and test parameter visibility", () => {
  it("records a confirmed service start once, with genuine pending and terminal phases", async () => {
    let value = configuredSnapshot(true); const pending = deferred<Snapshot>(); const api = makeApi({ snapshot: vi.fn(async () => value), start: vi.fn(() => pending.promise) }); const controller = new DesktopController(api); await controller.refresh(); const start = controller.start(false); const item = controller.getSnapshot().activities.find((entry) => entry.kind === "service")!; expect(item.status).toBe("running"); value = configuredSnapshot(); pending.resolve(value); await start; expect(controller.getSnapshot().activities.filter((entry) => entry.kind === "service")).toHaveLength(1); expect(controller.getSnapshot().activities.find((entry) => entry.id === item.id)?.status).toBe("completed");
  });
  it("marks an acknowledged configuration save with failed state refresh as needing reconciliation", async () => {
    const value = configuredSnapshot(true), after = structuredClone(value.configuration!); after.revision = revision("b"); const api = makeApi({ snapshot: vi.fn().mockResolvedValueOnce(value).mockRejectedValue({ code: "configuration_invalid", message: "bad subsequent config" }), configurationSave: vi.fn(async () => after) }); const controller = new DesktopController(api); await controller.refresh(); expect(await controller.saveConfiguration({ expected_revision: revision(), update: { kind: "request_defaults", request_defaults: after.saved.request_defaults } })).toBe(false); expect(controller.getSnapshot().activities.find((entry) => entry.kind === "configuration")?.status).toBe("recovery"); expect(api.configurationSave).toHaveBeenCalledTimes(1);
  });
  it("does not leave a never-ending running task when native close fails during loading", async () => {
    const value = configuredSnapshot(); const pending = deferred<NonNullable<Snapshot["runtime"]>>(); const api = makeApi({ snapshot: vi.fn(async () => value), loadModelProfile: vi.fn(() => pending.promise), close: vi.fn(async () => { throw { code: "desktop_busy", message: "cleanup not confirmed" }; }) }); const controller = new DesktopController(api); await controller.refresh(); const loading = controller.loadModel(model.id); expect(controller.getSnapshot().activities.find((entry) => entry.kind === "model")?.status).toBe("running"); await controller.close(); expect(controller.getSnapshot().activities.find((entry) => entry.kind === "model")?.status).toBe("running"); pending.resolve(value.runtime!); await loading; expect(controller.getSnapshot().operation).toBeNull(); expect(controller.getSnapshot().testing_model).toBeNull(); expect(controller.getSnapshot().activities.find((entry) => entry.kind === "model")).toMatchObject({ status: "completed", model: { phase: "finished", error: null, result: { state: "loaded", load_success: true } } });
  });
  it("shows actual resident context and current request defaults in auxiliary testing", async () => {
    const value = configuredSnapshot(); value.configuration!.saved.request_defaults.max_output_tokens = 4096; value.configuration!.pending_restart = true; const controller = new DesktopController(makeApi({ snapshot: vi.fn(async () => value) })); render(<App initialPage="chat" controller={controller} />); await waitFor(() => expect(controller.getSnapshot().booting).toBe(false)); fireEvent.click(screen.getByText("测试参数")); expect(screen.getByLabelText("辅助测试实际参数")).toHaveTextContent("实际上下文：2048 · 本次输出预算：768 tokens（服务默认）"); expect(screen.getByText(/temperature：0.8 · top_p：0.95/)).toBeVisible();
  });
});

describe("native close decision barrier", () => {
  it.each(["reject", "confirm"] as const)("holds a load response until native close decides to %s without replay", async (decision) => {
    const value = configuredSnapshot(); const response = deferred<NonNullable<Snapshot["runtime"]>>(), close = deferred<void>(); const api = makeApi({ snapshot: vi.fn(async () => value), loadModelProfile: vi.fn(() => response.promise), close: vi.fn(() => close.promise) }); const controller = new DesktopController(api); await controller.refresh(); const loading = controller.loadModel(model.id); const original = controller.getSnapshot().model_tests[model.id].id; const closing = controller.close(); response.resolve(value.runtime!); await Promise.resolve(); await Promise.resolve(); expect(controller.getSnapshot().model_tests[model.id]?.phase).toBe("running");
    if (decision === "reject") close.reject({ code: "desktop_busy", message: "still busy" }); else close.resolve(); await closing; await loading;
    expect(api.loadModelProfile).toHaveBeenCalledTimes(1); expect(api.testModel).not.toHaveBeenCalled(); expect(controller.getSnapshot().activities.filter((item) => item.kind === "model")).toHaveLength(1);
    if (decision === "reject") expect(controller.getSnapshot().model_tests[model.id]).toMatchObject({ id: original, phase: "finished", result: { load_success: true }, error: null });
    else expect(controller.getSnapshot().model_tests[model.id]).toBeUndefined();
  });
});

describe("offline initialization acknowledgement ordering", () => {
  it("does not let a stale poll in the same microtask turn undo initialized state", async () => {
    const before = configuredSnapshot(true); before.initialized = false; before.configuration!.revision = "absent"; const after = configuredSnapshot(true); const init = deferred<Snapshot>(), stale = deferred<Snapshot>(); const api = makeApi({ snapshot: vi.fn().mockResolvedValueOnce(before).mockReturnValueOnce(stale.promise).mockResolvedValue(after), initialize: vi.fn(() => init.promise) }); const controller = new DesktopController(api); await controller.refresh(); const initializing = controller.initialize(); const polling = controller.refresh(); init.resolve(after); stale.resolve(before); await initializing; await polling; expect(controller.getSnapshot().snapshot?.initialized).toBe(true); expect(controller.getSnapshot().snapshot?.configuration?.revision).toBe(revision()); expect(api.start).not.toHaveBeenCalled(); expect(api.initialize).toHaveBeenCalledTimes(1);
  });
});
