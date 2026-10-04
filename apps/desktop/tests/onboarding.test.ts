import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DesktopController } from "../src/controller";
import { localValidationLabel, validLocalValidation } from "../src/localValidation";
import type { DesktopApi, LocalValidation, ModelPage, Snapshot } from "../src/types";
import { deferred, makeApi, model, runtime, snapshot } from "./fixtures";

export function proof(patch: Partial<LocalValidation> = {}): LocalValidation {
  return { state: "passed", load_success: true, generation_pass: true, checked_at_unix_ms: 1791080000000, error_code: null, ...patch };
}
function stopped(): Snapshot {
  return { ...snapshot(), connection: "stopped", runtime: null, model_directory: {
    configured: { directory_id: "directory-1", display_path: "D:\\models", library_generation: "generation-1" }, effective: null, state: "stopped",
  } };
}
function page(validation = proof(), source: "local" | "runtime" = "local"): ModelPage {
  return { source, data: [{ ...model, validated: false, compatibility: "unvalidated", local_validation: validation }], generation: `${source}-generation`, next_after: null };
}
async function create(overrides: Partial<DesktopApi> = {}) {
  const api = makeApi({ snapshot: vi.fn(async () => stopped()), modelsPage: vi.fn(async () => page()), ...overrides });
  const controller = new DesktopController(api);
  await controller.refresh();
  return { api, controller };
}
beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe("offline model inventory and local evidence", () => {
  it("accepts a runtime model without local evidence and never invents a receipt", async () => {
    const { controller } = await create({ snapshot: vi.fn(async () => snapshot()), modelsPage: vi.fn(async () => ({ ...page(), source: "runtime" as const, data: [{ ...model, local_validation: null }] })) });
    expect(controller.getSnapshot().models.data).toHaveLength(1);
    expect(controller.getSnapshot().models.data[0].local_validation).toBeNull();
    expect(controller.getSnapshot().error).toBeNull();
  });
  it("reads and refreshes registered models offline without starting an API or worker", async () => {
    const { api, controller } = await create();
    expect(controller.getSnapshot().models).toMatchObject({ source: "local", generation: "local-generation" });
    await controller.refreshModels();
    expect(api.modelsPage).toHaveBeenCalledTimes(2);
    expect(api.start).not.toHaveBeenCalled();
    expect(api.loadModel).not.toHaveBeenCalled();
    expect(api.stop).not.toHaveBeenCalled();
  });

  it("starts service only for explicit load, waits for native proof and never creates chat bubbles", async () => {
    let value = stopped();
    let validation = proof({ state: "untested", load_success: false, generation_pass: false, checked_at_unix_ms: null });
    const { api, controller } = await create({
      snapshot: vi.fn(async () => structuredClone(value)),
      start: vi.fn(async () => { value = snapshot(); value.runtime = { ...runtime(), state: "unloaded", selected_model: null }; return structuredClone(value); }),
      modelsPage: vi.fn(async () => page(validation, value.connection === "stopped" ? "local" : "runtime")),
      loadModel: vi.fn(async () => { validation = proof(); value.runtime = runtime(); return runtime(); }),
    });
    await controller.loadModel(model.id);
    expect(api.start).toHaveBeenCalledExactlyOnceWith(false);
    expect(api.loadModel).toHaveBeenCalledExactlyOnceWith(model.id, { context_size: 2048, threads: 2, batch_size: 128 });
    expect(controller.getSnapshot().models.data[0]).toMatchObject({ validated: false, local_validation: { state: "passed" } });
    expect(controller.getSnapshot().messages).toEqual([]);
    expect(api.chatStart).not.toHaveBeenCalled();
    expect(api.chatNext).not.toHaveBeenCalled();
  });

  it("blocks duplicate load/test clicks while allowing close during native testing", async () => {
    const nativeLoad = deferred<ReturnType<typeof runtime>>();
    const { api, controller } = await create({ snapshot: vi.fn(async () => snapshot()), loadModel: vi.fn(() => nativeLoad.promise) });
    const loading = controller.loadModel(model.id);
    await controller.loadModel(model.id);
    await controller.testModel(model.id);
    expect(api.loadModel).toHaveBeenCalledTimes(1);
    expect(api.testModel).not.toHaveBeenCalled();
    expect(controller.getSnapshot().testing_model).toBe(model.id);
    await controller.close();
    expect(api.close).toHaveBeenCalledTimes(1);
    nativeLoad.resolve(runtime()); await loading;
    expect(controller.getSnapshot().testing_model).toBeNull();
  });

  it.each(["request_cancelled", "deadline_exceeded", "worker_failed"])("keeps load proof but never grants a pass for %s", async (code) => {
    const failed = proof({ state: "failed", generation_pass: false, error_code: code });
    const { api, controller } = await create({ snapshot: vi.fn(async () => snapshot()), testModel: vi.fn(async () => failed), modelsPage: vi.fn(async () => page(failed, "runtime")) });
    await controller.testModel(model.id);
    expect(controller.getSnapshot().models.data[0].local_validation).toEqual(failed);
    expect(controller.getSnapshot().notice).toBe("本机加载通过 · 短文本测试失败");
    expect(api.chatStart).not.toHaveBeenCalled();
    expect(api.loadModel).not.toHaveBeenCalled();
  });

  it("shows backend deferral without switching models or replaying a test", async () => {
    const deferredProof = proof({ state: "deferred", load_success: false, generation_pass: false, error_code: "runtime_busy" });
    const { api, controller } = await create({ snapshot: vi.fn(async () => snapshot()), testModel: vi.fn(async () => deferredProof), modelsPage: vi.fn(async () => page(deferredProof, "runtime")) });
    await controller.testModel(model.id);
    expect(controller.getSnapshot().notice).toBe("本机测试已暂缓");
    expect(api.testModel).toHaveBeenCalledTimes(1);
    expect(api.loadModel).not.toHaveBeenCalled();
    expect(api.unloadModel).not.toHaveBeenCalled();
  });

  it("reports this attempt as deferred when the backend retains an older passed receipt", async () => {
    const result = proof({ state: "deferred", load_success: false, generation_pass: false, error_code: "runtime_busy" });
    const { controller } = await create({ snapshot: vi.fn(async () => snapshot()), testModel: vi.fn(async () => result), modelsPage: vi.fn(async () => page(proof(), "runtime")) });
    await controller.testModel(model.id);
    expect(controller.getSnapshot().models.data[0].local_validation?.state).toBe("passed");
    expect(controller.getSnapshot().notice).toBe("本机测试已暂缓；列表中的通过标签来自此前记录。");
  });

  it("reloads authoritative failed receipts when native load rejects", async () => {
    const failed = proof({ state: "failed", load_success: false, generation_pass: false, error_code: "unsupported_model" });
    const { controller } = await create({ snapshot: vi.fn(async () => snapshot()), modelsPage: vi.fn().mockResolvedValueOnce(page()).mockResolvedValue(page(failed, "runtime")), loadModel: vi.fn(async () => { throw { code: "unsupported_model", message: "模型无法加载" }; }) });
    await controller.loadModel(model.id);
    expect(controller.getSnapshot().models.data[0].local_validation?.state).toBe("failed");
    expect(controller.getSnapshot().error?.code).toBe("unsupported_model");
    expect(controller.getSnapshot().testing_model).toBeNull();
  });

  it("never attaches a late test result to a replaced model", async () => {
    const testing = deferred<LocalValidation>();
    let currentPage = page(proof(), "runtime");
    const { api, controller } = await create({ snapshot: vi.fn(async () => snapshot()), modelsPage: vi.fn(async () => currentPage), testModel: vi.fn(() => testing.promise) });
    const pending = controller.testModel(model.id);
    currentPage = { ...currentPage, generation: "replaced", data: [{ ...model, id: "replacement", local_validation: proof({ state: "untested", load_success: false, generation_pass: false, checked_at_unix_ms: null }) }] };
    await controller.loadPage(null);
    testing.resolve(proof()); await pending;
    expect(controller.getSnapshot().models.data[0]).toMatchObject({ id: "replacement", local_validation: { state: "untested" } });
    expect(controller.getSnapshot().notice).not.toContain("通过");
    expect(api.testModel).toHaveBeenCalledTimes(1);
  });

  it("invalidates in-flight local pages across a service-source transition", async () => {
    const oldPage = deferred<ModelPage>();
    const { controller } = await create({ snapshot: vi.fn().mockResolvedValueOnce(stopped()).mockResolvedValue(snapshot()), modelsPage: vi.fn().mockResolvedValueOnce(page()).mockImplementationOnce(() => oldPage.promise).mockResolvedValue(page(proof(), "runtime")) });
    const reading = controller.loadPage(null);
    await controller.refresh();
    oldPage.resolve({ ...page(), generation: "late-local" }); await reading;
    await controller.refresh();
    expect(controller.getSnapshot().models).toMatchObject({ source: "runtime", generation: "runtime-generation" });
  });

  it("marks previous proof stale on changed parameters even if inventory reread fails", async () => {
    const saved = stopped(); saved.settings.context_size = 4096;
    const { controller } = await create({ saveSettings: vi.fn(async () => saved), modelsPage: vi.fn().mockResolvedValueOnce(page()).mockRejectedValue({ code: "model_list_unavailable", message: "列表暂不可读" }) });
    await controller.saveSettings(saved.settings);
    expect(controller.getSnapshot().models.data[0].local_validation).toMatchObject({ state: "stale", generation_pass: true });
    expect(localValidationLabel(controller.getSnapshot().models.data[0].local_validation!)).toContain("待重测");
  });
});

