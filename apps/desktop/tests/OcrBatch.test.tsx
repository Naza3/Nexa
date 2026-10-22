import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import App from "../src/App";
import { DesktopController } from "../src/controller";
import * as ocrImage from "../src/ocrImage";
import type { ChatBatch, DesktopApi, OcrHistorySaveRequest, OcrHistoryList, PerformanceRecord, PerformanceSnapshot } from "../src/types";
import { deferred, makeApi, model, snapshot } from "./fixtures";

const usage = { prompt_tokens: 12, completion_tokens: 4, total_tokens: 16 };
const png = (name: string) => new File(["png"], name, { type: "image/png" });
const done = (id: string, text = id): ChatBatch => ({ request_id: id, runtime_instance_id: "runtime-instance", terminal: true, events: [{ type: "delta", text }, { type: "completed", finish_reason: "stop", usage }] });
const cancelled = (id: string, text = "partial"): ChatBatch => ({ request_id: id, runtime_instance_id: "runtime-instance", terminal: true, events: [{ type: "delta", text }, { type: "cancelled" }] });
function metric(id: string, sequence: number): PerformanceRecord {
  return { sequence, request_id: id, model_id: model.id, modality: "image", status: "completed", accepted_at_unix_ms: 100, max_output_tokens: 2048, usage: { prompt_tokens: 12, completion_tokens: 4 }, timings: { queue_ms: 0, load_ms: 0, execution_ms: 10 }, performance: { timings: { prepare_us: 10, prefill_us: 1000, decode_us: 2000, output_callback_us: 10 }, load_options: { context_size: 8192, threads: 4, batch_size: 256 } }, error_code: null, finish_reason: "stop" };
}
async function mount(overrides: Partial<DesktopApi> = {}) {
  let count = 0;
  const api = makeApi({
    snapshot: vi.fn(async () => { const value = snapshot(); value.runtime!.load_options!.context_size = 8192; return value; }),
    modelsPage: vi.fn(async () => ({ generation: "g", data: [{ ...model, has_projector: true }], next_after: null })),
    ocrStart: vi.fn(async () => ({ request_id: `ocr-${++count}` })),
    chatNext: vi.fn(async (id: string) => done(id)),
    ocrHistoryList: vi.fn(async () => ({ capacity: 100 as const, entries: [] })),
    ocrHistorySave: vi.fn(async () => ({ capacity: 100 as const, entries: [] })),
    ...overrides,
  });
  const controller = new DesktopController(api);
  render(<App controller={controller} initialPage="ocr" />);
  await screen.findByRole("option", { name: /视觉配对/ });
  fireEvent.change(screen.getByLabelText("OCR 模型"), { target: { value: model.id } });
  return { api, controller };
}
function select(files: File[]) { fireEvent.change(screen.getByLabelText("OCR 图片"), { target: { files } }); }
function saves(api: DesktopApi, mode: OcrHistorySaveRequest["mode"]) {
  return vi.mocked(api.ocrHistorySave!).mock.calls.map(([request]) => request).filter((request) => request.mode === mode);
}
beforeEach(() => { vi.spyOn(ocrImage, "prepareOcrImage").mockImplementation(async (file) => `data:image/png;base64,${btoa(file.name)}`); });
afterEach(() => vi.restoreAllMocks());

it("rejects more than 20 images before preparing or starting any request", async () => {
  const { api } = await mount();
  select(Array.from({ length: 21 }, (_, index) => png(`page${index}.png`)));
  expect(await within(screen.getByRole("status", { name: "OCR 图片准备状态" })).findByText(/最多.*20.*张/)).toBeVisible();
  expect(ocrImage.prepareOcrImage).not.toHaveBeenCalled();
  expect(api.ocrStart).not.toHaveBeenCalled();
});

it("rejects an oversized image without partially accepting the other selected images", async () => {
  const { api } = await mount();
  const oversized = png("large.png");
  Object.defineProperty(oversized, "size", { value: 4 * 1024 * 1024 + 1 });
  select([png("small.png"), oversized]);
  expect(await within(screen.getByRole("status", { name: "OCR 图片准备状态" })).findByText(/4 MiB/)).toBeVisible();
  expect(ocrImage.prepareOcrImage).not.toHaveBeenCalled();
  expect(api.ocrStart).not.toHaveBeenCalled();
});

