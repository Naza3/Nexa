import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_SETTINGS, DesktopController, validatePreferences } from "../src/controller";
import type { CatalogEntry, DesktopApi, DownloadOperation, Snapshot } from "../src/types";
import { deferred, makeApi, snapshot } from "./fixtures";
export const entry: CatalogEntry = {
  catalog_id: "test-model", display_name: "测试 2B", file_name: "test.gguf", architecture: "qwen3", quantization: "Q4_K_M",
  size_bytes: 1024, sha256: "a".repeat(64), license: "Apache-2.0", context_hint: 2048, recommendation: "测试目录，仅合成数据",
  sources: [{ source: "modelscope", repository: "test/repo", revision: "a".repeat(40), url: "https://modelscope.cn/test" }],
};
function stopped(): Snapshot { return { ...snapshot(), connection: "stopped", runtime: null, model_directory: { configured: { directory_id: "directory-1", display_path: "D:\\models", library_generation: "generation-1" }, effective: null, state: "stopped" } }; }
function progress(patch: Partial<DownloadOperation> = {}): DownloadOperation { return {
  operation_id: "download-1", catalog_id: "test-model", source: "modelscope", file_name: "test.gguf", directory_id: "directory-1", target_display_path: "D:\\models",
  downloaded_bytes: 128, total_bytes: 1024, phase: "downloading", status: "running", terminal: false, result: null, error: null, ...patch,
}; }
async function create(overrides: Partial<DesktopApi> = {}) {
  const api = makeApi({ snapshot: vi.fn(async () => stopped()), catalog: vi.fn(async () => ({ entries: [entry] })), ...overrides });
  const controller = new DesktopController(api); await controller.refresh(); await controller.loadCatalog(); return { api, controller };
}
beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());
describe("download ownership and directory discovery", () => {
  it("defaults to ModelScope and rejects invalid saved source", () => {
    expect(DEFAULT_SETTINGS.download_source).toBe("modelscope");
    expect(validatePreferences({ ...DEFAULT_SETTINGS, download_source: "other" as never })).toMatch(/下载源/);
  });
  it("never falls back to a different source or downloads while running", async () => {
    const value = stopped(); value.settings.download_source = "huggingface";
    const { api, controller } = await create({ snapshot: vi.fn(async () => value) });
    await controller.startDownload(entry.catalog_id); expect(api.downloadStart).not.toHaveBeenCalled();
    expect(controller.getSnapshot().error?.code).toBe("download_source_unavailable");
    value.settings.download_source = "modelscope"; value.connection = "connected"; await controller.refresh();
    await controller.startDownload(entry.catalog_id); expect(api.downloadStart).not.toHaveBeenCalled();
  });
  it("sends only catalog id, keeps actual byte progress and does not auto scan or load", async () => {
    const { api, controller } = await create({ downloadNext: vi.fn().mockResolvedValueOnce(progress()).mockResolvedValueOnce(progress({ downloaded_bytes: 1024, status: "completed", phase: "finished", terminal: true, result: { saved: true, registered: false, file_name: "test.gguf", cleanup_warning: null } })) });
    await controller.startDownload(entry.catalog_id); await vi.advanceTimersByTimeAsync(1);
    expect(api.downloadStart).toHaveBeenCalledWith("test-model"); expect(controller.getSnapshot().download?.downloaded_bytes).toBe(128);
    await vi.advanceTimersByTimeAsync(1000); expect(controller.getSnapshot().download_phase).toBe("idle");
    expect(controller.getSnapshot().notice).toMatch(/尚未登记/); expect(api.scanModels).not.toHaveBeenCalled(); expect(api.loadModel).not.toHaveBeenCalled();
  });
  it("preserves early cancel and rejects overlapping download or directory changes", async () => {
    const start = deferred<{ operation_id: string }>();
    const { api, controller } = await create({ downloadStart: vi.fn(() => start.promise), downloadNext: vi.fn(async () => progress({ status: "cancelled", phase: "finished", terminal: true })) });
    const pending = controller.startDownload(entry.catalog_id); await controller.cancelDownload();
    await controller.startDownload(entry.catalog_id); await controller.scanModels(); await controller.pickDirectory();
    expect(api.downloadStart).toHaveBeenCalledTimes(1); expect(api.scanModels).not.toHaveBeenCalled(); expect(api.pickDirectory).not.toHaveBeenCalled();
    start.resolve({ operation_id: "download-1" }); await pending; await vi.advanceTimersByTimeAsync(1);
    expect(api.downloadCancel).toHaveBeenCalledWith("download-1"); expect(controller.getSnapshot().download?.status).toBe("cancelled");
  });
  it("keeps unknown status blocked and never automatically replays a failed poll", async () => {
    const { api, controller } = await create({ downloadNext: vi.fn(async () => { throw { code: "network_error", message: "状态读取失败" }; }) });
    await controller.startDownload(entry.catalog_id); await vi.advanceTimersByTimeAsync(1);
    expect(controller.getSnapshot().download_phase).toBe("recovery"); await controller.startDownload(entry.catalog_id);
    expect(api.downloadStart).toHaveBeenCalledTimes(1); expect(api.downloadCancel).toHaveBeenCalledTimes(1);
  });
  it("rejects false success without a saved result", async () => {
    const { controller } = await create({ downloadNext: vi.fn(async () => progress({ status: "completed", phase: "finished", terminal: true })) });
    await controller.startDownload(entry.catalog_id); await vi.advanceTimersByTimeAsync(1);
    expect(controller.getSnapshot().download_phase).toBe("recovery"); expect(controller.getSnapshot().error?.code).toBe("invalid_download_operation");
  });
  it("preserves a configured missing directory without fallback discovery", async () => {
    const value = stopped(); value.model_directory.state = "missing";
    const { api, controller } = await create({ snapshot: vi.fn(async () => value) });
    await controller.discoverDirectory(); expect(api.discoverDirectory).not.toHaveBeenCalled();
    expect(controller.getSnapshot().snapshot?.model_directory.configured?.directory_id).toBe("directory-1");
  });
  it("does not discover or scan an absent configuration during startup", async () => {
    const value = stopped(); value.model_directory = { configured: null, effective: null, state: "default" };
    const { api, controller } = await create({ snapshot: vi.fn(async () => value) });
    const unmount = controller.mount(); await vi.advanceTimersByTimeAsync(2001);
    expect(api.discoverDirectory).not.toHaveBeenCalled(); expect(api.reconcileModels).not.toHaveBeenCalled(); expect(api.scanModels).not.toHaveBeenCalled(); expect(controller.getSnapshot().discovery).toBe("unchecked"); unmount();
  });
  it("rejects identity changes and regressing byte counters", async () => {
    for (const patch of [{ source: "huggingface" as const }, { downloaded_bytes: 1 }]) {
      const { controller } = await create({ downloadNext: vi.fn().mockResolvedValueOnce(progress()).mockResolvedValueOnce(progress(patch)) });
      await controller.startDownload(entry.catalog_id); await vi.advanceTimersByTimeAsync(1001);
      expect(controller.getSnapshot().download_phase).toBe("recovery");
      expect(controller.getSnapshot().error?.code).toBe("invalid_download_operation");
    }
  });
  it.each([2, 3])("accepts reset bytes on a later attempt, including missed polls: %i", async (attempt) => {
    const { api, controller } = await create({ downloadNext: vi.fn().mockResolvedValueOnce(progress()).mockResolvedValueOnce(progress({ attempt, downloaded_bytes: 0 })) });
    await controller.startDownload(entry.catalog_id); await vi.advanceTimersByTimeAsync(1);
    expect(controller.getSnapshot().download?.attempt).toBe(1);
    await vi.advanceTimersByTimeAsync(1000);
    expect(controller.getSnapshot().download_phase).toBe("running");
    expect(controller.getSnapshot().download).toMatchObject({ attempt, downloaded_bytes: 0 });
    expect(api.downloadStart).toHaveBeenCalledTimes(1);
    expect(api.downloadCancel).not.toHaveBeenCalled();
  });
  it.each([0, -1, 4, 1.5, NaN, Infinity, "2", null, {}, true])("rejects malformed or out-of-budget attempts: %j", async (attempt) => {
    const { api, controller } = await create({ downloadNext: vi.fn(async () => progress({ attempt: attempt as number })) });
    await controller.startDownload(entry.catalog_id); await vi.advanceTimersByTimeAsync(1);
    expect(controller.getSnapshot().download_phase).toBe("recovery");
    expect(controller.getSnapshot().download).toBeNull();
    expect(controller.getSnapshot().error?.code).toBe("invalid_download_operation");
    expect(api.downloadCancel).toHaveBeenCalledTimes(1);
  });
  it.each([
    { attempt: 1, downloaded_bytes: 256 },
    { attempt: undefined, downloaded_bytes: 256 },
    { attempt: 2, downloaded_bytes: 0 },
    { attempt: 3, downloaded_bytes: 0, source: "huggingface" },
    { attempt: 3, downloaded_bytes: 0, directory_id: "other" },
    { attempt: 3, downloaded_bytes: 0, operation_id: "other" },
    { attempt: 3, downloaded_bytes: 0, catalog_id: "other" },
    { attempt: 3, downloaded_bytes: 0, file_name: "other.gguf" },
    { attempt: 3, downloaded_bytes: 0, target_display_path: "other" },
    { attempt: 3, downloaded_bytes: 0, total_bytes: 2048 },
    { attempt: 3, downloaded_bytes: 0, total_bytes: null },
  ])("preserves attempt monotonicity and identity binding during retries: %j", async (patch) => {
    const { controller } = await create({ downloadNext: vi.fn().mockResolvedValueOnce(progress({ attempt: 2 })).mockResolvedValueOnce(progress(patch as Partial<DownloadOperation>)) });
    await controller.startDownload(entry.catalog_id); await vi.advanceTimersByTimeAsync(1001);
    expect(controller.getSnapshot().download_phase).toBe("recovery");
    expect(controller.getSnapshot().download).toMatchObject({ attempt: 2, downloaded_bytes: 128 });
    expect(controller.getSnapshot().error?.code).toBe("invalid_download_operation");
  });
  it("keeps retry exhaustion terminal without automatically starting a new operation", async () => {
    const failure = { code: "model_download_network_failed", message: "下载尝试已结束。" };
    const { api, controller } = await create({ downloadNext: vi.fn().mockResolvedValueOnce(progress()).mockResolvedValueOnce(progress({ attempt: 3, downloaded_bytes: 0, status: "failed", phase: "finished", terminal: true, error: failure })) });
    await controller.startDownload(entry.catalog_id); await vi.advanceTimersByTimeAsync(5001);
    expect(controller.getSnapshot().download_phase).toBe("idle");
    expect(controller.getSnapshot().download).toMatchObject({ attempt: 3, status: "failed", downloaded_bytes: 0 });
    expect(controller.getSnapshot().error).toMatchObject({ code: failure.code, message: expect.stringContaining("当前下载源") });
    expect(api.downloadStart).toHaveBeenCalledTimes(1);
    expect(api.downloadNext).toHaveBeenCalledTimes(2);
    expect(api.scanModels).not.toHaveBeenCalled();
  });
  it("cancels the same operation after a retry and waits for authoritative terminal status", async () => {
    const { api, controller } = await create({ downloadNext: vi.fn().mockResolvedValueOnce(progress()).mockResolvedValueOnce(progress({ attempt: 2, downloaded_bytes: 0 })).mockResolvedValueOnce(progress({ attempt: 2, downloaded_bytes: 0, status: "cancelled", phase: "finished", terminal: true })) });
    await controller.startDownload(entry.catalog_id); await vi.advanceTimersByTimeAsync(1001);
    await controller.cancelDownload();
    expect(controller.getSnapshot().download_phase).toBe("stopping");
    expect(api.downloadCancel).toHaveBeenCalledWith("download-1");
    await vi.advanceTimersByTimeAsync(1000);
    expect(controller.getSnapshot().download_phase).toBe("idle");
    expect(controller.getSnapshot().download?.status).toBe("cancelled");
    expect(api.downloadStart).toHaveBeenCalledTimes(1);
  });
  it("retains verification and saved-but-unregistered semantics after a retry", async () => {
    const { api, controller } = await create({ downloadNext: vi.fn().mockResolvedValueOnce(progress()).mockResolvedValueOnce(progress({ attempt: 2, downloaded_bytes: 1024, phase: "verifying" })).mockResolvedValueOnce(progress({ attempt: 2, downloaded_bytes: 1024, status: "completed", phase: "finished", terminal: true, result: { saved: true, registered: false, file_name: "test.gguf", cleanup_warning: null } })) });
    await controller.startDownload(entry.catalog_id); await vi.advanceTimersByTimeAsync(1001);
    expect(controller.getSnapshot().download_phase).toBe("running");
    expect(controller.getSnapshot().download?.phase).toBe("verifying");
    await vi.advanceTimersByTimeAsync(1000);
    expect(controller.getSnapshot().download_phase).toBe("idle");
    expect(controller.getSnapshot().notice).toMatch(/尚未登记/);
    expect(api.downloadStart).toHaveBeenCalledTimes(1);
    expect(api.scanModels).not.toHaveBeenCalled();
    expect(api.loadModel).not.toHaveBeenCalled();
  });
  it("finishes a discovered directory through existing library polling without auto starting", async () => {
    const value = stopped(); value.model_directory = { configured: null, effective: null, state: "default" };
    const { api, controller } = await create({ snapshot: vi.fn(async () => value), discoverDirectory: vi.fn(async () => ({ operation_id: "library-1" })) });
    await controller.discoverDirectory(); await vi.advanceTimersByTimeAsync(1);
    expect(api.libraryNext).toHaveBeenCalledWith("library-1"); expect(controller.getSnapshot().discovery).toBe("configured");
    expect(api.start).not.toHaveBeenCalled(); expect(api.loadModel).not.toHaveBeenCalled();
  });
  it("does not interpret a saved file with a cleanup warning as a failed download", async () => {
    const { controller } = await create({ downloadNext: vi.fn(async () => progress({ downloaded_bytes: 1024, status: "completed", phase: "finished", terminal: true, result: { saved: true, registered: false, file_name: "test.gguf", cleanup_warning: "清理待确认" } })) });
    await controller.startDownload(entry.catalog_id); await vi.advanceTimersByTimeAsync(1);
    expect(controller.getSnapshot().download_phase).toBe("idle"); expect(controller.getSnapshot().notice).toMatch(/请勿重复下载/);
    expect(controller.getSnapshot().download?.status).toBe("completed");
  });

  it("rejects a claimed complete file with incomplete byte progress", async () => {
    const { controller } = await create({ downloadNext: vi.fn(async () => progress({ status: "completed", phase: "finished", terminal: true, result: { saved: true, registered: false, file_name: "test.gguf", cleanup_warning: null } })) });
    await controller.startDownload(entry.catalog_id); await vi.advanceTimersByTimeAsync(1);
    expect(controller.getSnapshot().download_phase).toBe("recovery");
  });

  it("rejects malformed display data and cleanup warnings before rendering", async () => {
    for (const patch of [
      { target_display_path: { unsafe: "object" } },
      { file_name: "../other.gguf" },
      { total_bytes: 0 },
      { status: "completed", phase: "finished", terminal: true, downloaded_bytes: 1024,
        result: { saved: true, registered: false, file_name: "test.gguf", cleanup_warning: { unsafe: "object" } } },
    ]) {
      const { controller } = await create({ downloadNext: vi.fn(async () => progress(patch as unknown as Partial<DownloadOperation>)) });
      await controller.startDownload(entry.catalog_id); await vi.advanceTimersByTimeAsync(1);
      expect(controller.getSnapshot().download_phase).toBe("recovery");
      expect(controller.getSnapshot().download).toBeNull();
      expect(controller.getSnapshot().error?.code).toBe("invalid_download_operation");
    }
  });

  it("replaces the old download notice with authoritative same-directory scan results", async () => {
    const { controller } = await create({ downloadNext: vi.fn(async () => progress({ downloaded_bytes: 1024, status: "completed", phase: "finished", terminal: true, result: { saved: true, registered: false, file_name: "test.gguf", cleanup_warning: null } })) });
    await controller.startDownload(entry.catalog_id); await vi.advanceTimersByTimeAsync(1);
    expect(controller.getSnapshot().download?.status).toBe("completed");
    await controller.scanModels(); await vi.advanceTimersByTimeAsync(1);
    expect(controller.getSnapshot().download).toBeNull();
    expect(controller.getSnapshot().library?.status).toBe("completed");
  });

});

