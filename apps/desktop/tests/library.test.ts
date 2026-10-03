import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DesktopController } from "../src/controller";
import type { LibraryOperation, Snapshot, DesktopApi } from "../src/types";
import { deferred, makeApi, model, snapshot } from "./fixtures";
const identity = {
  directory_id: "directory-old",
  display_path: "D:\\原有 模型",
  library_generation: "generation-old",
};
const selection = {
  selection_id: "native-selection",
  display_path: "D:\\中文 模型",
};
function stopped(): Snapshot {
  return {
    ...snapshot(),
    connection: "stopped",
    runtime: null,
    model_directory: {
      configured: identity,
      effective: null,
      state: "stopped",
    },
  };
}
function progress(
  status: LibraryOperation["status"] = "running",
): LibraryOperation {
  return {
    operation_id: "library-1",
    status,
    phase: status === "running" ? "verifying" : "finished",
    examined_entries: 3,
    candidate_files: 2,
    verified_files: status === "running" ? 1 : 2,
    terminal: status !== "running",
    result:
      status === "completed"
        ? {
            directory_id: "new-directory",
            library_generation: "new-generation",
            registered_files: 2,
            available_files: 1,
          }
        : null,
    failed_file_name: null,
    error:
      status === "failed"
        ? {
            code: "model_library_limit",
            message: "目录超过已确认上限，未保存新目录。",
          }
        : null,
  };
}
async function create(overrides: Partial<DesktopApi> = {}) {
  const api = makeApi({
    snapshot: vi.fn(async () => stopped()),
    pickDirectory: vi.fn(async () => selection),
    ...overrides,
  });
  const controller = new DesktopController(api);
  await controller.refresh();
  return { api, controller };
}
beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe("native model-directory operation ownership", () => {
  it("keeps a selected directory when picker is cancelled and never sends its display path", async () => {
    const { api, controller } = await create({
      pickDirectory: vi
        .fn()
        .mockResolvedValueOnce(selection)
        .mockResolvedValueOnce(null),
    });
    await controller.pickDirectory();
    await controller.pickDirectory();
    expect(controller.getSnapshot().directory_selection).toEqual(selection);
    expect(
      controller.getSnapshot().snapshot?.model_directory.configured,
    ).toEqual(identity);
    await controller.applyDirectory();
    await vi.advanceTimersByTimeAsync(0);
    expect(api.applyDirectory).toHaveBeenCalledExactlyOnceWith(
      selection.selection_id,
    );
    expect(api.stop).not.toHaveBeenCalled();
  });
  it("requires explicit stopped status and never automatically shuts down a running instance", async () => {
    const { api, controller } = await create({
      snapshot: vi.fn(async () => snapshot()),
    });
    await controller.pickDirectory();
    await controller.applyDirectory();
    await controller.scanModels();
    expect(api.applyDirectory).not.toHaveBeenCalled();
    expect(api.scanModels).not.toHaveBeenCalled();
    expect(api.stop).not.toHaveBeenCalled();
    expect(controller.getSnapshot().error?.code).toBe("runtime_running");
    expect(controller.getSnapshot().directory_selection).toEqual(selection);
  });
  it("preserves selection when apply is not admitted and the original library stays intact", async () => {
    const { api, controller } = await create({
      applyDirectory: vi.fn().mockRejectedValue({
        code: "runtime_running",
        message: "另一个客户端已启动服务",
      }),
    });
    await controller.pickDirectory();
    await controller.applyDirectory();
    expect(controller.getSnapshot().directory_selection).toEqual(selection);
    expect(
      controller.getSnapshot().snapshot?.model_directory.configured,
    ).toEqual(identity);
    expect(controller.getSnapshot().library_phase).toBe("idle");
    expect(api.libraryNext).not.toHaveBeenCalled();
  });
  it("blocks duplicate submissions and retains cancellation before operation id arrives", async () => {
    const handle = deferred<{ operation_id: string }>();
    const { api, controller } = await create({
      applyDirectory: vi.fn(() => handle.promise),
      libraryNext: vi.fn(async () => progress("cancelled")),
    });
    await controller.pickDirectory();
    const first = controller.applyDirectory();
    await controller.applyDirectory();
    await controller.scanModels();
    await controller.cancelLibrary();
    expect(api.applyDirectory).toHaveBeenCalledTimes(1);
    expect(api.libraryCancel).not.toHaveBeenCalled();
    expect(api.scanModels).not.toHaveBeenCalled();
    handle.resolve({ operation_id: "library-1" });
    await first;
    await vi.advanceTimersByTimeAsync(0);
    expect(api.libraryCancel).toHaveBeenCalledExactlyOnceWith("library-1");
    expect(controller.getSnapshot().library_phase).toBe("idle");
    expect(controller.getSnapshot().library?.status).toBe("cancelled");
  });
  it("polls no more than once per second with one consumer and retains busy until terminal", async () => {
    const terminal = deferred<LibraryOperation>();
    const { api, controller } = await create({
      libraryNext: vi
        .fn()
        .mockResolvedValueOnce(progress())
        .mockImplementationOnce(() => terminal.promise),
    });
    await controller.pickDirectory();
    await controller.applyDirectory();
    await vi.advanceTimersByTimeAsync(0);
    expect(api.libraryNext).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(999);
    expect(api.libraryNext).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(api.libraryNext).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(5000);
    expect(api.libraryNext).toHaveBeenCalledTimes(2);
    await controller.start(false);
    expect(api.start).not.toHaveBeenCalled();
    await controller.cancelLibrary();
    expect(controller.getSnapshot().library_phase).toBe("stopping");
    terminal.resolve(progress("cancelled"));
    await vi.advanceTimersByTimeAsync(0);
    expect(controller.getSnapshot().library_phase).toBe("idle");
    expect(
      controller.getSnapshot().snapshot?.model_directory.configured,
    ).toEqual(identity);
  });
  it("a failed cancel acknowledgement is not a terminal and can be retried", async () => {
    const terminal = deferred<LibraryOperation>();
    const { api, controller } = await create({
      libraryNext: vi.fn(() => terminal.promise),
      libraryCancel: vi
        .fn()
        .mockRejectedValueOnce({
          code: "connection_failed",
          message: "未能确认取消",
        })
        .mockResolvedValueOnce({
          operation_id: "library-1",
          status: "stopping",
        }),
    });
    await controller.pickDirectory();
    await controller.applyDirectory();
    await vi.advanceTimersByTimeAsync(0);
    await controller.cancelLibrary();
    expect(controller.getSnapshot().library_phase).toBe("stopping");
    await controller.cancelLibrary();
    expect(api.libraryCancel).toHaveBeenCalledTimes(2);
    terminal.resolve(progress("cancelled"));
    await vi.advanceTimersByTimeAsync(0);
    expect(controller.getSnapshot().library?.status).toBe("cancelled");
  });
  it("keeps successful commit when cancellation arrives too late", async () => {
    const terminal = deferred<LibraryOperation>();
    const { controller } = await create({
      libraryNext: vi.fn(() => terminal.promise),
    });
    await controller.pickDirectory();
    await controller.applyDirectory();
    await vi.advanceTimersByTimeAsync(0);
    await controller.cancelLibrary();
    terminal.resolve(progress("completed"));
    await vi.advanceTimersByTimeAsync(0);
    expect(controller.getSnapshot().library?.status).toBe("completed");
    expect(controller.getSnapshot().notice).toContain("已保存");
    expect(controller.getSnapshot().notice).not.toContain("已取消");
  });
  it("reports bound failure without applying partial registration or losing the old configured path", async () => {
    const { api, controller } = await create({
      libraryNext: vi.fn(async () => progress("failed")),
    });
    await controller.pickDirectory();
    await controller.applyDirectory();
    await vi.advanceTimersByTimeAsync(0);
    expect(controller.getSnapshot().library_phase).toBe("idle");
    expect(controller.getSnapshot().error?.code).toBe("model_library_limit");
    expect(
      controller.getSnapshot().snapshot?.model_directory.configured,
    ).toEqual(identity);
    expect(api.start).not.toHaveBeenCalled();
  });
  it("retains committed outcome when independent snapshot refresh fails", async () => {
    const { controller } = await create({
      snapshot: vi.fn().mockResolvedValueOnce(stopped()).mockRejectedValueOnce({
        code: "connection_failed",
        message: "状态读取失败",
      }),
      libraryNext: vi.fn(async () => progress("completed")),
    });
    await controller.pickDirectory();
    await controller.applyDirectory();
    await vi.advanceTimersByTimeAsync(0);
    expect(controller.getSnapshot().library?.status).toBe("completed");
    expect(controller.getSnapshot().notice).toContain("已保存");
    expect(controller.getSnapshot().error?.code).toBe("connection_failed");
  });
  it("does not replay apply after transport loss; recovery only cancels and reads the known operation", async () => {
    const { api, controller } = await create({
      libraryNext: vi
        .fn()
        .mockRejectedValueOnce({
          code: "connection_failed",
          message: "连接丢失",
        })
        .mockResolvedValueOnce(progress("cancelled")),
    });
    await controller.pickDirectory();
    await controller.applyDirectory();
    await vi.advanceTimersByTimeAsync(0);
    expect(controller.getSnapshot().library_phase).toBe("recovery");
    await controller.recoverLibrary();
    await vi.advanceTimersByTimeAsync(1000);
    expect(api.applyDirectory).toHaveBeenCalledTimes(1);
    expect(api.libraryNext).toHaveBeenCalledTimes(2);
    expect(controller.getSnapshot().library_phase).toBe("idle");
  });
  it("rejects a mismatched operation or unclosed success without declaring completion", async () => {
    const { controller, api } = await create({
      libraryNext: vi.fn(async () => ({
        ...progress("completed"),
        terminal: false,
      })),
    });
    await controller.pickDirectory();
    await controller.applyDirectory();
    await vi.advanceTimersByTimeAsync(0);
    expect(controller.getSnapshot().library_phase).toBe("recovery");
    expect(api.libraryCancel).toHaveBeenCalledTimes(1);
    expect(controller.getSnapshot().notice).toBeNull();
  });
  it("does not scan when no external directory has been selected", async () => {
    const initial = stopped();
    initial.model_directory = {
      configured: null,
      effective: null,
      state: "default",
    };
    const { controller, api } = await create({
      snapshot: vi.fn(async () => initial),
    });
    await controller.scanModels();
    expect(api.scanModels).not.toHaveBeenCalled();
    expect(controller.getSnapshot().error?.code).toBe(
      "model_directory_required",
    );
  });
});

