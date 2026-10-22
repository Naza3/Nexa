import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { OcrHistory } from "../src/OcrHistory";
import { OcrPage } from "../src/OcrPage";
import { DesktopController } from "../src/controller";
import * as ocrImage from "../src/ocrImage";
import type { ChatBatch, ChatEvent, DesktopApi, OcrHistoryEntry, OcrHistoryList, PerformanceRecord, PerformanceSnapshot } from "../src/types";
import { deferred, makeApi, model, snapshot } from "./fixtures";

const usage = { prompt_tokens: 12, completion_tokens: 4, total_tokens: 16 };
const performanceRecord = (): PerformanceRecord => ({ sequence: 1, request_id: "request-1", model_id: model.id, modality: "image", status: "completed", accepted_at_unix_ms: 123456, max_output_tokens: 2048, usage: { prompt_tokens: 12, completion_tokens: 4 }, timings: { queue_ms: 0, load_ms: 0, execution_ms: 3000 }, performance: { timings: { prepare_us: 1000, prefill_us: 1000000, decode_us: 2000000, output_callback_us: 1000 }, load_options: { context_size: 8192, batch_size: 256, threads: 4 } }, error_code: null, finish_reason: "stop" });
const metrics = (): PerformanceSnapshot => ({ instance_id: "instance", capacity: 200, records: [performanceRecord()] });
function entry(id = "request-1", image_name = "page.png"): OcrHistoryEntry { return { id, image_name, model_id: model.id, status: "completed", finish_reason: "stop", error_code: null, incomplete: false, markdown: `# ${image_name}`, performance: null, first_saved_at_unix_ms: 123456 }; }
function list(entries: OcrHistoryEntry[]): OcrHistoryList { return { capacity: 100, entries: entries.map((e) => { const { markdown, performance: _metrics, ...summary } = e; void _metrics; return { ...summary, markdown_bytes: new TextEncoder().encode(markdown).length }; }) }; }
function storedApi(initial: OcrHistoryEntry[] = []) {
  const store = new Map(initial.map((e) => [e.id, e]));
  const current = () => list([...store.values()].reverse());
  const api = makeApi({
    ocrHistoryList: vi.fn(async () => current()),
    ocrHistoryGet: vi.fn(async (id) => { const item = store.get(id); if (!item) throw { code: "ocr_history_not_found" }; return structuredClone(item); }),
    ocrHistorySave: vi.fn(async (request) => {
      const existing = store.get(request.id);
      if (request.mode === "update_performance" && !existing) throw { code: "ocr_history_not_found" };
      const { mode: _mode, ...row } = request; void _mode;
      store.set(request.id, { ...row, first_saved_at_unix_ms: existing?.first_saved_at_unix_ms ?? 123456 });
      return current();
    }),
    ocrHistoryDelete: vi.fn(async (id) => { store.delete(id); return current(); }),
    saveOcrMarkdown: vi.fn(async () => ({ saved: true })),
  });
  return { api, store };
}
function batch(event: ChatEvent = { type: "completed", finish_reason: "stop", usage }, text = "# 识别原文"): ChatBatch {
  return { request_id: "request-1", runtime_instance_id: "instance", events: [...(text ? [{ type: "delta" as const, text }] : []), event], terminal: true };
}
async function mountOcr(api: DesktopApi) {
  vi.spyOn(ocrImage, "prepareOcrImage").mockResolvedValue("data:image/png;base64,AQ==");
  api.ocrStart = vi.fn(async () => ({ request_id: "request-1" }));
  api.chatNext = vi.fn(async () => batch());
  const controller = new DesktopController(api); const value = snapshot(); value.runtime!.load_options!.context_size = 8192;
  api.snapshot = vi.fn(async () => value); await controller.refresh();
  const state = { ...controller.getSnapshot(), booting: false, snapshot: value, models: { generation: "g", data: [{ ...model, has_projector: true }], next_after: null } };
  const view = render(<OcrPage controller={controller} state={state} />);
  fireEvent.change(screen.getByLabelText("OCR 模型"), { target: { value: model.id } });
  fireEvent.change(screen.getByLabelText("OCR 图片"), { target: { files: [new File(["png"], "original.png", { type: "image/png" })] } });
  await screen.findByAltText("待识别图片预览");
  return { ...view, controller, state };
}
function expandHistory() { const summary = screen.getByText("最近识别结果").closest("summary")!; fireEvent.click(summary); }
afterEach(() => vi.restoreAllMocks());

