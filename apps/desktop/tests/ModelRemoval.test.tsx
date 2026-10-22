import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import App from "../src/App";
import { DesktopController } from "../src/controller";
import type { DesktopApi, ModelSummary, ModelUnregisterResult } from "../src/types";
import { deferred, makeApi, model, snapshot } from "./fixtures";
import { configuredSnapshot, modelConfiguration } from "./configurationFixtures";

const generation = "00000000-0000-4000-8000-000000000001";
async function mount(value = configuredSnapshot(true), entries = [model], overrides: Partial<DesktopApi> = {}) {
  let models = entries;
  let currentGeneration = generation;
  const api = makeApi({
    snapshot: vi.fn(async () => structuredClone(value)),
    modelsPage: vi.fn(async () => ({ data: structuredClone(models), next_after: null, generation: currentGeneration })),
    unregisterModel: vi.fn(async (model_id: string) => {
      models = models.filter((entry) => entry.id !== model_id); currentGeneration = "generation-after";
      if (value.runtime?.selected_model === model_id) {
        value.runtime.selected_model = null; value.runtime.selected_model_display_name = null; value.runtime.load_options = null;
      }
      return { model_id, removed: true as const, files_preserved: true as const };
    }),
    ...overrides,
  });
  const controller = new DesktopController(api);
  render(<App initialPage="models" controller={controller} />);
  await waitFor(() => expect(controller.getSnapshot().booting).toBe(false));
  await screen.findAllByRole("heading", { name: model.display_name });
  return { controller, api, value, user: userEvent.setup(), setModels: (next: ModelSummary[]) => { models = next; currentGeneration = "reimported"; } };
}
async function openRemove(user: ReturnType<typeof userEvent.setup>, name = model.display_name) {
  await user.click(screen.getByRole("button", { name: `${name} 的更多操作` }));
  await user.click(screen.getByRole("button", { name: "从模型库移除" }));
  return screen.getByRole("dialog");
}

