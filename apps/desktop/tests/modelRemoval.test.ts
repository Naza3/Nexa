import { describe, expect, it, vi } from "vitest";
import { DesktopController } from "../src/controller";
import { modelRemovalBlocker } from "../src/modelRemoval";
import type { DesktopApi, ModelPage, ModelUnregisterResult, Snapshot } from "../src/types";
import { deferred, makeApi, model, snapshot } from "./fixtures";
import { configuredSnapshot, modelConfiguration } from "./configurationFixtures";

export const generation = "00000000-0000-4000-8000-000000000001";
function stopped(): Snapshot { return { ...snapshot(), connection: "stopped" as const, runtime: null }; }
async function create(value = stopped(), overrides: Partial<DesktopApi> = {}) {
  let models = [model];
  let currentGeneration = generation;
  const api = makeApi({
    snapshot: vi.fn(async () => structuredClone(value)),
    modelsPage: vi.fn(async () => ({ data: structuredClone(models), next_after: null, generation: currentGeneration })),
    unregisterModel: vi.fn(async (model_id: string) => {
      models = models.filter((entry) => entry.id !== model_id); currentGeneration = "generation-2";
      if (value.runtime?.selected_model === model_id) {
        value.runtime.selected_model = null; value.runtime.selected_model_display_name = null; value.runtime.load_options = null;
      }
      return { model_id, removed: true as const, files_preserved: true as const };
    }),
    ...overrides,
  });
  const controller = new DesktopController(api);
  await controller.refresh();
  return { controller, api, value, setModels: (next: typeof models) => { models = next; currentGeneration = "reimported"; } };
}
const receipt = (): ModelUnregisterResult => ({ model_id: model.id, removed: true, files_preserved: true });

