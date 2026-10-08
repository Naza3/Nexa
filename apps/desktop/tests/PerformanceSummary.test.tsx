import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { readRequestPerformance } from "../src/performance";
import type { PerformanceIdentity } from "../src/performance";
import { PerformanceSummary } from "../src/PerformanceSummary";
import { DesktopController } from "../src/controller";
import App from "../src/App";
import { OcrPage } from "../src/OcrPage";
import * as ocrImage from "../src/ocrImage";
import { deferred, makeApi, model, snapshot } from "./fixtures";
import type { ChatBatch, ChatEvent, PerformanceRecord, PerformanceSnapshot } from "../src/types";

const usage = { prompt_tokens: 120, completion_tokens: 40, total_tokens: 160 };
function record(patch: Partial<PerformanceRecord> = {}): PerformanceRecord {
  return { sequence: 1, request_id: "request-1", model_id: model.id, modality: "text", status: "completed", accepted_at_unix_ms: 123456, max_output_tokens: 512, usage: { prompt_tokens: 120, completion_tokens: 40 }, timings: { queue_ms: 99, load_ms: 99, execution_ms: 3000 }, performance: { timings: { prepare_us: 100, prefill_us: 1000000, decode_us: 2000000, output_callback_us: 1000 }, load_options: { context_size: 8192, batch_size: 256, threads: 4 } }, error_code: null, finish_reason: "stop", ...patch };
}
function history(rows = [record()], instance_id = "instance-1"): PerformanceSnapshot { return { instance_id, capacity: 200, records: rows }; }
function identity(patch: Partial<PerformanceIdentity> = {}): PerformanceIdentity { return { instance_id: "instance-1", request_id: "request-1", model_id: model.id, modality: "text", status: "completed", max_output_tokens: 512, usage, finish_reason: "stop", ...patch }; }
function terminal(event: ChatEvent = { type: "completed", finish_reason: "stop", usage }, patch: Partial<ChatBatch> = {}): ChatBatch { return { request_id: "request-1", runtime_instance_id: "instance-1", events: [{ type: "delta", text: "# 原始结果" }, event], terminal: true, ...patch }; }
async function chat(api = makeApi()) { const controller = new DesktopController(api); await controller.refresh(); await controller.send("hello"); await waitFor(() => expect(controller.getSnapshot().chat_phase).toBe("idle")); return controller; }
afterEach(() => vi.restoreAllMocks());

