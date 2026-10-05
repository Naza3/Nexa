import { FollowOnLoadControl } from "../src/ModelLoadControl";
import { StatusBar } from "../src/StatusBar";
import { LocalValidationFeedback } from "../src/ModelTestFeedback";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import App from "../src/App";
import { DesktopController } from "../src/controller";
import type { DesktopApi, ModelLoadOperation, DownloadOperation, LibraryOperation } from "../src/types";
import { deferred, makeApi, model, runtime, snapshot } from "./fixtures";

async function mount(overrides: Partial<DesktopApi> = {}) {
  let id = "";
  const next = deferred<ModelLoadOperation>();
  const value = snapshot(); value.runtime = { ...runtime(), state: "unloaded" };
  const api = makeApi({ snapshot: vi.fn(async () => value), loadModelStart: vi.fn(async (operation_id) => { id = operation_id; return { operation_id }; }),
    modelLoadNext: vi.fn(() => next.promise), modelLoadCancel: vi.fn(async () => ({ stopping: true })), ...overrides });
  const controller = new DesktopController(api); render(<App controller={controller} initialPage="models" />);
  await waitFor(() => expect(controller.getSnapshot().booting).toBe(false));
  const finish = async (status: "cancelled" | "completed" = "cancelled") => {
    await act(async () => next.resolve({ operation_id: id, model_id: model.id, phase: "finished", status, terminal: true,
      runtime: value.runtime, local_validation: null, error: status === "cancelled" ? { code: "request_cancelled", message: "cancelled" } : null }));
    await waitFor(() => expect(controller.getSnapshot().operation).toBeNull());
  };
  return { api, controller, next, finish, id: () => id };
}

describe("compact owned load controls", () => {
  it("keeps Stop reachable through model detail, settings and activity, then shows true neutral cancellation", async () => {
    const { api, controller, finish } = await mount();
    fireEvent.click(screen.getByRole("button", { name: "加载模型" }));
    const footer = within(screen.getByLabelText("应用状态栏"));
    await waitFor(() => expect(footer.getByRole("button", { name: "停止加载" })).toBeEnabled());
    fireEvent.click(screen.getByRole("button", { name: `查看 ${model.display_name} 的详情` }));
    expect(within(screen.getByRole("article", { name: `${model.display_name} 的详情` })).getByRole("button", { name: "停止加载" })).toBeEnabled();
    fireEvent.click(screen.getByRole("button", { name: "设置" }));
    fireEvent.click(footer.getByRole("button", { name: "停止加载" }));
    expect(footer.getByRole("button", { name: "停止中…" })).toBeDisabled();
    expect(footer.getByText("停止中 · 等待任务清理确认")).toBeVisible();
    fireEvent.click(footer.getByRole("button", { name: "查看活动" }));
    expect(screen.getByRole("heading", { name: "活动", level: 1 })).toBeVisible();
    expect(controller.getSnapshot().activities[0].status).toBe("stopping");
    await finish();
    expect(footer.queryByRole("button", { name: /停止/ })).not.toBeInTheDocument();
    expect(controller.getSnapshot().activities[0].status).toBe("cancelled");
    fireEvent.click(screen.getByRole("button", { name: "模型库" }));
    expect(screen.getByLabelText(`${model.display_name}：本次操作已停止`)).toHaveClass("neutral");
    expect(api.modelLoadCancel).toHaveBeenCalledTimes(1); expect(api.stop).not.toHaveBeenCalled();
  });
  it("shows a recoverable stop failure and keeps the original task pending", async () => {
    const { api, controller, finish } = await mount({ modelLoadCancel: vi.fn().mockRejectedValueOnce({ code: "desktop_unavailable", message: "hidden secret" }).mockResolvedValue({ stopping: true }) });
    fireEvent.click(screen.getByRole("button", { name: "加载模型" }));
    const footer = within(screen.getByLabelText("应用状态栏"));
    await waitFor(() => expect(api.modelLoadNext).toHaveBeenCalledTimes(1));
    fireEvent.click(footer.getByRole("button", { name: "停止加载" }));
    const retry = await footer.findByRole("button", { name: "重试停止" });
    expect(screen.getByRole("alert")).toHaveTextContent("停止请求尚未确认");
    expect(screen.queryByText(/hidden secret/)).not.toBeInTheDocument();
    expect(controller.getSnapshot().operation).not.toBeNull(); fireEvent.click(retry);
    await waitFor(() => expect(api.modelLoadCancel).toHaveBeenCalledTimes(2)); await finish();
  });
  it("changes the control to Stop This Test during the owned short-test phase", async () => {
    let id = ""; const last = deferred<ModelLoadOperation>();
    const { api, controller } = await mount({ loadModelStart: vi.fn(async (operation_id) => { id = operation_id; return { operation_id }; }),
      modelLoadNext: vi.fn().mockImplementationOnce(async () => ({ operation_id: id, model_id: model.id, phase: "testing", status: "running", terminal: false, runtime: runtime(), local_validation: null, error: null })).mockImplementationOnce(() => last.promise) });
    fireEvent.click(screen.getByRole("button", { name: "加载模型" }));
    const footer = within(screen.getByLabelText("应用状态栏"));
    await footer.findByRole("button", { name: "停止本次测试" });
    expect(footer.queryByRole("button", { name: "停止加载" })).not.toBeInTheDocument();
    expect(footer.getByText("加载完成 · 正在基础测试")).toBeVisible();
    fireEvent.click(footer.getByRole("button", { name: "停止本次测试" }));
    await waitFor(() => expect(api.modelLoadNext).toHaveBeenCalledTimes(2));
    await act(async () => last.resolve({ operation_id: id, model_id: model.id, phase: "finished", status: "cancelled", terminal: true, runtime: runtime(), local_validation: null, error: { code: "request_cancelled", message: "cancelled" } }));
    await waitFor(() => expect(controller.getSnapshot().operation).toBeNull());
    expect(api.chatCancel).not.toHaveBeenCalled(); expect(api.unloadModel).not.toHaveBeenCalled();
  });
  it("offers both Stop and status recovery when polling fails, without a new load", async () => {
    let id = ""; const last = deferred<ModelLoadOperation>();
    const { api, controller } = await mount({ loadModelStart: vi.fn(async (operation_id) => { id = operation_id; return { operation_id }; }),
      modelLoadNext: vi.fn().mockRejectedValueOnce({ code: "model_load_interrupted", message: "private" }).mockImplementationOnce(() => last.promise) });
    fireEvent.click(screen.getByRole("button", { name: "加载模型" }));
    const footer = within(screen.getByLabelText("应用状态栏"));
    expect(await footer.findByRole("button", { name: "重新确认加载任务" })).toBeEnabled();
    expect(footer.getByRole("button", { name: "停止加载" })).toBeEnabled();
    fireEvent.click(footer.getByRole("button", { name: "停止加载" }));
    await waitFor(() => expect(api.modelLoadCancel).toHaveBeenCalledExactlyOnceWith(id));
    expect(controller.getSnapshot().operation).not.toBeNull();
    await waitFor(() => expect(api.modelLoadNext).toHaveBeenCalledTimes(2));
    await act(async () => last.resolve({ operation_id: id, model_id: model.id, phase: "finished", status: "cancelled", terminal: true, runtime: null, local_validation: null, error: { code: "request_cancelled", message: "cancelled" } }));
    await waitFor(() => expect(controller.getSnapshot().operation).toBeNull()); expect(api.loadModelStart).toHaveBeenCalledTimes(1);
  });
  it("does not offer Stop for another client's loading or generation", async () => {
    const value = snapshot(); value.runtime!.state = "loading";
    const { api } = await mount({ snapshot: vi.fn(async () => value) });
    expect(within(screen.getByLabelText("应用状态栏")).queryByRole("button", { name: /停止加载|停止本次测试/ })).not.toBeInTheDocument();
    expect(api.modelLoadCancel).not.toHaveBeenCalled();
  });
});


