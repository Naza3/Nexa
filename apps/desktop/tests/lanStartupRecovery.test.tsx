import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import App from "../src/App";
import { DesktopController } from "../src/controller";
import { lanStartupMessage } from "../src/lanApi";
import { runtimeView } from "../src/runtimeView";
import type { LanStartupError } from "../src/types";
import { deferred, makeApi, snapshot } from "./fixtures";

function degraded(error: LanStartupError = "address_unavailable") {
  const value = snapshot();
  value.lan_api = { enabled: true, listen: "192.168.1.20:18081", allowed_cidrs: ["192.168.1.30/32"] };
  value.runtime!.lan_api = { enabled: true, listen: value.lan_api.listen, running: false, startup_error: error };
  return value;
}
describe("LAN startup recovery", () => {
  it.each([
    ["address_unavailable", "网卡"], ["port_in_use", "占用"],
    ["permission_denied", "系统拒绝"], ["bind_failed", "检查网络"],
  ] as const)("renders bounded actionable %s separately from model readiness", (code, guidance) => {
    const value = degraded(code);
    expect(lanStartupMessage(code)).toContain(guidance);
    expect(runtimeView(value)).toMatchObject({ lanListening: false, lanDegraded: true, resident: true, localListening: true });
  });
  it("handles absent legacy diagnostics and unknown errors without rendering raw text", () => {
    expect(lanStartupMessage(undefined)).toBeNull();
    expect(lanStartupMessage("secret arbitrary server text")).not.toContain("secret");
    const value = degraded();
    delete value.runtime!.lan_api!.startup_error;
    expect(runtimeView(value).lanDegraded).toBe(false);
    value.runtime!.lan_api!.running = true;
    expect(runtimeView(value).lanListening).toBe(true);
  });
  it("requires explicit stop confirmation and unlocks settings only after a verified stop", async () => {
    let current = degraded();
    const pending = deferred<{ stopped: true }>();
    const api = makeApi({ snapshot: vi.fn(async () => current), stop: vi.fn(() => pending.promise) });
    const controller = new DesktopController(api);
    render(<App initialPage="api" controller={controller} />);
    const fix = await screen.findByRole("button", { name: "停止服务以修正局域网设置" });
    expect(await screen.findByText("局域网 API 启动失败，本机服务仍可用")).toBeVisible();
    expect(api.stop).not.toHaveBeenCalled();
    expect(screen.getByLabelText("启用局域网 API")).toBeDisabled();
    fireEvent.click(fix);
    const first = await screen.findByRole("dialog");
    expect(within(first).getByText(/终止所有客户端的任务并卸载模型/)).toBeVisible();
    fireEvent.click(within(first).getByRole("button", { name: "取消" }));
    expect(api.stop).not.toHaveBeenCalled();
    fireEvent.click(fix);
    fireEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "停止运行服务" }));
    await waitFor(() => expect(api.stop).toHaveBeenCalledTimes(1));
    expect(fix).toBeDisabled();
    expect(controller.getSnapshot().snapshot?.connection).toBe("connected");
    current = { ...current, connection: "stopped", runtime: null };
    pending.resolve({ stopped: true });
    await waitFor(() => expect(controller.getSnapshot().snapshot?.connection).toBe("stopped"));
    expect(api.start).not.toHaveBeenCalled();
    expect(api.saveLanSettings).not.toHaveBeenCalled();
    expect(screen.getByLabelText("启用局域网 API")).toBeEnabled();
    expect(screen.queryByText("局域网 API 启动失败，本机服务仍可用")).toBeNull();
  });
  it("failed stop keeps settings locked and never retries or starts", async () => {
    const api = makeApi({ snapshot: vi.fn(async () => degraded()), stop: vi.fn(async () => { throw new Error("private OS detail"); }) });
    const controller = new DesktopController(api);
    render(<App initialPage="api" controller={controller} />);
    fireEvent.click(await screen.findByRole("button", { name: "停止服务以修正局域网设置" }));
    fireEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "停止运行服务" }));
    await waitFor(() => expect(controller.getSnapshot().snapshot?.connection).toBe("error"));
    expect(api.stop).toHaveBeenCalledTimes(1);
    expect(screen.getByLabelText("启用局域网 API")).toBeDisabled();
    expect(api.start).not.toHaveBeenCalled();
    expect(api.saveLanSettings).not.toHaveBeenCalled();
  });
});
