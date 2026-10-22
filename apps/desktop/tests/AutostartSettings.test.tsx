import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { AutostartSettings } from "../src/AutostartSettings";
import type { AutostartSnapshot } from "../src/types";
import { deferred, makeApi } from "./fixtures";

const off = { registered: false, current_executable: false };
const on = { registered: true, current_executable: true };

describe("current-user Windows autostart", () => {
  it("explains the Windows Run length limit without claiming a failed registration succeeded", async () => {
    const api = makeApi({ autostartGet: vi.fn(async () => off), autostartSet: vi.fn().mockRejectedValue({ code: "autostart_path_too_long" }) });
    render(<AutostartSettings api={api} closing={false} />);
    const toggle = screen.getByRole("switch", { name: "开机启动" });
    await waitFor(() => expect(toggle).toBeEnabled()); fireEvent.click(toggle);
    expect(await screen.findByRole("alert")).toHaveTextContent("260 字符限制");
    expect(screen.getByRole("alert")).toHaveTextContent("将完整 Nexa 目录移到更短路径");
    expect(toggle).not.toBeChecked(); expect(toggle).toBeDisabled();
    expect(screen.queryByText(/已登记当前用户/)).not.toBeInTheDocument();
  });
  it("does not register on mount and waits for read and write confirmation", async () => {
    const initial = deferred<AutostartSnapshot>(); const saving = deferred<AutostartSnapshot>();
    const api = makeApi({ autostartGet: vi.fn(() => initial.promise), autostartSet: vi.fn(() => saving.promise) });
    render(<AutostartSettings api={api} closing={false} />);
    const toggle = screen.getByRole("switch", { name: "开机启动" });
    expect(toggle).toBeDisabled(); expect(api.autostartSet).not.toHaveBeenCalled();
    initial.resolve(off); await waitFor(() => expect(toggle).toBeEnabled());
    expect(toggle).not.toBeChecked(); fireEvent.click(toggle);
    expect(api.autostartSet).toHaveBeenCalledWith(true); expect(toggle).not.toBeChecked(); expect(toggle).toBeDisabled();
    saving.resolve(on); await waitFor(() => expect(toggle).toBeChecked());
    expect(screen.getByText(/Windows 的启动应用设置也可能单独禁用/)).toBeInTheDocument();
  });
  it("restores registered state, disables only its setting, and reports write uncertainty until reread", async () => {
    const api = makeApi({ autostartGet: vi.fn().mockResolvedValueOnce(on).mockResolvedValueOnce(off), autostartSet: vi.fn().mockRejectedValueOnce(new Error("denied")) });
    render(<AutostartSettings api={api} closing={false} />);
    const toggle = screen.getByRole("switch", { name: "开机启动" });
    await waitFor(() => expect(toggle).toBeChecked()); fireEvent.click(toggle);
    await screen.findByRole("alert"); expect(api.autostartSet).toHaveBeenCalledWith(false); expect(toggle).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "重新读取启动状态" }));
    await waitFor(() => expect(toggle).toBeEnabled()); expect(toggle).not.toBeChecked();
    expect(api.close).not.toHaveBeenCalled();
  });
  it("updates a stale executable only after an explicit request and blocks changes while closing", async () => {
    const api = makeApi({ autostartGet: vi.fn(async () => ({ ...on, current_executable: false })), autostartSet: vi.fn(async () => on) });
    const view = render(<AutostartSettings api={api} closing={false} />);
    const repair = await screen.findByRole("button", { name: "更新为当前 Nexa" });
    expect(api.autostartSet).not.toHaveBeenCalled(); fireEvent.click(repair);
    await waitFor(() => expect(screen.queryByRole("button", { name: "更新为当前 Nexa" })).not.toBeInTheDocument());
    expect(api.autostartSet).toHaveBeenCalledWith(true);
    view.rerender(<AutostartSettings api={api} closing={true} />);
    expect(screen.getByRole("switch", { name: "开机启动" })).toBeDisabled();
  });
  it("keeps failed reads and mismatched write results unconfirmed", async () => {
    const api = makeApi({ autostartGet: vi.fn().mockRejectedValueOnce(new Error("denied")).mockResolvedValueOnce(off), autostartSet: vi.fn(async () => off) });
    render(<AutostartSettings api={api} closing={false} />);
    await screen.findByRole("alert"); expect(screen.getByRole("switch", { name: "开机启动" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "重新读取启动状态" }));
    await waitFor(() => expect(screen.getByRole("switch", { name: "开机启动" })).toBeEnabled());
    fireEvent.click(screen.getByRole("switch", { name: "开机启动" }));
    await screen.findByText(/修改失败或结果未确认/);
    expect(screen.getByRole("switch", { name: "开机启动" })).toBeDisabled();
  });
  it("ignores late read completion after unmount without writing startup state", async () => {
    const pending = deferred<AutostartSnapshot>();
    const api = makeApi({ autostartGet: vi.fn(() => pending.promise), autostartSet: vi.fn(async () => on) });
    const view = render(<AutostartSettings api={api} closing={false} />); view.unmount();
    pending.resolve(on); await act(async () => {}); expect(api.autostartSet).not.toHaveBeenCalled();
  });
});