it("preserves FileList import order and lets the user reorder and remove unstarted files without decoding them", async () => {
  await mount();
  select([png("page10.png"), png("page2.png"), png("page1.png")]);
  const queue = await screen.findByRole("region", { name: "OCR 图片队列" });
  const names = () => within(queue).getAllByRole("listitem").map((row) => row.getAttribute("aria-label"));
  expect(names()).toEqual(["队列图片 1：page10.png", "队列图片 2：page2.png", "队列图片 3：page1.png"]);
  fireEvent.click(within(queue).getByRole("button", { name: "上移 page1.png" }));
  expect(names()).toEqual(["队列图片 1：page10.png", "队列图片 2：page1.png", "队列图片 3：page2.png"]);
  fireEvent.click(within(queue).getByRole("button", { name: "下移 page10.png" }));
  fireEvent.click(within(queue).getByRole("button", { name: "移除 page2.png" }));
  expect(names()).toEqual(["队列图片 1：page1.png", "队列图片 2：page10.png"]);
  expect(ocrImage.prepareOcrImage).not.toHaveBeenCalled();
});

it("accepts exactly 20 files totaling 80 MiB while retaining files without eager preparation", async () => {
  await mount();
  const files = Array.from({ length: 20 }, (_, index) => { const file = png(`page${index + 1}.png`); Object.defineProperty(file, "size", { value: 4 * 1024 * 1024 }); return file; });
  select(files);
  const queue = await screen.findByRole("region", { name: "OCR 图片队列" });
  expect(within(queue).getAllByRole("listitem")).toHaveLength(20);
  expect(screen.getByRole("button", { name: "开始批量识别" })).toBeEnabled();
  expect(ocrImage.prepareOcrImage).not.toHaveBeenCalled();
});

it("prepares only the current file and waits for its terminal and saved body before starting the next", async () => {
  const terminal = deferred<ChatBatch>();
  const saved = deferred<OcrHistoryList>();
  const { api, controller } = await mount({ chatNext: vi.fn(async (id) => id === "ocr-1" ? terminal.promise : done(id)), ocrHistorySave: vi.fn().mockReturnValueOnce(saved.promise).mockResolvedValue({ capacity: 100 as const, entries: [] }) });
  const first = png("page1.png"), second = png("page2.png");
  select([first, second]);
  fireEvent.click(screen.getByRole("button", { name: "开始批量识别" }));
  await waitFor(() => expect(api.chatNext).toHaveBeenCalledWith("ocr-1"));
  expect(ocrImage.prepareOcrImage).toHaveBeenCalledTimes(1);
  expect(ocrImage.prepareOcrImage).toHaveBeenCalledWith(first, 0);
  expect(api.ocrStart).toHaveBeenCalledTimes(1);
  await act(async () => terminal.resolve(done("ocr-1", "first body")));
  await waitFor(() => expect(saves(api, "create")).toHaveLength(1));
  expect(controller.getSnapshot().ocr_batch_active).toBe(true);
  expect(controller.acquireOcrBatch()).toBeNull();
  expect(ocrImage.prepareOcrImage).toHaveBeenCalledTimes(1);
  expect(api.ocrStart).toHaveBeenCalledTimes(1);
  await act(async () => saved.resolve({ capacity: 100 as const, entries: [] }));
  await waitFor(() => expect(saves(api, "create")).toHaveLength(2));
  expect(ocrImage.prepareOcrImage).toHaveBeenNthCalledWith(2, second, 0);
  expect(saves(api, "create").map((row) => [row.id, row.image_name, row.markdown])).toEqual([["ocr-1", "page1.png", "first body"], ["ocr-2", "page2.png", "ocr-2"]]);
  await waitFor(() => expect(controller.getSnapshot().ocr_batch_active).toBe(false));
});

