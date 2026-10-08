import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PerformancePage } from "../src/PerformancePage";
import { filterPerformance, performanceCsv, tokenRate, validatePerformance } from "../src/performance";
import { createPreviewApi, PREVIEW_PERFORMANCE } from "../src/preview";
import { deferred, makeApi } from "./fixtures";
import type { PerformanceSnapshot } from "../src/types";
import App from "../src/App";
import { DesktopController } from "../src/controller";
import { snapshot as desktopSnapshot } from "./fixtures";

const sample = () => structuredClone(PREVIEW_PERFORMANCE);
afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); window.history.replaceState({}, "", "/"); });

describe("performance measurements", () => {
  it("uses phase microseconds, excluding queue/load/execution/callback from denominators", () => {
    const row = sample().records[0];
    expect(tokenRate(row, "prefill")).toBe(600);
    expect(tokenRate(row, "decode")).toBeCloseTo(256 / 3);
    row.timings = { queue_ms: 999999, load_ms: 999999, execution_ms: 999999 };
    row.performance!.timings.output_callback_us = 999999;
    expect(tokenRate(row, "decode")).toBeCloseTo(256 / 3);
    row.performance!.timings.decode_us = 1;
    expect(tokenRate(row, "decode")).toBe(256000000);
  });
  it("keeps zero-time, missing, failed and cancelled speed unavailable", () => {
    const row = sample().records[0];
    row.performance!.timings.decode_us = 0;
    expect(tokenRate(row, "decode")).toBeNull();
    row.performance = null;
    expect(tokenRate(row, "prefill")).toBeNull();
    for (const status of ["failed", "cancelled"] as const) {
      const terminal = { ...sample().records[0], status };
      expect(tokenRate(terminal, "prefill")).toBeNull();
      expect(tokenRate(terminal, "decode")).toBeNull();
    }
    row.performance = sample().records[0].performance;
    row.usage.completion_tokens = 0;
    expect(tokenRate(row, "decode")).toBe(0);
  });
  it("validates bounded safe integer metrics, shape and newest-first unique sequences", () => {
    expect(validatePerformance(sample())).toEqual(sample());
    for (const invalid of [-1, 0.5, Infinity, NaN, Number.MAX_SAFE_INTEGER + 1, undefined]) {
      const data = sample();
      data.records[0].performance!.timings.decode_us = invalid as number;
      expect(() => validatePerformance(data)).toThrow();
    }
    for (const mutate of [
      (s: PerformanceSnapshot) => { s.capacity = 201; },
      (s: PerformanceSnapshot) => { s.records[1].sequence = s.records[0].sequence; },
      (s: PerformanceSnapshot) => { s.records[0].usage.prompt_tokens = -1; },
      (s: PerformanceSnapshot) => { s.records[0].performance!.load_options.threads = NaN; },
      (s: PerformanceSnapshot) => { s.records[0].timings.load_ms = -1; },
      (s: PerformanceSnapshot) => { s.records[0].error_code = "private/path"; },
      (s: PerformanceSnapshot) => { s.records[5].sequence = 0; },
      (s: PerformanceSnapshot) => { s.records[0].max_output_tokens = 0; },
      (s: PerformanceSnapshot) => { s.records[0].usage.completion_tokens = 257; },
      (s: PerformanceSnapshot) => { s.records[0].timings.queue_ms = Number.MAX_SAFE_INTEGER; },
      (s: PerformanceSnapshot) => { s.records[0].performance!.timings.decode_us = Number.MAX_SAFE_INTEGER; },
      (s: PerformanceSnapshot) => { s.records[0].status = "failed"; },
      (s: PerformanceSnapshot) => { s.records[0].error_code = "cancelled"; },
      (s: PerformanceSnapshot) => { s.records[0].finish_reason = null; },
      (s: PerformanceSnapshot) => { s.records[5].error_code = null; },
      (s: PerformanceSnapshot) => { s.records[0].performance!.load_options.context_size = 31; },
      (s: PerformanceSnapshot) => { s.records[0].performance!.load_options.threads = 257; },
      (s: PerformanceSnapshot) => { s.records[0].performance!.load_options.context_size = 32; },
      (s: PerformanceSnapshot) => { s.records[0].usage.prompt_tokens = 0; },
    ]) { const data = sample(); mutate(data); expect(() => validatePerformance(data)).toThrow(); }
    expect(validatePerformance(sample()).records[5].performance).toBeNull();
  });
  it("filters records and exports explicit CSV units with blank missing fields and formula protection", () => {
    const data = sample();
    const rows = filterPerformance(data.records, "模拟-GLM-OCR", "completed", "image");
    expect(rows.map((r) => r.sequence)).toEqual([6, 4]);
    data.records[0].request_id = '=SUM(1,2)"';
    const csv = performanceCsv(data, [data.records[0], data.records[5]]);
    expect(csv).toContain('"prefill_us","decode_us","output_callback_us"');
    expect(csv).toContain('"\'=SUM(1,2)"""');
    expect(csv).toContain('"2000000","3000000","20000","600"');
    expect(csv.split("\r\n")[2]).not.toContain('"600"');
    expect(csv).not.toMatch(/Infinity|NaN|undefined|null/);
  });
  it("only supplies synthetic records in the dedicated preview scenario", async () => {
    expect((await createPreviewApi().performanceGet!()).records).toHaveLength(0);
    window.history.replaceState({}, "", "/?scenario=performance");
    const result = await createPreviewApi().performanceGet!();
    expect(result.instance_id).toContain("模拟数据");
    expect(validatePerformance(result).records).toHaveLength(6);
  });
});

