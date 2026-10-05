import { openModelDetails } from "./navigation";
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import App from "../src/App";
import { DesktopController } from "../src/controller";
import type { DesktopApi, LibraryOperation, Snapshot } from "../src/types";
import { deferred, makeApi, model, snapshot } from "./fixtures";
const identity = {
  directory_id: "old-directory",
  display_path: "D:\\原有 模型",
  library_generation: "old-generation",
};
const selection = {
  selection_id: "chosen-directory",
  display_path: "D:\\中文 GGUF 模型",
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
const cancelled: LibraryOperation = {
  operation_id: "library-1",
  status: "cancelled",
  phase: "finished",
  examined_entries: 0,
  candidate_files: 0,
  verified_files: 0,
  terminal: true,
  result: null,
  error: null,
  failed_file_name: null,
};
async function setup(overrides: Partial<DesktopApi> = {}) {
  const api = makeApi({
    pickDirectory: vi.fn(async () => selection),
    ...overrides,
  });
  const controller = new DesktopController(api);
  const result = render(<App initialPage="models" controller={controller} />);
  await waitFor(() => expect(controller.getSnapshot().booting).toBe(false));
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "设置" }));
  return { api, controller, user, ...result };
}
async function configure(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("button", { name: "选择模型目录" }));
  await user.click(screen.getByRole("button", { name: "设置默认下载目录" }));
  await user.click(screen.getByRole("button", { name: "保存默认下载目录" }));
}
async function scan(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("button", { name: "手动扫描默认目录" }));
  await user.click(screen.getByRole("button", { name: "开始核验" }));
}
describe("directory settings React flow", () => {
  it("shows native read-only path and explicit stop gate with no manual model id or path input", async () => {
    const { user, api } = await setup();
    await user.click(screen.getByRole("button", { name: "选择模型目录" }));
    expect(screen.getByText(selection.display_path)).toBeInTheDocument();
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "设置默认下载目录" })).toBeDisabled();
    expect(api.stop).not.toHaveBeenCalled();
    expect(screen.getByText(/卸载模型不会释放源文件保护/)).toBeInTheDocument();
    expect(screen.getByText(/下载目标须可写/)).toBeInTheDocument();
    expect(screen.getByText(/1024 个条目、64 个 GGUF/)).toHaveTextContent(
      "300 秒",
    );
    await user.click(screen.getByRole("button", { name: "先停止运行服务" }));
    expect(screen.getByRole("dialog")).toHaveTextContent("所有客户端任务");
    await user.click(
      within(screen.getByRole("dialog")).getByRole("button", { name: "取消" }),
    );
    expect(api.stop).not.toHaveBeenCalled();
  });
  it("requires configure-only confirmation and keeps old configured directory until terminal", async () => {
    const terminal = deferred<LibraryOperation>();
    const { user, api, controller } = await setup({
      snapshot: vi.fn(async () => stopped()),
      libraryNext: vi.fn(() => terminal.promise),
    });
    await user.click(screen.getByRole("button", { name: "选择模型目录" }));
    await user.click(screen.getByRole("button", { name: "设置默认下载目录" }));
    expect(api.configureDirectory).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog")).toHaveTextContent(
      "不会复制或删除模型文件",
    );
    const confirm = screen.getByRole("button", { name: "保存默认下载目录" });
    fireEvent.click(confirm);
    fireEvent.click(confirm);
    await screen.findByRole("region", { name: "模型库操作" });
    expect(api.configureDirectory).toHaveBeenCalledTimes(1);
    expect(
      controller.getSnapshot().snapshot?.model_directory.configured,
    ).toEqual(identity);
    expect(screen.queryByRole("progressbar")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "启动运行服务" })).toBeDisabled();
    await act(async () => terminal.resolve(cancelled));
    await waitFor(() =>
      expect(controller.getSnapshot().library_phase).toBe("idle"),
    );
  });
  it("cancels across page changes and waits for the actual terminal", async () => {
    const terminal = deferred<LibraryOperation>();
    const { user, api, controller } = await setup({
      snapshot: vi.fn(async () => stopped()),
      libraryNext: vi.fn(() => terminal.promise),
    });
    await configure(user);
    await user.click(screen.getByRole("button", { name: "模型库" }));
    expect(
      screen.getByRole("region", { name: "模型库操作" }),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "取消模型库操作" }));
    expect(api.libraryCancel).toHaveBeenCalledExactlyOnceWith("library-1");
    expect(screen.getByRole("button", { name: "启动运行服务" })).toBeDisabled();
    expect(controller.getSnapshot().library_phase).toBe("stopping");
    expect(api.start).not.toHaveBeenCalled();
    await act(async () => terminal.resolve(cancelled));
    await waitFor(() =>
      expect(controller.getSnapshot().library_phase).toBe("idle"),
    );
    expect(screen.getByText(/原目录与索引保持不变/)).toBeInTheDocument();
  });
  it("keeps native close available during scanning so the shell can cancel and await cleanup", async () => {
    const terminal = deferred<LibraryOperation>();
    const { user, api } = await setup({
      snapshot: vi.fn(async () => stopped()),
      libraryNext: vi.fn(() => terminal.promise),
    });
    await configure(user);
    await user.click(screen.getByRole("button", { name: "关闭窗口并保留服务" }));
    expect(api.close).toHaveBeenCalledTimes(1);
    await act(async () => terminal.resolve(cancelled));
  });
  it("renders Chinese and spaces unchanged, keeps managed entries, and uses actual loaded name after pagination", async () => {
    const display = "千问 中文 0.6B Q8_0";
    const external = {
      ...model,
      id: "ext-00000000000040008000000000000001",
      display_name: display,
      storage: "external" as const,
    };
    const loaded = snapshot();
    loaded.runtime = {
      ...loaded.runtime!,
      selected_model: external.id,
      selected_model_display_name: display,
    };
    const { user } = await setup({
      snapshot: vi.fn(async () => loaded),
      modelsPage: vi
        .fn()
        .mockResolvedValueOnce({
          data: [external, model],
          next_after: "cursor",
          generation: "same",
        })
        .mockResolvedValueOnce({
          data: [{ ...model, id: "next" }],
          next_after: null,
          generation: "same",
        }),
    });
    await user.click(screen.getByRole("button", { name: "模型库" }));
    expect(screen.getAllByRole("heading", { name: display })).toHaveLength(1);
    expect(screen.getByLabelText("应用状态栏")).toHaveTextContent(`驻留：${display}`);
    expect(screen.queryByText(`API ID：${external.id}`)).not.toBeInTheDocument();
    await openModelDetails(display); expect(screen.getByText(`API ID：${external.id}`)).toBeVisible(); await user.click(screen.getByRole("button", { name: /返回模型库/ }));
    await user.click(screen.getByRole("button", { name: "下一页" }));
    await waitFor(() =>
      expect(screen.queryAllByRole("heading", { name: display })).toHaveLength(0),
    );
    await user.click(screen.getByRole("button", { name: "模型库" }));
    await user.click(screen.getByRole("button", { name: "聊天测试" }));
    expect(screen.getByLabelText("应用状态栏")).toHaveTextContent(display);
  });
  it("shows duplicate names with source and short id without renaming files", async () => {
    const { user } = await setup({
      modelsPage: vi.fn(async () => ({
        data: [
          model,
          { ...model, id: "ext-unique-12345678", storage: "external" as const },
        ],
        next_after: null,
        generation: "same",
      })),
    });
    await user.click(screen.getByRole("button", { name: "模型库" }));
    expect(screen.getByText("12345678")).toBeInTheDocument();
    expect(
      screen.getByText(model.id),
    ).toBeInTheDocument();
  });
  it("clearly separates configured and effective paths and blocks generation when stale", async () => {
    const stale = snapshot();
    stale.model_directory = {
      configured: identity,
      effective: {
        ...identity,
        display_path: "D:\\当前服务 旧目录",
        library_generation: "older",
      },
      state: "stale",
    };
    const { user, api } = await setup({ snapshot: vi.fn(async () => stale) });
    expect(screen.getByText(identity.display_path)).toBeInTheDocument();
    expect(screen.getByText("D:\\当前服务 旧目录")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "模型库" }));
    await user.click(screen.getByRole("button", { name: "聊天测试" }));
    await user.type(
      screen.getByRole("textbox", { name: "输入消息" }),
      "不要发给错误实例",
    );
    expect(screen.getByRole("button", { name: "发送" })).toBeDisabled();
    expect(
      screen.getByRole("button", { name: "查看运行设置" }),
    ).toBeInTheDocument();
    expect(api.chatStart).not.toHaveBeenCalled();
  });
  it("shows bounded failure as failed, preserves the old path and allows explicit retry", async () => {
    const failed = {
      ...cancelled,
      status: "failed" as const,
      failed_file_name: "坏模型 中文 <标签>.gguf",
      error: {
        code: "model_library_limit",
        message: "目录超过64个GGUF上限，原目录与索引未改变。",
      },
    };
    const { user, controller } = await setup({
      snapshot: vi.fn(async () => stopped()),
      libraryNext: vi.fn(async () => failed),
    });
    await scan(user);
    await screen.findByText(failed.error.message);
    expect(screen.getByText("坏模型 中文 <标签>.gguf")).toBeInTheDocument();
    expect(screen.queryByRole("link")).not.toBeInTheDocument();
    expect(
      controller.getSnapshot().snapshot?.model_directory.configured,
    ).toEqual(identity);
    expect(screen.getByRole("button", { name: "选择模型目录" })).toBeEnabled();
    expect(screen.queryByText(/默认目录扫描完成，本次登记/)).not.toBeInTheDocument();
  });
});

