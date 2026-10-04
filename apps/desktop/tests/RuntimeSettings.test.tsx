import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import App from "../src/App";
import { DesktopController } from "../src/controller";
import type { DesktopApi, Snapshot } from "../src/types";
import { deferred, makeApi, snapshot } from "./fixtures";

const stopped = (): Snapshot => ({ ...snapshot(), connection: "stopped", runtime: null });
async function setup(initial = stopped(), overrides: Partial<DesktopApi> = {}) {
  let current = initial;
  const api = makeApi({
    snapshot: vi.fn(async () => current),
    saveIdle: vi.fn(async (idle_unload_seconds, idle_unload_enabled) => {
      current = { ...current, settings: { ...current.settings, idle_unload_seconds, ...(idle_unload_enabled === undefined ? {} : { idle_unload_enabled }) } }; return current;
    }),
    saveVerificationTimeout: vi.fn(async (model_verification_timeout_seconds) => {
      current = { ...current, settings: { ...current.settings, model_verification_timeout_seconds } }; return current;
    }),
    saveSettings: vi.fn(async (settings) => { current = { ...current, settings: { ...current.settings, ...settings } }; return current; }),
    saveLanSettings: vi.fn(async (lan_api) => { current = { ...current, lan_api }; return current; }),
    ...overrides,
  });
  const controller = new DesktopController(api);
  const rendered = render(<App controller={controller} />);
  await waitFor(() => expect(controller.getSnapshot().booting).toBe(false));
  const user = userEvent.setup(); await user.click(screen.getByRole("button", { name: "设置" }));
  return { api, controller, user, ...rendered };
}
const idleInput = () => screen.getByRole("spinbutton", { name: /空闲等待时间/ });
const verificationInput = () => screen.getByRole("spinbutton", { name: /模型文件校验时间上限/ });
const noUnload = () => screen.getByRole("switch", { name: "不自动卸载" });
const saveIdle = () => screen.getByRole("button", { name: "应用空闲卸载设置" });
const saveVerification = () => screen.getByRole("button", { name: "保存模型文件校验超时" });

