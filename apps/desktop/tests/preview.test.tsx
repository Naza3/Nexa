import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it } from "vitest";
import App from "../src/App";
import { DesktopController } from "../src/controller";
import { createPreviewApi } from "../src/preview";

it("walks the explicitly labelled mock initialization, import, load, chat, stop and clear flow", async () => {
  window.history.replaceState({}, "", "/?scenario=initial");
  const controller = new DesktopController(createPreviewApi());
  const user = userEvent.setup();
  const { unmount } = render(<App controller={controller} preview />);
  expect(screen.getByText(/全部运行数据与回复为模拟/)).toBeInTheDocument();
  await user.click(await screen.findByRole("button", { name: "初始化并启动" }));
  const pick = screen.getByRole("button", { name: "选择 GGUF 文件" });
  await waitFor(() => expect(pick).toBeEnabled(), { timeout: 2000 });
  await user.click(pick);
  await user.type(
    screen.getByRole("textbox", { name: /模型 ID/ }),
    "preview-model",
  );
  await user.click(screen.getByRole("button", { name: "确认导入" }));
  const load = await screen.findByRole(
    "button",
    { name: "加载模型" },
    { timeout: 2500 },
  );
  await waitFor(() => expect(load).toBeEnabled());
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
  await user.click(screen.getAllByRole("button", { name: "清空会话" })[1]);
  await waitFor(() =>
    expect(controller.getSnapshot().messages).toHaveLength(0),
  );
  expect(screen.getByText("从一句话开始")).toBeInTheDocument();
  unmount();
  window.history.replaceState({}, "", "/");
}, 12000);