describe("bounded directory reconciliation", () => {
  it("waits beyond the native two-second stability window and starts only one registration", async () => {
    const operation = deferred<Awaited<ReturnType<DesktopApi["libraryNext"]>>>();
    const { api, controller } = await create({ reconcileModels: vi.fn().mockResolvedValueOnce({ status: "observing" as const, operation_id: null }).mockResolvedValueOnce({ status: "started", operation_id: "library-1" }), libraryNext: vi.fn(() => operation.promise) });
    await controller.reconcileModels();
    await vi.advanceTimersByTimeAsync(2000);
    expect(api.reconcileModels).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(201);
    expect(api.reconcileModels).toHaveBeenCalledTimes(2);
    expect(api.libraryNext).toHaveBeenCalledExactlyOnceWith("library-1");
    expect(api.start).not.toHaveBeenCalled();
    expect(api.loadModel).not.toHaveBeenCalled();
  });

  it("does not loop indefinitely when a file keeps changing", async () => {
    const { api, controller } = await create({ reconcileModels: vi.fn(async () => ({ status: "observing" as const, operation_id: null })) });
    await controller.reconcileModels();
    await vi.advanceTimersByTimeAsync(60000);
    expect(api.reconcileModels).toHaveBeenCalledTimes(2);
    expect(controller.getSnapshot().reconcile_status).toBe("observing");
    expect(api.scanModels).not.toHaveBeenCalled();
  });

  it("reports pending registration while running without stopping or changing the model", async () => {
    const value = stopped(); value.connection = "connected"; value.runtime = runtime();
    const { api, controller } = await create({ snapshot: vi.fn(async () => value), reconcileModels: vi.fn(async () => ({ status: "pending" as const, operation_id: null })) });
    await controller.reconcileModels();
    expect(controller.getSnapshot().reconcile_status).toBe("pending");
    expect(api.stop).not.toHaveBeenCalled(); expect(api.start).not.toHaveBeenCalled();
    expect(api.loadModel).not.toHaveBeenCalled(); expect(api.libraryNext).not.toHaveBeenCalled();
  });

  it("does not reconcile on startup or the one-second status heartbeat", async () => {
    const { api, controller } = await create();
    const unmount = controller.mount();
    await vi.advanceTimersByTimeAsync(10000);
    expect(api.reconcileModels).not.toHaveBeenCalled();
    expect(api.scanModels).not.toHaveBeenCalled();
    unmount();
  });

  it("cancels a scheduled observation when the app is unmounted", async () => {
    const { api, controller } = await create({ reconcileModels: vi.fn(async () => ({ status: "observing" as const, operation_id: null })) });
    const unmount = controller.mount(); await vi.advanceTimersByTimeAsync(1);
    await controller.reconcileModels(); unmount();
    await vi.advanceTimersByTimeAsync(10000);
    expect(api.reconcileModels).toHaveBeenCalledTimes(1);
  });
});

describe("receipt honesty", () => {
  it.each([
    { generation_pass: false }, { load_success: false }, { checked_at_unix_ms: null }, { error_code: "request_cancelled" },
    { state: "failed", generation_pass: true }, { state: "loaded", generation_pass: true }, { state: "untested", load_success: true },
    { checked_at_unix_ms: NaN }, { error_code: "private\npath" },
  ])("rejects contradictory or malformed local evidence: %j", (patch) => {
    expect(validLocalValidation(proof(patch as Partial<LocalValidation>))).toBe(false);
  });
  it("preserves expired proof only as historical fact", () => {
    const value = proof({ state: "stale" });
    expect(validLocalValidation(value)).toBe(true);
    expect(localValidationLabel(value)).toBe("本机记录已过期 · 待重测");
  });
});
