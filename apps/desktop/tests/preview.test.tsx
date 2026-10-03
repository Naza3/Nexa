import { act, configure, getConfig, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import App from "../src/App";
import { DesktopController } from "../src/controller";
import { createPreviewApi } from "../src/preview";

/** Exercise the preview's real delays and 1 Hz polling without wall-clock CI races. */
async function untilPreview<T>(stage: string, assert: () => T, budget = 3000): Promise<T> {
  let failure: unknown;
  for (let elapsed = 0; elapsed < budget; elapsed += 50) {
    await act(async () => { await vi.advanceTimersByTimeAsync(50); });
    try { return assert(); } catch (error) { failure = error; }
  }
  throw new Error(`预览阶段“${stage}”在 ${budget}ms 虚拟时间内未就绪`, { cause: failure });
}

function readyButton(name: string) {
  return untilPreview(`按钮可操作：${name}`, () => {
    const button = screen.getByRole("button", { name });
    expect(button).toBeEnabled();
    return button;
  });
}

it("walks labelled mock initialization, explicit stop, directory apply, restart, load and chat", async () => {
  const previousAsyncWrapper = getConfig().asyncWrapper;
  const previousUrl = window.location.href;
  let unmount: (() => void) | undefined;
  try {
    vi.useFakeTimers();
    // Keep user-event inside React act while its injected timer driver advances.
    // The default RTL wrapper drains a real zero-delay task via Jest detection,
    // which does not recognize Vitest's clock.
    configure({ asyncWrapper: async <T,>(callback: () => Promise<T>) => {
      let result!: T;
      await act(async () => { result = await callback(); });
      return result;
    } });
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    window.history.replaceState({}, "", "/?scenario=initial");
    // The preview reads its scenario at construction, before the UI is mounted.
    const controller = new DesktopController(createPreviewApi());
    ({ unmount } = render(<App controller={controller} preview />));
    expect(screen.getByText(/全部运行数据与回复为模拟/)).toBeInTheDocument();
    // Auto-discovery may briefly disable this button after the initial snapshot.
    // Appearance alone does not establish that a click can be accepted.
    await untilPreview("初始化前的发现操作已结束", () => {
      expect(controller.getSnapshot().booting).toBe(false);
      expect(controller.getSnapshot().operation).toBeNull();
      expect(controller.getSnapshot().library_phase).toBe("idle");
    });
    await user.click(await readyButton("初始化并启动"));
    await untilPreview("初始化已连接且操作完成", () => {
      expect(controller.getSnapshot().snapshot?.connection).toBe("connected");
      expect(controller.getSnapshot().operation).toBeNull();
      expect(controller.getSnapshot().library_phase).toBe("idle");
    });
    await user.click(await readyButton("设置"));
    await user.click(await readyButton("选择模型目录"));
    expect(screen.queryByRole("textbox", { name: /模型 ID/ })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "使用此目录" })).toBeDisabled();
    await user.click(await readyButton("先停止运行服务"));
    await user.click(within(screen.getByRole("dialog")).getByRole("button", { name: "停止运行服务" }));
    await user.click(await readyButton("使用此目录"));
    await user.click(await readyButton("应用并核验目录"));
    await untilPreview("目录核验完成", () => {
      expect(screen.getByText(/模型目录已保存，登记 1 个文件/)).toBeInTheDocument();
      expect(controller.getSnapshot().library_phase).toBe("idle");
      expect(controller.getSnapshot().operation).toBeNull();
    });
    await user.click(await readyButton("启动运行服务"));
    await untilPreview("重启已连接且操作完成", () => {
      expect(controller.getSnapshot().snapshot?.connection).toBe("connected");
      expect(controller.getSnapshot().operation).toBeNull();
    });
    await user.click(await readyButton("模型"));
    const load = await readyButton("加载模型");
    expect(screen.getByRole("heading", { name: "Qwen3 中文 0.6B Q8_0" })).toBeInTheDocument();
    await user.click(load);
    await user.click(await readyButton("开始聊天"));
    await user.type(screen.getByRole("textbox", { name: "输入消息" }), "你好");
    await user.click(await readyButton("发送"));
    await untilPreview("收到模拟流式回复", () => expect(screen.getByText(/这是显式开发预览中的模拟回复/)).toBeInTheDocument());
    await user.click(await readyButton("停止生成"));
    await untilPreview("取消产生不完整回复", () => expect(screen.getByText("不完整")).toBeInTheDocument());
    await user.click(await readyButton("清空会话"));
    await user.click(within(screen.getByRole("dialog")).getByRole("button", { name: "清空会话" }));
    await untilPreview("清空已完成", () => expect(controller.getSnapshot().messages).toHaveLength(0));
  } finally {
    try {
      unmount?.();
    } finally {
      vi.clearAllTimers();
      vi.useRealTimers();
      configure({ asyncWrapper: previousAsyncWrapper });
      window.history.replaceState({}, "", previousUrl);
    }
  }
}, 15000);
