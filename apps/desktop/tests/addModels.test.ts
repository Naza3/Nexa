import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DesktopController } from "../src/controller";
import { validAddOperation, validModelSelection } from "../src/modelSelection";
import type { DesktopApi, LibraryOperation, ModelFileSelection } from "../src/types";
import { deferred, makeApi, model, snapshot } from "./fixtures";

const selection: ModelFileSelection = { selection_id: "opaque-native-selection", expires_in_seconds: 600, files: [{ selection_index: 0, file_name: "模型 中文.gguf", size_bytes: 1024 }] };
const batch: ModelFileSelection = { ...selection, files: [...selection.files, { selection_index: 1, file_name: "模型 中文.gguf", size_bytes: 2048 }, { selection_index: 2, file_name: "坏文件.gguf", size_bytes: 512 }] };
function progress(patch: Partial<LibraryOperation> = {}): LibraryOperation { return {
  operation_id: "library-1", status: "completed", phase: "finished", terminal: true,
  examined_entries: 1, candidate_files: 1, verified_files: 1, failed_file_name: null, file_errors: [], error: null,
  result: { directory_id: null, library_generation: "new-generation", registered_files: 1, available_files: 1, rejected_files: 0 },
  files: [{ ...selection.files[0], status: "registered", model_id: "new-model" }], ...patch,
}; }
async function create(overrides: Partial<DesktopApi> = {}) {
  const api = makeApi({ snapshot: vi.fn(async () => ({ ...snapshot(), connection: "stopped" as const, runtime: null })), pickModels: vi.fn(async () => selection), libraryNext: vi.fn(async () => progress()), ...overrides });
  const controller = new DesktopController(api); await controller.refresh();
  return { api, controller };
}
beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());
describe("explicit selected-file ownership", () => {
  it("only lists on startup and refresh, even when no default directory exists", async () => {
    const { api, controller } = await create();
    const unmount = controller.mount(); await vi.advanceTimersByTimeAsync(5000);
    await controller.refreshModels();
    expect(controller.getSnapshot().models.data).toEqual([model]);
    expect(api.modelsPage).toHaveBeenCalled();
    for (const call of [api.start, api.stop, api.discoverDirectory, api.reconcileModels, api.scanModels, api.applyDirectory]) expect(call).not.toHaveBeenCalled();
    unmount();
  });
  it("sends only an opaque handle, defaults to registration only and preserves the old index", async () => {
    const { api, controller } = await create();
    await controller.pickModels(); await controller.addModels(); await vi.advanceTimersByTimeAsync(1);
    expect(api.addModels).toHaveBeenCalledExactlyOnceWith(selection.selection_id, false);
    expect(controller.getSnapshot().model_selection).toBeNull();
    expect(controller.getSnapshot().library?.files?.[0].status).toBe("registered");
    expect(controller.getSnapshot().models.data).toEqual([model]);
    expect(controller.getSnapshot().library_phase).toBe("idle");
    for (const call of [api.start, api.stop, api.loadModel, api.unloadModel, api.scanModels, api.applyDirectory]) expect(call).not.toHaveBeenCalled();
  });
  it("shows selected bytes and clears a previous selection when the replacement picker is cancelled", async () => {
    const { api, controller } = await create({ pickModels: vi.fn().mockResolvedValueOnce(selection).mockResolvedValueOnce(null) });
    await controller.pickModels(); expect(controller.getSnapshot().model_selection).toEqual(selection);
    await controller.pickModels(); expect(controller.getSnapshot().model_selection).toBeNull();
    expect(controller.getSnapshot().notice).toContain("现有模型索引未改变");
    expect(api.addModels).not.toHaveBeenCalled();
  });
  it("discards the native lease when selection is dismissed", async () => {
    const { api, controller } = await create(); await controller.pickModels();
    await controller.discardModelSelection();
    expect(api.discardModelSelection).toHaveBeenCalledExactlyOnceWith(selection.selection_id);
    await controller.addModels(); expect(api.addModels).not.toHaveBeenCalled();
  });
  it("does not allow repeated picker calls or accept a late selection after unmount", async () => {
    const picker = deferred<ModelFileSelection | null>();
    const { api, controller } = await create({ pickModels: vi.fn(() => picker.promise) });
    const unmount = controller.mount(); const pending = controller.pickModels();
    await controller.pickModels(); unmount(); picker.resolve(selection); await pending;
    expect(api.pickModels).toHaveBeenCalledTimes(1);
    expect(controller.getSnapshot().model_selection).toBeNull();
    expect(api.discardModelSelection).toHaveBeenCalledExactlyOnceWith(selection.selection_id);
    expect(api.addModels).not.toHaveBeenCalled();
  });
  it("allows close while the picker is pending and discards its late reply", async () => {
    const picker = deferred<ModelFileSelection | null>();
    const { api, controller } = await create({ pickModels: vi.fn(() => picker.promise) });
    const pending = controller.pickModels(); await controller.close();
    expect(api.close).toHaveBeenCalledTimes(1);
    picker.resolve(selection); await pending;
    expect(api.discardModelSelection).toHaveBeenCalledWith(selection.selection_id);
    expect(controller.getSnapshot().model_selection).toBeNull();
  });
  it("rejects malformed picker data and releases its native lease", async () => {
    const { api, controller } = await create({ pickModels: vi.fn(async () => ({ ...selection, files: [{ ...selection.files[0], file_name: "C:\\private\\model.gguf" }] })) });
    await controller.pickModels();
    expect(controller.getSnapshot().error?.code).toBe("invalid_model_selection");
    expect(api.discardModelSelection).toHaveBeenCalledWith(selection.selection_id);
    expect(controller.getSnapshot().model_selection).toBeNull();
  });
  it("keeps a busy admission selection for explicit retry and consumes it only after acceptance", async () => {
    const apiAdd = vi.fn().mockRejectedValueOnce({ code: "runtime_running", message: "请先停止运行服务。" }).mockResolvedValueOnce({ operation_id: "library-1" });
    const { api, controller } = await create({ addModels: apiAdd }); await controller.pickModels();
    await controller.addModels(); expect(controller.getSnapshot().model_selection).toEqual(selection);
    expect(controller.getSnapshot().error?.code).toBe("runtime_running");
    expect(api.stop).not.toHaveBeenCalled();
    await controller.addModels(); await controller.addModels(); await vi.advanceTimersByTimeAsync(1);
    expect(apiAdd).toHaveBeenCalledTimes(2); expect(controller.getSnapshot().model_selection).toBeNull();
  });
  it("reports runtime busy without implicitly stopping or starting", async () => {
    const { api, controller } = await create({ snapshot: vi.fn(async () => snapshot()) });
    await controller.pickModels(); await controller.addModels();
    expect(controller.getSnapshot().error?.code).toBe("runtime_running");
    expect(controller.getSnapshot().model_selection).toEqual(selection);
    expect(api.addModels).not.toHaveBeenCalled(); expect(api.stop).not.toHaveBeenCalled(); expect(api.start).not.toHaveBeenCalled();
  });
  it("clears expired selections without replaying", async () => {
    const { api, controller } = await create({ addModels: vi.fn(async () => { throw { code: "selection_expired", message: "请重新选择文件。" }; }) });
    await controller.pickModels(); await controller.addModels(); await controller.addModels();
    expect(controller.getSnapshot().model_selection).toBeNull(); expect(api.addModels).toHaveBeenCalledTimes(1);
  });
  it("cancels an accepted add even if cancellation preceded its operation ID", async () => {
    const start = deferred<{ operation_id: string }>(); const terminal = deferred<LibraryOperation>();
    const { api, controller } = await create({ addModels: vi.fn(() => start.promise), libraryNext: vi.fn(() => terminal.promise) });
    await controller.pickModels(); const pending = controller.addModels(); await controller.cancelLibrary();
    expect(api.libraryCancel).not.toHaveBeenCalled(); start.resolve({ operation_id: "library-1" }); await pending;
    expect(api.libraryCancel).toHaveBeenCalledExactlyOnceWith("library-1");
    expect(controller.getSnapshot().library_phase).toBe("stopping");
    terminal.resolve(progress({ status: "cancelled", result: null, verified_files: 0, files: [{ ...selection.files[0], status: "not_processed", error_code: "model_scan_cancelled" }] }));
    await vi.advanceTimersByTimeAsync(1); expect(controller.getSnapshot().library_phase).toBe("idle");
    expect(api.addModels).toHaveBeenCalledTimes(1);
  });
  it("blocks batch auto-test and reports each duplicate-name result by selection index", async () => {
    const { api, controller } = await create({ pickModels: vi.fn(async () => batch), libraryNext: vi.fn(async () => progress({ status: "partial", examined_entries: 3, candidate_files: 3, verified_files: 2,
      files: [{ ...batch.files[0], status: "registered", model_id: "first" }, { ...batch.files[1], status: "already_registered", model_id: "second" }, { ...batch.files[2], status: "rejected", error_code: "unsupported_model" }],
      result: { directory_id: null, library_generation: "new-generation", registered_files: 2, available_files: 2, rejected_files: 1 },
    })) });
    await controller.pickModels(); controller.setAddAutoTest(true); await controller.addModels(); await vi.advanceTimersByTimeAsync(1);
    expect(api.addModels).toHaveBeenCalledExactlyOnceWith(selection.selection_id, false);
    expect(controller.getSnapshot().library?.status).toBe("partial"); expect(controller.getSnapshot().library?.files).toHaveLength(3);
    expect(api.loadModel).not.toHaveBeenCalled(); expect(api.unloadModel).not.toHaveBeenCalled();
  });
  it("sends an explicit single-file test opt-in while keeping registration success after test cancellation", async () => {
    const { api, controller } = await create({ libraryNext: vi.fn(async () => progress({ files: [{ ...selection.files[0], status: "registered", model_id: "first", local_validation: { state: "failed", load_success: true, generation_pass: false, checked_at_unix_ms: 1234, error_code: "request_cancelled" } }] })) });
    await controller.pickModels(); controller.setAddAutoTest(true); await controller.addModels(); await vi.advanceTimersByTimeAsync(1);
    expect(api.addModels).toHaveBeenCalledExactlyOnceWith(selection.selection_id, true);
    expect(controller.getSnapshot().library?.status).toBe("completed"); expect(controller.getSnapshot().error).toBeNull();
  });
  it.each(["model_scan_timeout", "model_scan_no_usable_files", "settings_durability_unconfirmed"])("shows %s without discarding previous inventory or replaying", async (code) => {
    const { api, controller } = await create({ libraryNext: vi.fn(async () => progress({ status: "failed", verified_files: 0, result: null, error: { code, message: "登记未完成。" }, files: [{ ...selection.files[0], status: "not_processed", error_code: code }] })) });
    await controller.pickModels(); await controller.addModels(); await vi.advanceTimersByTimeAsync(1);
    expect(controller.getSnapshot().error?.code).toBe(code); expect(controller.getSnapshot().models.data).toEqual([model]); expect(api.addModels).toHaveBeenCalledTimes(1);
  });
  it("retains an uncertain operation for terminal recovery rather than retrying add", async () => {
    const { api, controller } = await create({ libraryNext: vi.fn().mockRejectedValueOnce({ code: "desktop_unavailable", message: "连接中断。" }).mockResolvedValueOnce(progress()) });
    await controller.pickModels(); await controller.addModels(); await vi.advanceTimersByTimeAsync(1);
    expect(controller.getSnapshot().library_phase).toBe("recovery"); await controller.addModels();
    await controller.recoverLibrary(); await vi.advanceTimersByTimeAsync(1000);
    expect(controller.getSnapshot().library_phase).toBe("idle"); expect(api.addModels).toHaveBeenCalledTimes(1);
    expect(api.libraryCancel).toHaveBeenCalledWith("library-1");
  });
});
describe("selected-file wire validation", () => {
  it.each([null, { ...selection, files: [] }, { ...selection, expires_in_seconds: 0 }, { ...selection, files: [{ ...selection.files[0], selection_index: 1 }] }, { ...selection, files: [{ ...selection.files[0], file_name: "../x.gguf" }] }])("rejects malformed selection %j", (value) => expect(validModelSelection(value as ModelFileSelection)).toBe(false));
  it.each([{ files: [null] }, { candidate_files: 2 }, { files: [{ ...selection.files[0], status: "registered", model_id: "first", error_code: "failed" }] }, { files: [{ ...selection.files[0], status: "registered", model_id: "first", file_name: "unselected.gguf" }] }, { result: { directory_id: null, library_generation: "x", registered_files: 2, available_files: 1, rejected_files: 0 } }])("rejects malformed or unbound result %j", (patch) => expect(validAddOperation(progress(patch as unknown as Partial<LibraryOperation>), selection)).toBe(false));
});


