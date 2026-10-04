import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import App from "../src/App";
import { DesktopController } from "../src/controller";
import type { ViewState } from "../src/controller";
import { AddModelProgress } from "../src/ModelAdd";
import type { DesktopApi, LibraryOperation, ModelFileSelection } from "../src/types";
import { deferred, makeApi, model, snapshot } from "./fixtures";

const selection: ModelFileSelection = { selection_id: "native-files", expires_in_seconds: 600, files: [
  { selection_index: 0, file_name: "千问 中文.gguf", size_bytes: 1024 ** 3 },
  { selection_index: 1, file_name: "千问 中文.gguf", size_bytes: 2 * 1024 ** 3 },
  { selection_index: 2, file_name: "坏 <script>.gguf", size_bytes: 1024 },
] };
const partial: LibraryOperation = { operation_id: "library-1", status: "partial", phase: "finished", terminal: true,
  candidate_files: 3, examined_entries: 3, verified_files: 2, failed_file_name: null, error: null, file_errors: [],
  result: { directory_id: null, library_generation: "new", registered_files: 2, available_files: 2, rejected_files: 1 },
  files: [{ ...selection.files[0], status: "registered", model_id: "new" }, { ...selection.files[1], status: "already_registered", model_id: "existing" }, { ...selection.files[2], status: "rejected", error_code: "unsupported_model" }],
};
function stopped() { return { ...snapshot(), connection: "stopped" as const, runtime: null }; }
async function mount(overrides: Partial<DesktopApi> = {}) {
  const api = makeApi({ snapshot: vi.fn(async () => stopped()), pickModels: vi.fn(async () => selection), libraryNext: vi.fn(async () => partial), ...overrides });
  const controller = new DesktopController(api); const view = render(<App initialPage="models" controller={controller} />);
  await screen.findByRole("heading", { name: model.display_name, level: 3 });
  return { api, controller, ...view };
}
describe("adding models from native file selection", () => {
  it("offers a prominent native picker, file sizes and additive zero-copy scope without arbitrary paths", async () => {
    const { api } = await mount(); fireEvent.click(screen.getByRole("button", { name: "添加模型" }));
    const panel = await screen.findByRole("region", { name: "添加选中的模型" });
    expect(within(panel).getAllByText("千问 中文.gguf")).toHaveLength(2);
    expect(within(panel).getByText("1.00 GiB")).toBeInTheDocument();
    expect(within(panel).getByText("2.00 GiB")).toBeInTheDocument();
    expect(within(panel).getByText(/不扫描父目录、不复制到 AppData/)).toBeInTheDocument();
    expect(within(panel).queryByRole("textbox")).not.toBeInTheDocument();
    expect(within(panel).getByRole("checkbox")).not.toBeChecked(); expect(within(panel).getByRole("checkbox")).toBeDisabled();
    expect(api.addModels).not.toHaveBeenCalled(); expect(api.applyDirectory).not.toHaveBeenCalled();
    fireEvent.click(within(panel).getByRole("button", { name: "取消文件选择" }));
    expect(api.discardModelSelection).toHaveBeenCalledExactlyOnceWith(selection.selection_id);
    expect(screen.queryByRole("region", { name: "添加选中的模型" })).not.toBeInTheDocument();
  });
  it("does not run discovery or scan on initial, repeated model page navigation or refresh", async () => {
    const { api } = await mount();
    fireEvent.click(screen.getByRole("button", { name: "设置" })); fireEvent.click(screen.getByRole("button", { name: "模型库" }));
    fireEvent.click(screen.getByRole("button", { name: "刷新" })); await waitFor(() => expect(api.modelsPage).toHaveBeenCalledTimes(2));
    for (const call of [api.discoverDirectory, api.reconcileModels, api.scanModels, api.start, api.stop]) expect(call).not.toHaveBeenCalled();
    expect(screen.getByText("原有管理模型")).toBeInTheDocument();
  });
  it("requires explicit service stop, preserves the selection, and never auto-adds after stopping", async () => {
    let value = snapshot();
    const { api } = await mount({ snapshot: vi.fn(async () => value), stop: vi.fn(async () => { value = stopped(); return { stopped: true as const }; }) });
    fireEvent.click(screen.getByRole("button", { name: "添加模型" }));
    const add = await screen.findByRole("button", { name: "确认添加 3 个模型" }); expect(add).toBeDisabled();
    expect(api.stop).not.toHaveBeenCalled(); fireEvent.click(screen.getByRole("button", { name: "停止服务以添加" }));
    expect(screen.getByRole("dialog")).toHaveTextContent("终止所有客户端任务并卸载当前模型");
    fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "取消" })); expect(api.stop).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "停止服务以添加" })); fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "停止运行服务" }));
    await waitFor(() => expect(add).toBeEnabled()); expect(api.stop).toHaveBeenCalledTimes(1); expect(api.addModels).not.toHaveBeenCalled();
    expect(screen.getAllByText("千问 中文.gguf")).toHaveLength(2);
  });
  it("keeps per-file partial results across pages and disables repeated add clicks", async () => {
    const terminal = deferred<LibraryOperation>();
    const { api, container } = await mount({ libraryNext: vi.fn(() => terminal.promise) });
    fireEvent.click(screen.getByRole("button", { name: "添加模型" })); const add = await screen.findByRole("button", { name: "确认添加 3 个模型" });
    fireEvent.click(add); fireEvent.click(add); await waitFor(() => expect(api.libraryNext).toHaveBeenCalledTimes(1));
    expect(api.addModels).toHaveBeenCalledExactlyOnceWith(selection.selection_id, false);
    fireEvent.click(screen.getByRole("button", { name: "设置" })); expect(screen.getByRole("region", { name: "添加模型结果" })).toBeInTheDocument();
    await act(async () => terminal.resolve(partial));
    expect(await screen.findByRole("heading", { name: "部分添加完成：2 个已登记" })).toBeInTheDocument();
    expect(screen.getByText("已存在 · 保留原登记")).toBeInTheDocument(); expect(screen.getByText("坏 <script>.gguf")).toBeInTheDocument();
    expect(container.querySelector(".model-add-progress script")).toBeNull();
    expect(api.loadModel).not.toHaveBeenCalled(); expect(api.unloadModel).not.toHaveBeenCalled();
    expect(screen.queryByText("本机基础测试通过")).not.toBeInTheDocument();
  });
  it("offers visible cancel and native close during a selected-file operation", async () => {
    const terminal = deferred<LibraryOperation>(); const { api, controller } = await mount({ libraryNext: vi.fn(() => terminal.promise) });
    fireEvent.click(screen.getByRole("button", { name: "添加模型" })); fireEvent.click(await screen.findByRole("button", { name: "确认添加 3 个模型" }));
    await waitFor(() => expect(api.libraryNext).toHaveBeenCalledTimes(1)); fireEvent.click(screen.getByRole("button", { name: "取消添加" }));
    expect(api.libraryCancel).toHaveBeenCalledExactlyOnceWith("library-1"); expect(controller.getSnapshot().library_phase).toBe("stopping");
    fireEvent.click(screen.getByRole("button", { name: "关闭窗口并保留服务" })); expect(api.close).toHaveBeenCalledTimes(1);
    await act(async () => terminal.resolve({ ...partial, status: "cancelled", result: null, verified_files: 0, files: selection.files.map((file) => ({ ...file, status: "not_processed", error_code: "model_scan_cancelled" })) }));
    await waitFor(() => expect(controller.getSnapshot().library_phase).toBe("idle")); expect(screen.getByRole("heading", { name: "添加已取消" })).toBeInTheDocument();
  });
  it("keeps successful registration visibly separate from an optional failed test", async () => {
    const single = { ...selection, files: [selection.files[0]] };
    const { api } = await mount({ pickModels: vi.fn(async () => single), libraryNext: vi.fn(async (): Promise<LibraryOperation> => ({ ...partial, status: "completed", candidate_files: 1, examined_entries: 1, verified_files: 1, result: { ...partial.result!, registered_files: 1, available_files: 1, rejected_files: 0 }, files: [{ ...partial.files![0], local_validation: { state: "failed", load_success: true, generation_pass: false, checked_at_unix_ms: 1234, error_code: "deadline_exceeded" } }] })) });
    fireEvent.click(screen.getByRole("button", { name: "添加模型" })); const choice = await screen.findByRole("checkbox", { name: "添加后加载并基础测试（仅单个模型）" });
    expect(choice).not.toBeChecked(); fireEvent.click(choice); fireEvent.click(screen.getByRole("button", { name: "确认添加 1 个模型" }));
    expect(await screen.findByRole("heading", { name: "添加完成：1 个已登记" })).toBeInTheDocument();
    expect(screen.getByText("本机加载通过 · 短文本测试失败")).toBeInTheDocument();
    expect(screen.getByText("本机测试诊断码：deadline_exceeded")).toBeInTheDocument(); expect(api.addModels).toHaveBeenCalledWith(selection.selection_id, true);
  });
});

