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
  it("discovers an absent configuration once and gives an explicit no-folder state", async () => {
    const value = stopped(); value.model_directory = { configured: null, effective: null, state: "default" };
    const { api, controller } = await create({ snapshot: vi.fn(async () => value) });
    const unmount = controller.mount(); await vi.advanceTimersByTimeAsync(2001);
    expect(api.discoverDirectory).toHaveBeenCalledTimes(1); expect(controller.getSnapshot().discovery).toBe("none"); unmount();
  });
  it("rejects identity changes and regressing byte counters", async () => {
    for (const patch of [{ source: "huggingface" as const }, { downloaded_bytes: 1 }]) {
      const { controller } = await create({ downloadNext: vi.fn().mockResolvedValueOnce(progress()).mockResolvedValueOnce(progress(patch)) });
      await controller.startDownload(entry.catalog_id); await vi.advanceTimersByTimeAsync(1001);
      expect(controller.getSnapshot().download_phase).toBe("recovery");
      expect(controller.getSnapshot().error?.code).toBe("invalid_download_operation");
    }
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