describe("owned auto-load phases from add and download tasks", () => {
  it.each(["library", "download"] as const)("%s uses the same task cancel with phase-accurate controls", (kind) => {
    const controller = new DesktopController(makeApi());
    const cancel = vi.spyOn(controller, kind === "library" ? "cancelLibrary" : "cancelDownload").mockResolvedValue();
    const other = vi.spyOn(controller, kind === "library" ? "cancelDownload" : "cancelLibrary").mockResolvedValue();
    const state = { ...controller.getSnapshot(), booting: false, snapshot: snapshot(),
      [kind]: { phase: "testing", load_phase: "preparing", operation_id: "owned-auto" } as LibraryOperation | DownloadOperation,
      [`${kind}_phase`]: "running" as const };
    const { rerender } = render(<StatusBar state={state} controller={controller} goActivity={() => {}} />);
    expect(screen.getByText("正在准备并校验模型文件")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "停止加载" }));
    expect(cancel).toHaveBeenCalledTimes(1); expect(other).not.toHaveBeenCalled();
    const task = kind === "library" ? state.library! : state.download!;
    rerender(<StatusBar state={{ ...state, [kind]: { ...task, load_phase: "testing" } }} controller={controller} goActivity={() => {}} />);
    expect(screen.getByText("加载完成 · 正在基础测试")).toBeVisible();
    expect(screen.queryByRole("button", { name: "停止加载" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "停止本次测试" })); expect(cancel).toHaveBeenCalledTimes(2);
    rerender(<FollowOnLoadControl state={{ ...state, [`${kind}_phase`]: "stopping" }} controller={controller} />);
    expect(screen.getByRole("button", { name: "停止中…" })).toBeDisabled();
    rerender(<FollowOnLoadControl state={{ ...state, [`${kind}_phase`]: "idle" }} controller={controller} />);
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });
  it("a cancelled Deferred follow-on result is neutral, with saved data preserved", () => {
    render(<LocalValidationFeedback value={{ state: "deferred", load_success: false, generation_pass: false, error_code: "request_cancelled", checked_at_unix_ms: 1 }} />);
    expect(screen.getByText("本次加载或测试已停止")).toBeVisible();
    expect(screen.getByText(/已保存和登记的文件保留/)).toBeVisible();
    expect(document.querySelector(".local-validation-record")).toHaveClass("test-neutral");
    expect(screen.queryByText(/测试失败/)).not.toBeInTheDocument();
  });
});