describe("explicitly closing add results", () => {
  it.each(["completed", "partial", "cancelled", "failed"] as const)("offers a keyboard-accessible close for %s and removes the whole card across pages", async (status) => {
    const result: LibraryOperation = status === "partial" ? partial : status === "completed"
      ? { ...partial, status, verified_files: 3, result: { ...partial.result!, registered_files: 3, available_files: 3, rejected_files: 0 }, files: selection.files.map((file, index) => ({ ...file, status: "registered", model_id: `registered-${index}` })) }
      : { ...partial, status, result: null, verified_files: 0, error: status === "failed" ? { code: "model_scan_timeout", message: "校验超时。" } : null, files: selection.files.map((file) => ({ ...file, status: "not_processed", error_code: "model_scan_timeout" })) };
    const { api, controller } = await mount({ libraryNext: vi.fn(async () => result) });
    fireEvent.click(screen.getByRole("button", { name: "添加模型" })); fireEvent.click(await screen.findByRole("button", { name: "确认添加 3 个模型" }));
    const close = await screen.findByRole("button", { name: "关闭添加结果" });
    expect(close).toBeEnabled(); expect(within(screen.getByRole("region", { name: "添加模型结果" })).getByText(/关闭仅收起本次结果，不影响已登记模型/)).toBeVisible();
    const user = userEvent.setup(); close.focus(); expect(close).toHaveFocus(); await user.keyboard("{Enter}");
    expect(screen.queryByRole("region", { name: "添加模型结果" })).not.toBeInTheDocument();
    expect(controller.getSnapshot().library).toBeNull(); expect(controller.getSnapshot().library_selection).toBeNull();
    expect(await screen.findByRole("heading", { name: model.display_name, level: 3 })).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "设置" })); fireEvent.click(screen.getByRole("button", { name: "模型库" }));
    expect(screen.queryByRole("region", { name: "添加模型结果" })).not.toBeInTheDocument();
    for (const call of [api.libraryCancel, api.discardModelSelection, api.scanModels, api.unloadModel, api.stop]) expect(call).not.toHaveBeenCalled();
    if (status === "failed") expect(controller.getSnapshot().error?.code).toBe("model_scan_timeout");
  });

  it.each([
    ["starting", null, "取消添加"],
    ["running", "verifying", "取消添加"],
    ["running", "testing", "取消后续测试"],
    ["stopping", "testing", "等待取消确认"],
    ["recovery", "testing", "重新确认添加结果"],
  ] as const)("keeps control visible without dismissal in %s / %s", (library_phase, phase, control) => {
    const controller = new DesktopController(makeApi());
    const state: ViewState = { ...controller.getSnapshot(), library_kind: "add", library_phase, library_selection: selection,
      library: phase ? { ...partial, status: "running", phase, terminal: false, result: null } : null };
    render(<AddModelProgress state={state} controller={controller} />);
    expect(screen.getByRole("region", { name: "添加模型结果" })).toBeVisible();
    expect(screen.queryByRole("button", { name: "关闭添加结果" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: control })).toBeVisible();
    if (library_phase === "stopping") expect(screen.getByRole("button", { name: control })).toBeDisabled();
    else expect(screen.getByRole("button", { name: control })).toBeEnabled();
  });

  it("does not hide unconfirmed idle data just because the phase appears idle", () => {
    const controller = new DesktopController(makeApi());
    const state: ViewState = { ...controller.getSnapshot(), library_kind: "add", library: { ...partial, status: "running", phase: "testing", terminal: false, result: null } };
    render(<AddModelProgress state={state} controller={controller} />);
    expect(screen.getByRole("region", { name: "添加模型结果" })).toBeVisible();
    expect(screen.queryByRole("button", { name: "关闭添加结果" })).not.toBeInTheDocument();
  });

  it("preserves new file choices when closing an older result, then shows the next add", async () => {
    const second = deferred<LibraryOperation>();
    const { api, controller } = await mount({ libraryNext: vi.fn().mockResolvedValueOnce(partial).mockImplementationOnce(() => second.promise) });
    fireEvent.click(screen.getByRole("button", { name: "添加模型" })); fireEvent.click(await screen.findByRole("button", { name: "确认添加 3 个模型" }));
    await screen.findByRole("button", { name: "关闭添加结果" });
    await waitFor(() => expect(screen.getByRole("button", { name: "添加模型" })).toBeEnabled());
    fireEvent.click(screen.getByRole("button", { name: "添加模型" }));
    const chosen = await screen.findByRole("region", { name: "添加选中的模型" });
    fireEvent.click(screen.getByRole("button", { name: "关闭添加结果" }));
    expect(chosen).toBeVisible(); expect(controller.getSnapshot().model_selection).toEqual(selection);
    fireEvent.click(within(chosen).getByRole("button", { name: "确认添加 3 个模型" }));
    expect(screen.getByRole("region", { name: "添加模型结果" })).toBeVisible();
    expect(screen.queryByRole("button", { name: "关闭添加结果" })).not.toBeInTheDocument();
    await waitFor(() => expect(api.libraryNext).toHaveBeenCalledTimes(2));
    await act(async () => second.resolve({ ...partial, operation_id: "library-1" }));
    fireEvent.click(await screen.findByRole("button", { name: "关闭添加结果" }));
    expect(screen.queryByRole("region", { name: "添加模型结果" })).not.toBeInTheDocument();
    expect(api.addModels).toHaveBeenCalledTimes(2); expect(api.discardModelSelection).not.toHaveBeenCalled();
  });

  it("allows dismissal during the final inventory reread without unlocking or cancelling it", async () => {
    const reread = deferred<ReturnType<typeof snapshot>>();
    const { api, controller } = await mount({ snapshot: vi.fn().mockResolvedValueOnce(stopped()).mockImplementationOnce(() => reread.promise).mockResolvedValue(stopped()) });
    fireEvent.click(screen.getByRole("button", { name: "添加模型" })); fireEvent.click(await screen.findByRole("button", { name: "确认添加 3 个模型" }));
    fireEvent.click(await screen.findByRole("button", { name: "关闭添加结果" }));
    expect(screen.queryByRole("region", { name: "添加模型结果" })).not.toBeInTheDocument();
    expect(controller.getSnapshot().operation).toMatchObject({ kind: "read_models", label: "正在重新读取模型目录" });
    expect(screen.getByRole("button", { name: "添加模型" })).toBeDisabled();
    await act(async () => reread.resolve(stopped()));
    await waitFor(() => expect(screen.getByRole("button", { name: "添加模型" })).toBeEnabled());
    expect(screen.getByRole("heading", { name: model.display_name, level: 3 })).toBeVisible();
    expect(screen.queryByRole("region", { name: "添加模型结果" })).not.toBeInTheDocument();
    expect(api.libraryCancel).not.toHaveBeenCalled();
  });

  it("keeps the result dismissed after native close and an App remount", async () => {
    const { api, controller, unmount } = await mount();
    fireEvent.click(screen.getByRole("button", { name: "添加模型" })); fireEvent.click(await screen.findByRole("button", { name: "确认添加 3 个模型" }));
    fireEvent.click(await screen.findByRole("button", { name: "关闭添加结果" }));
    await waitFor(() => expect(controller.getSnapshot().operation).toBeNull());
    fireEvent.click(screen.getByRole("button", { name: "关闭窗口并保留服务" })); await waitFor(() => expect(api.close).toHaveBeenCalledTimes(1));
    unmount(); render(<App initialPage="models" controller={controller} />);
    expect(await screen.findByRole("heading", { name: model.display_name, level: 3 })).toBeVisible();
    expect(screen.queryByRole("region", { name: "添加模型结果" })).not.toBeInTheDocument();
    expect(api.libraryCancel).not.toHaveBeenCalled();
  });
});
