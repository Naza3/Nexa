import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "../src/App";
import { DesktopController } from "../src/controller";
import type { DesktopApi, LanApiSettings, LibraryOperation, ModelFileSelection, Snapshot } from "../src/types";
import { deferred, makeApi, snapshot } from "./fixtures";

const enabled: LanApiSettings = { enabled: true, listen: "192.168.1.20:18081", allowed_cidrs: ["192.168.1.30/32"] };
const selection: ModelFileSelection = { selection_id: "native-selection", expires_in_seconds: 600, files: [{ selection_index: 0, file_name: "本地模型.gguf", size_bytes: 1024 }] };
const stopped = (): Snapshot => ({ ...snapshot(), connection: "stopped", runtime: null, lan_api: enabled });
const cancelled: LibraryOperation = {
  operation_id: "library-1", status: "cancelled", phase: "finished", terminal: true,
  examined_entries: 0, candidate_files: 1, verified_files: 0, failed_file_name: null, file_errors: [], error: null, result: null,
  files: [{ ...selection.files[0], status: "not_processed", error_code: "model_scan_cancelled" }],
};
async function create(overrides: Partial<DesktopApi> = {}) {
  const api = makeApi({ snapshot: vi.fn(async () => stopped()), pickModels: vi.fn(async () => selection), ...overrides });
  const controller = new DesktopController(api);
  await controller.refresh();
  return { api, controller };
}

describe("selected-file and LAN controller integration", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("keeps a selected-file lease through LAN saving and blocks add, repick and close until save finishes", async () => {
    const saved = deferred<Snapshot>();
    const { api, controller } = await create({ saveLanSettings: vi.fn(() => saved.promise) });
    await controller.pickModels();
    const saving = controller.saveLanSettings({ ...enabled, enabled: false });
    await controller.pickModels(); await controller.addModels(); await controller.close();
    expect(api.pickModels).toHaveBeenCalledTimes(1);
    expect(api.addModels).not.toHaveBeenCalled(); expect(api.close).not.toHaveBeenCalled();
    expect(controller.getSnapshot().model_selection).toEqual(selection);
    saved.resolve({ ...stopped(), lan_api: { ...enabled, enabled: false } }); await saving;
    expect(controller.getSnapshot().model_selection).toEqual(selection);
    await controller.discardModelSelection();
    expect(api.discardModelSelection).toHaveBeenCalledExactlyOnceWith(selection.selection_id);
    for (const call of [api.start, api.stop, api.discoverDirectory, api.reconcileModels, api.scanModels]) expect(call).not.toHaveBeenCalled();
  });

  it("blocks LAN actions while the native picker is pending and releases its late selection after close", async () => {
    const picker = deferred<ModelFileSelection | null>();
    const { api, controller } = await create({ pickModels: vi.fn(() => picker.promise) });
    const picking = controller.pickModels();
    await controller.saveLanSettings(enabled); await controller.copyLanToken(); await controller.copyLanBaseUrl();
    expect(api.saveLanSettings).not.toHaveBeenCalled(); expect(api.copyLanToken).not.toHaveBeenCalled();
    expect(controller.getSnapshot().operation).toBe("正在选择 GGUF 文件");
    await controller.close(); expect(api.close).toHaveBeenCalledTimes(1);
    picker.resolve(selection); await picking;
    expect(api.discardModelSelection).toHaveBeenCalledExactlyOnceWith(selection.selection_id);
    expect(controller.getSnapshot().model_selection).toBeNull();
    expect(controller.getSnapshot().operation).toBeNull();
  });

  it("keeps LAN blocked through add admission and cancellation until the authoritative terminal reply", async () => {
    const admitted = deferred<{ operation_id: string }>();
    const terminal = deferred<LibraryOperation>();
    const { api, controller } = await create({ addModels: vi.fn(() => admitted.promise), libraryNext: vi.fn(() => terminal.promise) });
    await controller.pickModels(); const adding = controller.addModels();
    await controller.saveLanSettings(enabled); await controller.copyLanToken();
    expect(api.saveLanSettings).not.toHaveBeenCalled(); expect(api.copyLanToken).not.toHaveBeenCalled();
    await controller.cancelLibrary(); admitted.resolve({ operation_id: "library-1" }); await adding;
    await vi.advanceTimersByTimeAsync(1);
    expect(api.libraryCancel).toHaveBeenCalledExactlyOnceWith("library-1");
    expect(controller.getSnapshot().library_phase).toBe("stopping");
    await controller.saveLanSettings(enabled); expect(api.saveLanSettings).not.toHaveBeenCalled();
    terminal.resolve(cancelled); await vi.advanceTimersByTimeAsync(1);
    expect(controller.getSnapshot().library_phase).toBe("idle");
    await controller.saveLanSettings(enabled); expect(api.saveLanSettings).toHaveBeenCalledExactlyOnceWith(enabled);
    expect(controller.getSnapshot().library?.files).toEqual(cancelled.files);
    expect(api.addModels).toHaveBeenCalledTimes(1);
  });

  it("keeps LAN blocked during add recovery without replaying registration", async () => {
    const { api, controller } = await create({ libraryNext: vi.fn()
      .mockRejectedValueOnce({ code: "desktop_unavailable", message: "连接中断。" })
      .mockResolvedValueOnce(cancelled) });
    await controller.pickModels(); await controller.addModels(); await vi.advanceTimersByTimeAsync(1);
    expect(controller.getSnapshot().library_phase).toBe("recovery");
    await controller.saveLanSettings(enabled); await controller.copyLanToken();
    expect(api.saveLanSettings).not.toHaveBeenCalled(); expect(api.copyLanToken).not.toHaveBeenCalled();
    await controller.recoverLibrary(); await vi.advanceTimersByTimeAsync(1000);
    expect(controller.getSnapshot().library_phase).toBe("idle");
    await controller.copyLanToken(); expect(api.copyLanToken).toHaveBeenCalledTimes(1);
    expect(api.addModels).toHaveBeenCalledTimes(1);
  });

  it("does not admit LAN saves or another selection while close waits for lease release", async () => {
    const discarded = deferred<{ discarded: boolean }>();
    const { api, controller } = await create({ discardModelSelection: vi.fn(() => discarded.promise) });
    await controller.pickModels(); const closing = controller.close();
    await controller.saveLanSettings(enabled); await controller.copyLanToken(); await controller.pickModels(); await controller.addModels();
    expect(api.saveLanSettings).not.toHaveBeenCalled(); expect(api.copyLanToken).not.toHaveBeenCalled();
    expect(api.pickModels).toHaveBeenCalledTimes(1); expect(api.addModels).not.toHaveBeenCalled();
    expect(api.close).not.toHaveBeenCalled();
    discarded.resolve({ discarded: true }); await closing;
    expect(api.close).toHaveBeenCalledTimes(1);
    expect(controller.getSnapshot().model_selection).toBeNull();
  });
});

