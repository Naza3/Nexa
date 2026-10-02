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
import type { ChatBatch, DesktopApi } from "../src/types";
import { deferred, makeApi, model, snapshot } from "./fixtures";
async function mount(overrides: Partial<DesktopApi> = {}) {
  const api = makeApi(overrides),
    controller = new DesktopController(api);
  const result = render(<App controller={controller} />);
  await screen.findByText(model.display_name);
  return { api, controller, user: userEvent.setup(), ...result };
}
async function chat(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("button", { name: "聊天" }));
}
const terminal: ChatBatch = {
  request_id: "request-1",
  terminal: true,
  events: [{ type: "cancelled" }],
};

describe("desktop React interaction", () => {
  it("shows registered metadata, exact admission and source failures separately without enabling a candidate", async () => {
    const candidate = {
      ...model,
      id: "candidate",
      architecture: "qwen35",
      validated: false,
      available: false,
      compatibility: "architecture_unsupported" as const,
      availability_error: "model_file_changed",
    };
    const { api, user } = await mount({
      modelsPage: vi.fn(async () => ({
        data: [candidate],
        next_after: null,
        generation: "generation-1",
      })),
    });
    expect(screen.getByText("登记时已识别 GGUF")).toBeInTheDocument();
    expect(screen.getByText("本版本精确矩阵未准入")).toBeInTheDocument();
    expect(screen.getByText("引擎架构范围：不支持此架构")).toBeInTheDocument();
    expect(screen.getByText(/源文件已变动/)).toBeInTheDocument();
    expect(screen.queryByText("尚未校验")).not.toBeInTheDocument();
    const load = screen.getByRole("button", { name: "当前不可用" });
    expect(load).toBeDisabled();
    await user.click(load);
    expect(api.loadModel).not.toHaveBeenCalled();
    await user.click(screen.getByText("模型信息"));
    expect(screen.getByText(/不代表当前文件完整性/)).toBeVisible();
    expect(screen.getByText(`登记时 SHA-256：${model.sha256}`)).toBeVisible();
  });

  it("shows three pages and unknown backend honestly, including keyboard navigation", async () => {
    const { user } = await mount();
    expect(screen.getByText(/原生后端观测：unavailable/)).toBeInTheDocument();
    await chat(user);
    expect(screen.getByRole("heading", { name: "聊天" })).toBeInTheDocument();
    fireEvent.keyDown(document, { key: "3", altKey: true });
    expect(screen.getByRole("heading", { name: "设置" })).toBeInTheDocument();
    expect(screen.getByRole("spinbutton", { name: /上下文长度/ })).toHaveValue(
      2048,
    );
  });
  it("does not send the Enter used by a Chinese IME; Shift+Enter remains a newline", async () => {
    const next = deferred<ChatBatch>();
    const { user, api } = await mount({ chatNext: () => next.promise });
    await chat(user);
    const input = screen.getByRole("textbox", { name: "输入消息" });
    fireEvent.compositionStart(input);
    fireEvent.change(input, { target: { value: "中文输入" } });
    fireEvent.keyDown(input, {
      key: "Enter",
      code: "Enter",
      keyCode: 229,
      isComposing: true,
    });
    expect(api.chatStart).not.toHaveBeenCalled();
    fireEvent.compositionEnd(input);
    fireEvent.keyDown(input, { key: "Enter", shiftKey: true });
    expect(api.chatStart).not.toHaveBeenCalled();
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(api.chatStart).toHaveBeenCalledTimes(1));
    await act(async () => next.resolve(terminal));
  });
  it("keeps the same active stream after changing pages and prevents duplicate send clicks", async () => {
    const next = deferred<ChatBatch>();
    const { user, api, controller } = await mount({
      chatNext: () => next.promise,
    });
    await chat(user);
    await user.type(screen.getByRole("textbox", { name: "输入消息" }), "你好");
    const send = screen.getByRole("button", { name: "发送" });
    fireEvent.click(send);
    fireEvent.click(send);
    await user.click(screen.getByRole("button", { name: "模型" }));
    await chat(user);
    expect(screen.getByText("你好")).toBeInTheDocument();
    expect(api.chatStart).toHaveBeenCalledTimes(1);
    expect(api.chatCancel).not.toHaveBeenCalled();
    await act(async () => next.resolve(terminal));
    await waitFor(() =>
      expect(controller.getSnapshot().chat_phase).toBe("idle"),
    );
    expect(await screen.findByText("不完整")).toBeInTheDocument();
  });
  it("requires clear confirmation and waits for terminal without affecting the global runtime", async () => {
    const next = deferred<ChatBatch>();
    const { user, api, controller } = await mount({
      chatNext: () => next.promise,
    });
    await chat(user);
    await user.type(screen.getByRole("textbox", { name: "输入消息" }), "test");
    await user.click(screen.getByRole("button", { name: "发送" }));
    await user.click(screen.getByRole("button", { name: "清空会话" }));
    const dialog = screen.getByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "取消" }));
    expect(api.chatCancel).not.toHaveBeenCalled();
    expect(controller.getSnapshot().messages).toHaveLength(2);
    await user.click(screen.getByRole("button", { name: "清空会话" }));
    await user.click(screen.getByRole("button", { name: "停止并清空" }));
    expect(api.chatCancel).toHaveBeenCalledTimes(1);
    expect(controller.getSnapshot().messages).toHaveLength(2);
    expect(api.stop).not.toHaveBeenCalled();
    await act(async () => next.resolve(terminal));
    await waitFor(() =>
      expect(controller.getSnapshot().messages).toHaveLength(0),
    );
    expect(screen.getByText("从一句话开始")).toBeInTheDocument();
  });
  it("renders output as plain text, never as model-provided HTML", async () => {
    const { user } = await mount({
      chatNext: vi
        .fn()
        .mockResolvedValueOnce({
          request_id: "request-1",
          terminal: false,
          events: [
            {
              type: "delta",
              text: '<script>alert(1)</script><a href="https://example.com">unsafe</a>',
            },
          ],
        })
        .mockResolvedValueOnce(terminal),
    });
    await chat(user);
    await user.type(screen.getByRole("textbox", { name: "输入消息" }), "test");
    await user.click(screen.getByRole("button", { name: "发送" }));
    expect(await screen.findByText(/<script>alert/)).toBeInTheDocument();
    expect(screen.queryByRole("link")).not.toBeInTheDocument();
  });
  it("makes initialization an explicit user action and offers an empty-model recovery", async () => {
    const initial = snapshot();
    initial.initialized = false;
    initial.connection = "stopped";
    initial.runtime = null;
    const api = makeApi({
      snapshot: vi.fn(async () => initial),
      modelsPage: vi.fn(async () => ({
        data: [],
        next_after: null,
        generation: "generation-1",
      })),
    });
    render(<App controller={new DesktopController(api)} />);
    const init = await screen.findByRole("button", { name: "初始化并启动" });
    expect(api.start).not.toHaveBeenCalled();
    expect(screen.getByText("你的模型库还是空的")).toBeInTheDocument();
    fireEvent.click(init);
    fireEvent.click(init);
    await waitFor(() =>
      expect(api.start).toHaveBeenCalledExactlyOnceWith(true),
    );
  });
  it("saves preferences separately from runtime idle config, disclosing activation and shutdown scope", async () => {
    const { user, api } = await mount();
    await user.click(screen.getByRole("button", { name: "设置" }));
    const threads = screen.getByRole("spinbutton", { name: /推理线程/ });
    await user.clear(threads);
    await user.type(threads, "4");
    await user.click(screen.getByRole("button", { name: "保存偏好" }));
    await waitFor(() => expect(api.saveSettings).toHaveBeenCalledTimes(1));
    const saved = vi.mocked(api.saveSettings).mock.calls[0][0];
    expect(saved).toHaveProperty("threads", 4);
    expect(saved).not.toHaveProperty("idle_unload_seconds");
    expect(
      screen.getByRole("button", { name: "应用空闲卸载设置" }),
    ).toBeDisabled();
    await user.click(
      screen.getByRole("switch", { name: "关闭窗口时同时退出运行服务" }),
    );
    expect(
      screen.getByText(/关闭窗口会停止运行服务及所有客户端任务/),
    ).toBeInTheDocument();
  });
  it("copies via native command only after clipboard risk confirmation, with no token getter", async () => {
    const { user, api } = await mount();
    await user.click(screen.getByRole("button", { name: "设置" }));
    await user.click(screen.getByRole("button", { name: "复制 API 令牌" }));
    expect(api.copyToken).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog")).toHaveTextContent(
      "其他应用或剪贴板历史可能读取这份凭据",
    );
    await user.click(screen.getByRole("button", { name: "确认复制" }));
    expect(api.copyToken).toHaveBeenCalledTimes(1);
    expect(
      await screen.findByText(/令牌已复制到系统剪贴板/),
    ).toBeInTheDocument();
  });
  it("checks explicit destructive service-stop confirmation and can dismiss using Escape", async () => {
    const { user, api } = await mount();
    await user.click(screen.getByRole("button", { name: "设置" }));
    await user.click(screen.getByRole("button", { name: "停止运行服务" }));
    expect(screen.getByRole("dialog")).toHaveTextContent("所有客户端的任务");
    await user.keyboard("{Escape}");
    expect(api.stop).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
});