it("keeps late metrics and viewed results attached to their own image after later images finish", async () => {
  const slow = deferred<PerformanceSnapshot>();
  const { api } = await mount({ performanceGet: vi.fn().mockReturnValueOnce(slow.promise).mockResolvedValue({ instance_id: "runtime-instance", capacity: 200, records: [metric("ocr-2", 2)] }) });
  select([png("page1.png"), png("page2.png")]);
  fireEvent.click(screen.getByRole("button", { name: "开始批量识别" }));
  await waitFor(() => expect(saves(api, "create")).toHaveLength(2));
  expect(api.ocrStart).toHaveBeenCalledTimes(2);
  await act(async () => slow.resolve({ instance_id: "runtime-instance", capacity: 200, records: [metric("ocr-1", 1)] }));
  await waitFor(() => expect(saves(api, "update_performance")).toHaveLength(2));
  for (const row of saves(api, "update_performance")) {
    expect(row.performance!.record.request_id).toBe(row.id);
    expect(row.image_name).toBe(row.id === "ocr-1" ? "page1.png" : "page2.png");
    expect(row.markdown).toBe(row.id);
  }
  fireEvent.click(screen.getByRole("button", { name: "查看结果 page1.png" }));
  expect(screen.getByRole("region", { name: "识别结果内容" })).toHaveTextContent("ocr-1");
  expect(screen.getByRole("region", { name: "识别结果内容" })).not.toHaveTextContent("ocr-2");
  fireEvent.click(screen.getByRole("button", { name: "查看结果 page2.png" }));
  expect(screen.getByRole("region", { name: "识别结果内容" })).toHaveTextContent("ocr-2");
});

it("pauses on failed OCR and explicitly continues only files that never started", async () => {
  const { api } = await mount({ chatNext: vi.fn(async (id): Promise<ChatBatch> => id === "ocr-1" ? { request_id: id, terminal: true, events: [{ type: "delta", text: "failed partial" }, { type: "failed", code: "native_failure", message: "controlled failure" }] } : done(id)) });
  select([png("page1.png"), png("page2.png"), png("page3.png")]);
  fireEvent.click(screen.getByRole("button", { name: "开始批量识别" }));
  const resume = await screen.findByRole("button", { name: "继续未开始图片" });
  await waitFor(() => expect(resume).toBeEnabled());
  expect(api.ocrStart).toHaveBeenCalledTimes(1);
  expect(saves(api, "create")[0]).toMatchObject({ id: "ocr-1", status: "failed", incomplete: true, markdown: "failed partial" });
  fireEvent.click(resume);
  await waitFor(() => expect(saves(api, "create")).toHaveLength(3));
  expect(api.ocrStart).toHaveBeenCalledTimes(3);
  expect(vi.mocked(ocrImage.prepareOcrImage).mock.calls.map(([file]) => file.name)).toEqual(["page1.png", "page2.png", "page3.png"]);
});

it("pauses on explicit stop and does not replay the stopped image when continuing", async () => {
  const terminal = deferred<ChatBatch>();
  const { api } = await mount({ chatNext: vi.fn(async (id) => id === "ocr-1" ? terminal.promise : done(id)) });
  select([png("page1.png"), png("page2.png")]);
  fireEvent.click(screen.getByRole("button", { name: "开始批量识别" }));
  await waitFor(() => expect(api.chatNext).toHaveBeenCalledWith("ocr-1"));
  fireEvent.click(screen.getByRole("button", { name: "停止识别" }));
  await waitFor(() => expect(api.chatCancel).toHaveBeenCalledWith("ocr-1"));
  expect(api.ocrStart).toHaveBeenCalledTimes(1);
  await act(async () => terminal.resolve(cancelled("ocr-1")));
  const resume = await screen.findByRole("button", { name: "继续未开始图片" });
  await waitFor(() => expect(resume).toBeEnabled());
  expect(api.ocrStart).toHaveBeenCalledTimes(1);
  fireEvent.click(resume);
  await waitFor(() => expect(saves(api, "create")).toHaveLength(2));
  expect(api.ocrStart).toHaveBeenCalledTimes(2);
  expect(saves(api, "create")[0]).toMatchObject({ id: "ocr-1", status: "cancelled", incomplete: true });
});

