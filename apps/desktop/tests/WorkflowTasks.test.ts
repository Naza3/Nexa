import { waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { DesktopController } from "../src/controller";
import type { CatalogEntry, ChatBatch, DownloadOperation } from "../src/types";
import { deferred, makeApi, snapshot } from "./fixtures";
const entry: CatalogEntry = { catalog_id: "one", display_name: "One", file_name: "one.gguf", architecture: "qwen3", quantization: "Q8_0", size_bytes: 1, sha256: "a".repeat(64), license: "test", context_hint: 2048, recommendation: "test", sources: [{ source: "modelscope", repository: "repo", revision: "revision", url: "https://modelscope.cn/test" }] };
const cancelled: DownloadOperation = { operation_id: "native-download", catalog_id: "one", source: "modelscope", file_name: "one.gguf", directory_id: "directory", target_display_path: "D:\\models", downloaded_bytes: 0, total_bytes: 1, phase: "finished", status: "cancelled", terminal: true, result: null, error: null };
function stopped() { const value = snapshot(); value.connection = "stopped"; value.runtime = null; value.model_directory.configured = { directory_id: "directory", display_path: "D:\\models", library_generation: "g" }; return value; }
describe("global task ownership", () => {
  it("keeps one task identity from submitting through cancellation and true terminal", async () => {
    const handle = deferred<{ operation_id: string }>(); const api = makeApi({ snapshot: vi.fn(async () => stopped()), catalog: vi.fn(async () => ({ entries: [entry] })), downloadStart: vi.fn(() => handle.promise), downloadNext: vi.fn(async () => cancelled) }); const controller = new DesktopController(api); await controller.refresh(); await controller.loadCatalog(); const pending = controller.startDownload("one", false); const id = controller.getSnapshot().activities[0].id;
    expect(controller.getSnapshot().activities[0].status).toBe("running"); await controller.cancelDownload(); expect(controller.getSnapshot().activities[0].status).toBe("stopping"); expect(api.downloadCancel).not.toHaveBeenCalled();
    handle.resolve({ operation_id: "native-download" }); await pending; await waitFor(() => expect(controller.getSnapshot().download_phase).toBe("idle")); expect(controller.getSnapshot().activities).toHaveLength(1); expect(controller.getSnapshot().activities[0]).toMatchObject({ id, status: "cancelled" }); expect(api.downloadCancel).toHaveBeenCalledExactlyOnceWith("native-download");
    controller.dismissDownloadResult(controller.getSnapshot().download!); expect(controller.getSnapshot().download).toBeNull(); controller.restoreActivity(id); expect(controller.getSnapshot().download?.status).toBe("cancelled"); expect(api.downloadStart).toHaveBeenCalledTimes(1);
  });
  it("records submission failure without pretending there is a native handle to retry", async () => {
    const api = makeApi({ snapshot: vi.fn(async () => stopped()), catalog: vi.fn(async () => ({ entries: [entry] })), downloadStart: vi.fn(async () => { throw { code: "desktop_unavailable", message: "not confirmed" }; }) }); const controller = new DesktopController(api); await controller.refresh(); await controller.loadCatalog(); await controller.startDownload("one", false); expect(controller.getSnapshot().activities[0].status).toBe("failed"); expect(controller.getSnapshot().download).toBeNull(); expect(api.downloadStart).toHaveBeenCalledTimes(1);
  });
  it("a requested cancel does not turn a real backend failure into a cancelled success", async () => {
    const response = deferred<ChatBatch>(); const api = makeApi({ chatNext: vi.fn(() => response.promise) }); const controller = new DesktopController(api); await controller.refresh(); await controller.send("private prompt"); await controller.cancel(); response.resolve({ request_id: "request-1", terminal: true, events: [{ type: "failed", code: "worker_failed", message: "worker failed" }] }); await waitFor(() => expect(controller.getSnapshot().chat_phase).toBe("idle")); expect(controller.getSnapshot().activities.find((item) => item.kind === "chat")?.status).toBe("failed"); expect(localStorage.getItem("nexa.activity-summary.v1")).not.toContain("private prompt");
  });
});
