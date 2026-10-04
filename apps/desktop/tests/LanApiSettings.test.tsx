import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import App from "../src/App";
import { DesktopController } from "../src/controller";
import type { DesktopApi, LanApiSettings, Snapshot } from "../src/types";
import { deferred, makeApi, snapshot } from "./fixtures";

const enabled: LanApiSettings = { enabled: true, listen: "192.168.1.20:18081", allowed_cidrs: ["192.168.1.30/32"] };
const stopped = (): Snapshot => ({ ...snapshot(), connection: "stopped", runtime: null });
async function setup(state = stopped(), overrides: Partial<DesktopApi> = {}) {
  let current = state;
  const api = makeApi({ snapshot: vi.fn(async () => current), saveLanSettings: vi.fn(async (lan_api) => {
    current = { ...current, lan_api }; return current;
  }), ...overrides });
  const controller = new DesktopController(api);
  const rendered = render(<App controller={controller} />);
  await waitFor(() => expect(controller.getSnapshot().booting).toBe(false));
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "设置" }));
  return { api, controller, user, ...rendered };
}
async function enableDraft(user: ReturnType<typeof userEvent.setup>) {
  await user.click(screen.getByRole("switch", { name: "启用局域网 API" }));
  await user.click(screen.getByRole("button", { name: "了解风险，编辑配置" }));
}
async function fillDraft(user: ReturnType<typeof userEvent.setup>) {
  await user.type(screen.getByLabelText("本机局域网 IPv4", { exact: false }), "192.168.1.20");
  await user.type(screen.getByLabelText("允许的客户端 IP / CIDR", { exact: false }), "192.168.1.30");
}