describe("terminal performance association", () => {
  it("matches the unique owned request, not the newest record", async () => {
    const result = await readRequestPerformance(makeApi({ performanceGet: async () => history([record({ sequence: 2, request_id: "other" }), record()]) }), identity());
    expect(result).toEqual({ state: "ready", record: record() });
  });
  it.each([
    ["instance", { instance_id: "new-instance" }], ["request", { request_id: "other" }], ["model", { model_id: "other" }], ["modality", { modality: "image" }], ["status", { status: "failed" }], ["budget", { max_output_tokens: 2048 }], ["finish", { finish_reason: "length" }], ["prompt usage", { usage: { ...usage, prompt_tokens: 121 } }], ["output usage", { usage: { ...usage, completion_tokens: 41 } }], ["total usage", { usage: { ...usage, total_tokens: 161 } }],
  ] as [string, Partial<PerformanceIdentity>][]) ("rejects mismatched %s", async (_, patch) => {
    expect(await readRequestPerformance(makeApi({ performanceGet: async () => history() }), identity(patch))).toEqual({ state: "unavailable" });
  });
  it("rejects reused IDs even when only one row matches the model", async () => {
    const rows = [record({ sequence: 2, model_id: "other" }), record()];
    expect(await readRequestPerformance(makeApi({ performanceGet: async () => history(rows) }), identity())).toEqual({ state: "unavailable" });
  });
  it("quietly handles old bridges, eviction, malformed metrics and failed reads", async () => {
    const query = vi.fn().mockResolvedValue(history());
    expect(await readRequestPerformance(makeApi({ performanceGet: query }), identity({ instance_id: undefined }))).toEqual({ state: "unavailable" });
    expect(query).not.toHaveBeenCalled();
    expect(await readRequestPerformance(makeApi(), identity())).toEqual({ state: "unavailable" });
    const invalid = record(); invalid.performance!.timings.decode_us = -1;
    for (const performanceGet of [async () => history([]), async () => history([invalid]), async () => { throw new Error("offline"); }]) {
      expect(await readRequestPerformance(makeApi({ performanceGet }), identity())).toEqual({ state: "unavailable" });
    }
  });
  it.each(["failed", "cancelled"] as const)("retains the real %s terminal with unavailable speeds", async (status) => {
    const row = record({ status, performance: null, finish_reason: null, error_code: status });
    const value = await readRequestPerformance(makeApi({ performanceGet: async () => history([row]) }), identity({ status }));
    render(<PerformanceSummary value={value} />);
    expect(screen.getByRole("region", { name: "本次推理性能" })).toHaveTextContent("Prefill 不可用 · 不可用");
    expect(screen.getByRole("region", { name: "本次推理性能" })).toHaveTextContent("Decode 不可用 · 不可用");
    expect(screen.getByText("120 / 40")).toBeInTheDocument();
  });
  it("uses actual phase denominators and separates execution from queue/load", () => {
    const row = record();
    const { rerender } = render(<PerformanceSummary value={{ state: "ready", record: row }} image />);
    expect(screen.getByText("120.00 token/s")).toBeInTheDocument();
    expect(screen.getByText("20.00 token/s")).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "本次推理性能" })).toHaveTextContent("总执行耗时 3.000 s");
    expect(screen.getByText(/含图片编码/)).toBeInTheDocument();
    row.performance!.timings.prefill_us = 0;
    row.performance!.timings.decode_us = 0;
    rerender(<PerformanceSummary value={{ state: "ready", record: row }} />);
    expect(screen.getByRole("region", { name: "本次推理性能" })).not.toHaveTextContent("token/s");
    expect(screen.getByRole("region", { name: "本次推理性能" })).not.toHaveTextContent("Infinity");
  });
});