it("displays post-rename durability ambiguity with the refreshed path and no rollback promise", async () => {
  const committed = stopped();
  committed.model_directory.configured = {
    directory_id: "new-dir",
    display_path: "D:\\已实际提交的新目录",
    library_generation: "new-generation",
  };
  const failed = {
    ...cancelled,
    status: "failed" as const,
    error: {
      code: "settings_durability_unconfirmed",
      message: "持久化确认失败，索引可能已替换。",
    },
  };
  const { user, api, controller } = await setup({
    snapshot: vi
      .fn()
      .mockResolvedValueOnce(stopped())
      .mockResolvedValue(committed),
    libraryNext: vi.fn(async () => failed),
  });
  await configure(user);
  await screen.findByText("模型目录持久化尚未确认");
  await waitFor(() =>
    expect(
      controller.getSnapshot().snapshot?.model_directory.configured
        ?.library_generation,
    ).toBe("new-generation"),
  );
  expect(screen.getByText("D:\\已实际提交的新目录")).toBeInTheDocument();
  expect(
    screen.getByText(/不能保证旧目录仍在，也不代表已回滚/),
  ).toBeInTheDocument();
  expect(
    screen.queryByText("模型库操作已取消，原目录与索引保持不变。"),
  ).not.toBeInTheDocument();
  expect(api.configureDirectory).toHaveBeenCalledTimes(1);
  expect(api.stop).not.toHaveBeenCalled();
});