describe("performance page", () => {
  it("keeps performance accessible and refreshable while another client generates", async () => {
    const value = desktopSnapshot();
    value.runtime!.state = "generating";
    value.runtime!.active_request = "external-client";
    const query = vi.fn().mockResolvedValue(sample());
    const api = makeApi({ snapshot: async () => value, performanceGet: query });
    render(<App controller={new DesktopController(api)} initialPage="performance" />);
    await screen.findByRole("button", { name: "查看 #6" });
    expect(screen.getByRole("button", { name: "刷新性能" })).toBeEnabled();
    const before = query.mock.calls.length;
    fireEvent.click(screen.getByRole("button", { name: "刷新性能" }));
    await waitFor(() => expect(query).toHaveBeenCalledTimes(before + 1));
    expect(api.chatCancel).not.toHaveBeenCalled();
    expect(api.stop).not.toHaveBeenCalled();
  });
  it("shows real parameters, cap reached, filters, and copies only filtered records", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    render(<PerformancePage api={makeApi({ performanceGet: async () => sample() })} connection="connected" />);
    await screen.findByRole("button", { name: "查看 #6" });
    expect(screen.getByText("成功 · 输出达上限")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "查看 #6" }));
    expect(within(screen.getByRole("region", { name: "推理详情" })).getByText("上下文 8192 · 线程 4 · 批次 256")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("性能模型"), { target: { value: "模拟-GLM-OCR" } });
    fireEvent.change(screen.getByLabelText("性能状态"), { target: { value: "completed" } });
    fireEvent.change(screen.getByLabelText("性能输入类型"), { target: { value: "image" } });
    expect(screen.getAllByRole("button", { name: /查看 #/ })).toHaveLength(2);
    fireEvent.click(screen.getByRole("button", { name: "复制 CSV" }));
    await screen.findByText("已复制筛选记录 CSV。");
    expect(writeText.mock.calls[0][0].split("\r\n")).toHaveLength(3);
    expect(writeText.mock.calls[0][0]).not.toContain("模拟-Qwen3-0.6B");
  });
  it("deduplicates active refresh and discards an old response across disconnect and service replacement", async () => {
    vi.useFakeTimers();
    const old = deferred<PerformanceSnapshot>();
    const fresh = sample(); fresh.instance_id = "new-instance"; fresh.records = [{ ...fresh.records[0], sequence: 1 }];
    const query = vi.fn().mockReturnValueOnce(old.promise).mockResolvedValue(fresh);
    const api = makeApi({ performanceGet: query });
    const { rerender } = render(<PerformancePage api={api} connection="connected" />);
    await act(async () => { await vi.advanceTimersByTimeAsync(9000); });
    expect(query).toHaveBeenCalledTimes(1);
    rerender(<PerformancePage api={api} connection="stopped" />);
    expect(screen.getByText(/服务已停止。内存记录/)).toBeInTheDocument();
    rerender(<PerformancePage api={api} connection="connected" />);
    await act(async () => {});
    expect(screen.getByRole("button", { name: "查看 #1" })).toBeInTheDocument();
    await act(async () => { old.resolve(sample()); });
    expect(screen.queryByRole("button", { name: "查看 #6" })).not.toBeInTheDocument();
    expect(screen.getByText(/当前服务 new-instance/)).toBeInTheDocument();
  });
  it("resets selected details for a new instance even if sequence and request id are reused", async () => {
    const query = vi.fn().mockResolvedValueOnce(sample());
    render(<PerformancePage api={makeApi({ performanceGet: query })} connection="connected" />);
    fireEvent.click(await screen.findByRole("button", { name: "查看 #6" }));
    const fresh = sample(); fresh.instance_id = "replacement";
    query.mockResolvedValue(fresh);
    fireEvent.click(screen.getByRole("button", { name: "刷新性能" }));
    await screen.findByText(/当前服务 replacement/);
    expect(screen.queryByRole("region", { name: "推理详情" })).not.toBeInTheDocument();
  });
  it("keeps a missing selected model explicit after retention evicts its records", async () => {
    const query = vi.fn().mockResolvedValueOnce(sample());
    render(<PerformancePage api={makeApi({ performanceGet: query })} connection="connected" />);
    await screen.findByRole("button", { name: "查看 #6" });
    fireEvent.change(screen.getByLabelText("性能模型"), { target: { value: "模拟-GLM-OCR" } });
    const fresh = sample(); fresh.records = fresh.records.filter((r) => r.model_id !== "模拟-GLM-OCR");
    query.mockResolvedValue(fresh);
    fireEvent.click(screen.getByRole("button", { name: "刷新性能" }));
    await screen.findByText("没有符合筛选条件的记录。");
    expect(screen.getByLabelText("性能模型")).toHaveDisplayValue("模拟-GLM-OCR（当前无记录）");
    fireEvent.change(screen.getByLabelText("性能模型"), { target: { value: "" } });
    expect(screen.getAllByRole("button", { name: /查看 #/ })).toHaveLength(3);
  });
  it("does not query while stopped or hidden; refreshes on return to visibility", async () => {
    const query = vi.fn().mockResolvedValue(sample());
    const api = makeApi({ performanceGet: query });
    const { rerender } = render(<PerformancePage api={api} connection="stopped" />);
    expect(query).not.toHaveBeenCalled();
    vi.spyOn(document, "visibilityState", "get").mockReturnValue("hidden");
    rerender(<PerformancePage api={api} connection="connected" />);
    expect(query).not.toHaveBeenCalled();
    vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
    fireEvent(document, new Event("visibilitychange"));
    await screen.findByRole("button", { name: "查看 #6" });
    expect(query).toHaveBeenCalledTimes(1);
  });
  it("polls serially while visible and stops polling after leaving the page", async () => {
    vi.useFakeTimers();
    const query = vi.fn().mockResolvedValue(sample());
    const { unmount } = render(<PerformancePage api={makeApi({ performanceGet: query })} connection="connected" />);
    await act(async () => {});
    expect(query).toHaveBeenCalledTimes(1);
    await act(async () => { await vi.advanceTimersByTimeAsync(3000); });
    expect(query).toHaveBeenCalledTimes(2);
    unmount();
    await act(async () => { await vi.advanceTimersByTimeAsync(9000); });
    expect(query).toHaveBeenCalledTimes(2);
  });
  it("reports unsupported, empty, malformed and failed queries without inventing records", async () => {
    const api = makeApi();
    const { rerender } = render(<PerformancePage api={api} connection="connected" />);
    expect(screen.getByText(/当前版本不支持性能记录/)).toBeInTheDocument();
    rerender(<PerformancePage api={makeApi({ performanceGet: async () => ({ instance_id: "empty", capacity: 200, records: [] }) })} connection="connected" />);
    await screen.findByText("当前服务暂无已结束的推理记录。");
    rerender(<PerformancePage api={makeApi({ performanceGet: async () => { throw { code: "performance_unsupported" }; } })} connection="connected" />);
    await screen.findByText(/请更新并重启服务/);
    const malformed = sample(); malformed.records[0].performance!.timings.prefill_us = -1;
    rerender(<PerformancePage api={makeApi({ performanceGet: async () => malformed })} connection="connected" />);
    await screen.findByText(/性能查询失败/);
    expect(screen.queryByRole("table")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "复制 CSV" })).toBeDisabled();
  });
  it("a failed refresh clears stale measurements and supports manual recovery", async () => {
    const query = vi.fn().mockResolvedValueOnce(sample()).mockRejectedValueOnce(new Error()).mockResolvedValue(sample());
    render(<PerformancePage api={makeApi({ performanceGet: query })} connection="connected" />);
    await screen.findByRole("table");
    fireEvent.click(screen.getByRole("button", { name: "刷新性能" }));
    await screen.findByText(/性能查询失败/);
    expect(screen.queryByRole("table")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "刷新性能" }));
    await waitFor(() => expect(screen.getByRole("table")).toBeInTheDocument());
  });
});
