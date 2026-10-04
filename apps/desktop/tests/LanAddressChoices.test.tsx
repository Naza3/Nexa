import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import App from "../src/App";
import { DesktopController } from "../src/controller";
import type { DesktopApi, LanAddressDiscovery, LanApiSettings, Snapshot } from "../src/types";
import { deferred, makeApi, snapshot } from "./fixtures";

const lan: LanApiSettings = { enabled: true, listen: "192.168.1.99:18081", allowed_cidrs: ["192.168.1.30/32"] };
const detected: LanAddressDiscovery = { status: "available", addresses: [
  { interface_index: 2, interface_name: "以太网", address: "192.168.1.20" },
  { interface_index: 8, interface_name: "Wi-Fi", address: "10.20.0.5" },
  { interface_index: 9, interface_name: "VPN 虚拟网卡", address: "10.20.0.5" },
] };
const stopped = (): Snapshot => ({ ...snapshot(), connection: "stopped", runtime: null, lan_api: lan });
async function setup(overrides: Partial<DesktopApi> = {}) {
  const api = makeApi({ snapshot: vi.fn(async () => stopped()), lanAddresses: vi.fn(async () => detected), ...overrides });
  const controller = new DesktopController(api); render(<App initialPage="models" controller={controller} />);
  await waitFor(() => expect(controller.getSnapshot().booting).toBe(false));
  const user = userEvent.setup(); await user.click(screen.getByRole("button", { name: "API 接入" }));
  await waitFor(() => expect(api.lanAddresses).toHaveBeenCalledTimes(1));
  return { api, controller, user };
}
const host = () => screen.getByLabelText("本机局域网 IPv4", { exact: false });

describe("LAN interface choices and manual fallback", () => {
  it("automatically lists interface names and addresses, preserves saved input, and selects only after user action", async () => {
    const { api, user } = await setup();
    expect(host()).toHaveValue("192.168.1.99");
    expect(screen.getByRole("option", { name: "以太网 · 192.168.1.20" })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "Wi-Fi · 10.20.0.5" })).toBeInTheDocument();
    await user.selectOptions(screen.getByRole("combobox", { name: "选择网卡地址" }), "9:10.20.0.5");
    expect(screen.getByRole("combobox", { name: "选择网卡地址" })).toHaveValue("9:10.20.0.5");
    expect(host()).toHaveValue("10.20.0.5");
    expect(screen.getByLabelText("局域网端口", { exact: false })).toHaveValue("18081");
    expect(screen.getByLabelText("允许的客户端 IP / CIDR", { exact: false })).toHaveValue("192.168.1.30/32");
    for (const call of [api.saveLanSettings, api.start, api.stop, api.copyLanToken]) expect(call).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "保存局域网 API 设置" }));
    expect(api.saveLanSettings).toHaveBeenCalledExactlyOnceWith({ ...lan, listen: "10.20.0.5:18081" });
  });
  it("does not overwrite manual input when initial detection or a later refresh arrives", async () => {
    const first = deferred<LanAddressDiscovery>(); const later = deferred<LanAddressDiscovery>();
    const { api, controller, user } = await setup({ lanAddresses: vi.fn().mockReturnValueOnce(first.promise).mockReturnValueOnce(later.promise) });
    fireEvent.change(host(), { target: { value: "192.168.5.7" } });
    await act(async () => first.resolve(detected)); expect(host()).toHaveValue("192.168.5.7");
    const refresh = screen.getByRole("button", { name: "刷新网卡地址" }); fireEvent.click(refresh); fireEvent.click(refresh);
    await waitFor(() => expect(api.lanAddresses).toHaveBeenCalledTimes(2));
    expect(screen.getByRole("button", { name: "正在检测…" })).toBeDisabled();
    await user.clear(host()); await user.type(host(), "172.16.7.9");
    await act(async () => later.resolve({ status: "empty", addresses: [] }));
    expect(host()).toHaveValue("172.16.7.9");
    expect(controller.getSnapshot().lan_addresses?.status).toBe("empty");
    expect(api.saveLanSettings).not.toHaveBeenCalled();
  });
  it.each(["empty", "unsupported", "failed"])("keeps manual input usable for %s results", async (result) => {
    const apiResult = result === "failed" ? vi.fn().mockRejectedValue({ code: "lan_address_discovery_timeout", message: "private" })
      : vi.fn().mockResolvedValue({ status: result, addresses: [] });
    const { api, user } = await setup({ lanAddresses: apiResult });
    expect(screen.queryByRole("combobox", { name: "选择网卡地址" })).not.toBeInTheDocument();
    expect(host()).toBeEnabled();
    await user.clear(host()); await user.type(host(), "192.168.2.8");
    await user.click(screen.getByRole("button", { name: "保存局域网 API 设置" }));
    expect(api.saveLanSettings).toHaveBeenCalledExactlyOnceWith({ ...lan, listen: "192.168.2.8:18081" });
  });
  it("observes while running but does not enable address editing or save", async () => {
    const { api } = await setup({ snapshot: vi.fn(async () => ({ ...snapshot(), lan_api: lan })) });
    expect(screen.getByRole("combobox", { name: "选择网卡地址" })).toBeDisabled();
    expect(host()).toBeDisabled();
    expect(screen.getByRole("button", { name: "刷新网卡地址" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "保存局域网 API 设置" })).toBeDisabled();
    expect(api.stop).not.toHaveBeenCalled();
  });
  it("never enables LAN merely by detecting private addresses", async () => {
    const { api } = await setup({ snapshot: vi.fn(async () => ({ ...stopped(), lan_api: { enabled: false, listen: null, allowed_cidrs: [] } })) });
    expect(screen.getByRole("switch", { name: "启用局域网 API" })).not.toBeChecked();
    expect(api.saveLanSettings).not.toHaveBeenCalled(); expect(api.copyLanToken).not.toHaveBeenCalled();
  });
  it("preserves dirty manual values through saved-configuration polling changes", async () => {
    let current = stopped();
    const { controller } = await setup({ snapshot: vi.fn(async () => current) });
    fireEvent.change(host(), { target: { value: "192.168.5.7" } });
    fireEvent.change(screen.getByLabelText("允许的客户端 IP / CIDR", { exact: false }), { target: { value: "192.168.5.8" } });
    current = { ...current, lan_api: { ...lan, listen: "192.168.1.66:18082", allowed_cidrs: ["192.168.1.77/32"] } };
    await act(async () => controller.refresh());
    expect(host()).toHaveValue("192.168.5.7");
    expect(screen.getByLabelText("允许的客户端 IP / CIDR", { exact: false })).toHaveValue("192.168.5.8");
    expect(screen.getByText("已保存配置：启用 · 192.168.1.66:18082")).toBeInTheDocument();
  });
  it("updates an untouched draft when saved configuration changes", async () => {
    let current = stopped(); const { controller } = await setup({ snapshot: vi.fn(async () => current) });
    current = { ...current, lan_api: { ...lan, listen: "192.168.1.66:18082" } };
    await act(async () => controller.refresh());
    expect(host()).toHaveValue("192.168.1.66");
    expect(screen.getByLabelText("局域网端口", { exact: false })).toHaveValue("18082");
  });
});