it.each(["transport", "malformed terminal"])("keeps the original request reserved through %s failure and recovers it without advancing automatically", async (failure) => {
  const next = vi.fn<DesktopApi["chatNext"]>();
  if (failure === "transport") next.mockRejectedValueOnce(new Error("lost read"));
  else next.mockResolvedValueOnce({ request_id: "ocr-1", terminal: true, events: [{ type: "delta", text: "missing terminal acknowledgment" }] });
  next.mockResolvedValueOnce(done("ocr-1", "recovered text")).mockImplementation(async (id) => done(id));
  const { api, controller } = await mount({ chatNext: next });
  select([png("page1.png"), png("page2.png")]);
  fireEvent.click(screen.getByRole("button", { name: "开始批量识别" }));
  const recover = await screen.findByRole("button", { name: "重新确认识别任务" });
  expect(controller.getSnapshot().ocr_batch_active).toBe(true);
  expect(api.ocrStart).toHaveBeenCalledTimes(1);
  fireEvent.click(recover);
  const resume = await screen.findByRole("button", { name: "继续未开始图片" });
  await waitFor(() => expect(resume).toBeEnabled());
  expect(next.mock.calls.map(([id]) => id)).toEqual(["ocr-1", "ocr-1"]);
  expect(api.ocrStart).toHaveBeenCalledTimes(1);
  expect(saves(api, "create")[0]).toMatchObject({ id: "ocr-1", incomplete: true });
  fireEvent.click(resume);
  await waitFor(() => expect(saves(api, "create")).toHaveLength(2));
});

it.each(["stop", "close"])("ignores late image preparation after %s instead of starting a native request", async (operation) => {
  const prepared = deferred<string>();
  vi.mocked(ocrImage.prepareOcrImage).mockReturnValueOnce(prepared.promise);
  const { api, controller } = await mount();
  select([png("page1.png"), png("page2.png")]);
  fireEvent.click(screen.getByRole("button", { name: "开始批量识别" }));
  await waitFor(() => expect(ocrImage.prepareOcrImage).toHaveBeenCalledTimes(1));
  let closing: Promise<void> | undefined;
  if (operation === "stop") fireEvent.click(screen.getByRole("button", { name: "停止识别" }));
  else await act(async () => { closing = controller.close(); });
  await act(async () => prepared.resolve("data:image/png;base64,AQ=="));
  if (closing) await act(async () => closing);
  expect(api.ocrStart).not.toHaveBeenCalled();
  expect(api.chatNext).not.toHaveBeenCalled();
  expect(ocrImage.prepareOcrImage).toHaveBeenCalledTimes(1);
  expect(controller.getSnapshot().ocr_batch_active).toBe(false);
});

it("continues the sequential batch across page navigation without cancelling or replaying", async () => {
  const first = deferred<ChatBatch>();
  const second = deferred<ChatBatch>();
  const { api } = await mount({ chatNext: vi.fn(async (id) => id === "ocr-1" ? first.promise : second.promise) });
  select([png("page1.png"), png("page2.png")]);
  fireEvent.click(screen.getByRole("button", { name: "开始批量识别" }));
  await waitFor(() => expect(api.chatNext).toHaveBeenCalledWith("ocr-1"));
  fireEvent.click(screen.getByRole("button", { name: "概览" }));
  expect(api.chatCancel).not.toHaveBeenCalled();
  await act(async () => first.resolve(done("ocr-1")));
  await waitFor(() => expect(api.chatNext).toHaveBeenCalledWith("ocr-2"));
  fireEvent.click(screen.getByRole("button", { name: "图片 OCR" }));
  expect(screen.getByRole("region", { name: "OCR 图片队列" })).toBeVisible();
  await act(async () => second.resolve(done("ocr-2")));
  await waitFor(() => expect(saves(api, "create")).toHaveLength(2));
  expect(api.ocrStart).toHaveBeenCalledTimes(2);
  expect(api.chatCancel).not.toHaveBeenCalled();
});

