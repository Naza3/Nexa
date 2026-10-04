import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DesktopController } from "../src/controller";
import { localValidationReason, validLocalValidation } from "../src/localValidation";
import type { DesktopApi, LocalValidation, ModelPage, Snapshot } from "../src/types";
import { deferred, makeApi, model, runtime, snapshot } from "./fixtures";

const passed = (at = Date.now()): LocalValidation => ({ state: "passed", load_success: true, generation_pass: true, checked_at_unix_ms: at, error_code: null });
const page = (validation: LocalValidation | null = passed()): ModelPage => ({ source: "runtime", generation: "generation-1", next_after: null, data: [{ ...model, validated: false, local_validation: validation }] });
async function create(overrides: Partial<DesktopApi> = {}) {
  const api = makeApi({ modelsPage: vi.fn(async () => page()), ...overrides });
  const controller = new DesktopController(api);
  await controller.refresh();
  return { api, controller };
}
beforeEach(() => { vi.useFakeTimers(); vi.setSystemTime(new Date("2026-10-04T09:00:00.000Z")); });
afterEach(() => vi.useRealTimers());

describe("current model test attempts", () => {
  it("shows a new running attempt and completion timestamp for each repeated genuine pass", async () => {
    let evidence = passed();
    let next = deferred<LocalValidation>();
    const { api, controller } = await create({ modelsPage: vi.fn(async () => page(evidence)), testModel: vi.fn(() => next.promise) });
    for (const sequence of [1, 2]) {
      vi.setSystemTime(new Date(`2026-10-04T09:0${sequence}:00.000Z`));
      const pending = controller.testModel(model.id);
      const running = controller.getSnapshot().model_tests[model.id];
      expect(running).toMatchObject({ id: sequence, phase: "running", result: null, finished_at: null });
      await vi.advanceTimersByTimeAsync(1200);
      evidence = passed(); next.resolve(evidence); await pending;
      expect(controller.getSnapshot().model_tests[model.id]).toMatchObject({ id: sequence, phase: "finished", started_at: running.started_at, finished_at: running.started_at + 1200, result: evidence, error: null });
      expect(controller.getSnapshot().models.data[0].validated).toBe(false);
      next = deferred<LocalValidation>();
    }
    expect(api.testModel).toHaveBeenCalledTimes(2);
    expect(api.chatStart).not.toHaveBeenCalled();
  });

  it.each(["validation_record_write_failed", "validation_record_read_failed", "validation_engine_unavailable", "validation_scope_unavailable"])("keeps %s visible despite an old passed record", async (code) => {
    const { controller } = await create({ testModel: vi.fn(async () => { throw { code, message: "C:\\private\\tokens secret generated-body" }; }) });
    await controller.testModel(model.id);
    const attempt = controller.getSnapshot().model_tests[model.id];
    expect(attempt).toMatchObject({ phase: "finished", result: null, error: { code } });
    expect(attempt.error?.message).not.toContain("secret");
    expect(controller.getSnapshot().notice).toBeNull();
    expect(controller.getSnapshot().models.data[0].local_validation?.state).toBe("passed");
  });

  it.each(["missing", "stale", "old_passed", "loaded", "untested"])("never substitutes %s history for a returned pass", async (kind) => {
    const old = passed(Date.now() - 60_000);
    const next: LocalValidation | null = kind === "missing" ? null : kind === "stale" ? { ...old, state: "stale" } : kind === "loaded" ? { ...old, state: "loaded", generation_pass: false } : kind === "untested" ? { ...old, state: "untested", load_success: false, generation_pass: false, checked_at_unix_ms: null } : old;
    const result = passed();
    const { controller } = await create({ modelsPage: vi.fn(async () => page(next)), testModel: vi.fn(async () => result) });
    await controller.testModel(model.id);
    expect(controller.getSnapshot().model_tests[model.id]).toMatchObject({ phase: "finished", result, error: { code: kind === "stale" ? "validation_scope_changed" : "validation_result_unconfirmed" } });
    expect(controller.getSnapshot().notice).toBeNull();
  });

  it.each(["failed", "loaded"] as const)("does not attach a late %s result to an identity that the backend now marks stale", async (state) => {
    const result: LocalValidation = { ...passed(), state, generation_pass: false, error_code: state === "failed" ? "deadline_exceeded" : null };
    const { controller } = await create({ modelsPage: vi.fn(async () => page({ ...passed(), state: "stale" })), testModel: vi.fn(async () => result) });
    await controller.testModel(model.id);
    expect(controller.getSnapshot().model_tests[model.id]).toMatchObject({ phase: "finished", error: { code: "validation_scope_changed" } });
    expect(controller.getSnapshot().notice).toBeNull();
  });

  it("preserves the precise engine-read diagnostic on a stale historical record", async () => {
    const { controller } = await create({ modelsPage: vi.fn(async () => page({ ...passed(), state: "stale", error_code: "validation_engine_unavailable" })), testModel: vi.fn(async () => passed()) });
    await controller.testModel(model.id);
    expect(controller.getSnapshot().model_tests[model.id].error?.code).toBe("validation_engine_unavailable");
    expect(controller.getSnapshot().notice).toBeNull();
  });

  it("reports an inventory refresh error instead of redisplaying a previous pass", async () => {
    const result = passed();
    const { controller } = await create({ testModel: vi.fn(async () => result), modelsPage: vi.fn().mockResolvedValueOnce(page()).mockRejectedValue({ code: "model_list_unavailable", message: "not readable" }) });
    await controller.testModel(model.id);
    expect(controller.getSnapshot().model_tests[model.id]).toMatchObject({ phase: "finished", result, error: { code: "validation_record_read_failed" } });
    expect(controller.getSnapshot().notice).toBeNull();
  });

  it("reports a snapshot refresh failure even after generation returned a valid result", async () => {
    const { controller } = await create({ testModel: vi.fn(async () => passed()), snapshot: vi.fn().mockResolvedValueOnce(snapshot()).mockRejectedValue({ code: "connection_failed", message: "unavailable" }) });
    await controller.testModel(model.id);
    expect(controller.getSnapshot().model_tests[model.id]).toMatchObject({ phase: "finished", error: { code: "validation_refresh_failed" } });
    expect(controller.getSnapshot().notice).toBeNull();
  });

  it("distinguishes missing, unreadable and malformed evidence without hiding the model", async () => {
    for (const evidence of [null, { ...passed(), state: "unavailable", load_success: false, generation_pass: false, error_code: "validation_record_read_failed" }, { ...passed(), generation_pass: false }, { ...passed(), error_code: "C:\\private\\token" }]) {
      const { controller } = await create({ modelsPage: vi.fn(async () => page(evidence as LocalValidation | null)) });
      expect(controller.getSnapshot().models.data).toHaveLength(1);
      if (evidence === null) expect(controller.getSnapshot().models.data[0].local_validation).toBeNull();
      else expect(controller.getSnapshot().models.data[0].local_validation).toMatchObject({ state: "unavailable", load_success: false, generation_pass: false });
      expect(JSON.stringify(controller.getSnapshot().models.data)).not.toContain("private");
    }
  });

  it("does not treat a successful load or historical Passed as this attempt's short generation pass", async () => {
    const { api, controller } = await create({ modelsPage: vi.fn(async () => page(passed(Date.now() - 60000))) });
    await controller.loadModel(model.id);
    expect(controller.getSnapshot().model_tests[model.id]).toMatchObject({ phase: "finished", result: { state: "loaded", generation_pass: false, error_code: "validation_result_missing" } });
    expect(api.testModel).not.toHaveBeenCalled();
  });

  it.each(["operation", "generating", "queued", "other_model"])("explicitly reports deferral for %s busy state without issuing a native test", async (kind) => {
    const value = snapshot();
    if (kind === "generating" || kind === "other_model") value.runtime = { ...runtime(), state: "generating", selected_model: kind === "other_model" ? "other" : model.id };
    if (kind === "queued") value.runtime!.queued_jobs = 1;
    const copying = deferred<{ copied: true }>();
    const { api, controller } = await create({ snapshot: vi.fn(async () => value), copyToken: vi.fn(() => copying.promise) });
    const operation = kind === "operation" ? controller.copyToken() : null;
    await controller.testModel(model.id);
    expect(controller.getSnapshot().model_tests[model.id]).toMatchObject({ phase: "finished", result: { state: "deferred", error_code: "runtime_busy" } });
    expect(api.testModel).not.toHaveBeenCalled();
    copying.resolve({ copied: true }); await operation;
  });

  it.each(["unloaded", "other_model"])("clears the attempt and shows model_not_ready for %s", async (kind) => {
    const value = snapshot(); value.runtime = { ...runtime(), selected_model: kind === "other_model" ? "other" : null, state: kind === "other_model" ? "ready" : "unloaded", load_options: kind === "other_model" ? runtime().load_options : null };
    const { api, controller } = await create({ snapshot: vi.fn(async () => value) });
    await controller.testModel(model.id);
    expect(controller.getSnapshot().model_tests[model.id]).toMatchObject({ phase: "finished", result: null, error: { code: "model_not_ready" } });
    expect(controller.getSnapshot().testing_model).toBeNull();
    expect(api.testModel).not.toHaveBeenCalled();
  });

  it("retains the first running attempt on duplicate clicks and explains that no duplicate was started", async () => {
    const next = deferred<LocalValidation>();
    const { api, controller } = await create({ testModel: vi.fn(() => next.promise) });
    const pending = controller.testModel(model.id);
    const first = controller.getSnapshot().model_tests[model.id];
    await controller.testModel(model.id);
    expect(controller.getSnapshot().model_tests[model.id]).toBe(first);
    expect(controller.getSnapshot().notice).toContain("未重复启动");
    next.resolve(passed()); await pending;
    expect(api.testModel).toHaveBeenCalledTimes(1);
  });

  it.each(["hash", "page", "view", "options", "saved_options", "backend", "stopped", "other_model", "close"])("drops a late result after %s changes", async (kind) => {
    const next = deferred<LocalValidation>();
    let value = snapshot(); let current = page();
    const { controller } = await create({ testModel: vi.fn(() => next.promise), snapshot: vi.fn(async () => structuredClone(value)), modelsPage: vi.fn(async () => structuredClone(current)) });
    const pending = controller.testModel(model.id);
    if (kind === "hash") { current.data[0].sha256 = "b".repeat(64); await controller.loadPage(null); }
    if (kind === "page") { current = { ...page(), data: [{ ...model, id: "other" }] }; await controller.loadPage("next"); }
    if (kind === "view") controller.leaveModelPage();
    if (["options", "saved_options", "backend", "stopped", "other_model"].includes(kind)) {
      if (kind === "options") value.runtime!.load_options!.context_size = 4096;
      if (kind === "saved_options") value.settings.context_size = 4096;
      if (kind === "backend") value.runtime!.configured_backend = "new-engine";
      if (kind === "stopped") value = { ...value, connection: "stopped", runtime: null };
      if (kind === "other_model") value.runtime!.selected_model = "other";
      await controller.refresh();
    }
    if (kind === "close") await controller.close();
    next.resolve(passed()); await pending;
    expect(controller.getSnapshot().model_tests[model.id]).toBeUndefined();
    expect(controller.getSnapshot().testing_model).toBeNull();
    expect(controller.getSnapshot().notice).toBeNull();
  });

  it.each(["close", "view"])("does not start a late load when %s occurs during service startup", async (kind) => {
    const starting = deferred<Snapshot>();
    const { api, controller } = await create({ snapshot: vi.fn(async () => ({ ...snapshot(), connection: "stopped" as const, runtime: null })), start: vi.fn(() => starting.promise) });
    const pending = controller.loadModel(model.id);
    expect(controller.getSnapshot().testing_model).toBe(model.id);
    if (kind === "close") await controller.close(); else controller.leaveModelPage();
    starting.resolve(snapshot()); await pending;
    expect(api.loadModel).not.toHaveBeenCalled();
    expect(controller.getSnapshot().model_tests[model.id]).toBeUndefined();
    expect(controller.getSnapshot().testing_model).toBeNull();
  });
});

describe("validation record boundary", () => {
  it("accepts unavailable only with a bounded safe error and no claimed success", () => {
    const value: LocalValidation = { state: "unavailable", load_success: false, generation_pass: false, checked_at_unix_ms: null, error_code: "validation_record_read_failed" };
    expect(validLocalValidation(value)).toBe(true);
    expect(localValidationReason(value)).toContain("不能视为未测试");
    expect(validLocalValidation({ ...value, load_success: true })).toBe(false);
    expect(validLocalValidation({ ...value, error_code: null })).toBe(false);
    expect(validLocalValidation({ ...value, error_code: "a".repeat(96) })).toBe(true);
    expect(validLocalValidation({ ...value, error_code: "a".repeat(97) })).toBe(false);
    expect(validLocalValidation({ ...value, checked_at_unix_ms: Number.MAX_SAFE_INTEGER })).toBe(false);
  });
});
