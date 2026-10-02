import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it } from "vitest";
import App from "../src/App";
import { DesktopController } from "../src/controller";
import { createPreviewApi } from "../src/preview";

it("walks labelled mock initialization, explicit stop, directory apply, restart, load and chat", async () => {
  window.history.replaceState({}, "", "/?scenario=initial");
  const controller = new DesktopController(createPreviewApi());
  const user = userEvent.setup();
  const { unmount } = render(<App controller={controller} preview />);
  expect(screen.getByText(/全部运行数据与回复为模拟/)).toBeInTheDocument();
  await user.click(await screen.findByRole("button", { name: "初始化并启动" }));
  await waitFor(
    () =>
      expect(controller.getSnapshot().snapshot?.connection).toBe("connected"),
    { timeout: 2000 },
  );
  await user.click(screen.getByRole("button", { name: "设置" }));
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "选择模型目录" })).toBeEnabled(),
  );
  await user.click(screen.getByRole("button", { name: "选择模型目录" }));
  expect(
    screen.queryByRole("textbox", { name: /模型 ID/ }),
  ).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "使用此目录" })).toBeDisabled();
  await user.click(screen.getByRole("button", { name: "先停止运行服务" }));
  await user.click(
    within(screen.getByRole("dialog")).getByRole("button", {
      name: "停止运行服务",
    }),
  );
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "使用此目录" })).toBeEnabled(),
  );
  await user.click(screen.getByRole("button", { name: "使用此目录" }));
  await user.click(screen.getByRole("button", { name: "应用并核验目录" }));
  await screen.findByText(
    /模型目录已保存，登记 1 个文件/,
    {},
    { timeout: 2500 },
  );
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "启动运行服务" })).toBeEnabled(),
  );
  await user.click(screen.getByRole("button", { name: "启动运行服务" }));
  await waitFor(
    () =>
      expect(controller.getSnapshot().snapshot?.connection).toBe("connected"),
    { timeout: 2000 },
  );
  await user.click(screen.getByRole("button", { name: "模型" }));
  const load = await screen.findByRole("button", { name: "加载模型" });
  await waitFor(() => expect(load).toBeEnabled());
  expect(
    screen.getByRole("heading", { name: "Qwen3 中文 0.6B Q8_0" }),
  ).toBeInTheDocument();
  await user.click(load);
  await user.click(
    await screen.findByRole("button", { name: "开始聊天" }, { timeout: 1800 }),
  );
  await user.type(screen.getByRole("textbox", { name: "输入消息" }), "你好");
  await user.click(screen.getByRole("button", { name: "发送" }));
  await screen.findByText(/这是显式开发预览中的模拟回复/);
  await user.click(screen.getByRole("button", { name: "停止生成" }));
  await screen.findByText("不完整");
  await user.click(screen.getByRole("button", { name: "清空会话" }));
  await user.click(
    within(screen.getByRole("dialog")).getByRole("button", {
      name: "清空会话",
    }),
  );
  await waitFor(() =>
    expect(controller.getSnapshot().messages).toHaveLength(0),
  );
  unmount();
  window.history.replaceState({}, "", "/");
}, 15000);
