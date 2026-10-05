import { openModelDetails } from "./navigation";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import App from "../src/App";
import { DesktopController } from "../src/controller";
import { DownloadProgress } from "../src/ModelDownloads";
import type { CatalogEntry, DesktopApi, DownloadOperation, LocalValidation } from "../src/types";
import { deferred, makeApi, model, snapshot } from "./fixtures";

const proof: LocalValidation = { state: "passed", load_success: true, generation_pass: true, checked_at_unix_ms: 1791080000000, error_code: null };
function stopped() {
  return { ...snapshot(), connection: "stopped" as const, runtime: null, model_directory: {
    configured: { directory_id: "directory-1", display_path: "D:\\models", library_generation: "local-generation" }, effective: null, state: "stopped" as const,
  } };
}
async function mount(validation: LocalValidation = proof, overrides: Partial<DesktopApi> = {}) {
  const api = makeApi({ snapshot: vi.fn(async () => stopped()), modelsPage: vi.fn(async () => ({ source: "local" as const, generation: "local-generation", next_after: null, data: [{ ...model, validated: false, compatibility: "unvalidated" as const, local_validation: validation }] })), ...overrides });
  const controller = new DesktopController(api);
  render(<App initialPage="models" controller={controller} />);
  const article = await openModelDetails(model.display_name);
  return { api, controller, row: within(article) };
}
describe("model onboarding interface", () => {
  it("browses offline inventory and previous proof without implying live residency", async () => {
    const { api, row } = await mount();
    expect(screen.getByLabelText("应用状态栏")).toHaveTextContent("无驻留模型");
    expect(row.getByText("本机基础测试通过")).toBeInTheDocument();
    expect(row.getByRole("button", { name: "加载模型" })).toBeEnabled();
    expect(row.getByText("加载将启动服务并短测")).toBeInTheDocument();
    expect(row.queryByText("已加载", { exact: true })).not.toBeInTheDocument();
    expect(api.start).not.toHaveBeenCalled();
    expect(row.getByText(/不证明回答质量、长上下文、工具调用或全部功能/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /返回模型库/ })); expect(screen.getByRole("button", { name: "刷新" })).toBeEnabled();
  });
  it("keeps historical matrix validation independent of a failed local short test", async () => {
    const { row } = await mount({ ...proof, state: "failed", generation_pass: false, error_code: "deadline_exceeded" });
    expect(row.getByText("本机加载通过 · 短文本测试失败")).toBeInTheDocument();
    expect(row.getByText(/历史矩阵验证记录：未实测/)).toBeInTheDocument();
    expect(row.getByText("本机测试诊断码：deadline_exceeded")).toBeInTheDocument();
    expect(row.queryByText("本机基础测试通过")).not.toBeInTheDocument();
  });
  it("marks stale successful evidence pending retest", async () => {
    const { row } = await mount({ ...proof, state: "stale" });
    expect(row.getByText("本机记录已过期 · 待重测")).toBeInTheDocument();
    expect(row.getByText(/文件、引擎、设备或加载参数已变化/)).toBeInTheDocument();
    expect(row.queryByText("本机基础测试通过")).not.toBeInTheDocument();
  });
  it("keeps a loaded but untested model honestly labelled and allows a dedicated test", async () => {
    const testing = deferred<LocalValidation>();
    const { api, row } = await mount({ ...proof, state: "loaded", generation_pass: false }, { snapshot: vi.fn(async () => snapshot()), testModel: vi.fn(() => testing.promise) });
    fireEvent.click(row.getByRole("button", { name: `测试 ${model.display_name}` }));
    await waitFor(() => expect(api.testModel).toHaveBeenCalledTimes(1));
    expect(row.getByText("本次正在进行基础测试")).toBeInTheDocument();
    expect(screen.getByText(/短文本测试最多 30 秒/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "关闭窗口并保留服务" })).toBeEnabled();
    expect(api.chatStart).not.toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: "停止生成" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "关闭窗口并保留服务" }));
    expect(api.close).toHaveBeenCalledTimes(1);
    await act(async () => testing.resolve(proof));
  });
  it("explains deferred registration without an automatic stop action", async () => {
    const { api, controller } = await mount(proof, { reconcileModels: vi.fn(async () => ({ status: "pending" as const, operation_id: null })) });
    await act(async () => controller.reconcileModels());
    await waitFor(() => expect(controller.getSnapshot().reconcile_status).toBe("pending"));
    fireEvent.click(screen.getByRole("button", { name: /返回模型库/ })); expect(screen.getByText(/有待登记文件/)).toBeInTheDocument();
    expect(api.stop).not.toHaveBeenCalled();
    expect(api.start).not.toHaveBeenCalled();
  });
});