describe("selected-file and LAN settings UI integration", () => {
  it("preserves selected files when cancelling the extracted LAN modal, then releases them on explicit cancellation", async () => {
    const { api, controller } = await create(); render(<App controller={controller} />);
    fireEvent.click(screen.getByRole("button", { name: "添加模型" }));
    await screen.findByRole("region", { name: "添加选中的模型" });
    fireEvent.click(screen.getByRole("button", { name: "设置" }));
    fireEvent.click(screen.getByRole("button", { name: "复制局域网 API 密钥" }));
    expect(screen.getByRole("dialog")).toHaveTextContent("其他应用或剪贴板历史");
    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(api.copyLanToken).not.toHaveBeenCalled(); expect(api.discardModelSelection).not.toHaveBeenCalled();
    expect(controller.getSnapshot().model_selection).toEqual(selection);
    fireEvent.click(screen.getByRole("button", { name: "取消文件选择" }));
    expect(api.discardModelSelection).toHaveBeenCalledExactlyOnceWith(selection.selection_id);
    expect(screen.queryByRole("region", { name: "添加选中的模型" })).not.toBeInTheDocument();
  });

  it("disables LAN during add cancellation and restores it only after the native terminal state", async () => {
    const terminal = deferred<LibraryOperation>();
    const { api, controller } = await create({ libraryNext: vi.fn(() => terminal.promise) });
    render(<App controller={controller} />);
    fireEvent.click(screen.getByRole("button", { name: "添加模型" }));
    fireEvent.click(await screen.findByRole("button", { name: "确认添加 1 个模型" }));
    await waitFor(() => expect(api.libraryNext).toHaveBeenCalledTimes(1));
    fireEvent.click(screen.getByRole("button", { name: "设置" }));
    expect(screen.getByRole("switch", { name: "启用局域网 API" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "复制局域网 API 密钥" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "取消添加" }));
    expect(screen.getByRole("button", { name: "等待取消确认" })).toBeDisabled();
    expect(screen.getByRole("switch", { name: "启用局域网 API" })).toBeDisabled();
    await act(async () => terminal.resolve(cancelled));
    await waitFor(() => expect(screen.getByRole("switch", { name: "启用局域网 API" })).toBeEnabled());
    expect(screen.getByRole("heading", { name: "添加已取消" })).toBeInTheDocument();
    expect(api.saveLanSettings).not.toHaveBeenCalled(); expect(api.copyLanToken).not.toHaveBeenCalled();
  });
});
