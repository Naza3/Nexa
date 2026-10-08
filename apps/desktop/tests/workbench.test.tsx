import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { WorkbenchStore, defaultWorkbench, resolveOcrProfile, validWorkbench } from "../src/workbench";
import { DesktopController } from "../src/controller";
import App from "../src/App";
import { deferred, makeApi, model } from "./fixtures";
import { configuredSnapshot, modelConfiguration } from "./configurationFixtures";
import type { OcrHistoryList, WorkbenchPreferencesSnapshot } from "../src/types";
const prefs = (): WorkbenchPreferencesSnapshot => ({ revision: "revision-1", preferences: defaultWorkbench() });
afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); });
function preferencesApi(value = prefs()) {
  let saved = structuredClone(value); let generation = 1;
  return makeApi({ workbenchGet: vi.fn(async () => structuredClone(saved)), workbenchSave: vi.fn(async ({ preferences, expected_revision }) => {
    if (expected_revision !== saved.revision) throw { code: "workbench_conflict" };
    saved = { revision: `revision-${++generation}`, preferences: structuredClone(preferences) }; return structuredClone(saved);
  }) });
}

describe("workbench writer", () => {
  it("never saves defaults before hydration and reads once across consumers", async () => {
    const pending = deferred<WorkbenchPreferencesSnapshot>();
    const api = preferencesApi(); vi.mocked(api.workbenchGet!).mockReturnValue(pending.promise);
    const store = new WorkbenchStore(api, () => {});
    const one = store.load(), two = store.load();
    store.edit(defaultWorkbench());
    expect(await store.flush()).toBe(true);
    expect(api.workbenchGet).toHaveBeenCalledTimes(1); expect(api.workbenchSave).not.toHaveBeenCalled();
    const restored = prefs(); restored.preferences.chat.draft = "restored";
    pending.resolve(restored); await Promise.all([one, two]);
    expect(store.state.preferences.chat.draft).toBe("restored"); expect(api.workbenchSave).not.toHaveBeenCalled();
  });
  it("serializes edits made while saving and advances CAS revision for the next write", async () => {
    const first = deferred<WorkbenchPreferencesSnapshot>(); const api = preferencesApi();
    vi.mocked(api.workbenchSave!).mockReturnValueOnce(first.promise).mockImplementationOnce(async ({ preferences, expected_revision }) => ({ revision: `${expected_revision}-next`, preferences }));
    const store = new WorkbenchStore(api, () => {}); await store.load();
    store.edit({ ...store.state.preferences, chat: { draft: "first" } });
    const writing = store.flush();
    store.edit({ ...store.state.preferences, ocr: { ...store.state.preferences.ocr, prompt: "latest OCR prompt" } });
    expect(api.workbenchSave).toHaveBeenCalledTimes(1);
    first.resolve({ revision: "revision-2", preferences: { ...defaultWorkbench(), chat: { draft: "first" } } });
    expect(await writing).toBe(true);
    expect(api.workbenchSave).toHaveBeenCalledTimes(2);
    expect(vi.mocked(api.workbenchSave!).mock.calls[1][0]).toMatchObject({ expected_revision: "revision-2", preferences: { chat: { draft: "first" }, ocr: { prompt: "latest OCR prompt" } } });
    expect(store.state.dirty).toBe(false);
  });
  it("retains conflicts and only overwrites after an explicit new revision read", async () => {
    const api = preferencesApi(); const store = new WorkbenchStore(api, () => {}); await store.load();
    store.edit({ ...store.state.preferences, chat: { draft: "keep me" } });
    vi.mocked(api.workbenchSave!).mockRejectedValueOnce({ code: "configuration_conflict" });
    expect(await store.flush()).toBe(false); expect(store.state.conflict).toBe(true); expect(store.state.preferences.chat.draft).toBe("keep me");
    expect(await store.flush()).toBe(false); expect(api.workbenchSave).toHaveBeenCalledTimes(1);
    await store.overwrite(); expect(store.state.dirty).toBe(false); expect(store.state.preferences.chat.draft).toBe("keep me");
  });
  it("retains invalid UTF-8 sized inputs without truncating or claiming them saved", async () => {
    const api = preferencesApi(); const store = new WorkbenchStore(api, () => {}); await store.load();
    const oversized = "字".repeat(5500); store.edit({ ...store.state.preferences, chat: { draft: oversized } });
    expect(await store.flush()).toBe(false); expect(store.state.preferences.chat.draft).toBe(oversized); expect(store.state.error).toContain("16 KiB");
    expect(api.workbenchSave).not.toHaveBeenCalled();
    const prompt = "字".repeat(1400); const value = defaultWorkbench(); value.ocr.prompt = prompt;
    store.edit(value); expect(await store.flush()).toBe(false); expect(store.state.preferences.ocr.prompt).toBe(prompt);
    expect(validWorkbench(defaultWorkbench())).toBe(true);
  });
  it("keeps an invalid newer edit visibly unsaved after an older valid write succeeds", async () => {
    const first = deferred<WorkbenchPreferencesSnapshot>(); const api = preferencesApi(); vi.mocked(api.workbenchSave!).mockReturnValueOnce(first.promise);
    const store = new WorkbenchStore(api, () => {}); await store.load();
    const valid = { ...store.state.preferences, chat: { draft: "valid" } }; store.edit(valid); const writing = store.flush();
    const oversized = "字".repeat(5500); store.edit({ ...store.state.preferences, chat: { draft: oversized } });
    first.resolve({ revision: "revision-2", preferences: valid }); expect(await writing).toBe(false);
    expect(store.state.dirty).toBe(true); expect(store.state.preferences.chat.draft).toBe(oversized); expect(store.state.error).toContain("16 KiB");
    expect(api.workbenchSave).toHaveBeenCalledTimes(1);
  });
  it("blocks close on failed prefs but flushes debounced valid preferences before native close", async () => {
    const api = preferencesApi(); const controller = new DesktopController(api); const cleanup = controller.mount();
    await waitFor(() => expect(controller.getSnapshot().workbench.hydrated).toBe(true));
    controller.setChatDraft("saved before close"); vi.mocked(api.workbenchSave!).mockRejectedValueOnce(new Error("disk"));
    await controller.close(); expect(api.close).not.toHaveBeenCalled(); expect(controller.getSnapshot().workbench.preferences.chat.draft).toBe("saved before close");
    await controller.close(); expect(api.close).toHaveBeenCalledTimes(1);
    expect(vi.mocked(api.workbenchSave!).mock.invocationCallOrder.at(-1)).toBeLessThan(vi.mocked(api.close).mock.invocationCallOrder[0]); cleanup();
  });
  it("waits for accepted history writes before closing and retries failed text persistence", async () => {
    const pending = deferred<OcrHistoryList>(); const api = makeApi({ ocrHistorySave: vi.fn(() => pending.promise) }); const controller = new DesktopController(api);
    const saving = controller.persistOcrHistory({ mode: "create", id: "request", model_id: "model", image_name: "file.png", markdown: "body", status: "completed", finish_reason: "stop", error_code: null, incomplete: false, performance: null });
    const closing = controller.close(); await act(async () => {}); expect(api.close).not.toHaveBeenCalled();
    pending.resolve({ capacity: 100, entries: [] }); await saving; await closing; expect(api.close).toHaveBeenCalledTimes(1);
  });
});