describe("OCR saved history browsing", () => {
  it("reads stored results after remount without a loaded model; exports only the original text", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined); Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    const { api } = storedApi([entry()]);
    const first = render(<OcrHistory api={api} changed={0} />); expandHistory();
    fireEvent.click(await screen.findByRole("button", { name: "查看识别记录 page.png" }));
    await screen.findByRole("region", { name: "历史识别正文" });
    first.unmount();
    render(<OcrHistory api={api} changed={0} />); expandHistory();
    fireEvent.click(await screen.findByRole("button", { name: "查看识别记录 page.png" }));
    expect(await screen.findByRole("region", { name: "历史识别正文" })).toHaveTextContent("# page.png");
    fireEvent.click(screen.getByRole("button", { name: "历史 Markdown" }));
    expect(within(screen.getByRole("region", { name: "历史识别正文" })).getByRole("heading", { name: "page.png" })).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "复制历史原文" }));
    fireEvent.click(screen.getByRole("button", { name: "另存历史 .md" }));
    expect(writeText).toHaveBeenCalledWith("# page.png");
    expect(api.saveOcrMarkdown).toHaveBeenCalledWith("# page.png");
    expect(api.ocrHistorySave).not.toHaveBeenCalled();
    expect(api.loadModel).not.toHaveBeenCalled();
  });
  it("does not let a slower previous selection overwrite the selected result", async () => {
    const a = entry("a", "a.png"), b = entry("b", "b.png");
    const old = deferred<OcrHistoryEntry>();
    const { api } = storedApi([a, b]);
    vi.mocked(api.ocrHistoryGet!).mockImplementation((id) => id === "a" ? old.promise : Promise.resolve(b));
    render(<OcrHistory api={api} changed={0} />); expandHistory();
    fireEvent.click(await screen.findByRole("button", { name: "查看识别记录 a.png" }));
    fireEvent.click(screen.getByRole("button", { name: "查看识别记录 b.png" }));
    expect(await screen.findByRole("region", { name: "历史识别正文" })).toHaveTextContent("# b.png");
    old.resolve(a); await act(async () => {});
    expect(screen.getByRole("region", { name: "历史识别正文" })).toHaveTextContent("# b.png");
  });
  it("deletes a selected result and ignores its outstanding detail read", async () => {
    const old = deferred<OcrHistoryEntry>();
    const { api, store } = storedApi([entry()]);
    vi.mocked(api.ocrHistoryGet!).mockReturnValueOnce(old.promise);
    render(<OcrHistory api={api} changed={0} />); expandHistory();
    fireEvent.click(await screen.findByRole("button", { name: "查看识别记录 page.png" }));
    fireEvent.click(screen.getByRole("button", { name: "删除识别记录 page.png" }));
    await screen.findByText("已删除这条本机识别记录。");
    old.resolve(entry()); await act(async () => {});
    expect(store.size).toBe(0);
    expect(screen.queryByRole("region", { name: "历史识别正文" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "查看识别记录 page.png" })).not.toBeInTheDocument();
  });
  it("separately reports old bridge, read errors and malformed saved records", async () => {
    const view = render(<OcrHistory api={makeApi()} changed={0} />); expandHistory();
    expect(screen.getByText(/当前桌面版本不支持本机识别历史/)).toBeInTheDocument();
    view.rerender(<OcrHistory api={makeApi({ ocrHistoryList: async () => { throw new Error(); } })} changed={0} />);
    await screen.findByText(/无法读取本机识别历史/);
  });
});