describe("model list generation and runtime identity", () => {
  it("passes generation for later pages and restarts from an empty first-page cursor on changed generation", async () => {
    const { api, controller } = await create({
      snapshot: vi.fn(async () => snapshot()),
      modelsPage: vi
        .fn()
        .mockResolvedValueOnce({
          data: [model],
          next_after: "qwen",
          generation: "gen-old",
        })
        .mockRejectedValueOnce({
          code: "model_list_changed",
          message: "列表变化",
        })
        .mockResolvedValueOnce({
          data: [{ ...model, id: "new" }],
          next_after: null,
          generation: "gen-new",
        }),
    });
    await controller.loadPage("qwen");
    expect(api.modelsPage).toHaveBeenNthCalledWith(2, "qwen", "gen-old");
    expect(api.modelsPage).toHaveBeenNthCalledWith(3, null, null);
    expect(
      controller.getSnapshot().models.data.map((entry) => entry.id),
    ).toEqual(["new"]);
    expect(controller.getSnapshot().page_after).toBeNull();
  });
  it("does not accumulate retry loops if the first page keeps changing", async () => {
    const { api, controller } = await create({
      snapshot: vi.fn(async () => snapshot()),
      modelsPage: vi
        .fn()
        .mockRejectedValue({ code: "model_list_changed", message: "列表变化" }),
    });
    expect(api.modelsPage).toHaveBeenCalledTimes(2);
    expect(controller.getSnapshot().models.data).toEqual([]);
    expect(controller.getSnapshot().error?.code).toBe("model_list_changed");
  });
  it("invalidates an old in-flight page when effective directory changes to stale", async () => {
    const next = deferred<{
      data: (typeof model)[];
      next_after: null;
      generation: string;
    }>();
    const changed = snapshot();
    changed.model_directory = {
      configured: identity,
      effective: { ...identity, library_generation: "old" },
      state: "stale",
    };
    const { api, controller } = await create({
      snapshot: vi
        .fn()
        .mockResolvedValueOnce(snapshot())
        .mockResolvedValueOnce(changed),
      modelsPage: vi
        .fn()
        .mockResolvedValueOnce({
          data: [model],
          next_after: "qwen",
          generation: "one",
        })
        .mockImplementationOnce(() => next.promise),
    });
    const oldPage = controller.loadPage("qwen");
    await controller.refresh();
    next.resolve({ data: [model], next_after: null, generation: "one" });
    await oldPage;
    expect(controller.getSnapshot().models.data).toEqual([]);
    expect(await controller.send("do not send")).toBe(false);
    expect(api.chatStart).not.toHaveBeenCalled();
  });
  it("preserves the actual loaded display name independently of the only retained page", async () => {
    const { controller } = await create({
      snapshot: vi.fn(async () => snapshot()),
      modelsPage: vi
        .fn()
        .mockResolvedValueOnce({
          data: [model],
          next_after: "qwen",
          generation: "same",
        })
        .mockResolvedValueOnce({
          data: [{ ...model, id: "second", display_name: "另一个模型" }],
          next_after: null,
          generation: "same",
        }),
    });
    await controller.loadPage("qwen");
    expect(controller.getSnapshot().models.data).toHaveLength(1);
    expect(
      controller.getSnapshot().snapshot?.runtime?.selected_model_display_name,
    ).toBe(model.display_name);
  });
});