describe("model draft ancestry", () => {
  it("uses the full explicit saved profile and recommends OCR defaults only without a profile", () => {
    const config = modelConfiguration(); expect(resolveOcrProfile(config).draft.context_size).toBe(8192);
    config.load_overrides.threads = 8; config.saved_effective = { context_size: 32768, threads: 8, batch_size: 1024 };
    expect(resolveOcrProfile(config).draft).toEqual(config.saved_effective);
  });
  it("follows updated backend values for untouched drafts, restores changes against the same base and flags real conflicts", () => {
    const config = modelConfiguration(); const old = { ...config.saved_effective }; const draft = { ...old, threads: 8 };
    expect(resolveOcrProfile(config, { base: old, draft })).toMatchObject({ draft, conflict: false });
    config.saved_effective = { ...old, threads: 16 };
    expect(resolveOcrProfile(config, { base: old, draft: old })).toMatchObject({ draft: config.saved_effective, conflict: false });
    expect(resolveOcrProfile(config, { base: old, draft })).toMatchObject({ draft, conflict: true });
    config.configuration_revision = "unrelated-revision"; config.saved_effective = old;
    expect(resolveOcrProfile(config, { base: old, draft })).toMatchObject({ draft, conflict: false });
  });
});

describe("restored OCR and chat controls", () => {
  it("acknowledges native close before routing through the shared persistence flush", async () => {
    const ack = deferred<boolean>(); const api = preferencesApi(); api.closeAcknowledge = vi.fn(() => ack.promise);
    const controller = new DesktopController(api); render(<App controller={controller} initialPage="chat" />);
    await waitFor(() => expect(controller.getSnapshot().workbench.hydrated).toBe(true));
    fireEvent.change(screen.getByLabelText("输入消息"), { target: { value: "flush from native X" } });
    window.dispatchEvent(new CustomEvent("nexa-close-requested", { detail: { id: "native-close-id" } }));
    expect(api.closeAcknowledge).toHaveBeenCalledWith("native-close-id"); expect(api.close).not.toHaveBeenCalled();
    ack.resolve(true);
    await waitFor(() => expect(api.close).toHaveBeenCalledTimes(1));
    expect(vi.mocked(api.workbenchSave!).mock.calls.at(-1)?.[0].preferences.chat.draft).toBe("flush from native X");
  });
  it("ignores stale native close acknowledgements and does not close after unmount", async () => {
    const ack = deferred<boolean>(); const api = makeApi({ closeAcknowledge: vi.fn().mockResolvedValueOnce(false).mockReturnValueOnce(ack.promise) });
    const view = render(<App controller={new DesktopController(api)} />);
    window.dispatchEvent(new CustomEvent("nexa-close-requested", { detail: { id: "stale" } })); await act(async () => {});
    expect(api.close).not.toHaveBeenCalled();
    window.dispatchEvent(new CustomEvent("nexa-close-requested", { detail: { id: "pending" } })); view.unmount();
    ack.resolve(true); await act(async () => {}); expect(api.close).not.toHaveBeenCalled();
  });
  it("restores all OCR options and chat draft, preserves distinct model drafts, and persists edits", async () => {
    const saved = prefs(); saved.preferences.ocr = { ...saved.preferences.ocr, model_id: model.id, prompt: "saved prompt", max_output_tokens: 777, image_edge: 1600, markdown: true, context_size: 16384, threads: 6, batch_size: 512, model_drafts: { [model.id]: { base: { context_size: 4096, threads: 4, batch_size: 512 }, draft: { context_size: 16384, threads: 6, batch_size: 512 } } } }; saved.preferences.chat.draft = "saved draft";
    const api = preferencesApi(saved); api.snapshot = vi.fn(async () => configuredSnapshot()); api.modelsPage = vi.fn(async () => ({ generation: "g", data: [{ ...model, has_projector: true }], next_after: null })); api.configurationModelGet = vi.fn(async () => modelConfiguration());
    const controller = new DesktopController(api); const view = render(<App controller={controller} initialPage="ocr" />);
    await waitFor(() => expect(screen.getByLabelText("OCR 加载上下文")).toHaveValue(16384));
    expect(screen.getByLabelText("OCR 加载线程")).toHaveValue(6); expect(screen.getByLabelText("识别提示词")).toHaveValue("saved prompt"); expect(screen.getByLabelText("最大输出 token")).toHaveValue(777); expect(screen.getByRole("button", { name: "Markdown" })).toHaveAttribute("aria-pressed", "true"); expect(screen.getByLabelText("发送图片尺寸")).toHaveValue("1600");
    fireEvent.change(screen.getByLabelText("识别提示词"), { target: { value: "changed prompt" } });
    await act(async () => { await controller.saveWorkbench(); });
    fireEvent.keyDown(document, { altKey: true, key: "1" }); fireEvent.click(screen.getByRole("button", { name: "聊天测试" }));
    expect(screen.getByLabelText("输入消息")).toHaveValue("saved draft");
    fireEvent.change(screen.getByLabelText("输入消息"), { target: { value: "new draft" } }); await act(async () => { await controller.saveWorkbench(); });
    view.unmount();
    const next = new DesktopController(api); render(<App controller={next} initialPage="chat" />);
    await waitFor(() => expect(screen.getByLabelText("输入消息")).toHaveValue("new draft"));
    expect(next.getSnapshot().workbench.preferences.ocr.prompt).toBe("changed prompt");
  });
  it("restores legal low context/high thread profile values and requires explicit conflict resolution", async () => {
    const saved = prefs(); const old = { context_size: 8192, threads: 4, batch_size: 256 }; const draft = { ...old, threads: 8 };
    saved.preferences.ocr.model_id = model.id; saved.preferences.ocr.model_drafts[model.id] = { base: old, draft };
    const config = modelConfiguration(); config.saved_effective = { context_size: 32, threads: 256, batch_size: 32 }; config.load_overrides.threads = 256;
    const api = preferencesApi(saved); api.snapshot = vi.fn(async () => configuredSnapshot()); api.modelsPage = vi.fn(async () => ({ generation: "g", data: [{ ...model, has_projector: true }], next_after: null })); api.configurationModelGet = vi.fn(async () => config);
    render(<App controller={new DesktopController(api)} initialPage="ocr" />);
    await screen.findByText("所选模型参数已变化"); expect(screen.getByLabelText("OCR 加载线程")).toHaveValue(8); expect(screen.getByRole("button", { name: "加载所选 OCR 模型" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "采用已保存模型参数" }));
    expect(screen.getByLabelText("OCR 加载上下文")).toHaveValue(32); expect(screen.getByLabelText("OCR 加载线程")).toHaveValue(256); expect(screen.getByRole("button", { name: "加载所选 OCR 模型" })).toBeEnabled();
  });
});