describe("separate idle and verification settings", () => {
  it("shows defaults, seconds, distinct scope, and exact activation boundaries", async () => {
    const { api } = await setup();
    expect(idleInput()).toHaveValue(300); expect(verificationInput()).toHaveValue(300); expect(noUnload()).not.toBeChecked();
    expect(saveIdle()).toBeDisabled(); expect(saveVerification()).toBeDisabled();
    expect(screen.getByText(/一次校验操作内的所有文件共用时限/)).toBeInTheDocument();
    expect(screen.getByText(/不改变网络下载传输、原生模型加载或基础短文本生成/)).toBeInTheDocument();
    expect(screen.getByText(/不会阻止电脑关机或睡眠/)).toBeInTheDocument();
    expect(screen.getByText(/保存后在下次显式启动运行服务时生效/)).toBeInTheDocument();
    expect(screen.getByText(/保存后用于后续新校验/)).toBeInTheDocument();
    expect(api.discoverDirectory).not.toHaveBeenCalled(); expect(api.scanModels).not.toHaveBeenCalled(); expect(api.start).not.toHaveBeenCalled();
  });
  it.each(["", "29", "7201", "30.5"])("rejects timeout input '%s' visibly without saving", async (value) => {
    const { api } = await setup();
    fireEvent.change(verificationInput(), { target: { value } });
    expect(screen.getByRole("alert")).toHaveTextContent("30–7200 秒的整数");
    expect(saveVerification()).toBeDisabled(); expect(api.saveVerificationTimeout).not.toHaveBeenCalled();
  });
  it.each(["", "0", "86401", "1.5"])("rejects idle input '%s' visibly without saving", async (value) => {
    const { api } = await setup(); fireEvent.change(idleInput(), { target: { value } });
    expect(screen.getByRole("alert")).toHaveTextContent("1–86400 秒的整数"); expect(saveIdle()).toBeDisabled(); expect(api.saveIdle).not.toHaveBeenCalled();
  });
  it.each([30, 7200])("saves timeout boundary %s with successful saved status", async (value) => {
    const { api, user } = await setup(); fireEvent.change(verificationInput(), { target: { value: String(value) } }); await user.click(saveVerification());
    expect(api.saveVerificationTimeout).toHaveBeenCalledExactlyOnceWith(value);
    expect(screen.getByLabelText("已保存模型文件校验超时")).toHaveTextContent(`已保存配置：${value} 秒`);
    expect(screen.getByText(/模型文件校验超时已保存，后续新校验使用此值/)).toBeInTheDocument();
    expect(api.saveIdle).not.toHaveBeenCalled(); expect(api.start).not.toHaveBeenCalled(); expect(api.stop).not.toHaveBeenCalled();
  });
  it.each([1, 86400])("saves idle boundary %s with explicit enabled state", async (value) => {
    const { api, user } = await setup(); fireEvent.change(idleInput(), { target: { value: String(value) } }); await user.click(saveIdle());
    expect(api.saveIdle).toHaveBeenCalledExactlyOnceWith(value, true);
    expect(screen.getByLabelText("已保存空闲卸载配置")).toHaveTextContent(`空闲 ${value} 秒后自动卸载`);
  });
  it("disables automatic unload, restores a valid saved wait, and keeps it when re-enabled", async () => {
    const initial = stopped(); initial.settings.idle_unload_seconds = 900;
    const { api, user } = await setup(initial); fireEvent.change(idleInput(), { target: { value: "" } });
    await user.click(noUnload()); expect(idleInput()).toHaveValue(900); expect(idleInput()).toBeDisabled(); expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    await user.click(saveIdle()); expect(api.saveIdle).toHaveBeenLastCalledWith(900, false);
    expect(screen.getByLabelText("已保存空闲卸载配置")).toHaveTextContent("不自动卸载（保留等待时间 900 秒）");
    expect(noUnload()).toBeChecked(); expect(idleInput()).toBeDisabled(); expect(api.stop).not.toHaveBeenCalled();
    await user.click(noUnload()); expect(idleInput()).toBeEnabled(); expect(idleInput()).toHaveValue(900);
    await user.click(saveIdle()); expect(api.saveIdle).toHaveBeenLastCalledWith(900, true);
  });
  it("shows out-of-range legacy values without silently clamping, including disabled idle", async () => {
    const current = stopped(); current.settings.idle_unload_enabled = false; current.settings.idle_unload_seconds = 90000;
    const { api, user } = await setup(current);
    expect(idleInput()).toHaveValue(90000); expect(idleInput()).toBeEnabled(); expect(saveIdle()).toBeDisabled();
    expect(screen.getByText(/旧配置的等待时间超出当前可保存范围，原值保持不变/)).toBeInTheDocument();
    await user.click(noUnload()); await user.click(noUnload());
    expect(idleInput()).toHaveValue(90000); expect(saveIdle()).toBeDisabled(); expect(api.saveIdle).not.toHaveBeenCalled();
    fireEvent.change(idleInput(), { target: { value: "900" } });
    await user.click(saveIdle()); expect(api.saveIdle).toHaveBeenCalledExactlyOnceWith(900, false);
    expect(idleInput()).toHaveValue(900); expect(idleInput()).toBeDisabled();
  });
  it("cancels each unsaved draft independently without a request", async () => {
    const { api, user } = await setup(); await user.click(noUnload()); fireEvent.change(verificationInput(), { target: { value: "1200" } });
    await user.click(screen.getByRole("button", { name: "取消空闲卸载更改" }));
    expect(noUnload()).not.toBeChecked(); expect(verificationInput()).toHaveValue(1200);
    await user.click(screen.getByRole("button", { name: "取消校验超时更改" })); expect(verificationInput()).toHaveValue(300);
    expect(api.saveIdle).not.toHaveBeenCalled(); expect(api.saveVerificationTimeout).not.toHaveBeenCalled();
  });
  it("navigation discards unsaved settings and window close never implicitly saves", async () => {
    const { api, user } = await setup(); await user.click(noUnload()); fireEvent.change(verificationInput(), { target: { value: "1200" } });
    await user.keyboard("{Alt>}1{/Alt}"); await user.click(screen.getByRole("button", { name: "设置" }));
    expect(noUnload()).not.toBeChecked(); expect(verificationInput()).toHaveValue(300);
    await user.click(noUnload()); await user.click(screen.getByRole("button", { name: "关闭应用" }));
    expect(api.close).toHaveBeenCalledTimes(1); expect(api.saveIdle).not.toHaveBeenCalled(); expect(api.saveVerificationTimeout).not.toHaveBeenCalled();
  });
  it.each(["connected", "connecting", "error"] as const)("is read-only for connection %s and never implicitly stops", async (connection) => {
    const { api } = await setup({ ...snapshot(), connection });
    expect(noUnload()).toBeDisabled(); expect(idleInput()).toBeDisabled(); expect(verificationInput()).toBeDisabled();
    expect(saveIdle()).toBeDisabled(); expect(saveVerification()).toBeDisabled();
    expect(screen.getAllByText(/请先显式停止运行服务再修改/).length).toBeGreaterThan(0); expect(api.stop).not.toHaveBeenCalled();
  });
  it("leaves uninitialized setup read-only", async () => {
    const { api } = await setup({ ...stopped(), initialized: false });
    expect(noUnload()).toBeDisabled(); expect(idleInput()).toBeDisabled(); expect(verificationInput()).toBeDisabled();
    expect(screen.getAllByText(/保存不会初始化或启动服务/)).toHaveLength(2); expect(api.start).not.toHaveBeenCalled();
  });
  it("handles an old DTO without claiming new capabilities and preserves legacy idle save", async () => {
    const old = stopped(); delete old.settings.idle_unload_enabled; delete old.settings.model_verification_timeout_seconds;
    const { api, user } = await setup(old);
    expect(noUnload()).not.toBeChecked(); expect(noUnload()).toBeDisabled(); expect(verificationInput()).toHaveValue(300); expect(verificationInput()).toBeDisabled();
    expect(screen.getByText(/当前桌面版本未提供“不自动卸载”开关/)).toBeInTheDocument();
    fireEvent.change(idleInput(), { target: { value: "600" } }); await user.click(saveIdle());
    expect(api.saveIdle).toHaveBeenCalledExactlyOnceWith(600); expect(api.saveVerificationTimeout).not.toHaveBeenCalled();
  });
  it("keeps the other draft during independent saves and never resets disabled state via preferences", async () => {
    const { api, user } = await setup(); fireEvent.change(verificationInput(), { target: { value: "1200" } });
    await user.click(noUnload()); await user.click(saveIdle()); expect(verificationInput()).toHaveValue(1200);
    fireEvent.change(screen.getByRole("spinbutton", { name: /推理线程/ }), { target: { value: "4" } }); await user.click(screen.getByRole("button", { name: "保存偏好" }));
    expect(noUnload()).toBeChecked(); expect(verificationInput()).toHaveValue(1200);
    await user.click(noUnload()); fireEvent.change(idleInput(), { target: { value: "900" } });
    await user.click(saveVerification()); expect(idleInput()).toHaveValue(900); expect(noUnload()).not.toBeChecked();
    expect(screen.getByLabelText("已保存空闲卸载配置")).toHaveTextContent("不自动卸载");
    expect(api.saveSettings).toHaveBeenCalledExactlyOnceWith(expect.not.objectContaining({ idle_unload_enabled: false }));
    expect(api.saveLanSettings).not.toHaveBeenCalled();
  });
  it("locks controls, LAN and Add during save, deduplicates clicks and exposes failure", async () => {
    const pending = deferred<Snapshot>();
    const { api, user } = await setup(stopped(), { saveVerificationTimeout: vi.fn(() => pending.promise) });
    fireEvent.change(verificationInput(), { target: { value: "600" } }); const save = saveVerification(); fireEvent.click(save); fireEvent.click(save);
    expect(api.saveVerificationTimeout).toHaveBeenCalledTimes(1); expect(verificationInput()).toBeDisabled(); expect(noUnload()).toBeDisabled();
    expect(screen.getByRole("switch", { name: "启用局域网 API" })).toBeDisabled(); expect(screen.getByRole("button", { name: "关闭应用" })).toBeDisabled();
    expect(screen.getAllByText(/有其他操作正在进行/)).toHaveLength(2);
    await user.click(screen.getByRole("button", { name: "模型" }));
    expect(screen.getByRole("button", { name: "添加模型" })).toBeDisabled();
    await act(async () => pending.reject({ code: "settings_save_failed", message: "设置保存失败，请重试。" }));
    await user.click(screen.getByRole("button", { name: "设置" }));
    expect(screen.getByText("设置保存失败，请重试。")).toBeInTheDocument();
    expect(screen.getByLabelText("已保存模型文件校验超时")).toHaveTextContent("300 秒");
    expect(screen.queryByText(/模型文件校验超时已保存/)).not.toBeInTheDocument();
  });
  it("leaves a failed idle draft editable and keeps the prior saved status", async () => {
    const { api, user } = await setup(stopped(), { saveIdle: vi.fn().mockRejectedValue({ code: "runtime_running", message: "runtime_running" }) });
    await user.click(noUnload()); await user.click(saveIdle());
    expect(api.saveIdle).toHaveBeenCalledExactlyOnceWith(300, false);
    expect(screen.getByText(/运行服务已启动。请先显式停止服务，再保存设置/)).toBeInTheDocument();
    expect(screen.getByLabelText("已保存空闲卸载配置")).toHaveTextContent("空闲 300 秒后自动卸载");
    expect(noUnload()).toBeChecked(); expect(screen.queryByText(/空闲卸载设置已保存/)).not.toBeInTheDocument();
    expect(api.stop).not.toHaveBeenCalled();
  });
  it("does not replace saved state before a pending save finishes", async () => {
    const pending = deferred<Snapshot>();
    const { user } = await setup(stopped(), { saveIdle: vi.fn(() => pending.promise) });
    await user.click(noUnload()); fireEvent.click(saveIdle()); expect(screen.getByLabelText("已保存空闲卸载配置")).toHaveTextContent("空闲 300 秒后自动卸载");
    const saved = stopped(); saved.settings.idle_unload_enabled = false;
    await act(async () => pending.resolve(saved));
    expect(screen.getByLabelText("已保存空闲卸载配置")).toHaveTextContent("不自动卸载"); expect(saveIdle()).toBeDisabled();
  });
});
