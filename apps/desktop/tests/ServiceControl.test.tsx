import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import App from "../src/App";
import { DesktopController } from "../src/controller";
import type { ChatBatch, DesktopApi, Snapshot } from "../src/types";
import { deferred, makeApi, snapshot } from "./fixtures";

const stopped = (): Snapshot => ({ ...snapshot(), connection: "stopped", runtime: null, api_address: null });
async function setup(overrides: Partial<DesktopApi> = {}) {
  const api = makeApi(overrides); const controller = new DesktopController(api);
  render(<App initialPage="models" controller={controller} />);
  await waitFor(() => expect(controller.getSnapshot().booting).toBe(false));
  return { api, controller, user: userEvent.setup() };
}
const control = () => within(screen.getByRole("region", { name: "运行服务控制" }));

describe("persistent sidebar service control", () => {
  it("keeps one primary service control available on every page", async () => {
    const { user, api } = await setup();
    for (const page of ["模型库", "API 接入", "活动", "设置"]) {
      await user.click(screen.getByRole("button", { name: page }));
      expect(control().getByRole("button", { name: "停止运行服务" })).toBeEnabled();
      expect(screen.getAllByRole("button", { name: "停止运行服务" })).toHaveLength(1);
      expect(within(screen.getByLabelText("应用状态栏")).getByText("服务运行中")).toBeInTheDocument();
    }
    expect(api.start).not.toHaveBeenCalled(); expect(api.stop).not.toHaveBeenCalled();
  });
  it("starts a stopped service once and stays pending across navigation until completion", async () => {
    let current = stopped(); const pending = deferred<Snapshot>();
    const { api, user } = await setup({ snapshot: vi.fn(async () => current), start: vi.fn(() => pending.promise) });
    const start = control().getByRole("button", { name: "启动运行服务" });
    fireEvent.click(start); fireEvent.click(start);
    expect(api.start).toHaveBeenCalledExactlyOnceWith(false);
    expect(control().getByRole("button", { name: "正在启动服务…" })).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "设置" }));
    expect(control().getByRole("button", { name: "正在启动服务…" })).toBeDisabled();
    current = snapshot(); await act(async () => pending.resolve(current));
    expect(control().getByRole("button", { name: "停止运行服务" })).toBeEnabled();
  });
  it("requires explicit initialization and never uses start(false) for a new installation", async () => {
    const { api, user } = await setup({ snapshot: vi.fn(async () => ({ ...stopped(), initialized: false })) });
    expect(control().getByRole("button", { name: "初始化并启动" })).toBeEnabled();
    expect(api.start).not.toHaveBeenCalled();
    await user.click(control().getByRole("button", { name: "初始化并启动" }));
    expect(api.start).toHaveBeenCalledExactlyOnceWith(true);
  });
  it("discloses other applications before stopping, cancels with Escape or navigation, and deduplicates confirmation", async () => {
    let current = snapshot(); const pending = deferred<{ stopped: true }>();
    const { api, user } = await setup({ snapshot: vi.fn(async () => current), stop: vi.fn(() => pending.promise) });
    await user.click(control().getByRole("button", { name: "停止运行服务" }));
    expect(screen.getByRole("dialog")).toHaveTextContent("可能中断其他应用正在进行的调用");
    await user.keyboard("{Escape}"); expect(api.stop).not.toHaveBeenCalled();
    await user.click(control().getByRole("button", { name: "停止运行服务" }));
    fireEvent.keyDown(document, { altKey: true, key: "3" });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument(); expect(api.stop).not.toHaveBeenCalled();
    await user.click(control().getByRole("button", { name: "停止运行服务" }));
    const confirm = within(screen.getByRole("dialog")).getByRole("button", { name: "停止运行服务" });
    fireEvent.click(confirm); fireEvent.click(confirm);
    expect(api.stop).toHaveBeenCalledTimes(1);
    expect(control().getByRole("button", { name: "正在停止服务…" })).toBeDisabled();
    expect(control().queryByText("服务已停止")).not.toBeInTheDocument();
    current = stopped(); await act(async () => pending.resolve({ stopped: true }));
    expect(control().getByRole("button", { name: "启动运行服务" })).toBeEnabled();
  });
  it.each([true, false])("offers only a status check when connection is unknown (initialized=%s)", async (initialized) => {
    let current: Snapshot = { ...stopped(), initialized, connection: "error" };
    const pending = deferred<Snapshot>();
    const api = makeApi({ snapshot: vi.fn().mockResolvedValueOnce(current).mockImplementationOnce(() => pending.promise).mockImplementation(async () => current) });
    const controller = new DesktopController(api); render(<App initialPage="models" controller={controller} />);
    await waitFor(() => expect(controller.getSnapshot().booting).toBe(false));
    expect(within(screen.getByLabelText("应用状态栏")).getByText("服务连接失效")).toBeInTheDocument();
    const check = control().getByRole("button", { name: "重新检查服务" }); fireEvent.click(check); fireEvent.click(check);
    expect(control().getByRole("button", { name: "正在检查服务…" })).toBeDisabled();
    current = { ...stopped(), initialized }; await act(async () => pending.resolve(current));
    expect(control().getByRole("button", { name: initialized ? "启动运行服务" : "初始化并启动" })).toBeEnabled();
    expect(api.start).not.toHaveBeenCalled(); expect(api.stop).not.toHaveBeenCalled();
  });
  it.each(["connecting", "stopping"])("does not show a stopped service while %s", async (phase) => {
    const current = snapshot();
    if (phase === "connecting") current.connection = "connecting";
    else current.runtime!.stopping = true;
    const { api } = await setup({ snapshot: vi.fn(async () => current) });
    expect(control().getByRole("button")).toBeDisabled();
    expect(control().queryByText("服务已停止")).not.toBeInTheDocument();
    expect(api.start).not.toHaveBeenCalled(); expect(api.stop).not.toHaveBeenCalled();
  });
  it.each(["start", "stop"] as const)("leaves a failed %s outcome unknown and requires a status check", async (action) => {
    const current = action === "start" ? stopped() : snapshot();
    const { api, user } = await setup({ snapshot: vi.fn(async () => current), [action]: vi.fn().mockRejectedValue({ code: "desktop_unavailable", message: "连接中断" }) });
    if (action === "start") await user.click(control().getByRole("button", { name: "启动运行服务" }));
    else {
      await user.click(control().getByRole("button", { name: "停止运行服务" }));
      await user.click(within(screen.getByRole("dialog")).getByRole("button", { name: "停止运行服务" }));
    }
    expect(control().getByRole("button", { name: "重新检查服务" })).toBeEnabled();
    expect(within(screen.getByLabelText("应用状态栏")).getByText("服务连接失效")).toBeInTheDocument();
    expect(control().queryByText("服务已停止")).not.toBeInTheDocument();
    expect(api[action]).toHaveBeenCalledTimes(1);
  });
  it("disables global stop while the local chat needs cancellation or recovery", async () => {
    const terminal = deferred<ChatBatch>(); const { api, controller, user } = await setup({ chatNext: vi.fn(() => terminal.promise) });
    await user.click(screen.getByRole("button", { name: "聊天测试" }));
    await user.type(screen.getByRole("textbox", { name: "输入消息" }), "你好");
    await user.click(screen.getByRole("button", { name: "发送" }));
    expect(control().getByRole("button", { name: "停止运行服务" })).toBeDisabled();
    expect(control().getByText("请先完成或取消当前操作")).toBeInTheDocument();
    expect(api.stop).not.toHaveBeenCalled();
    await act(async () => terminal.resolve({ request_id: "request-1", terminal: true, events: [{ type: "cancelled" }] }));
    await waitFor(() => expect(controller.getSnapshot().chat_phase).toBe("idle"));
  });
  it("keeps stop available when another application's generation is active", async () => {
    const current = snapshot(); current.runtime!.state = "generating"; current.runtime!.active_request = "other-client";
    await setup({ snapshot: vi.fn(async () => current) });
    expect(control().getByRole("button", { name: "停止运行服务" })).toBeEnabled();
  });
});