describe("post-rename durability ambiguity", () => {
  const error = {
    code: "settings_durability_unconfirmed",
    message: "文件替换完成，但持久化确认失败。",
  };
  it("waits out a pre-commit status read and retrieves the actual newly committed generation", async () => {
    const beforeCommit = deferred<Snapshot>();
    const next = deferred<LibraryOperation>();
    const committed = stopped();
    committed.model_directory.configured = {
      directory_id: "new-dir",
      display_path: "D:\\实际新目录",
      library_generation: "new-gen",
    };
    const { controller, api } = await create({
      snapshot: vi
        .fn()
        .mockResolvedValueOnce(stopped())
        .mockImplementationOnce(() => beforeCommit.promise)
        .mockResolvedValueOnce(committed),
      libraryNext: vi.fn(() => next.promise),
    });
    await controller.pickDirectory();
    await controller.applyDirectory();
    await vi.advanceTimersByTimeAsync(0);
    const oldRead = controller.refresh();
    next.resolve({ ...progress("failed"), error });
    await vi.advanceTimersByTimeAsync(0);
    expect(api.snapshot).toHaveBeenCalledTimes(2);
    expect(controller.getSnapshot().operation).toBe("正在重新读取模型目录");
    beforeCommit.resolve(stopped());
    await oldRead;
    await vi.advanceTimersByTimeAsync(0);
    expect(api.snapshot).toHaveBeenCalledTimes(3);
    expect(
      controller.getSnapshot().snapshot?.model_directory.configured,
    ).toEqual(committed.model_directory.configured);
    expect(controller.getSnapshot().library?.status).toBe("failed");
    expect(controller.getSnapshot().error?.code).toBe(error.code);
    expect(controller.getSnapshot().notice ?? "").not.toContain("保持不变");
    expect(controller.getSnapshot().notice ?? "").not.toContain("已取消");
    expect(api.applyDirectory).toHaveBeenCalledTimes(1);
    expect(controller.getSnapshot().operation).toBeNull();
  });
  it("keeps the uncertain terminal if the fresh configuration read itself fails", async () => {
    const { controller, api } = await create({
      snapshot: vi
        .fn()
        .mockResolvedValueOnce(stopped())
        .mockRejectedValueOnce({
          code: "connection_failed",
          message: "无法重新读取配置",
        }),
      libraryNext: vi.fn(async () => ({ ...progress("failed"), error })),
    });
    await controller.pickDirectory();
    await controller.applyDirectory();
    await vi.advanceTimersByTimeAsync(0);
    expect(controller.getSnapshot().library?.error?.code).toBe(error.code);
    expect(controller.getSnapshot().snapshot?.connection).toBe("error");
    expect(controller.getSnapshot().models.generation).toBeNull();
    expect(controller.getSnapshot().notice ?? "").not.toContain("保持不变");
    expect(api.applyDirectory).toHaveBeenCalledTimes(1);
    expect(controller.getSnapshot().operation).toBeNull();
  });
});