it("closes only after cancelling the current image and persisting its partial terminal without starting remaining images", async () => {
  const terminal = deferred<ChatBatch>();
  const persisted = deferred<OcrHistoryList>();
  const { api, controller } = await mount({ chatNext: vi.fn(async () => terminal.promise), ocrHistorySave: vi.fn(() => persisted.promise) });
  select([png("page1.png"), png("page2.png")]);
  fireEvent.click(screen.getByRole("button", { name: "开始批量识别" }));
  await waitFor(() => expect(api.chatNext).toHaveBeenCalledWith("ocr-1"));
  let closing!: Promise<void>;
  await act(async () => { closing = controller.close(); });
  await waitFor(() => expect(api.chatCancel).toHaveBeenCalledWith("ocr-1"));
  expect(api.close).not.toHaveBeenCalled();
  await act(async () => terminal.resolve(cancelled("ocr-1", "saved before close")));
  await waitFor(() => expect(saves(api, "create")).toHaveLength(1));
  expect(saves(api, "create")[0]).toMatchObject({ id: "ocr-1", image_name: "page1.png", markdown: "saved before close", status: "cancelled", incomplete: true });
  expect(api.close).not.toHaveBeenCalled();
  await act(async () => persisted.resolve({ capacity: 100 as const, entries: [] }));
  await act(async () => closing);
  expect(api.close).toHaveBeenCalledTimes(1);
  expect(api.ocrStart).toHaveBeenCalledTimes(1);
  expect(ocrImage.prepareOcrImage).toHaveBeenCalledTimes(1);
});

it("keeps equal filenames as independent queue entries and preserves their individual File objects", async () => {
  const { api } = await mount();
  const first = png("same.png"), second = png("same.png"), third = png("same.png");
  select([first, second, third]);
  const queue = await screen.findByRole("region", { name: "OCR 图片队列" });
  const middle = within(queue).getByRole("listitem", { name: "队列图片 2：same.png" });
  fireEvent.click(within(middle).getByRole("button", { name: "移除 same.png" }));
  fireEvent.click(screen.getByRole("button", { name: "开始批量识别" }));
  await waitFor(() => expect(saves(api, "create")).toHaveLength(2));
  expect(ocrImage.prepareOcrImage).toHaveBeenNthCalledWith(1, first, 0);
  expect(ocrImage.prepareOcrImage).toHaveBeenNthCalledWith(2, third, 0);
  expect(saves(api, "create").map((row) => row.id)).toEqual(["ocr-1", "ocr-2"]);
});

it("does not treat an intermediate text chunk as permission to prepare the next image", async () => {
  const terminal = deferred<ChatBatch>();
  const next = vi.fn<DesktopApi["chatNext"]>()
    .mockResolvedValueOnce({ request_id: "ocr-1", terminal: false, events: [{ type: "delta", text: "first chunk" }] })
    .mockReturnValueOnce(terminal.promise)
    .mockImplementation(async (id) => done(id));
  const { api } = await mount({ chatNext: next });
  select([png("page1.png"), png("page2.png")]);
  fireEvent.click(screen.getByRole("button", { name: "开始批量识别" }));
  await waitFor(() => expect(next).toHaveBeenCalledTimes(2));
  expect(screen.getByRole("region", { name: "识别结果内容" })).toHaveTextContent("first chunk");
  expect(ocrImage.prepareOcrImage).toHaveBeenCalledTimes(1);
  expect(api.ocrStart).toHaveBeenCalledTimes(1);
  expect(api.ocrHistorySave).not.toHaveBeenCalled();
  await act(async () => terminal.resolve(done("ocr-1", " final chunk")));
  await waitFor(() => expect(saves(api, "create")).toHaveLength(2));
  expect(saves(api, "create")[0].markdown).toBe("first chunk final chunk");
});

it("pauses when the terminal body cannot be saved and does not replay the finished image", async () => {
  const { api } = await mount({ ocrHistorySave: vi.fn().mockRejectedValueOnce(new Error("disk unavailable")).mockResolvedValue({ capacity: 100 as const, entries: [] }) });
  select([png("page1.png"), png("page2.png")]);
  fireEvent.click(screen.getByRole("button", { name: "开始批量识别" }));
  await screen.findByText(/本张正文未能保存，批量已暂停/);
  expect(api.ocrStart).toHaveBeenCalledTimes(1);
  expect(ocrImage.prepareOcrImage).toHaveBeenCalledTimes(1);
  expect(screen.getByRole("region", { name: "识别结果内容" })).toHaveTextContent("ocr-1");
  const resume = screen.getByRole("button", { name: "继续未开始图片" });
  await waitFor(() => expect(resume).toBeEnabled());
  fireEvent.click(resume);
  await waitFor(() => expect(api.ocrStart).toHaveBeenCalledTimes(2));
  await waitFor(() => expect(saves(api, "create")).toHaveLength(2));
  expect(vi.mocked(ocrImage.prepareOcrImage).mock.calls.map(([file]) => file.name)).toEqual(["page1.png", "page2.png"]);
});