it("retains published registration while reporting unconfirmed durability", async () => {
  const { controller } = await create({ libraryNext: vi.fn(async () => progress({ status: "failed", error: { code: "settings_durability_unconfirmed", message: "索引已发布，持久性待确认。" } })) });
  await controller.pickModels(); await controller.addModels(); await vi.advanceTimersByTimeAsync(1);
  expect(controller.getSnapshot().library_phase).toBe("idle");
  expect(controller.getSnapshot().library?.files?.[0].status).toBe("registered");
  expect(controller.getSnapshot().library?.result?.registered_files).toBe(1);
  expect(controller.getSnapshot().error?.code).toBe("settings_durability_unconfirmed");
});

it("accepts native committing and testing progress with published rows and no early result", async () => {
  const registered = progress().files!;
  const { api, controller } = await create({ libraryNext: vi.fn()
    .mockResolvedValueOnce(progress({ status: "running", phase: "committing", terminal: false, result: null, files: registered }))
    .mockResolvedValueOnce(progress({ status: "running", phase: "testing", terminal: false, result: null, files: registered }))
    .mockResolvedValueOnce(progress()),
  });
  await controller.pickModels(); controller.setAddAutoTest(true); await controller.addModels(); await vi.advanceTimersByTimeAsync(1);
  expect(controller.getSnapshot().library?.phase).toBe("committing"); expect(controller.getSnapshot().library_phase).toBe("running");
  await vi.advanceTimersByTimeAsync(1000); expect(controller.getSnapshot().library?.phase).toBe("testing");
  await controller.cancelLibrary(); await vi.advanceTimersByTimeAsync(1000);
  expect(controller.getSnapshot().library_phase).toBe("idle"); expect(controller.getSnapshot().library?.status).toBe("completed");
  expect(controller.getSnapshot().library?.files?.[0].status).toBe("registered"); expect(api.libraryCancel).toHaveBeenCalledTimes(1);
});