describe("service lifecycle confirmation boundary", () => {
  it.each([undefined, null, {}, { stopped: false }, { stopped: "true" }])("does not turn an invalid stop reply into success: %#", async (reply) => {
    const api = makeApi({ stop: vi.fn().mockResolvedValue(reply) }); const controller = new DesktopController(api);
    await controller.refresh(); await controller.stop();
    expect(controller.getSnapshot().snapshot?.connection).toBe("error");
    expect(controller.getSnapshot().notice).toBeNull();
    expect(controller.getSnapshot().error?.code).toBe("runtime_stop_unconfirmed");
    await controller.stop(); expect(api.stop).toHaveBeenCalledTimes(1);
  });
  it("rejects a stale poll arriving after confirmed stop and rereads authoritative state", async () => {
    const stale = deferred<Snapshot>(); const stopping = deferred<{ stopped: true }>();
    const api = makeApi({ snapshot: vi.fn().mockResolvedValueOnce(snapshot()).mockReturnValueOnce(stale.promise).mockResolvedValue(stopped()), stop: vi.fn(() => stopping.promise) });
    const controller = new DesktopController(api); await controller.refresh();
    const stop = controller.stop(); const poll = controller.refresh();
    stopping.resolve({ stopped: true }); await Promise.resolve();
    expect(controller.getSnapshot().snapshot?.connection).toBe("stopped");
    stale.resolve(snapshot()); await poll; await stop;
    expect(controller.getSnapshot().snapshot?.connection).toBe("stopped");
    expect(api.snapshot).toHaveBeenCalledTimes(3);
  });
  it("does not retain a stopped success notice after a failed final status read", async () => {
    const api = makeApi({ snapshot: vi.fn().mockResolvedValueOnce(snapshot()).mockRejectedValueOnce({ code: "desktop_unavailable", message: "读取失败" }) });
    const controller = new DesktopController(api); await controller.refresh(); await controller.stop();
    expect(controller.getSnapshot().snapshot?.connection).toBe("error");
    expect(controller.getSnapshot().notice).toBeNull();
  });
  it("does not start an unconfirmed or already connected instance", async () => {
    const api = makeApi(); const controller = new DesktopController(api);
    await controller.start(true); await controller.refresh(); await controller.start(false);
    expect(api.start).not.toHaveBeenCalled();
  });
});
