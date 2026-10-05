import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DesktopController } from "../src/controller";
import { modelTestStatus } from "../src/modelTestStatus";
import { validModelLoadOperation } from "../src/modelLoad";
import type { DesktopApi, ModelLoadOperation, LocalValidation } from "../src/types";
import { configuredSnapshot } from "./configurationFixtures";
import { deferred, makeApi, model, runtime, snapshot } from "./fixtures";

const receipt = (patch: Partial<ModelLoadOperation> = {}): ModelLoadOperation => ({
  operation_id: "00000000-0000-4000-8000-000000000001", model_id: model.id, phase: "preparing", status: "running", terminal: false,
  runtime: null, local_validation: null, error: null, ...patch,
});
const cancelled = (patch: Partial<ModelLoadOperation> = {}) => receipt({ phase: "finished", status: "cancelled", terminal: true,
  runtime: { ...runtime(), state: "unloaded" }, error: { code: "request_cancelled", message: "cancelled" }, ...patch });
const proof = (): LocalValidation => ({ state: "passed", load_success: true, generation_pass: true, error_code: null, checked_at_unix_ms: Date.now() });
async function create(overrides: Partial<DesktopApi> = {}) {
  const next = deferred<ModelLoadOperation>();
  const api = makeApi({ loadModelStart: vi.fn(async () => ({ operation_id: "00000000-0000-4000-8000-000000000001" })),
    modelLoadNext: vi.fn(() => next.promise), modelLoadCancel: vi.fn(async () => ({ stopping: true })), ...overrides });
  const controller = new DesktopController(api); await controller.refresh();
  return { api, controller, next };
}
beforeEach(() => { vi.useFakeTimers(); vi.spyOn(crypto, "randomUUID").mockReturnValue("00000000-0000-4000-8000-000000000001"); });
afterEach(() => { vi.useRealTimers(); });
const tick = () => vi.advanceTimersByTimeAsync(0);