describe("LAN settings explicit consent and lifecycle", () => {
  it("starts closed, exposes HTTP risk, and cancels enabling without actions", async () => {
    const { user, api } = await setup();
    expect(screen.getByRole("switch", { name: "启用局域网 API" })).not.toBeChecked();
    expect(screen.getByText(/使用明文 HTTP，不提供 TLS/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "复制局域网 API 密钥" })).toBeDisabled();
    await user.click(screen.getByRole("switch", { name: "启用局域网 API" }));
    expect(screen.getByRole("dialog")).toHaveTextContent("窃听或篡改");
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.getByRole("switch", { name: "启用局域网 API" })).not.toBeChecked();
    expect(api.saveLanSettings).not.toHaveBeenCalled();
    expect(api.start).not.toHaveBeenCalled();
    expect(api.copyLanToken).not.toHaveBeenCalled();
  });
  it("requires nonempty private inputs and saves without starting or reading a credential", async () => {
    const { user, api } = await setup();
    await enableDraft(user);
    const save = screen.getByRole("button", { name: "保存局域网 API 设置" });
    expect(save).toBeDisabled();
    await fillDraft(user);
    expect(save).toBeEnabled();
    await user.click(save);
    expect(api.saveLanSettings).toHaveBeenCalledExactlyOnceWith(enabled);
    expect(await screen.findByText("已保存配置：启用 · 192.168.1.20:18081")).toBeInTheDocument();
    expect(screen.getByText("实际监听：未运行（服务已停止）")).toBeInTheDocument();
    expect(screen.getByLabelText("局域网客户端 Base URL")).toHaveTextContent("http://192.168.1.20:18081/v1");
    expect(api.start).not.toHaveBeenCalled();
    expect(api.stop).not.toHaveBeenCalled();
    expect(api.copyLanToken).not.toHaveBeenCalled();
  });
  it("shows validation for public or overlapping inputs and does not submit", async () => {
    const { user, api } = await setup();
    await enableDraft(user);
    await user.type(screen.getByLabelText("本机局域网 IPv4", { exact: false }), "0.0.0.0");
    await user.type(screen.getByLabelText("允许的客户端 IP / CIDR", { exact: false }), "192.168.1.0/24\n192.168.1.30");
    expect(screen.getByRole("alert")).toHaveTextContent("不接受公网");
    await user.clear(screen.getByLabelText("本机局域网 IPv4", { exact: false }));
    await user.type(screen.getByLabelText("本机局域网 IPv4", { exact: false }), "192.168.1.20");
    expect(screen.getByRole("alert")).toHaveTextContent("重叠");
    expect(screen.getByRole("button", { name: "保存局域网 API 设置" })).toBeDisabled();
    expect(api.saveLanSettings).not.toHaveBeenCalled();
  });
  it("disables without retaining invalid edits and preserves the saved valid address and whitelist", async () => {
    const { user, api } = await setup({ ...stopped(), lan_api: enabled });
    await user.clear(screen.getByLabelText("本机局域网 IPv4", { exact: false }));
    await user.type(screen.getByLabelText("本机局域网 IPv4", { exact: false }), "8.8.8.8");
    await user.click(screen.getByRole("switch", { name: "启用局域网 API" }));
    await user.click(screen.getByRole("button", { name: "保存局域网 API 设置" }));
    expect(api.saveLanSettings).toHaveBeenCalledExactlyOnceWith({ ...enabled, enabled: false });
    expect(screen.getByRole("button", { name: "复制局域网 API 密钥" })).toBeDisabled();
    expect(screen.queryByLabelText("局域网客户端 Base URL")).not.toBeInTheDocument();
    expect(api.stop).not.toHaveBeenCalled();
  });
  it("does not conflate saved configuration and actual listener or implicitly stop a running service", async () => {
    const current = snapshot();
    current.lan_api = enabled;
    current.runtime!.lan_api = { enabled: true, listen: "192.168.1.21:18082", running: true };
    const { api } = await setup(current);
    expect(screen.getByText("已保存配置：启用 · 192.168.1.20:18081")).toBeInTheDocument();
    expect(screen.getByText("实际监听：正在监听 http://192.168.1.21:18082/v1")).toBeInTheDocument();
    expect(screen.getByRole("switch", { name: "启用局域网 API" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "保存局域网 API 设置" })).toBeDisabled();
    expect(api.stop).not.toHaveBeenCalled();
  });
  it("treats missing old-service status as unknown and missing old-bridge settings as unsupported", async () => {
    const { unmount, api } = await setup({ ...snapshot(), lan_api: enabled });
    expect(screen.getByText("实际监听：当前服务未报告局域网监听状态")).toBeInTheDocument();
    expect(api.copyLanToken).not.toHaveBeenCalled();
    unmount();
    const old = stopped(); delete old.lan_api;
    await setup(old);
    expect(screen.getByText(/当前桌面版本未提供局域网 API 设置/)).toBeInTheDocument();
    expect(screen.getByRole("switch", { name: "启用局域网 API" })).toBeDisabled();
  });
  it("keeps uninitialized setup read-only and leaves offline list behavior alone", async () => {
    const { api } = await setup({ ...stopped(), initialized: false });
    expect(screen.getByRole("switch", { name: "启用局域网 API" })).toBeDisabled();
    expect(screen.getByText(/此页面不会初始化服务或生成密钥/)).toBeInTheDocument();
    expect(api.start).not.toHaveBeenCalled();
    expect(api.saveLanSettings).not.toHaveBeenCalled();
    expect(api.copyLanToken).not.toHaveBeenCalled();
  });
  it("deduplicates save and does not replace saved status before completion", async () => {
    const pending = deferred<Snapshot>();
    const { user, api } = await setup(stopped(), { saveLanSettings: vi.fn(() => pending.promise) });
    await enableDraft(user); await fillDraft(user);
    const save = screen.getByRole("button", { name: "保存局域网 API 设置" });
    fireEvent.click(save); fireEvent.click(save);
    expect(api.saveLanSettings).toHaveBeenCalledTimes(1);
    expect(screen.getByText("已保存配置：关闭")).toBeInTheDocument();
    expect(screen.getByRole("switch", { name: "启用局域网 API" })).toBeDisabled();
    await act(async () => pending.resolve({ ...stopped(), lan_api: enabled }));
    expect(screen.getByText("已保存配置：启用 · 192.168.1.20:18081")).toBeInTheDocument();
  });
  it("cancels copy on Escape or navigation, and copying never displays or retains a secret", async () => {
    const { user, api, controller } = await setup({ ...stopped(), lan_api: enabled });
    await user.click(screen.getByRole("button", { name: "复制局域网 API 密钥" }));
    expect(api.copyLanToken).not.toHaveBeenCalled();
    await user.keyboard("{Escape}");
    await user.click(screen.getByRole("button", { name: "复制局域网 API 密钥" }));
    // The app's normal keyboard navigation unmounts the settings dialog.
    await user.keyboard("{Alt>}1{/Alt}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(api.copyLanToken).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "设置" }));
    await user.click(screen.getByRole("button", { name: "复制局域网 API 密钥" }));
    await user.click(within(screen.getByRole("dialog")).getByRole("button", { name: "确认复制局域网密钥" }));
    expect(api.copyLanToken).toHaveBeenCalledTimes(1);
    expect(screen.getByText(/独立密钥已复制到系统剪贴板/)).toBeInTheDocument();
    expect(controller.getSnapshot()).not.toHaveProperty("token");
    expect(api.copyToken).not.toHaveBeenCalled();
    expect(localStorage.length).toBe(0);
  });
  it("deduplicates repeated key confirmations and reports expired-key errors without residual success", async () => {
    const pending = deferred<{ copied: true }>();
    const { user, api, controller } = await setup({ ...stopped(), lan_api: enabled }, { copyLanToken: vi.fn(() => pending.promise) });
    await user.click(screen.getByRole("button", { name: "复制局域网 API 密钥" }));
    const confirm = screen.getByRole("button", { name: "确认复制局域网密钥" });
    fireEvent.click(confirm); fireEvent.click(confirm);
    expect(api.copyLanToken).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    await act(async () => pending.reject({ code: "lan_token_unavailable", message: "expired private-secret" }));
    expect(screen.getByText(/未能复制局域网 API 密钥/)).toBeInTheDocument();
    expect(screen.queryByText(/密钥已复制/)).not.toBeInTheDocument();
    expect(document.body.textContent).not.toContain("private-secret");
    expect(JSON.stringify(controller.getSnapshot())).not.toContain("private-secret");
  });
});