describe("download registration and optional basic test", () => {
  const localProof = { state: "passed" as const, load_success: true, generation_pass: true, checked_at_unix_ms: 1791080000000, error_code: null };
  it("sends the explicit automatic-test choice and accepts registered completion", async () => {
    const result = { saved: true as const, registered: true, file_name: "test.gguf", cleanup_warning: null, registration_error: null, local_validation: localProof };
    const { api, controller } = await create({ downloadNext: vi.fn(async () => progress({ downloaded_bytes: 1024, status: "completed", phase: "finished", terminal: true, result })) });
    await controller.startDownload(entry.catalog_id, true); await vi.advanceTimersByTimeAsync(1);
    expect(api.downloadStart).toHaveBeenCalledExactlyOnceWith("test-model", true);
    expect(controller.getSnapshot().download?.result).toEqual(result);
    expect(controller.getSnapshot().notice).toContain("自动登记");
    expect(api.modelsPage).toHaveBeenCalledTimes(2);
    expect(api.chatStart).not.toHaveBeenCalled();
    expect(api.start).not.toHaveBeenCalled();
  });
  it("shows post-save registration failure separately from successful file saving", async () => {
    const registration_error = { code: "runtime_running", message: "运行服务已启动，登记已暂缓" };
    const { controller } = await create({ downloadNext: vi.fn(async () => progress({ downloaded_bytes: 1024, status: "completed", phase: "finished", terminal: true, result: { saved: true, registered: false, file_name: "test.gguf", cleanup_warning: null, registration_error } })) });
    await controller.startDownload(entry.catalog_id, false); await vi.advanceTimersByTimeAsync(1);
    expect(controller.getSnapshot().download?.status).toBe("completed");
    expect(controller.getSnapshot().error).toBeNull();
    expect(controller.getSnapshot().notice).toContain("自动登记未完成");
    expect(controller.getSnapshot().download?.result?.registration_error).toEqual(registration_error);
  });
  it.each(["failed", "deferred"] as const)("does not confuse %s local testing with a failed download", async (state) => {
    const result = { saved: true as const, registered: true, file_name: "test.gguf", cleanup_warning: null, registration_error: null, local_validation: { ...localProof, state, generation_pass: false, error_code: state === "failed" ? "deadline_exceeded" : "runtime_busy" } };
    const { controller } = await create({ downloadNext: vi.fn(async () => progress({ downloaded_bytes: 1024, status: "completed", phase: "finished", terminal: true, result })) });
    await controller.startDownload(entry.catalog_id, true); await vi.advanceTimersByTimeAsync(1);
    expect(controller.getSnapshot().download_phase).toBe("idle");
    expect(controller.getSnapshot().download?.result?.local_validation?.generation_pass).toBe(false);
    expect(controller.getSnapshot().download?.status).toBe("completed");
    expect(controller.getSnapshot().error).toBeNull();
  });
  it("cancels subsequent testing without claiming the saved file was cancelled", async () => {
    const result = { saved: true as const, registered: true, file_name: "test.gguf", cleanup_warning: null, local_validation: { ...localProof, state: "failed" as const, generation_pass: false, error_code: "request_cancelled" } };
    const { api, controller } = await create({ downloadNext: vi.fn().mockResolvedValueOnce(progress({ phase: "testing", downloaded_bytes: 1024 })).mockResolvedValueOnce(progress({ downloaded_bytes: 1024, status: "completed", phase: "finished", terminal: true, result })) });
    await controller.startDownload(entry.catalog_id, true); await vi.advanceTimersByTimeAsync(1);
    await controller.cancelDownload();
    expect(api.downloadCancel).toHaveBeenCalledExactlyOnceWith("download-1");
    await vi.advanceTimersByTimeAsync(1000);
    expect(controller.getSnapshot().download?.status).toBe("completed");
    expect(controller.getSnapshot().download?.result?.local_validation?.error_code).toBe("request_cancelled");
    expect(controller.getSnapshot().notice).not.toContain("下载已取消");
  });
  it("does not grant proof to an unregistered file or contradictory success", async () => {
    for (const result of [
      { saved: true as const, registered: false, file_name: "test.gguf", cleanup_warning: null, local_validation: localProof },
      { saved: true as const, registered: true, file_name: "test.gguf", cleanup_warning: null, registration_error: { code: "model_scan_failed", message: "登记失败" } },
    ]) {
      const { controller } = await create({ downloadNext: vi.fn(async () => progress({ downloaded_bytes: 1024, status: "completed", phase: "finished", terminal: true, result })) });
      await controller.startDownload(entry.catalog_id, true); await vi.advanceTimersByTimeAsync(1);
      expect(controller.getSnapshot().download_phase).toBe("recovery");
      expect(controller.getSnapshot().download).toBeNull();
    }
  });
});
