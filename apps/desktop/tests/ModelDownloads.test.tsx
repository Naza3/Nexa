import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import App from "../src/App";
import { DesktopController } from "../src/controller";
import { DownloadProgress } from "../src/ModelDownloads";
import type { CatalogEntry, DownloadOperation } from "../src/types";
import { makeApi, snapshot } from "./fixtures";
const entry: CatalogEntry = {
  catalog_id: "candidate", display_name: "候选模型 2B", file_name: "candidate.Q4_K_M.gguf", architecture: "qwen3", quantization: "Q4_K_M", size_bytes: 1024 ** 3,
  sha256: "a".repeat(64), license: "Apache-2.0", context_hint: 2048, recommendation: "资源需求须实际验证",
  sources: [{ source: "modelscope", repository: "test/gguf", revision: "a".repeat(40), url: "https://modelscope.cn/test" }],
};
function stopped() { const value = snapshot(); value.connection = "stopped"; value.runtime = null; value.model_directory = { configured: { directory_id: "directory", display_path: "D:\\模型", library_generation: "generation" }, effective: null, state: "stopped" }; return value; }
function failedDownload(patch: Partial<DownloadOperation> = {}): DownloadOperation {
  return { operation_id: "download", catalog_id: "candidate", source: "modelscope", file_name: entry.file_name, directory_id: "directory", target_display_path: "D:\\模型", downloaded_bytes: 0, total_bytes: null, phase: "finished", status: "failed", terminal: true, result: null, error: { code: "model_download_network_failed", message: "请求下载源：网络等待超时。未切换下载源，未发布模型文件。" }, ...patch };
}
async function mount(source: "modelscope" | "huggingface" = "modelscope") {
  const value = stopped(); value.settings.download_source = source;
  const api = makeApi({ snapshot: vi.fn(async () => value), catalog: vi.fn(async () => ({ entries: [entry] })) });
  const controller = new DesktopController(api); render(<App initialPage="models" controller={controller} />);
  await waitFor(() => expect(controller.getSnapshot().snapshot).not.toBeNull());
  return { api, controller };
}
describe("GGUF catalog interface", () => {
  it("shows local filtering, metadata and honest memory/verification warning", async () => {
    await mount(); fireEvent.click(screen.getByRole("button", { name: "下载模型" }));
    await screen.findByRole("heading", { name: "候选模型 2B" });
    expect(screen.getByText("Q4_K_M")).toBeInTheDocument(); expect(screen.getByText("1.00 GiB")).toBeInTheDocument();
    expect(screen.getByText(/文件大小不等于运行内存/)).toBeInTheDocument(); expect(screen.getByText(/精选下载目录不是加载白名单/)).toBeInTheDocument();
    fireEvent.change(screen.getByRole("searchbox", { name: "筛选下载目录" }), { target: { value: "missing" } });
    expect(screen.queryByRole("heading", { name: "候选模型 2B" })).not.toBeInTheDocument(); expect(screen.getByText(/没有匹配的模型/)).toBeInTheDocument();
  });
  it("disables an unavailable saved source without fallback", async () => {
    const { api } = await mount("huggingface"); fireEvent.click(screen.getByRole("button", { name: "下载模型" }));
    const button = await screen.findByRole("button", { name: "下载 候选模型 2B" }); expect(button).toBeDisabled(); expect(screen.getByText(/Hugging Face 暂无此文件/)).toBeInTheDocument();
    fireEvent.click(button); expect(api.downloadStart).not.toHaveBeenCalled();
  });
  it("persists the chosen default source only through save preferences", async () => {
    const { api } = await mount(); fireEvent.click(screen.getByRole("button", { name: "设置" }));
    fireEvent.change(screen.getByRole("combobox", { name: /默认下载源/ }), { target: { value: "huggingface" } });
    expect(api.saveSettings).not.toHaveBeenCalled(); fireEvent.click(screen.getByRole("button", { name: "保存偏好" }));
    await waitFor(() => expect(api.saveSettings).toHaveBeenCalledWith(expect.objectContaining({ download_source: "huggingface" })));
  });
  it("uses actual byte progress and shows no invented percentage for unknown totals", () => {
    const controller = new DesktopController(makeApi()); const state = controller.getSnapshot();
    const task: DownloadOperation = { operation_id: "download", catalog_id: "candidate", source: "modelscope", file_name: entry.file_name, directory_id: "directory", target_display_path: "D:\\模型", downloaded_bytes: 129, total_bytes: null, phase: "downloading", status: "running", terminal: false, result: null, error: null };
    render(<DownloadProgress state={{ ...state, download: task, download_phase: "running" }} controller={controller} />);
    expect(screen.getByText(/129 B/)).toBeInTheDocument(); expect(screen.getByText(/总大小未知/)).toBeInTheDocument(); expect(screen.queryByRole("progressbar")).not.toBeInTheDocument(); expect(screen.getByRole("button", { name: "取消下载" })).toBeEnabled();
  });
  it.each([undefined, 1, 2, 3])("shows the reported in-task attempt without inventing resume or restart: %j", (attempt) => {
    const api = makeApi(); const controller = new DesktopController(api);
    render(<DownloadProgress state={{ ...controller.getSnapshot(), snapshot: stopped(), download: failedDownload({ attempt }) }} controller={controller} />);
    expect(screen.getByText(`第${attempt ?? 1}次传输尝试`)).toBeInTheDocument();
    expect(screen.queryByText(/断点续传|代理|重新开始传输|恢复传输/)).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "重新下载" })).toBeEnabled();
    expect(api.downloadStart).not.toHaveBeenCalled();
  });
  it("keeps a download visible when changing pages", async () => {
    const { controller } = await mount(); fireEvent.click(screen.getByRole("button", { name: "下载模型" }));
    await screen.findByRole("heading", { name: "候选模型 2B" });
    fireEvent.click(screen.getByRole("button", { name: "下载 候选模型 2B" }));
    await waitFor(() => expect(controller.getSnapshot().download_phase).not.toBe("idle"));
    fireEvent.click(screen.getByRole("button", { name: "设置" }));
    expect(screen.getByRole("region", { name: "模型下载进度" })).toBeInTheDocument();
  });
  it("does not scan another directory for a completed download", () => {
    const controller = new DesktopController(makeApi());
    const task: DownloadOperation = { operation_id: "download", catalog_id: "candidate", source: "modelscope", file_name: entry.file_name, directory_id: "old-directory", target_display_path: "D:\\old", downloaded_bytes: 1024, total_bytes: 1024, phase: "finished", status: "completed", terminal: true, result: { saved: true, registered: false, file_name: entry.file_name, cleanup_warning: "清理待确认" }, error: null };
    render(<DownloadProgress state={{ ...controller.getSnapshot(), snapshot: stopped(), download: task }} controller={controller} />);
    expect(screen.getByRole("button", { name: "选择已保存文件以登记" })).toBeEnabled();
    expect(screen.getByText(/下载目录已变化/)).toBeInTheDocument(); expect(screen.getByText(/请勿重复下载/)).toBeInTheDocument();
  });

  it("shows controlled failure details and a reportable diagnostic code without retrying automatically", () => {
    const api = makeApi(); const controller = new DesktopController(api);
    render(<DownloadProgress state={{ ...controller.getSnapshot(), snapshot: stopped(), download: failedDownload() }} controller={controller} />);
    expect(screen.getByText(/请求下载源：网络等待超时/)).toBeInTheDocument();
    expect(screen.getByText("诊断码：model_download_network_failed")).toBeInTheDocument();
    expect(screen.getByText("保存到：D:\\模型")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "重新下载" })).toBeEnabled();
    expect(api.downloadStart).not.toHaveBeenCalled();
  });

  it("keeps an unknown but bounded diagnostic code reportable", () => {
    const controller = new DesktopController(makeApi());
    render(<DownloadProgress state={{ ...controller.getSnapshot(), download: failedDownload({ error: { code: "future_download_error", message: "下载未完成。" } }) }} controller={controller} />);
    expect(screen.getByText("诊断码：future_download_error")).toBeInTheDocument();
  });

  it.each([
    { code: {}, message: {} },
    { code: "https://example.test/?token=private", message: "https://example.test/?token=private" },
    { code: "x".repeat(81), message: "界".repeat(167) },
    { code: "<img src=x onerror=alert(1)>", message: "<img src=x onerror=alert(1)>" },
    { code: null, message: "Bearer private-token" },
    { code: "bad\ncode", message: "unexpected\0message" },
    { code: "", message: "   " },
  ])("handles malformed error data without exposing raw details or injecting HTML: %j", (error) => {
    const controller = new DesktopController(makeApi());
    const { container } = render(<DownloadProgress state={{ ...controller.getSnapshot(), download: failedDownload({ error: error as unknown as DownloadOperation["error"] }) }} controller={controller} />);
    expect(screen.getByText("诊断码：invalid_download_error")).toBeInTheDocument();
    expect(screen.getByText("下载未完成，请提供诊断码和当前进度以便排查。")).toBeInTheDocument();
    expect(container.querySelector("img, script, a")).toBeNull();
    expect(container.textContent).not.toMatch(/private|example\.test|onerror/);
  });

  it.each(["", "   ", null, {}, "bad\0path"])("shows a safe placeholder for unavailable target display paths: %j", (path) => {
    const controller = new DesktopController(makeApi());
    render(<DownloadProgress state={{ ...controller.getSnapshot(), download: failedDownload({ target_display_path: path as unknown as string }) }} controller={controller} />);
    expect(screen.getByText("保存到：路径信息不可用")).toBeInTheDocument();
  });

});