it("shows partial registration and rejected basenames as text across navigation", async () => {
  const partial: LibraryOperation = {
    ...cancelled, status: "partial", candidate_files: 3, examined_entries: 3, verified_files: 1,
    result: { directory_id: "new-dir", library_generation: "new-generation", registered_files: 1, available_files: 1, rejected_files: 2 },
    file_errors: [
      { file_name: "坏 <script>.gguf", code: "invalid_manifest", message: "Invalid structure" },
      { file_name: "缺模板 中文.gguf", code: "unsupported_chat_template", message: "Missing template" },
    ],
  };
  const { user, controller } = await setup({ snapshot: vi.fn(async () => stopped()), libraryNext: vi.fn(async () => partial) });
  await scan(user);
  expect(await screen.findByText("目录已部分登记：1 个已登记，2 个未登记")).toBeInTheDocument();
  expect(screen.getByText("坏 <script>.gguf")).toBeInTheDocument();
  expect(screen.getByText("缺模板 中文.gguf")).toBeInTheDocument();
  expect(screen.queryByText(/默认目录扫描完成，本次登记/)).not.toBeInTheDocument();
  expect(controller.getSnapshot().notice).toBeNull();
  expect(document.querySelector(".library-diagnostics script")).toBeNull();
  await user.click(screen.getByRole("button", { name: "模型库" }));
  expect(screen.getByLabelText("模型目录核验结果")).toHaveTextContent("仅保留在当前窗口");
  expect(screen.getByText("坏 <script>.gguf")).toBeInTheDocument();
});