describe("download opt-in and saved outcome display", () => {
  const entry: CatalogEntry = { catalog_id: "test-model", display_name: "测试下载模型", file_name: "test.gguf", architecture: "qwen3", quantization: "Q4_K_M", size_bytes: 1024, sha256: "a".repeat(64), license: "Apache-2.0", context_hint: 2048, recommendation: "测试", sources: [{ source: "modelscope", repository: "test/repo", revision: "revision", url: "https://modelscope.cn/test" }] };
  const task: DownloadOperation = { operation_id: "download-1", catalog_id: "test-model", source: "modelscope", directory_id: "directory-1", target_display_path: "D:\\models", file_name: "test.gguf", phase: "finished", status: "completed", downloaded_bytes: 1024, total_bytes: 1024, terminal: true, error: null, result: { saved: true, registered: true, file_name: "test.gguf", cleanup_warning: null, registration_error: null, local_validation: proof } };
  it.each([true, false])("makes the service-starting download choice explicit: %s", async (autoTest) => {
    const { api } = await mount(proof, { catalog: vi.fn(async () => ({ entries: [entry] })) });
    fireEvent.click(screen.getByRole("button", { name: /返回模型库/ }));
    fireEvent.click(screen.getByRole("button", { name: "下载模型" }));
    const choice = screen.getByRole("checkbox", { name: "下载后加载并进行基础测试（空闲时）" });
    expect(choice).not.toBeChecked();
    if (autoTest) fireEvent.click(choice);
    fireEvent.click(await screen.findByRole("button", { name: "下载 测试下载模型" }));
    await waitFor(() => expect(api.downloadStart).toHaveBeenCalledExactlyOnceWith("test-model", autoTest));
    expect(api.start).not.toHaveBeenCalled();
  });
  it("shows saved and registered even when a subsequent short test was cancelled", () => {
    const controller = new DesktopController(makeApi());
    render(<DownloadProgress state={{ ...controller.getSnapshot(), snapshot: stopped(), download: { ...task, result: { ...task.result!, local_validation: { ...proof, state: "failed", generation_pass: false, error_code: "request_cancelled" } } } }} controller={controller} />);
    expect(screen.getByText("文件已保存并登记")).toBeInTheDocument();
    expect(screen.getByText("本机加载通过 · 短文本测试失败")).toBeInTheDocument();
    expect(screen.queryByText("下载已取消")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "重新下载" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "选择已保存文件以登记" })).not.toBeInTheDocument();
  });
  it("offers cancellation of post-save steps without implying file deletion", () => {
    const controller = new DesktopController(makeApi());
    render(<DownloadProgress state={{ ...controller.getSnapshot(), download_phase: "running", download: { ...task, phase: "testing", status: "running", terminal: false, result: null } }} controller={controller} />);
    expect(screen.getByRole("button", { name: "取消后续步骤" })).toBeEnabled();
    expect(screen.getByText("文件已登记 · 正在加载与基础测试")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "取消下载" })).not.toBeInTheDocument();
  });
  it("shows a registration diagnostic while retaining saved-file success", () => {
    const controller = new DesktopController(makeApi());
    render(<DownloadProgress state={{ ...controller.getSnapshot(), snapshot: stopped(), download: { ...task, result: { ...task.result!, registered: false, local_validation: null, registration_error: { code: "model_scan_cancelled", message: "目录登记已取消。" } } } }} controller={controller} />);
    expect(screen.getByText("自动登记未完成，已保存文件仍保留")).toBeInTheDocument();
    expect(screen.getByText("诊断码：model_scan_cancelled")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "选择已保存文件以登记" })).toBeEnabled();
    expect(screen.queryByText("下载失败")).not.toBeInTheDocument();
  });
});
