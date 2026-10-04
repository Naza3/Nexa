import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import App from "../src/App";
import { DesktopController } from "../src/controller";
import { deferred, makeApi, model, snapshot } from "./fixtures";
import type { Snapshot } from "../src/types";

describe("audit J01–J06 corrected product journeys (React mock, not Windows validation)", () => {
  it("J01 unloaded real actor retains selection without a current-residency claim", async () => {
    const value = snapshot(); value.runtime!.state = "unloaded"; render(<App initialPage="models" controller={new DesktopController(makeApi({ snapshot: vi.fn(async () => value) }))} />);
    const region = await screen.findByRole("region", { name: "模型运行状态" }); await waitFor(() => expect(within(region).getByText(/上次选择：Qwen 测试模型/)).toBeVisible()); expect(within(region).queryByRole("heading", { name: model.display_name })).not.toBeInTheDocument(); expect(within(region).queryByText(/上下文 2048/)).not.toBeInTheDocument();
  });
  it("J02 directory guidance reflects the saved verification budget", async () => {
    const value = snapshot(); value.settings.model_verification_timeout_seconds = 600; render(<App initialPage="settings" controller={new DesktopController(makeApi({ snapshot: vi.fn(async () => value) }))} />); expect(await screen.findByText(/总核验时间 600 秒/)).toBeVisible();
  });
  it("J03 the first-use settings page exposes an offline initialization action", async () => {
    const value = snapshot(); value.initialized = false; value.connection = "stopped"; value.runtime = null; const api = makeApi({ snapshot: vi.fn(async () => value), initialize: vi.fn(async () => ({ ...value, initialized: true })) }); render(<App initialPage="settings" controller={new DesktopController(api)} />); fireEvent.click(await screen.findByRole("button", { name: "仅初始化配置" })); await waitFor(() => expect(api.initialize).toHaveBeenCalledTimes(1)); expect(api.start).not.toHaveBeenCalled();
  });
  it("J04 one confirmed idle switch invokes one backend load and never a manual unload", async () => {
    const second = { ...model, id: "second", display_name: "第二模型" }; const api = makeApi({ modelsPage: vi.fn(async () => ({ data: [model, second], generation: "g", next_after: null })) }); render(<App initialPage="models" controller={new DesktopController(api)} />); fireEvent.click(await screen.findByRole("button", { name: "切换并测试" })); expect(api.loadModel).not.toHaveBeenCalled(); fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "切换并测试" })); await waitFor(() => expect(api.loadModel).toHaveBeenCalledTimes(1)); expect(api.unloadModel).not.toHaveBeenCalled();
  });
  it("J05 untouched legacy settings follow an external refresh instead of writing stale values", async () => {
    let value = snapshot(); const api = makeApi({ snapshot: vi.fn(async () => value) }); const controller = new DesktopController(api); render(<App initialPage="settings" controller={controller} />); await screen.findByRole("spinbutton", { name: /上下文长度/ }); value = { ...value, settings: { ...value.settings, context_size: 8192 } }; await act(async () => controller.refresh()); expect(screen.getByRole("spinbutton", { name: /上下文长度/ })).toHaveValue(8192); fireEvent.click(screen.getByRole("button", { name: "保存偏好" })); await waitFor(() => expect(api.saveSettings).toHaveBeenCalledWith(expect.objectContaining({ context_size: 8192 })));
  });
  it("J06 overview is primary and client connection data has direct copy actions", async () => {
    render(<App controller={new DesktopController(makeApi())} />); await screen.findByRole("heading", { name: "概览" }); fireEvent.click(screen.getByRole("button", { name: "API 接入" })); expect(await screen.findByLabelText("本机客户端 Base URL")).toHaveTextContent("http://127.0.0.1:12345/v1"); expect(screen.getByRole("button", { name: "复制模型 ID" })).toBeEnabled(); expect(screen.getByRole("button", { name: "复制本机 Base URL" })).toBeEnabled();
  });
  it("a confirmed start-then-load survives navigation with one retrievable terminal activity", async () => {
    const start = deferred<Snapshot>(); let value: Snapshot = { ...snapshot(), connection: "stopped", runtime: null }; const api = makeApi({ snapshot: vi.fn(async () => value), start: vi.fn(() => start.promise) }); const controller = new DesktopController(api); render(<App initialPage="models" controller={controller} />); fireEvent.click(await screen.findByRole("button", { name: "加载模型" })); fireEvent.click(screen.getByRole("button", { name: "API 接入" })); value = snapshot(); await act(async () => start.resolve(value)); await waitFor(() => expect(api.loadModel).toHaveBeenCalledTimes(1)); fireEvent.click(screen.getByRole("button", { name: "活动" })); await waitFor(() => expect(controller.getSnapshot().activities.filter((item) => item.kind === "model")).toHaveLength(1)); expect(screen.getByText(/加载并短测 · qwen/)).toBeVisible();
  });
});