function partialProgress(): LibraryOperation {
  return {
    ...progress("completed"), status: "partial", verified_files: 1,
    result: { directory_id: "new-directory", library_generation: "new-generation", registered_files: 1, available_files: 1, rejected_files: 1 },
    file_errors: [{ file_name: "坏 文件.gguf", code: "invalid_manifest", message: "Invalid GGUF structure" }],
  };
}
it("accepts partial publication after late cancellation without claiming complete success", async () => {
  const terminal = deferred<LibraryOperation>();
  const { api, controller } = await create({ libraryNext: vi.fn(() => terminal.promise) });
  await controller.pickDirectory(); await controller.applyDirectory(); await vi.advanceTimersByTimeAsync(0);
  await controller.cancelLibrary(); terminal.resolve(partialProgress()); await vi.advanceTimersByTimeAsync(0);
  expect(controller.getSnapshot().library?.status).toBe("partial");
  expect(controller.getSnapshot().library?.file_errors).toHaveLength(1);
  expect(controller.getSnapshot().models.generation).toBeNull();
  expect(controller.getSnapshot().notice).toBeNull();
  expect(controller.getSnapshot().error).toBeNull();
  expect(api.snapshot).toHaveBeenCalledTimes(2);
  expect(api.applyDirectory).toHaveBeenCalledTimes(1);
});
it("retains all-rejected diagnostics and the original configured directory", async () => {
  const failed: LibraryOperation = { ...partialProgress(), status: "failed", verified_files: 0, candidate_files: 1, result: null, error: { code: "model_scan_no_usable_files", message: "No usable GGUF files" } };
  const { controller } = await create({ libraryNext: vi.fn(async () => failed) });
  await controller.pickDirectory(); await controller.applyDirectory(); await vi.advanceTimersByTimeAsync(0);
  expect(controller.getSnapshot().library_phase).toBe("idle");
  expect(controller.getSnapshot().library?.file_errors).toHaveLength(1);
  expect(controller.getSnapshot().snapshot?.model_directory.configured).toEqual(identity);
  expect(controller.getSnapshot().error?.code).toBe("model_scan_no_usable_files");
});
it("accepts a 1025-entry hard failure instead of hiding its terminal", async () => {
  const { controller } = await create({ libraryNext: vi.fn(async () => ({ ...progress("failed"), examined_entries: 1025 })) });
  await controller.pickDirectory(); await controller.applyDirectory(); await vi.advanceTimersByTimeAsync(0);
  expect(controller.getSnapshot().library_phase).toBe("idle");
  expect(controller.getSnapshot().error?.code).toBe("model_library_limit");
});
it.each([
  (value: LibraryOperation) => ({ ...value, status: "future_status" }),
  (value: LibraryOperation) => ({ ...value, file_errors: [] }),
  (value: LibraryOperation) => ({ ...value, examined_entries: 0 }),
  (value: LibraryOperation) => ({ ...value, result: { ...value.result!, registered_files: 2 } }),
  (value: LibraryOperation) => ({ ...value, result: { ...value.result!, rejected_files: 0 } }),
  (value: LibraryOperation) => ({ ...value, file_errors: [...value.file_errors!, ...value.file_errors!] }),
  (value: LibraryOperation) => ({ ...value, file_errors: [{ ...value.file_errors![0], file_name: "D:\\secret\\bad.gguf" }] }),
  (value: LibraryOperation) => ({ ...value, status: "cancelled", result: null, error: { code: "settings_durability_unconfirmed", message: "uncertain" } }),
])("does not accept inconsistent or unknown partial outcomes", async (change) => {
  const invalid = change(partialProgress()) as LibraryOperation;
  const { api, controller } = await create({ libraryNext: vi.fn(async () => invalid) });
  await controller.pickDirectory(); await controller.applyDirectory(); await vi.advanceTimersByTimeAsync(0);
  expect(controller.getSnapshot().library_phase).toBe("recovery");
  expect(controller.getSnapshot().error?.code).toBe("invalid_library_operation");
  expect(controller.getSnapshot().notice).toBeNull();
  expect(api.applyDirectory).toHaveBeenCalledTimes(1);
});
it("keeps a partial commit even when its independent refresh fails", async () => {
  const { controller } = await create({ snapshot: vi.fn().mockResolvedValueOnce(stopped()).mockRejectedValue({ code: "connection_failed", message: "Refresh failed" }), libraryNext: vi.fn(async () => partialProgress()) });
  await controller.pickDirectory(); await controller.applyDirectory(); await vi.advanceTimersByTimeAsync(0);
  expect(controller.getSnapshot().library?.status).toBe("partial");
  expect(controller.getSnapshot().library?.file_errors).toHaveLength(1);
  expect(controller.getSnapshot().library_phase).toBe("idle");
});