describe("model unregister controller (mock only)", () => {
  it("removes only the exact ID and resets pagination after a validated acknowledgement", async () => {
    const { controller, api } = await create();
    expect(await controller.unregisterModel(model.id, generation)).toBe(true);
    expect(api.unregisterModel).toHaveBeenCalledExactlyOnceWith(model.id, generation);
    expect(controller.getSnapshot().models.data).toEqual([]);
    expect(controller.getSnapshot().page_after).toBeNull();
    expect(controller.getSnapshot().model_removal).toMatchObject({ model_id: model.id });
    expect(controller.getSnapshot().notice).toContain("模型文件已保留");
    expect(api.stop).not.toHaveBeenCalled(); expect(api.unloadModel).not.toHaveBeenCalled();
    expect(api.scanModels).not.toHaveBeenCalled(); expect(api.reconcileModels).not.toHaveBeenCalled();
    expect(api.start).not.toHaveBeenCalled(); expect(api.loadModel).not.toHaveBeenCalled();
  });
  it("allows selected-but-unloaded history and clears only its restore selection", async () => {
    const value = snapshot(); value.runtime!.state = "unloaded";
    const { controller } = await create(value);
    expect(await controller.unregisterModel(model.id, generation)).toBe(true);
    expect(controller.getSnapshot().snapshot!.runtime).toMatchObject({ selected_model: null, selected_model_display_name: null, load_options: null });
  });
  it.each(["ready", "faulted"] as const)("rejects selected %s without implicitly unloading or stopping", async (state) => {
    const value = snapshot(); value.runtime!.state = state;
    const { controller, api } = await create(value);
    expect(await controller.unregisterModel(model.id, generation)).toBe(false);
    expect(controller.getSnapshot().error?.code).toBe(state === "faulted" ? "runtime_faulted" : "model_unregister_loaded");
    expect(api.unregisterModel).not.toHaveBeenCalled(); expect(api.unloadModel).not.toHaveBeenCalled(); expect(api.stop).not.toHaveBeenCalled();
  });
  it("blocks even an unrelated model while the runtime is faulted", async () => {
    const value = snapshot(); value.runtime!.state = "faulted"; value.runtime!.selected_model = "other";
    const { controller, api } = await create(value);
    expect(await controller.unregisterModel(model.id, generation)).toBe(false);
    expect(controller.getSnapshot().error?.code).toBe("runtime_faulted");
    expect(api.unregisterModel).not.toHaveBeenCalled(); expect(api.stop).not.toHaveBeenCalled();
  });
  it("allows an unrelated idle resident without changing it", async () => {
    const value = snapshot(); value.runtime!.selected_model = "other";
    const { controller } = await create(value);
    expect(await controller.unregisterModel(model.id, generation)).toBe(true);
    expect(controller.getSnapshot().snapshot!.runtime!.selected_model).toBe("other");
  });
  it.each([
    { state: "loading" as const }, { state: "generating" as const }, { state: "unloading" as const },
    { active_request: "external-client" }, { queued_jobs: 1 }, { registry_busy: true }, { stopping: true },
  ])("refuses busy runtime %j without an external side effect", async (patch) => {
    const value = snapshot(); Object.assign(value.runtime!, patch);
    const { controller, api } = await create(value);
    expect(await controller.unregisterModel(model.id, generation)).toBe(false);
    expect(controller.getSnapshot().error?.code).toBe("runtime_busy");
    expect(api.unregisterModel).not.toHaveBeenCalled(); expect(api.chatCancel).not.toHaveBeenCalled(); expect(api.stop).not.toHaveBeenCalled();
  });
  it.each(["error", "connecting"] as const)("refuses unconfirmed service status %s", async (connection) => {
    const { controller, api } = await create({ ...stopped(), connection });
    expect(await controller.unregisterModel(model.id, generation)).toBe(false);
    expect(api.unregisterModel).not.toHaveBeenCalled();
  });
  it("binds to the exact page generation and rejects unknown IDs", async () => {
    const { controller, api } = await create();
    expect(await controller.unregisterModel(model.id, "old-generation")).toBe(false);
    expect(controller.getSnapshot().error?.code).toBe("model_list_changed");
    expect(await controller.unregisterModel("constructor", generation)).toBe(false);
    expect(api.unregisterModel).not.toHaveBeenCalled();
  });
  it("deduplicates confirmation while native acknowledgement is pending", async () => {
    const result = deferred<ModelUnregisterResult>();
    const { controller, api, setModels } = await create(stopped(), { unregisterModel: vi.fn(() => result.promise) });
    const first = controller.unregisterModel(model.id, generation);
    expect(await controller.unregisterModel(model.id, generation)).toBe(false);
    expect(api.unregisterModel).toHaveBeenCalledTimes(1);
    expect(controller.getSnapshot().notice).toBeNull();
    setModels([]); result.resolve(receipt()); await first;
    expect(controller.getSnapshot().operation).toBeNull();
  });
  it.each([
    null, { ...receipt(), model_id: "different" }, { ...receipt(), removed: false }, { ...receipt(), files_preserved: false },
  ])("does not present an invalid acknowledgement as success: %j", async (response) => {
    const { controller, api } = await create(stopped(), { unregisterModel: vi.fn(async () => response as ModelUnregisterResult) });
    expect(await controller.unregisterModel(model.id, generation)).toBe(false);
    expect(controller.getSnapshot().model_removal).toBeNull();
    expect(controller.getSnapshot().notice).toBeNull();
    expect(controller.getSnapshot().error?.code).toBe("model_unregister_unconfirmed");
    expect(controller.getSnapshot().models.data).toHaveLength(1);
    expect(api.unregisterModel).toHaveBeenCalledTimes(1);
  });
  it("fences a page started before acknowledgement and reloads the first page", async () => {
    const old = deferred<ModelPage>();
    const { controller, api } = await create();
    vi.mocked(api.modelsPage).mockImplementationOnce(() => old.promise);
    const oldRead = controller.loadPage("older-cursor");
    const removing = controller.unregisterModel(model.id, generation);
    await Promise.resolve();
    expect(controller.getSnapshot().models.data).toEqual([]);
    old.resolve({ data: [model], next_after: "old", generation });
    await oldRead; await removing;
    expect(controller.getSnapshot().models.data).toEqual([]);
    expect(api.modelsPage).toHaveBeenLastCalledWith(null, null);
  });
  it("fences a snapshot begun during removal so old selected state cannot return", async () => {
    const old = deferred<Snapshot>(); const ack = deferred<ModelUnregisterResult>();
    const value = snapshot(); value.runtime!.state = "unloaded";
    const { controller, api, setModels } = await create(value, { unregisterModel: vi.fn(() => ack.promise) });
    const removing = controller.unregisterModel(model.id, generation);
    vi.mocked(api.snapshot).mockImplementationOnce(() => old.promise);
    const reading = controller.refresh();
    ack.resolve(receipt()); setModels([]); await Promise.resolve();
    expect(controller.getSnapshot().snapshot!.runtime!.selected_model).toBeNull();
    const after = structuredClone(value); after.runtime!.selected_model = null; after.runtime!.selected_model_display_name = null; after.runtime!.load_options = null;
    vi.mocked(api.snapshot).mockResolvedValue(after);
    old.resolve(value); await reading; await removing;
    expect(controller.getSnapshot().snapshot!.runtime!.selected_model).toBeNull();
  });
  it("evicts only removed-model UI caches and ignores a late profile receipt", async () => {
    const profile = deferred<ReturnType<typeof modelConfiguration>>();
    const { controller } = await create(configuredSnapshot(true), { configurationModelGet: vi.fn(() => profile.promise) });
    const reading = controller.refreshModelConfiguration(model.id);
    await controller.unregisterModel(model.id, generation);
    profile.resolve(modelConfiguration()); await reading;
    expect(controller.getSnapshot().model_configurations[model.id]).toBeUndefined();
    expect(controller.getSnapshot().snapshot!.configuration!.saved.model_profiles).toEqual([]);
  });
  it.each(["model_list_changed", "model_unregister_loaded", "runtime_busy", "settings_durability_unconfirmed", "connection_failed"])("rereads after %s without replaying or asserting rollback", async (code) => {
    const { controller, api } = await create(stopped(), { unregisterModel: vi.fn().mockRejectedValue({ code, message: "native rejection" }) });
    expect(await controller.unregisterModel(model.id, generation)).toBe(false);
    expect(controller.getSnapshot().error?.code).toBe(code);
    expect(controller.getSnapshot().notice).toBeNull();
    expect(controller.getSnapshot().models.data).toHaveLength(1);
    expect(api.unregisterModel).toHaveBeenCalledTimes(1);
  });
  it("keeps a confirmed result if the subsequent status read fails", async () => {
    const { controller, api } = await create();
    vi.mocked(api.snapshot).mockRejectedValueOnce({ code: "connection_failed", message: "offline" });
    expect(await controller.unregisterModel(model.id, generation)).toBe(true);
    expect(controller.getSnapshot().models.data).toEqual([]);
    expect(controller.getSnapshot().notice).toContain("已从模型库移除");
    expect(controller.getSnapshot().error?.code).toBe("connection_failed");
  });
  it("keeps uncertainty and invalidates the old inventory when both command and refresh fail", async () => {
    const { controller, api } = await create(stopped(), { unregisterModel: vi.fn().mockRejectedValue({ code: "connection_failed", message: "offline" }) });
    vi.mocked(api.snapshot).mockRejectedValueOnce({ code: "connection_failed", message: "offline" });
    expect(await controller.unregisterModel(model.id, generation)).toBe(false);
    expect(controller.getSnapshot().models.data).toEqual([model]);
    expect(controller.getSnapshot().models.generation).toBeNull();
    await controller.loadModel(model.id);
    expect(api.loadModel).not.toHaveBeenCalled();
    expect(api.start).not.toHaveBeenCalled();
    expect(controller.getSnapshot().error?.message).toContain("列表待确认");
    expect(controller.getSnapshot().notice).toBeNull();
  });
  it("allows a later explicit reimport to return with backend-scoped evidence", async () => {
    const { controller, setModels } = await create();
    await controller.unregisterModel(model.id, generation);
    const restored = { ...model, local_validation: { state: "stale" as const, load_success: true, generation_pass: true, checked_at_unix_ms: 10, error_code: null } };
    setModels([restored]); await controller.refreshModels();
    expect(controller.getSnapshot().models.data).toEqual([restored]);
  });
  it("blocks local task phases independently of runtime status", async () => {
    const { controller } = await create();
    const state = controller.getSnapshot();
    for (const patch of [{ library_phase: "running" as const }, { download_phase: "recovery" as const }, { chat_phase: "streaming" as const }, { testing_model: "other" }])
      expect(modelRemovalBlocker({ ...state, ...patch }, model.id)?.code).toBe("runtime_busy");
  });
});