describe("chat inline metrics", () => {
  it("releases chat before the metric read resolves and binds a late result to its original message", async () => {
    const old = deferred<PerformanceSnapshot>();
    const query = vi.fn().mockReturnValueOnce(old.promise).mockResolvedValueOnce(history([record({ request_id: "request-2", performance: null })]));
    const api = makeApi({ performanceGet: query, chatNext: vi.fn(async () => terminal()) });
    const controller = await chat(api);
    expect(controller.getSnapshot().messages[1].performance).toEqual({ state: "pending" });
    vi.mocked(api.chatStart).mockResolvedValueOnce({ request_id: "request-2" });
    vi.mocked(api.chatNext).mockResolvedValueOnce(terminal(undefined, { request_id: "request-2" }));
    await controller.send("again");
    await waitFor(() => expect(controller.getSnapshot().messages[3]?.performance?.state).toBe("ready"));
    old.resolve(history());
    await waitFor(() => expect(controller.getSnapshot().messages[1].performance?.state).toBe("ready"));
    expect(controller.getSnapshot().messages[3].performance).toEqual({ state: "ready", record: record({ request_id: "request-2", performance: null }) });
    expect(query).toHaveBeenCalledTimes(2);
  });
  it.each(["clear", "remove"])("does not resurrect a message after %s", async (action) => {
    const pending = deferred<PerformanceSnapshot>();
    const event: ChatEvent = { type: "cancelled" };
    const controller = await chat(makeApi({ performanceGet: () => pending.promise, chatNext: async () => terminal(event) }));
    if (action === "clear") await controller.clear(); else controller.removeIncomplete();
    pending.resolve(history([record({ status: "cancelled", performance: null, finish_reason: null, error_code: "cancelled" })]));
    await act(async () => {});
    expect(controller.getSnapshot().messages).toEqual([]);
  });
  it("keeps successful text and terminal usage if metrics fail, below Markdown content", async () => {
    const query = vi.fn().mockRejectedValue(new Error("offline"));
    const controller = new DesktopController(makeApi({ performanceGet: query, chatNext: async () => terminal() }));
    render(<App controller={controller} initialPage="chat" />);
    await waitFor(() => expect(controller.getSnapshot().booting).toBe(false));
    await act(async () => { await controller.send("hello"); });
    const summary = await screen.findByRole("region", { name: "本次推理性能" });
    await waitFor(() => expect(summary).toHaveTextContent("本次性能记录不可用"));
    const response = screen.getByRole("article", { name: "Nexa 回复" });
    const heading = within(response).getByRole("heading", { name: "原始结果" });
    expect(heading.compareDocumentPosition(summary) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(summary).toHaveTextContent("120 / 40");
    expect(controller.getSnapshot().messages[1].state).toBe("complete");
    expect(controller.getSnapshot().error).toBeNull();
  });
  it("does not read metrics without terminal identity or before a confirmed terminal", async () => {
    const next = deferred<ChatBatch>();
    const query = vi.fn().mockResolvedValue(history());
    const api = makeApi({ performanceGet: query, chatNext: () => next.promise });
    const controller = new DesktopController(api); await controller.refresh(); await controller.send("hello");
    expect(query).not.toHaveBeenCalled();
    next.resolve(terminal(undefined, { runtime_instance_id: undefined }));
    await waitFor(() => expect(controller.getSnapshot().chat_phase).toBe("idle"));
    expect(query).not.toHaveBeenCalled();
    expect(controller.getSnapshot().messages[1].performance).toEqual({ state: "unavailable" });
  });
  it("discards a pending lookup after App cleanup", async () => {
    const pending = deferred<PerformanceSnapshot>();
    const controller = new DesktopController(makeApi({ performanceGet: () => pending.promise, chatNext: async () => terminal() }));
    const cleanup = controller.mount();
    await waitFor(() => expect(controller.getSnapshot().booting).toBe(false));
    await controller.send("hello");
    await waitFor(() => expect(controller.getSnapshot().chat_phase).toBe("idle"));
    cleanup(); pending.resolve(history()); await act(async () => {});
    expect(controller.getSnapshot().messages[1].performance?.state).toBe("pending");
  });
});

async function mountOcr(performanceGet = vi.fn(async () => history([record({ modality: "image", max_output_tokens: 2048 })]))) {
  vi.spyOn(ocrImage, "prepareOcrImage").mockResolvedValue("data:image/png;base64,AQ==");
  const api = makeApi({ performanceGet, ocrStart: vi.fn(async () => ({ request_id: "request-1" })), chatNext: vi.fn(async () => terminal()), saveOcrMarkdown: vi.fn(async () => ({ saved: true })) });
  const controller = new DesktopController(api);
  const value = snapshot(); value.runtime!.load_options!.context_size = 8192;
  api.snapshot = vi.fn(async () => value); await controller.refresh();
  const state = { ...controller.getSnapshot(), booting: false, snapshot: value, models: { generation: "g", data: [{ ...model, has_projector: true }], next_after: null } };
  const view = render(<OcrPage controller={controller} state={state} />);
  fireEvent.change(screen.getByLabelText("OCR 模型"), { target: { value: model.id } });
  fireEvent.change(screen.getByLabelText("OCR 图片"), { target: { files: [new File(["png"], "image.png", { type: "image/png" })] } });
  await screen.findByAltText("待识别图片预览");
  return { ...view, api, controller, state };
}
describe("OCR inline metrics", () => {
  it("renders below both result modes, caches across view switches, and excludes metrics from exports", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined); Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    const { api } = await mountOcr();
    fireEvent.click(screen.getByRole("button", { name: "识别图片" }));
    const summary = await screen.findByRole("region", { name: "本次推理性能" });
    await waitFor(() => expect(summary).toHaveTextContent("120.00 token/s"));
    const output = screen.getByRole("region", { name: "识别结果内容" });
    expect(output).not.toContainElement(summary);
    expect(output.compareDocumentPosition(summary) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Markdown" }));
    expect(within(output).getByRole("heading", { name: "原始结果" })).toBeVisible();
    expect(summary).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "原文" }));
    fireEvent.click(screen.getByRole("button", { name: "复制识别原文" }));
    fireEvent.click(screen.getByRole("button", { name: "保存 .md" }));
    expect(writeText).toHaveBeenCalledWith("# 原始结果");
    expect(api.saveOcrMarkdown).toHaveBeenCalledWith("# 原始结果");
    expect(api.performanceGet).toHaveBeenCalledTimes(1);
  });
  it("clears the old summary for a new task and discards the old request's late read", async () => {
    const old = deferred<PerformanceSnapshot>();
    const query = vi.fn().mockReturnValueOnce(old.promise).mockResolvedValueOnce(history([record({ modality: "image", max_output_tokens: 2048, request_id: "request-2", performance: null })]));
    const { api } = await mountOcr(query);
    fireEvent.click(screen.getByRole("button", { name: "识别图片" }));
    await screen.findByText("识别完成，请核对原图。");
    const next = deferred<ChatBatch>();
    vi.mocked(api.ocrStart!).mockResolvedValueOnce({ request_id: "request-2" });
    vi.mocked(api.chatNext).mockReturnValueOnce(next.promise);
    fireEvent.click(screen.getByRole("button", { name: "识别图片" }));
    expect(screen.queryByRole("region", { name: "本次推理性能" })).not.toBeInTheDocument();
    old.resolve(history([record({ modality: "image", max_output_tokens: 2048 })])); await act(async () => {});
    expect(screen.queryByRole("region", { name: "本次推理性能" })).not.toBeInTheDocument();
    next.resolve(terminal(undefined, { request_id: "request-2" }));
    await screen.findByText("识别完成，请核对原图。");
    const summary = screen.getByRole("region", { name: "本次推理性能" });
    expect(summary).not.toHaveTextContent("120.00 token/s");
    expect(summary).toHaveTextContent("120 / 40");
  });
  it("keeps OCR completed when a performance query fails", async () => {
    await mountOcr(vi.fn().mockRejectedValue(new Error("offline")));
    fireEvent.click(screen.getByRole("button", { name: "识别图片" }));
    await screen.findByText("识别完成，请核对原图。");
    await screen.findByText(/本次性能记录不可用/);
    expect(screen.getByRole("region", { name: "识别结果内容" })).toHaveTextContent("# 原始结果");
    expect(screen.queryByText(/识别结果与停止尚未确认/)).not.toBeInTheDocument();
  });
  it.each(["failed", "cancelled"] as const)("shows unavailable phase metrics for a confirmed OCR %s", async (status) => {
    const row = record({ modality: "image", max_output_tokens: 2048, status, performance: null, finish_reason: null, error_code: status });
    const { api } = await mountOcr(vi.fn(async () => history([row])));
    vi.mocked(api.chatNext).mockResolvedValueOnce(terminal(status === "failed" ? { type: "failed", code: "failed", message: "failed" } : { type: "cancelled" }));
    fireEvent.click(screen.getByRole("button", { name: "识别图片" }));
    const summary = await screen.findByRole("region", { name: "本次推理性能" });
    await waitFor(() => expect(summary).toHaveTextContent("120 / 40"));
    expect(summary).toHaveTextContent("Decode 不可用 · 不可用");
    expect(screen.getByRole("region", { name: "识别结果内容" })).toHaveTextContent("# 原始结果");
  });
  it("quietly degrades for old OCR batches and never queries without the original instance", async () => {
    const { api } = await mountOcr();
    vi.mocked(api.chatNext).mockResolvedValueOnce(terminal(undefined, { runtime_instance_id: null }));
    fireEvent.click(screen.getByRole("button", { name: "识别图片" }));
    await screen.findByText(/本次性能记录不可用/);
    expect(api.performanceGet).not.toHaveBeenCalled();
    expect(screen.getByText("识别完成，请核对原图。")).toBeInTheDocument();
  });
  it("does not request performance for a terminal arriving after OCR unmount", async () => {
    const { api, unmount } = await mountOcr();
    const next = deferred<ChatBatch>();
    vi.mocked(api.chatNext).mockReturnValueOnce(next.promise);
    fireEvent.click(screen.getByRole("button", { name: "识别图片" }));
    await waitFor(() => expect(api.chatNext).toHaveBeenCalledTimes(1));
    unmount();
    next.resolve(terminal()); await act(async () => {});
    expect(api.performanceGet).not.toHaveBeenCalled();
  });
});