it("rechecks the frozen model after preparation and preserves the unstarted image until explicit continuation", async () => {
  const prepared = deferred<string>();
  vi.mocked(ocrImage.prepareOcrImage).mockReturnValueOnce(prepared.promise);
  const { api, controller } = await mount();
  select([png("page1.png"), png("page2.png")]);
  fireEvent.click(screen.getByRole("button", { name: "开始批量识别" }));
  await waitFor(() => expect(ocrImage.prepareOcrImage).toHaveBeenCalledTimes(1));
  vi.mocked(api.snapshot).mockImplementation(async () => {
    const value = snapshot(); value.runtime!.selected_model = "other-model"; value.runtime!.load_options!.context_size = 8192; return value;
  });
  await act(async () => prepared.resolve("data:image/png;base64,AQ=="));
  await screen.findByText(/冻结的 OCR 模型状态已变化，批量已暂停/);
  const first = screen.getByRole("listitem", { name: "队列图片 1：page1.png" });
  expect(first).toHaveTextContent("待开始");
  expect(api.ocrStart).not.toHaveBeenCalled();
  expect(api.loadModel).not.toHaveBeenCalled();
  expect(controller.getSnapshot().ocr_batch_active).toBe(false);
  vi.mocked(api.snapshot).mockImplementation(async () => {
    const value = snapshot(); value.runtime!.load_options!.context_size = 8192; return value;
  });
  await act(async () => controller.refresh());
  expect(api.ocrStart).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "继续未开始图片" }));
  await waitFor(() => expect(saves(api, "create")).toHaveLength(2));
  expect(saves(api, "create").map((row) => row.image_name)).toEqual(["page1.png", "page2.png"]);
  expect(api.ocrStart).toHaveBeenCalledTimes(2);
});

it("stops oversized OCR text while retaining only the previously accepted partial body and never starts the next image", async () => {
  const next = vi.fn<DesktopApi["chatNext"]>()
    .mockResolvedValueOnce({ request_id: "ocr-1", terminal: false, events: [{ type: "delta", text: "保留" }] })
    .mockResolvedValueOnce({ request_id: "ocr-1", terminal: false, events: [{ type: "delta", text: "x".repeat(256 * 1024 + 1) }] })
    .mockResolvedValueOnce({ request_id: "ocr-1", terminal: true, events: [{ type: "cancelled" }] });
  const { api, controller } = await mount({ chatNext: next });
  select([png("page1.png"), png("page2.png")]);
  fireEvent.click(screen.getByRole("button", { name: "开始批量识别" }));
  await waitFor(() => expect(saves(api, "create")).toHaveLength(1));
  expect(api.chatCancel).toHaveBeenCalledWith("ocr-1");
  expect(saves(api, "create")[0]).toMatchObject({ id: "ocr-1", image_name: "page1.png", status: "cancelled", incomplete: true, markdown: "保留" });
  expect(screen.getByRole("region", { name: "识别结果内容" })).toHaveTextContent(/^保留$/);
  await waitFor(() => expect(controller.getSnapshot().ocr_batch_active).toBe(false));
  expect(screen.getByRole("button", { name: "继续未开始图片" })).toBeEnabled();
  expect(screen.getByText(/正文达到 256 KiB 上限/)).toBeVisible();
  expect(api.ocrStart).toHaveBeenCalledTimes(1);
  expect(ocrImage.prepareOcrImage).toHaveBeenCalledTimes(1);
  expect(screen.getByRole("listitem", { name: "队列图片 2：page2.png" })).toHaveTextContent("待开始");
});