describe("OCR terminal autosave", () => {
  it("rejects late workbench edits and new OCR while close waits for a history write", async () => {
    const pending = deferred<OcrHistoryList>(); const { api } = storedApi(); vi.mocked(api.ocrHistorySave!).mockReturnValueOnce(pending.promise);
    const { controller } = await mountOcr(api);
    fireEvent.click(screen.getByRole("button", { name: "识别图片" })); await screen.findByText("识别完成，请核对原图。");
    const closing = controller.close();
    expect(controller.getSnapshot().closing).toBe(true);
    const before = controller.getSnapshot().workbench.preferences;
    controller.setChatDraft("late draft"); controller.setOcrPreferences({ prompt: "late prompt" });
    expect(controller.getSnapshot().workbench.preferences).toBe(before);
    expect(await controller.send("late request")).toBe(false);
    // This standalone fixture deliberately passes a stale ViewState, exercising the action guard.
    fireEvent.click(screen.getByRole("button", { name: "识别图片" })); expect(api.ocrStart).toHaveBeenCalledTimes(1);
    pending.resolve({ capacity: 100, entries: [] }); await act(async () => { await closing; });
    expect(controller.getSnapshot().closing).toBe(false); expect(api.close).toHaveBeenCalledTimes(1);
  });
  it("forgets a published-but-failed create after deletion so later flush cannot resurrect it", async () => {
    const { api, store } = storedApi(); const controller = new DesktopController(api);
    vi.mocked(api.ocrHistorySave!).mockImplementationOnce(async ({ mode: _mode, ...request }) => { void _mode; store.set(request.id, { ...request, first_saved_at_unix_ms: 123456 }); throw { code: "durability_unconfirmed" }; });
    expect(await controller.persistOcrHistory({ ...entry(), mode: "create" })).toBe(false);
    expect(store.size).toBe(1);
    await controller.deleteOcrHistory("request-1"); expect(store.size).toBe(0);
    expect(await controller.flushOcrHistory()).toBe(true); expect(store.size).toBe(0);
    expect(api.ocrHistorySave).toHaveBeenCalledTimes(1);
  });
  it("normal close cancels a running OCR, consumes its terminal and persists partial text before native exit", async () => {
    const terminal = deferred<ChatBatch>(); const { api, store } = storedApi();
    const { controller } = await mountOcr(api);
    vi.mocked(api.chatNext).mockResolvedValueOnce({ request_id: "request-1", runtime_instance_id: "instance", events: [{ type: "delta", text: "部分结果" }], terminal: false }).mockReturnValueOnce(terminal.promise);
    fireEvent.click(screen.getByRole("button", { name: "识别图片" }));
    await waitFor(() => expect(api.chatNext).toHaveBeenCalledTimes(2));
    const closing = controller.close();
    await waitFor(() => expect(api.chatCancel).toHaveBeenCalledWith("request-1"));
    expect(api.close).not.toHaveBeenCalled();
    terminal.resolve(batch({ type: "cancelled" }, "")); await act(async () => { await closing; });
    expect(store.get("request-1")).toMatchObject({ status: "cancelled", incomplete: true, markdown: "部分结果" });
    expect(api.close).toHaveBeenCalledTimes(1);
    expect(vi.mocked(api.ocrHistorySave!).mock.invocationCallOrder[0]).toBeLessThan(vi.mocked(api.close).mock.invocationCallOrder[0]);
  });
  it("saves text independently while metrics are pending, then updates the same row after create succeeds", async () => {
    const create = deferred<OcrHistoryList>(); const metric = deferred<PerformanceSnapshot>();
    const { api } = storedApi(); api.performanceGet = vi.fn(() => metric.promise);
    vi.mocked(api.ocrHistorySave!).mockReturnValueOnce(create.promise);
    await mountOcr(api);
    fireEvent.click(screen.getByRole("button", { name: "识别图片" }));
    await screen.findByText("识别完成，请核对原图。");
    await waitFor(() => expect(api.ocrHistorySave).toHaveBeenCalledTimes(1));
    const first = vi.mocked(api.ocrHistorySave!).mock.calls[0][0];
    expect(first).toMatchObject({ mode: "create", id: "request-1", image_name: "original.png", model_id: model.id, markdown: "# 识别原文", incomplete: false, performance: null });
    // The OCR reservation now remains held until this image's text is durable.
    expect(screen.getByRole("button", { name: "识别图片" })).toBeDisabled();
    metric.resolve(metrics()); await act(async () => {});
    expect(api.ocrHistorySave).toHaveBeenCalledTimes(1);
    create.resolve({ capacity: 100, entries: [] });
    await waitFor(() => expect(api.ocrHistorySave).toHaveBeenCalledTimes(2));
    expect(vi.mocked(api.ocrHistorySave!).mock.calls[1][0]).toEqual({ ...first, mode: "update_performance", performance: { instance_id: "instance", record: performanceRecord() } });
  });
  it("saves successful text when performance is unavailable and reports disk errors separately", async () => {
    const { api, store } = storedApi(); api.performanceGet = vi.fn().mockRejectedValue(new Error());
    await mountOcr(api); fireEvent.click(screen.getByRole("button", { name: "识别图片" }));
    await screen.findByText("已保存到本机最近识别结果。");
    expect(store.get("request-1")?.markdown).toBe("# 识别原文");
    expect(store.get("request-1")?.performance).toBeNull();
    vi.mocked(api.ocrHistorySave!).mockRejectedValueOnce(new Error("disk full"));
    fireEvent.click(screen.getByRole("button", { name: "识别图片" }));
    await screen.findByText(/本机历史保存失败/);
    expect(screen.getByText("识别完成，请核对原图。")).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "识别结果内容" })).toHaveTextContent("# 识别原文");
  });
  it.each(["cancelled", "failed"] as const)("saves %s partial output and marks it incomplete", async (status) => {
    const { api, store } = storedApi(); await mountOcr(api);
    vi.mocked(api.chatNext).mockResolvedValueOnce(batch(status === "failed" ? { type: "failed", code: "execution_timeout", message: "timeout" } : { type: "cancelled" }, "部分正文"));
    fireEvent.click(screen.getByRole("button", { name: "识别图片" }));
    await screen.findByText("已保存到本机最近识别结果。");
    expect(store.get("request-1")).toMatchObject({ status, incomplete: true, markdown: "部分正文", finish_reason: null });
  });
  it("does not save empty terminal output", async () => {
    const { api } = storedApi(); await mountOcr(api);
    vi.mocked(api.chatNext).mockResolvedValueOnce(batch({ type: "cancelled" }, ""));
    fireEvent.click(screen.getByRole("button", { name: "识别图片" }));
    await screen.findByText("已停止，已生成内容可能不完整。");
    expect(api.ocrHistorySave).not.toHaveBeenCalled();
  });
  it("does not recreate a deleted row when its late performance arrives", async () => {
    const late = deferred<PerformanceSnapshot>(); const { api, store } = storedApi(); api.performanceGet = vi.fn(() => late.promise);
    await mountOcr(api); fireEvent.click(screen.getByRole("button", { name: "识别图片" }));
    await screen.findByText("已保存到本机最近识别结果。"); expandHistory();
    fireEvent.click(await screen.findByRole("button", { name: "删除识别记录 original.png" }));
    await screen.findByText("已删除这条本机识别记录。");
    late.resolve(metrics()); await act(async () => {});
    expect(store.size).toBe(0);
    expect(vi.mocked(api.ocrHistorySave!).mock.calls.map(([request]) => request.mode)).toEqual(["create", "update_performance"]);
    expect(screen.queryByText(/性能信息未能补存/)).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "查看识别记录 original.png" })).not.toBeInTheDocument();
  });
});