describe("model library remove interaction (mock DOM, not native GUI)", () => {
  it("keeps removal contextual and confirms file preservation with Cancel focused", async () => {
    const { user, api } = await mount();
    expect(screen.queryByRole("button", { name: "从模型库移除" })).not.toBeInTheDocument();
    const dialog = await openRemove(user);
    expect(dialog).toHaveTextContent("GGUF 文件、运行档案和历史测试记录均会保留");
    expect(dialog).toHaveTextContent("可重新添加该文件恢复");
    expect(within(dialog).getByRole("button", { name: "取消" })).toHaveFocus();
    await user.click(within(dialog).getByRole("button", { name: "取消" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(api.unregisterModel).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: `${model.display_name} 的更多操作` })).toHaveFocus();
  });
  it("traps keyboard focus, Escape cancels, and a refresh preserves the chosen focus", async () => {
    const { controller, user, api } = await mount();
    const dialog = await openRemove(user);
    const confirm = within(dialog).getByRole("button", { name: "确认移除" });
    await user.tab({ shift: true }); expect(confirm).toHaveFocus();
    await act(async () => controller.refresh()); expect(confirm).toHaveFocus();
    await user.tab(); expect(within(dialog).getByRole("button", { name: "取消" })).toHaveFocus();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument(); expect(api.unregisterModel).not.toHaveBeenCalled();
  });
  it("removes a list row once, focuses the list heading and offers a dismissible short result", async () => {
    const { user, api, controller } = await mount();
    const dialog = await openRemove(user);
    await user.click(within(dialog).getByRole("button", { name: "确认移除" }));
    await waitFor(() => expect(controller.getSnapshot().operation).toBeNull());
    expect(api.unregisterModel).toHaveBeenCalledExactlyOnceWith(model.id, generation);
    expect(screen.queryByRole("article", { name: model.display_name })).not.toBeInTheDocument();
    expect(screen.getByRole("heading", { level: 1, name: "模型库" })).toHaveFocus();
    expect(screen.getByText(/已从模型库移除.*模型文件已保留/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "收起操作结果" }));
    expect(screen.queryByText(/已从模型库移除/)).not.toBeInTheDocument();
    await act(async () => controller.refreshModels());
    expect(screen.queryByRole("article", { name: model.display_name })).not.toBeInTheDocument();
    expect(api.stop).not.toHaveBeenCalled(); expect(api.unloadModel).not.toHaveBeenCalled();
  });
  it("returns from a removed detail to the list with the correct focus", async () => {
    const { user, controller } = await mount();
    await user.click(screen.getByRole("button", { name: `查看 ${model.display_name} 的详情` }));
    await user.click(screen.getByRole("button", { name: "从模型库移除" }));
    await user.click(within(screen.getByRole("dialog")).getByRole("button", { name: "确认移除" }));
    await waitFor(() => expect(controller.getSnapshot().operation).toBeNull());
    expect(screen.getByRole("heading", { level: 1, name: "模型库" })).toHaveFocus();
    expect(screen.queryByText(/模型已不在当前列表/)).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /返回模型库/ })).not.toBeInTheDocument();
  });
  it("names the exact ID when names collide and only removes the selected row", async () => {
    const other = { ...model, id: "same-name-other" };
    const { user, api } = await mount(configuredSnapshot(true), [model, other]);
    const rows = screen.getAllByRole("article", { name: model.display_name });
    await user.click(within(rows[1]).getByRole("button", { name: /更多操作/ }));
    await user.click(screen.getByRole("button", { name: "从模型库移除" }));
    const dialog = screen.getByRole("dialog"); expect(dialog).toHaveTextContent(`模型 ID：${other.id}`);
    await user.click(within(dialog).getByRole("button", { name: "确认移除" }));
    await waitFor(() => expect(api.unregisterModel).toHaveBeenCalledExactlyOnceWith(other.id, generation));
    expect(screen.getAllByRole("article", { name: model.display_name })).toHaveLength(1);
  });
  it("disables an obsolete confirmation when the library generation changes", async () => {
    const { user, api, controller, setModels } = await mount();
    await openRemove(user); setModels([model]);
    await act(async () => controller.refreshModels());
    const dialog = screen.getByRole("dialog");
    expect(within(dialog).getByRole("button", { name: "确认移除" })).toBeDisabled();
    expect(dialog).toHaveTextContent("模型列表已变化");
    expect(api.unregisterModel).not.toHaveBeenCalled();
  });
  it("rechecks busy state while a confirmation is open without stopping someone else's request", async () => {
    const value = snapshot(); value.runtime!.state = "unloaded";
    const { user, api, controller } = await mount(value);
    await openRemove(user);
    value.runtime!.state = "generating"; value.runtime!.active_request = "external-client";
    await act(async () => controller.refresh());
    expect(within(screen.getByRole("dialog")).getByRole("button", { name: "确认移除" })).toBeDisabled();
    expect(screen.getByRole("dialog")).toHaveTextContent("当前有任务进行中");
    expect(api.unregisterModel).not.toHaveBeenCalled(); expect(api.chatCancel).not.toHaveBeenCalled(); expect(api.stop).not.toHaveBeenCalled();
  });
  it("explains the selected ready blocker and leaves an explicit unload action", async () => {
    const state = "ready";
    const value = snapshot(); value.runtime!.state = state;
    const { user, api } = await mount(value);
    await user.click(screen.getByRole("button", { name: `${model.display_name} 的更多操作` }));
    expect(screen.getByRole("button", { name: "从模型库移除" })).toBeDisabled();
    expect(screen.getByText(/请先在模型详情中卸载/)).toBeInTheDocument();
    await user.keyboard("{Escape}");
    await user.click(screen.getByRole("button", { name: `查看 ${model.display_name} 的详情` }));
    expect(screen.getByRole("button", { name: "卸载模型" })).toBeEnabled();
    expect(screen.getByRole("button", { name: "从模型库移除" })).toBeDisabled();
    expect(api.unregisterModel).not.toHaveBeenCalled(); expect(api.unloadModel).not.toHaveBeenCalled();
  });
  it("requires an explicit service stop for a faulted runtime, not an impossible unload", async () => {
    const value = snapshot(); value.runtime!.state = "faulted";
    const { user, api } = await mount(value);
    await user.click(screen.getByRole("button", { name: `${model.display_name} 的更多操作` }));
    expect(screen.getByRole("button", { name: "从模型库移除" })).toBeDisabled();
    expect(screen.getByText(/服务当前故障.*显式停止服务/)).toBeInTheDocument();
    await user.keyboard("{Escape}");
    await user.click(screen.getByRole("button", { name: `查看 ${model.display_name} 的详情` }));
    expect(screen.queryByRole("button", { name: "卸载模型" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "从模型库移除" })).toBeDisabled();
    expect(api.unregisterModel).not.toHaveBeenCalled(); expect(api.unloadModel).not.toHaveBeenCalled(); expect(api.stop).not.toHaveBeenCalled();
  });
  it("deduplicates repeated confirmation and preserves navigation to another detail", async () => {
    const ack = deferred<ModelUnregisterResult>();
    const other = { ...model, id: "other", display_name: "另一模型" };
    const { user, api, setModels, controller } = await mount(configuredSnapshot(true), [model, other], { unregisterModel: vi.fn(() => ack.promise) });
    const dialog = await openRemove(user);
    const confirm = within(dialog).getByRole("button", { name: "确认移除" });
    fireEvent.click(confirm); fireEvent.click(confirm);
    expect(api.unregisterModel).toHaveBeenCalledTimes(1);
    await user.click(screen.getByRole("button", { name: `查看 ${other.display_name} 的详情` }));
    setModels([other]);
    await act(async () => { ack.resolve({ model_id: model.id, removed: true, files_preserved: true }); });
    await waitFor(() => expect(controller.getSnapshot().operation).toBeNull());
    expect(screen.getByRole("heading", { level: 1, name: other.display_name })).toBeInTheDocument();
  });
  it("does not navigate away from newer settings while a confirmed removal completes", async () => {
    const ack = deferred<ModelUnregisterResult>();
    const { user, setModels, controller } = await mount(configuredSnapshot(true), [model], { unregisterModel: vi.fn(() => ack.promise) });
    await user.click(screen.getByRole("button", { name: `查看 ${model.display_name} 的详情` }));
    await user.click(screen.getByRole("button", { name: "从模型库移除" }));
    await user.click(within(screen.getByRole("dialog")).getByRole("button", { name: "确认移除" }));
    await user.click(screen.getByRole("button", { name: "设置" }));
    setModels([]); await act(async () => { ack.resolve({ model_id: model.id, removed: true, files_preserved: true }); });
    await waitFor(() => expect(controller.getSnapshot().operation).toBeNull());
    expect(screen.getByRole("button", { name: "设置" })).toHaveAttribute("aria-current", "page");
    await user.click(screen.getByRole("button", { name: "模型库" }));
    expect(screen.getByRole("heading", { level: 1, name: "模型库" })).toBeInTheDocument();
  });
  it("cancelled navigation does not submit an unconfirmed removal", async () => {
    const { user, api } = await mount(); await openRemove(user);
    fireEvent.keyDown(document, { altKey: true, key: "5" });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "模型库" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument(); expect(api.unregisterModel).not.toHaveBeenCalled();
  });
  it("keeps a rejected model visible and reports failure without a success status", async () => {
    const { user, controller } = await mount(configuredSnapshot(true), [model], { unregisterModel: vi.fn().mockRejectedValue({ code: "runtime_busy", message: "另一个客户端开始工作，请稍后重试。" }) });
    const dialog = await openRemove(user); await user.click(within(dialog).getByRole("button", { name: "确认移除" }));
    await waitFor(() => expect(controller.getSnapshot().operation).toBeNull());
    expect(screen.getByRole("article", { name: model.display_name })).toBeInTheDocument();
    expect(screen.getByText(/当前任务或队列繁忙/)).toBeInTheDocument();
    expect(screen.queryByText(/已从模型库移除/)).not.toBeInTheDocument();
  });
  it("retains an unsaved model profile draft for explicit reimport", async () => {
    const { user, controller, setModels } = await mount(configuredSnapshot(true), [model], { configurationModelGet: vi.fn(async () => modelConfiguration()) });
    await user.click(screen.getByRole("button", { name: `查看 ${model.display_name} 的详情` }));
    fireEvent.click(screen.getByText("运行档案与当前参数"));
    fireEvent.change(await screen.findByRole("spinbutton", { name: /模型上下文长度/ }), { target: { value: "8192" } });
    await user.click(screen.getByRole("button", { name: "从模型库移除" }));
    await user.click(within(screen.getByRole("dialog")).getByRole("button", { name: "确认移除" }));
    await waitFor(() => expect(controller.getSnapshot().operation).toBeNull());
    setModels([model]); await act(async () => controller.refreshModels());
    await user.click(screen.getByRole("button", { name: `查看 ${model.display_name} 的详情` }));
    fireEvent.click(screen.getByText("运行档案与当前参数"));
    expect(await screen.findByRole("spinbutton", { name: /模型上下文长度/ })).toHaveValue(8192);
  });
});
