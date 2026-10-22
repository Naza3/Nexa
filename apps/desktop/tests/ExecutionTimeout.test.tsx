import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { DRAFT_STORAGE_KEY, readUnsavedDrafts } from "../src/unsavedDrafts";
import App from "../src/App";
import { DesktopController } from "../src/controller";
import { createPreviewApi } from "../src/preview";
import { makeApi, snapshot } from "./fixtures";
import { configuredSnapshot, revision } from "./configurationFixtures";
import type { Snapshot } from "../src/types";

const input = () => screen.getByRole("spinbutton", { name: /推理执行时间上限/ });
const save = () => screen.getByRole("button", { name: "保存推理执行超时" });
async function mount(current: Snapshot = configuredSnapshot(true)) {
  const api = makeApi({ snapshot: vi.fn(async () => structuredClone(current)), configurationSave: vi.fn(async (request) => {
    if (request.update.kind === "runtime") current.configuration!.saved.runtime = structuredClone(request.update.runtime);
    current.configuration!.revision = revision("b");
    return structuredClone(current.configuration!);
  }) });
  const controller = new DesktopController(api);
  const rendered = render(<App controller={controller} initialPage="settings" />);
  await waitFor(() => expect(controller.getSnapshot().booting).toBe(false));
  return { api, controller, current, ...rendered };
}
it.each(["", "0", "86401", "1.5"])("rejects invalid execution timeout %s", async (value) => {
  const { api } = await mount();
  fireEvent.change(input(), { target: { value } });
  expect(screen.getByRole("alert")).toHaveTextContent("推理执行超时须为 1–86400 秒的整数");
  expect(save()).toBeDisabled(); expect(api.configurationSave).not.toHaveBeenCalled();
});
it.each([1, 1800, 86400])("persists timeout %s across UI remount preserving other runtime policies", async (seconds) => {
  const { api, current, unmount } = await mount();
  const original = structuredClone(current.configuration!.saved.runtime);
  fireEvent.change(input(), { target: { value: String(seconds) } }); fireEvent.click(save());
  await waitFor(() => expect(screen.getByLabelText("已保存推理执行超时")).toHaveTextContent(`${seconds} 秒`));
  expect(api.configurationSave).toHaveBeenCalledExactlyOnceWith({ expected_revision: revision(), update: { kind: "runtime", runtime: { ...original, execution_timeout_seconds: seconds } } });
  expect(api.start).not.toHaveBeenCalled(); expect(api.stop).not.toHaveBeenCalled();
  unmount(); await mount(current); expect(input()).toHaveValue(seconds); expect(save()).toBeDisabled();
});
it("preserves draft on navigation and cancels explicitly", async () => {
  await mount(); fireEvent.change(input(), { target: { value: "1800" } });
  fireEvent.click(screen.getByRole("button", { name: "模型库" })); fireEvent.click(screen.getByRole("button", { name: "设置" }));
  expect(input()).toHaveValue(1800); fireEvent.click(screen.getByRole("button", { name: "取消推理超时更改" })); expect(input()).toHaveValue(300);
});
it("blocks stale draft and rebases onto the new revision without losing other runtime settings", async () => {
  const { current, controller, api } = await mount(); fireEvent.change(input(), { target: { value: "1800" } });
  current.configuration!.revision = revision("d"); current.configuration!.saved.runtime.idle_unload_seconds = 900;
  await act(() => controller.refresh()); expect(input()).toHaveValue(1800); expect(save()).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: /保留草稿/ })); fireEvent.click(save());
  await waitFor(() => expect(api.configurationSave).toHaveBeenCalledWith({ expected_revision: revision("d"), update: { kind: "runtime", runtime: expect.objectContaining({ execution_timeout_seconds: 1800, idle_unload_seconds: 900 }) } }));
});
it.each(["connected", "connecting", "error"] as const)("requires stopped service for %s", async (connection) => {
  const current = configuredSnapshot(); current.connection = connection; const { api } = await mount(current);
  expect(input()).toBeDisabled(); expect(save()).toBeDisabled(); expect(api.stop).not.toHaveBeenCalled();
});
it.each([false, true])("does not default or save legacy snapshots (configuration=%s)", async (withConfig) => {
  const current = withConfig ? configuredSnapshot(true) : { ...snapshot(), connection: "stopped" as const };
  if (current.configuration) Reflect.deleteProperty(current.configuration.saved.runtime, "execution_timeout_seconds");
  const { api } = await mount(current); expect(input()).toHaveValue(null); expect(input()).toBeDisabled(); expect(save()).toBeDisabled();
  expect(screen.getByText(/当前桌面版本不支持推理执行超时设置/)).toBeInTheDocument(); expect(api.configurationSave).not.toHaveBeenCalled();
});
it("preview saves the shared policy and applies it on the next start", async () => {
  window.history.replaceState(null, "", "?scenario=runtime-settings");
  try {
    const api = createPreviewApi(); const before = (await api.snapshot()).configuration!;
    const after = await api.configurationSave!({ expected_revision: before.revision, update: { kind: "runtime", runtime: { ...before.saved.runtime, model_verification_timeout_seconds: 600, execution_timeout_seconds: 1800 } } });
    expect(after.saved.runtime.execution_timeout_seconds).toBe(1800); expect(after.runtime_effective).toBeNull();
    const effective = (await api.start(false)).configuration!.runtime_effective!;
    expect(effective.values.runtime.execution_timeout_seconds).toBe(1800);
    expect(effective.chat_response_timeout_seconds).toBe(2850);
  } finally { window.history.replaceState(null, "", "/"); }
});

it.each(["1800", ""])("stores recoverable execution drafts %s without silently saving", async (value) => {
  const { api } = await mount(); fireEvent.change(input(), { target: { value } });
  await waitFor(() => expect(localStorage.getItem(DRAFT_STORAGE_KEY)).not.toBeNull());
  const recovered = readUnsavedDrafts().get("execution_policy")!;
  expect(recovered.source).toBe(300); expect(recovered.revision).toBe(revision());
  if (value) expect(recovered.draft).toBe(1800); else expect(recovered.draft).toBeNaN();
  expect(screen.queryByText(/草稿恢复缓存未能保存/)).not.toBeInTheDocument();
  expect(api.configurationSave).not.toHaveBeenCalled();
});
