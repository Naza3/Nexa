import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import App from "../src/App";
import { DesktopController } from "../src/controller";
import { LocalValidationFeedback, ModelTestFeedback } from "../src/ModelTestFeedback";
import type { DesktopApi, LocalValidation } from "../src/types";
import { deferred, makeApi, model, snapshot } from "./fixtures";

const proof = (time = 1791104400000): LocalValidation => ({ state: "passed", load_success: true, generation_pass: true, checked_at_unix_ms: time, error_code: null });
async function mount(overrides: Partial<DesktopApi> = {}) {
  const api = makeApi({ modelsPage: vi.fn(async () => ({ data: [{ ...model, validated: false, local_validation: proof() }], generation: "generation-1", next_after: null })), ...overrides });
  const controller = new DesktopController(api);
  const rendered = render(<App controller={controller} />);
  const heading = await screen.findByRole("heading", { name: model.display_name, level: 3 });
  const article = heading.closest("article")!;
  return { api, controller, article, row: within(article), ...rendered };
}
afterEach(() => vi.useRealTimers());

describe("visible current-attempt feedback", () => {
  it("separates current loaded status from historical validated=false and previous proof", async () => {
    const { row } = await mount();
    expect(row.getByRole("button", { name: "已加载" })).toBeDisabled();
    expect(row.queryByRole("button", { name: "尝试加载" })).not.toBeInTheDocument();
    expect(row.getByText("本机基础测试通过")).toBeVisible();
    expect(row.getByText(/历史本机记录 · 不表示当前已加载或本次测试通过/)).toBeVisible();
    expect(row.getByText(/历史矩阵验证记录：未实测/)).not.toBeVisible();
    expect(row.getByText(/记录时间/)).toBeVisible();
    expect(row.getByRole("button", { name: `测试 ${model.display_name}` })).toBeEnabled();
  });

  it.each([false, true])("uses the same honest load label for eligible unloaded candidates (historical validated=%s)", async (validated) => {
    const value = snapshot(); value.runtime = { ...value.runtime!, state: "unloaded", selected_model: null, load_options: null };
    const { row } = await mount({ snapshot: vi.fn(async () => value), modelsPage: vi.fn(async () => ({ data: [{ ...model, validated, local_validation: null }], generation: "g", next_after: null })) });
    expect(row.getByRole("button", { name: "加载模型" })).toBeEnabled();
    expect(row.getByText(/本机记录未提供/)).toBeVisible();
    expect(row.queryByRole("button", { name: "尝试加载" })).not.toBeInTheDocument();
  });

  it("shows each repeated test's new running feedback and completion time directly on its row", async () => {
    let evidence = proof(); let next = deferred<LocalValidation>();
    const { row, article, api } = await mount({ modelsPage: vi.fn(async () => ({ data: [{ ...model, local_validation: evidence }], generation: "g", next_after: null })), testModel: vi.fn(() => next.promise) });
    const seen = [];
    const now = vi.spyOn(Date, "now");
    for (let index = 1; index <= 2; index++) {
      now.mockReturnValue(1791104400000 + index * 60000);
      fireEvent.click(row.getByRole("button", { name: `测试 ${model.display_name}` }));
      expect(row.getByText("本次正在进行基础测试")).toBeVisible();
      expect(row.getByRole("button", { name: `测试 ${model.display_name}` })).toBeDisabled();
      const feedback = within(row.getByLabelText("本次模型测试"));
      expect(feedback.getByText(/已用时/)).toBeVisible();
      expect(article.querySelector(".model-test-feedback .spinner")).not.toBeNull();
      evidence = proof(1791104400000 + index * 60000);
      await act(async () => { next.resolve(evidence); });
      await waitFor(() => expect(feedback.getByText("本次基础测试通过")).toBeVisible());
      expect(feedback.getByText(/本次耗时/)).toBeVisible();
      seen.push(article.querySelector(".model-test-feedback time")!.getAttribute("datetime"));
      next = deferred<LocalValidation>();
    }
    expect(seen[0]).not.toBe(seen[1]);
    expect(api.testModel).toHaveBeenCalledTimes(2);
  });

  it("updates elapsed time while the native test is pending", () => {
    vi.useFakeTimers(); vi.setSystemTime(1791104400000);
    render(<ModelTestFeedback attempt={{ id: 1, model_id: model.id, model_signature: "test", mode: "test", phase: "running", started_at: Date.now(), finished_at: null, result: null, error: null }} />);
    expect(screen.getByText(/已用时 0.0 秒/)).toBeVisible();
    act(() => vi.advanceTimersByTime(2500));
    expect(screen.getByText(/已用时 2.5 秒/)).toBeVisible();
  });

  it.each(["validation_record_write_failed", "validation_record_read_failed"])("shows %s on the model row above historical proof without leaking raw error text", async (code) => {
    const { row } = await mount({ testModel: vi.fn(async () => { throw { code, message: "C:\\secret-path\\token generated-body" }; }) });
    fireEvent.click(row.getByRole("button", { name: `测试 ${model.display_name}` }));
    const feedback = within(await row.findByLabelText("本次模型测试"));
    await waitFor(() => expect(feedback.getByText(`本次诊断码：${code}`)).toBeVisible());
    expect(feedback.queryByText("本次基础测试通过")).not.toBeInTheDocument();
    expect(feedback.getByText(code.endsWith("write_failed") ? "本次测试记录无法保存" : "本次测试记录无法读取")).toBeVisible();
    expect(row.getByText("本机基础测试通过")).toBeVisible();
    expect(screen.queryByText(/secret-path|generated-body/)).not.toBeInTheDocument();
  });

  it.each(["unavailable", "invalid"])("keeps an %s history visible as a record problem instead of untested", async (state) => {
    const evidence = state === "unavailable" ? { ...proof(), state: "unavailable" as const, load_success: false, generation_pass: false, error_code: "validation_record_read_failed" } : { ...proof(), checked_at_unix_ms: Number.MAX_SAFE_INTEGER };
    const { row } = await mount({ modelsPage: vi.fn(async () => ({ data: [{ ...model, local_validation: evidence }], generation: "g", next_after: null })) });
    expect(row.getByText("本机测试记录不可用")).toBeVisible();
    expect(row.queryByText("本机待测试")).not.toBeInTheDocument();
    expect(row.queryByText("本机基础测试通过")).not.toBeInTheDocument();
    expect(row.getByText(/不能视为未测试/)).toBeVisible();
  });

  it("explains a disabled test button when another runtime request is active", async () => {
    const value = snapshot(); value.runtime = { ...value.runtime!, state: "generating", active_request: "other-client" };
    const { row, api } = await mount({ snapshot: vi.fn(async () => value) });
    const button = row.getByRole("button", { name: `测试 ${model.display_name}` });
    expect(button).toBeDisabled();
    expect(row.getByText("当前有任务进行中，空闲后可基础测试")).toBeVisible();
    fireEvent.click(button);
    expect(api.testModel).not.toHaveBeenCalled();
  });

  it("shows a native Deferred as this attempt while preserving the older receipt label", async () => {
    const result: LocalValidation = { ...proof(), state: "deferred", load_success: false, generation_pass: false, error_code: "runtime_busy" };
    const { row } = await mount({ testModel: vi.fn(async () => result) });
    fireEvent.click(row.getByRole("button", { name: `测试 ${model.display_name}` }));
    await waitFor(() => expect(row.getByText("本次基础测试已暂缓")).toBeVisible());
    expect(row.getByText("本次诊断码：runtime_busy")).toBeVisible();
    expect(row.getByText("本机基础测试通过")).toBeVisible();
  });

  it.each(["settings", "download", "close", "unmount"])("does not publish late completion after %s", async (destination) => {
    const next = deferred<LocalValidation>();
    const { row, controller, unmount } = await mount({ testModel: vi.fn(() => next.promise) });
    fireEvent.click(row.getByRole("button", { name: `测试 ${model.display_name}` }));
    if (destination === "settings") fireEvent.click(screen.getByRole("button", { name: "设置" }));
    if (destination === "download") fireEvent.click(screen.getByRole("button", { name: "下载模型" }));
    if (destination === "close") fireEvent.click(screen.getByRole("button", { name: "关闭应用" }));
    if (destination === "unmount") unmount();
    await act(async () => next.resolve(proof()));
    expect(controller.getSnapshot().model_tests[model.id]).toBeUndefined();
    expect(controller.getSnapshot().notice).toBeNull();
    expect(screen.queryByText("本次基础测试通过")).not.toBeInTheDocument();
  });

  it("defensively displays a malformed shared add/download record without raw text or date crashes", () => {
    render(<LocalValidationFeedback value={{ ...proof(), error_code: "C:\\private-path\\token", checked_at_unix_ms: Number.MAX_SAFE_INTEGER }} />);
    expect(screen.getByText("本机测试记录不可用")).toBeVisible();
    expect(screen.getByText("本机测试诊断码：validation_record_invalid")).toBeVisible();
    expect(screen.queryByText(/private-path/)).not.toBeInTheDocument();
  });
});