describe("operation-owned model loading cancellation", () => {
  it("remains busy until the native terminal, sends only one cancellation, and keeps old proof neutral", async () => {
    const old = proof();
    const { api, controller, next } = await create({ modelsPage: vi.fn(async () => ({ data: [{ ...model, local_validation: old }], generation: "g", next_after: null })) });
    const pending = controller.loadModel(model.id); await tick();
    await controller.cancelModelLoad(); await controller.cancelModelLoad();
    expect(api.modelLoadCancel).toHaveBeenCalledExactlyOnceWith("00000000-0000-4000-8000-000000000001");
    expect(controller.getSnapshot().model_load?.phase).toBe("stopping");
    expect(controller.getSnapshot().operation).not.toBeNull();
    expect(controller.getSnapshot().activities[0].status).toBe("stopping");
    await controller.loadModel(model.id);
    expect(api.loadModelStart).toHaveBeenCalledTimes(1);
    next.resolve(cancelled()); await pending;
    expect(controller.getSnapshot().operation).toBeNull();
    expect(controller.getSnapshot().model_load).toBeNull();
    expect(controller.getSnapshot().activities[0].status).toBe("cancelled");
    expect(modelTestStatus(old, controller.getSnapshot().model_tests[model.id])).toMatchObject({ tone: "neutral", label: "本次操作已停止" });
    expect(api.unloadModel).not.toHaveBeenCalled(); expect(api.chatCancel).not.toHaveBeenCalled(); expect(api.stop).not.toHaveBeenCalled();
  });
  it("queues Stop while the start handle is pending and applies it only to that returned handle", async () => {
    const handle = deferred<{ operation_id: string }>();
    const { api, controller, next } = await create({ loadModelStart: vi.fn(() => handle.promise) });
    const pending = controller.loadModel(model.id); await tick();
    await controller.cancelModelLoad(); await controller.cancelModelLoad();
    expect(api.modelLoadCancel).not.toHaveBeenCalled();
    expect(controller.getSnapshot().model_load?.phase).toBe("stopping");
    handle.resolve({ operation_id: "00000000-0000-4000-8000-000000000001" }); await tick();
    expect(api.modelLoadCancel).toHaveBeenCalledExactlyOnceWith("00000000-0000-4000-8000-000000000001");
    next.resolve(cancelled()); await pending;
  });
  it("waits for service startup then skips loading if Stop was requested before submission", async () => {
    const started = deferred<ReturnType<typeof snapshot>>();
    const stopped = { ...snapshot(), connection: "stopped" as const, runtime: null };
    const { api, controller } = await create({ snapshot: vi.fn(async () => stopped), start: vi.fn(() => started.promise) });
    const pending = controller.loadModel(model.id); await tick();
    await controller.cancelModelLoad(); expect(controller.getSnapshot().operation).not.toBeNull();
    started.resolve(snapshot()); await pending;
    expect(api.loadModelStart).not.toHaveBeenCalled(); expect(api.modelLoadCancel).not.toHaveBeenCalled();
    expect(controller.getSnapshot().model_tests[model.id].outcome).toBe("cancelled"); expect(api.stop).not.toHaveBeenCalled();
  });
  it("uses canonical profile-start without forwarding legacy effective defaults", async () => {
    const { api, controller, next } = await create({ snapshot: vi.fn(async () => configuredSnapshot()), loadModelProfileStart: vi.fn(async () => ({ operation_id: "00000000-0000-4000-8000-000000000001" })) });
    const pending = controller.loadModel(model.id, { threads: 3 }); await tick();
    expect(api.loadModelProfileStart).toHaveBeenCalledExactlyOnceWith("00000000-0000-4000-8000-000000000001", model.id, { threads: 3 });
    expect(api.loadModelStart).not.toHaveBeenCalled(); expect(api.loadModel).not.toHaveBeenCalled();
    next.resolve(cancelled()); await pending;
  });
  it("preserves completed success when completion wins the cancellation race, without unloading", async () => {
    const value = proof();
    const { api, controller, next } = await create({ modelLoadCancel: vi.fn(async () => ({ stopping: false })), modelsPage: vi.fn(async () => ({ data: [{ ...model, local_validation: value }], generation: "g", next_after: null })) });
    const pending = controller.loadModel(model.id); await tick(); await controller.cancelModelLoad();
    next.resolve(receipt({ phase: "finished", status: "completed", terminal: true, runtime: runtime(), local_validation: value })); await pending;
    expect(controller.getSnapshot().model_tests[model.id].outcome).toBeUndefined();
    expect(controller.getSnapshot().activities[0].status).toBe("completed");
    expect(modelTestStatus(value, controller.getSnapshot().model_tests[model.id]).tone).toBe("passed");
    await controller.cancelModelLoad(); expect(api.modelLoadCancel).toHaveBeenCalledTimes(1); expect(api.unloadModel).not.toHaveBeenCalled();
  });
  it("preserves real failure after a requested stop", async () => {
    const { controller, next } = await create(); const pending = controller.loadModel(model.id); await tick(); await controller.cancelModelLoad();
    next.resolve(receipt({ phase: "finished", status: "failed", terminal: true, error: { code: "worker_failed", message: "private raw text" } })); await pending;
    expect(controller.getSnapshot().activities[0].status).toBe("failed"); expect(controller.getSnapshot().error?.message).not.toContain("private");
    expect(controller.getSnapshot().model_tests[model.id].outcome).toBeUndefined();
  });
  it("a cancel acknowledgement failure remains visible and retryable while polling continues", async () => {
    const cancel = vi.fn().mockRejectedValueOnce({ code: "desktop_unavailable", message: "secret path" }).mockResolvedValue({ stopping: true });
    const { api, controller, next } = await create({ modelLoadCancel: cancel });
    const pending = controller.loadModel(model.id); await tick(); await controller.cancelModelLoad();
    expect(controller.getSnapshot().model_load?.cancel_error).not.toBeNull(); expect(controller.getSnapshot().operation).not.toBeNull();
    expect(controller.getSnapshot().error?.message).not.toContain("secret");
    await controller.cancelModelLoad(); expect(api.modelLoadCancel).toHaveBeenCalledTimes(2);
    next.resolve(cancelled()); await pending;
  });
  it("does not automatically resend an unconfirmed cancel on every progress poll", async () => {
    const final = deferred<ModelLoadOperation>();
    const { api, controller } = await create({ modelLoadNext: vi.fn().mockResolvedValueOnce(receipt()).mockResolvedValueOnce(receipt()).mockImplementationOnce(() => final.promise),
      modelLoadCancel: vi.fn(async () => { throw { code: "desktop_unavailable", message: "lost" }; }) });
    const pending = controller.loadModel(model.id); await tick(); await controller.cancelModelLoad();
    await vi.advanceTimersByTimeAsync(500);
    expect(api.modelLoadCancel).toHaveBeenCalledTimes(1); expect(controller.getSnapshot().model_load?.cancel_error).not.toBeNull();
    final.resolve(cancelled()); await pending; expect(controller.getSnapshot().error).toBeNull();
  });
  it("a matching native cancelling update resolves an unconfirmed acknowledgement without replay", async () => {
    const final = deferred<ModelLoadOperation>();
    const { api, controller } = await create({ modelLoadNext: vi.fn().mockResolvedValueOnce(receipt()).mockResolvedValueOnce(receipt({ status: "cancelling" })).mockImplementationOnce(() => final.promise),
      modelLoadCancel: vi.fn(async () => { throw { code: "desktop_unavailable", message: "lost" }; }) });
    const pending = controller.loadModel(model.id); await tick(); await controller.cancelModelLoad();
    await vi.advanceTimersByTimeAsync(500);
    expect(api.modelLoadCancel).toHaveBeenCalledTimes(1); expect(controller.getSnapshot().model_load?.cancel_error).toBeNull();
    expect(controller.getSnapshot().model_load?.phase).toBe("stopping"); expect(controller.getSnapshot().operation).not.toBeNull();
    final.resolve(cancelled()); await pending;
  });
  it.each(["read-failure", "foreign-receipt", "false-terminal"])("fails closed on %s and recovers by polling the same handle without replay", async (kind) => {
    const reads = vi.fn().mockImplementationOnce(async () => {
      if (kind === "read-failure") throw { code: "desktop_unavailable", message: "private" };
      return kind === "foreign-receipt" ? cancelled({ operation_id: "other-window" }) : receipt({ terminal: true });
    }).mockResolvedValue(cancelled());
    const { api, controller } = await create({ modelLoadNext: reads }); const pending = controller.loadModel(model.id); await tick();
    expect(controller.getSnapshot().model_load?.phase).toBe("recovery"); expect(controller.getSnapshot().operation).not.toBeNull();
    await controller.loadModel(model.id); expect(api.loadModelStart).toHaveBeenCalledTimes(1);
    controller.recoverModelLoad(); await vi.advanceTimersByTimeAsync(250); await pending;
    expect(reads).toHaveBeenNthCalledWith(2, "00000000-0000-4000-8000-000000000001"); expect(api.loadModelStart).toHaveBeenCalledTimes(1);
    expect(controller.getSnapshot().model_load).toBeNull();
  });
  it("a delayed cancellation rejection from an old task cannot overwrite the new task", async () => {
    const oldCancel = deferred<{ stopping: boolean }>();
    const oldNext = deferred<ModelLoadOperation>(); const newNext = deferred<ModelLoadOperation>();
    const { api, controller } = await create({ loadModelStart: vi.fn().mockResolvedValueOnce({ operation_id: "00000000-0000-4000-8000-000000000001" }).mockResolvedValueOnce({ operation_id: "00000000-0000-4000-8000-000000000002" }),
      modelLoadNext: vi.fn().mockImplementationOnce(() => oldNext.promise).mockImplementationOnce(() => newNext.promise), modelLoadCancel: vi.fn(() => oldCancel.promise) });
    const first = controller.loadModel(model.id); await tick(); const cancelPending = controller.cancelModelLoad();
    oldNext.resolve(cancelled()); await first;
    vi.mocked(crypto.randomUUID).mockReturnValue("00000000-0000-4000-8000-000000000002");
    const second = controller.loadModel(model.id); await tick();
    oldCancel.reject({ code: "desktop_unavailable", message: "old private failure" }); await cancelPending;
    expect(controller.getSnapshot().model_load).toMatchObject({ operation_id: "00000000-0000-4000-8000-000000000002", phase: "running", cancel_error: null });
    expect(controller.getSnapshot().error).toBeNull();
    newNext.resolve(cancelled({ operation_id: "00000000-0000-4000-8000-000000000002" })); await second;
    expect(api.modelLoadCancel).toHaveBeenCalledExactlyOnceWith("00000000-0000-4000-8000-000000000001");
  });
  it.each(["lost-response", "wrong-handle"])("recovers %s by the original nonce without resubmission or foreign cancellation", async (kind) => {
    const { api, controller, next } = await create({ loadModelStart: vi.fn(async () => {
      if (kind === "lost-response") throw { code: "desktop_unavailable", message: "lost" };
      return { operation_id: "foreign-window" };
    }) });
    const pending = controller.loadModel(model.id); await tick();
    expect(api.modelLoadNext).toHaveBeenCalledExactlyOnceWith("00000000-0000-4000-8000-000000000001");
    expect(controller.getSnapshot().operation).not.toBeNull();
    next.resolve(cancelled()); await pending;
    expect(api.loadModelStart).toHaveBeenCalledTimes(1); expect(api.modelLoadCancel).not.toHaveBeenCalled();
    expect(controller.getSnapshot().activities[0].status).toBe("cancelled");
  });
  it.each([false, true])("Stop remains available during a poll failure even when start was uncertain: %s", async (uncertain) => {
    const reads = vi.fn().mockRejectedValueOnce({ code: "model_load_interrupted", message: "poll failed" }).mockResolvedValue(cancelled());
    const { api, controller } = await create({ modelLoadNext: reads, ...(uncertain ? { loadModelStart: vi.fn(async () => { throw { code: "desktop_unavailable", message: "lost" }; }) } : {}) });
    const pending = controller.loadModel(model.id); await tick();
    expect(controller.getSnapshot().model_load?.phase).toBe("recovery");
    await controller.cancelModelLoad();
    expect(api.modelLoadCancel).toHaveBeenCalledExactlyOnceWith("00000000-0000-4000-8000-000000000001");
    expect(controller.getSnapshot().model_load?.stop_requested).toBe(true); expect(controller.getSnapshot().operation).not.toBeNull();
    await vi.advanceTimersByTimeAsync(250); await pending;
    expect(controller.getSnapshot().activities[0].status).toBe("cancelled");
  });
  it("only a bridge non-ownership receipt makes a failed start safely retryable", async () => {
    const { api, controller } = await create({ loadModelStart: vi.fn(async () => { throw { code: "invalid_request", message: "private" }; }),
      modelLoadNext: vi.fn(async () => { throw { code: "request_not_owned", message: "not admitted" }; }) });
    await controller.loadModel(model.id);
    expect(controller.getSnapshot().operation).toBeNull(); expect(controller.getSnapshot().error?.code).toBe("invalid_request");
    expect(api.modelLoadCancel).not.toHaveBeenCalled();
  });
  it("an uncertain start and failed lookup keeps ownership until the same nonce can be confirmed", async () => {
    const { api, controller } = await create({ loadModelStart: vi.fn(async () => { throw { code: "desktop_unavailable", message: "lost" }; }),
      modelLoadNext: vi.fn().mockRejectedValueOnce({ code: "desktop_unavailable", message: "lost" }).mockResolvedValue(cancelled()) });
    const pending = controller.loadModel(model.id); await tick();
    expect(controller.getSnapshot().model_load?.phase).toBe("recovery"); expect(controller.getSnapshot().operation).not.toBeNull();
    controller.recoverModelLoad(); await vi.advanceTimersByTimeAsync(250); await pending;
    expect(api.loadModelStart).toHaveBeenCalledTimes(1);
  });
  it("accepts a matching completed terminal without runtime and rereads real state", async () => {
    const { api, controller, next } = await create(); const pending = controller.loadModel(model.id); await tick();
    next.resolve(receipt({ phase: "finished", status: "completed", terminal: true })); await pending;
    expect(controller.getSnapshot().model_load).toBeNull(); expect(controller.getSnapshot().operation).toBeNull();
    expect(vi.mocked(api.snapshot).mock.calls.length).toBeGreaterThan(1);
    expect(controller.getSnapshot().model_tests[model.id].result?.generation_pass).not.toBe(true);
  });
  it("does not apply a late runtime receipt after a successful window close", async () => {
    const value = snapshot(); value.runtime = { ...runtime(), state: "unloaded" };
    const { controller, next } = await create({ snapshot: vi.fn(async () => value) });
    const pending = controller.loadModel(model.id); await tick(); await controller.close();
    next.resolve(cancelled({ runtime: runtime() })); await pending;
    expect(controller.getSnapshot().snapshot?.runtime?.state).toBe("unloaded");
    expect(controller.getSnapshot().model_tests[model.id]).toBeUndefined();
  });
  it("never claims cancellation ownership from another client's runtime state", async () => {
    const value = snapshot(); value.runtime!.state = "loading";
    const { api, controller } = await create({ snapshot: vi.fn(async () => value) });
    await controller.cancelModelLoad(); await controller.loadModel(model.id);
    expect(controller.getSnapshot().model_load).toBeNull(); expect(api.modelLoadCancel).not.toHaveBeenCalled(); expect(api.loadModelStart).not.toHaveBeenCalled();
  });
  it("stops only the task's private test and reports the model still resident", async () => {
    const testing = receipt({ phase: "testing", runtime: runtime() }); const terminal = deferred<ModelLoadOperation>();
    const { api, controller } = await create({ modelLoadNext: vi.fn().mockResolvedValueOnce(testing).mockImplementationOnce(() => terminal.promise) });
    const pending = controller.loadModel(model.id); await tick(); expect(controller.getSnapshot().model_load?.progress?.phase).toBe("testing");
    await controller.cancelModelLoad(); await vi.advanceTimersByTimeAsync(250);
    terminal.resolve(cancelled({ runtime: runtime() })); await pending;
    expect(controller.getSnapshot().notice).toContain("模型仍已加载"); expect(api.chatCancel).not.toHaveBeenCalled(); expect(api.unloadModel).not.toHaveBeenCalled();
  });
});

describe("strict load receipt validation", () => {
  it.each([
    { operation_id: "foreign" }, { model_id: "foreign" }, { status: "unknown" }, { terminal: true }, { phase: "finished" },
    { local_validation: undefined }, { runtime: undefined }, { error: undefined },
    { status: "failed", phase: "finished", terminal: true, error: null },
    { status: "cancelled", phase: "finished", terminal: true, local_validation: proof() },
  ])("rejects a contradictory receipt %j", (patch) => { expect(validModelLoadOperation(receipt(patch as Partial<ModelLoadOperation>), "00000000-0000-4000-8000-000000000001", model.id)).toBe(false); });
});
