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
async function mount(source: "modelscope" | "huggingface" = "modelscope") {
  const value = stopped(); value.settings.download_source = source;
  const api = makeApi({ snapshot: vi.fn(async () => value), catalog: vi.fn(async () => ({ entries: [entry] })) });
  const controller = new DesktopController(api); render(<App controller={controller} />);
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
    expect(screen.getByRole("button", { name: "扫描目录以登记" })).toBeDisabled();
    expect(screen.getByText(/当前目录已变化/)).toBeInTheDocument(); expect(screen.getByText(/请勿重复下载/)).toBeInTheDocument();
  });

});
