import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import App from "../src/App";
import { DesktopController } from "../src/controller";
import type { DesktopApi, ModelConfiguration, Snapshot } from "../src/types";
import { configuredSnapshot, configuration, modelConfiguration, revision } from "./configurationFixtures";
import { deferred, makeApi, model, runtime } from "./fixtures";
async function setup(initial = configuredSnapshot(true), overrides: Partial<DesktopApi> = {}, page: "settings" | "models" | "api" = "settings") {
  let value = initial;
  const api = makeApi({ snapshot: vi.fn(async () => structuredClone(value)), configurationGet: vi.fn(async () => structuredClone(value.configuration!)), configurationModelGet: vi.fn(async () => modelConfiguration()),
    configurationSave: vi.fn(async (request) => { const config = structuredClone(value.configuration!); config.revision = revision("b"); if (request.update.kind === "global_defaults") config.saved.global_defaults = request.update.global_defaults; if (request.update.kind === "request_defaults") { config.saved.request_defaults = request.update.request_defaults; if (config.runtime_effective) config.runtime_effective.values.request_defaults = request.update.request_defaults; } value = { ...value, configuration: config }; return config; }), ...overrides });
  const controller = new DesktopController(api); const ui = render(<App initialPage={page} controller={controller} />); await waitFor(() => expect(controller.getSnapshot().booting).toBe(false));
  return { api, controller, ...ui, setSnapshot: (next: Snapshot) => { value = next; } };
}
describe("canonical configuration UI", () => {
  it("uses new global and separate UI preferences without the old inference save form", async () => {
    const { api } = await setup(); expect(screen.getByRole("spinbutton", { name: "默认上下文长度" })).toHaveValue(4096); expect(screen.getByRole("spinbutton", { name: "默认推理线程" })).toHaveValue(null);
    expect(screen.queryByRole("button", { name: "保存偏好" })).not.toBeInTheDocument(); expect(api.saveSettings).not.toHaveBeenCalled();
  });
  it("updates a clean draft when polling returns a newer revision", async () => {
    const { controller, setSnapshot } = await setup(); const next = configuredSnapshot(true); next.configuration!.revision = revision("b"); next.configuration!.saved.global_defaults.context_size = 8192;
    setSnapshot(next); await act(async () => controller.refresh()); expect(screen.getByRole("spinbutton", { name: "默认上下文长度" })).toHaveValue(8192);
  });
  it("preserves a dirty draft, blocks stale saving, and lets the user explicitly reread", async () => {
    const { controller, setSnapshot, api } = await setup(); const input = screen.getByRole("spinbutton", { name: "默认上下文长度" }); fireEvent.change(input, { target: { value: "2048" } });
    const next = configuredSnapshot(true); next.configuration!.revision = revision("b"); next.configuration!.saved.global_defaults.context_size = 8192; setSnapshot(next); await act(async () => controller.refresh());
    expect(input).toHaveValue(2048); expect(screen.getByRole("button", { name: "保存全局默认值" })).toBeDisabled(); expect(screen.getByText("配置版本已变化")).toBeVisible(); expect(api.configurationSave).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "丢弃草稿并读取新配置" })); expect(input).toHaveValue(8192);
  });
  it("sends one scoped global save with its base revision", async () => {
    const { api } = await setup(); fireEvent.change(screen.getByRole("spinbutton", { name: "默认上下文长度" }), { target: { value: "8192" } }); fireEvent.click(screen.getByRole("button", { name: "保存全局默认值" }));
    await waitFor(() => expect(api.configurationSave).toHaveBeenCalledExactlyOnceWith({ expected_revision: revision(), update: { kind: "global_defaults", global_defaults: { context_size: 8192, threads: null, batch_size: 512 } } })); expect(api.stop).not.toHaveBeenCalled(); expect(api.loadModel).not.toHaveBeenCalled();
  });
  it("allows online request defaults without stopping or reloading", async () => {
    const { api } = await setup(configuredSnapshot()); fireEvent.change(screen.getByRole("spinbutton", { name: "默认输出预算" }), { target: { value: "900" } }); fireEvent.click(screen.getByRole("button", { name: "保存请求默认值" }));
    await waitFor(() => expect(api.configurationSave).toHaveBeenCalledExactlyOnceWith({ expected_revision: revision(), update: { kind: "request_defaults", request_defaults: { max_output_tokens: 900, temperature: 0.8, top_p: 0.95 } } })); expect(api.stop).not.toHaveBeenCalled(); expect(api.loadModel).not.toHaveBeenCalled();
  });
  it.each(["configuration_conflict", "configuration_durability_unconfirmed"])("keeps the draft and does not retry after %s", async (code) => {
    const latest = configuration(); latest.revision = revision("b"); latest.saved.global_defaults.context_size = 16384;
    const { api } = await setup(configuredSnapshot(true), { configurationGet: vi.fn(async () => latest), configurationSave: vi.fn(async () => { throw { code, message: "controlled failure" }; }) });
    fireEvent.change(screen.getByRole("spinbutton", { name: "默认上下文长度" }), { target: { value: "8192" } }); fireEvent.click(screen.getByRole("button", { name: "保存全局默认值" }));
    await waitFor(() => expect(screen.getByText("配置版本已变化")).toBeVisible()); expect(api.configurationSave).toHaveBeenCalledTimes(1); expect(api.configurationGet).toHaveBeenCalledTimes(1); expect(screen.getByRole("spinbutton", { name: "默认上下文长度" })).toHaveValue(8192);
  });
  it("initializes offline without starting, scanning or generating LAN keys", async () => {
    const initial = configuredSnapshot(true); initial.initialized = false; initial.configuration!.revision = "absent";
    const { api } = await setup(initial, { initialize: vi.fn(async () => configuredSnapshot(true)) }); fireEvent.click(screen.getByRole("button", { name: "仅初始化配置" })); await waitFor(() => expect(api.initialize).toHaveBeenCalledTimes(1)); expect(api.start).not.toHaveBeenCalled(); expect(api.scanModels).not.toHaveBeenCalled(); expect(api.copyLanToken).not.toHaveBeenCalled();
  });
  it("presents migration differences and sends both source revisions only after confirmation", async () => {
    const value = configuredSnapshot(true); value.configuration!.schema_version = 1; value.configuration!.migration = { state: "required", differences: [{ field: "context_size", api: 4096, desktop: 2048 }], preferences_revision: revision("d"), backup_available: false };
    const { api } = await setup(value, { configurationMigrate: vi.fn(async () => configuration()) }); expect(screen.getByText("context_size：API 4096 / 桌面 2048")).toBeVisible(); fireEvent.click(screen.getByRole("button", { name: "采用旧桌面值" })); expect(api.configurationMigrate).not.toHaveBeenCalled(); fireEvent.click(screen.getByRole("button", { name: "备份并迁移" }));
    await waitFor(() => expect(api.configurationMigrate).toHaveBeenCalledExactlyOnceWith({ expected_revision: revision(), expected_preferences_revision: revision("d"), choice: "desktop", custom: null }));
  });
  it("shows saved effective sources beside actual and historical parameters", async () => {
    const config = modelConfiguration(); config.current_load_options = null; config.restore_load_options = { context_size: 2048, threads: 2, batch_size: 128 };
    const value = configuredSnapshot(); value.runtime!.state = "unloaded";
    await setup(value, { configurationModelGet: vi.fn(async () => config) }, "models"); fireEvent.click(screen.getByText("运行档案与当前参数")); await screen.findByText(/已保存解析值 4 · 来源 automatic/); expect(screen.getByText("当前驻留参数：无")).toBeVisible(); expect(screen.getByText(/上次会话恢复参数/)).toBeVisible();
  });
  it("saves inherited nulls without loading the model", async () => {
    const config = modelConfiguration(); config.load_overrides = { context_size: 2048, threads: 2, batch_size: 128 };
    const { api } = await setup(configuredSnapshot(), { configurationModelGet: vi.fn(async () => config), loadModelProfile: vi.fn(async () => runtime()) }, "models"); fireEvent.click(screen.getByText("运行档案与当前参数")); fireEvent.click(await screen.findByRole("button", { name: "恢复继承" })); fireEvent.click(screen.getByRole("button", { name: "保存模型档案" }));
    await waitFor(() => expect(api.configurationSave).toHaveBeenCalledExactlyOnceWith({ expected_revision: revision(), update: { kind: "model_profile", model_id: model.id, load_overrides: { context_size: null, threads: null, batch_size: null } } })); expect(api.loadModelProfile).not.toHaveBeenCalled();
  });
  it("blocks a known context overflow before profile save", async () => {
    const config = modelConfiguration(); config.context_limit = 2048;
    const { api } = await setup(configuredSnapshot(), { configurationModelGet: vi.fn(async () => config) }, "models"); fireEvent.click(screen.getByText("运行档案与当前参数")); const input = await screen.findByRole("spinbutton", { name: /模型上下文长度/ }); fireEvent.change(input, { target: { value: "4096" } }); expect(screen.getByRole("button", { name: "保存模型档案" })).toBeDisabled(); expect(api.configurationSave).not.toHaveBeenCalled();
  });
});

describe("configuration controller boundaries", () => {
  it("ordinary profile load sends only the model ID", async () => {
    const api = makeApi({ snapshot: vi.fn(async () => configuredSnapshot()), loadModelProfile: vi.fn(async () => runtime()) }); const controller = new DesktopController(api); await controller.refresh(); await controller.loadModel(model.id); expect(api.loadModelProfile).toHaveBeenCalledExactlyOnceWith(model.id); expect(api.loadModel).not.toHaveBeenCalled();
  });
  it("temporary load forwards only explicit overrides and never saves", async () => {
    const api = makeApi({ snapshot: vi.fn(async () => configuredSnapshot()), loadModelProfile: vi.fn(async () => runtime()), configurationSave: vi.fn() }); const controller = new DesktopController(api); await controller.refresh(); await controller.loadModel(model.id, { threads: 3 }); expect(api.loadModelProfile).toHaveBeenCalledExactlyOnceWith(model.id, { threads: 3 }); expect(api.configurationSave).not.toHaveBeenCalled();
  });
  it.each(["required", "legacy_compatible"] as const)("new load requires schema2 even when migration is %s", async (state) => {
    const value = configuredSnapshot(); value.configuration!.schema_version = 1; value.configuration!.migration.state = state; const api = makeApi({ snapshot: vi.fn(async () => value), loadModelProfile: vi.fn() }); const controller = new DesktopController(api); await controller.refresh(); await controller.loadModel(model.id); expect(api.loadModelProfile).not.toHaveBeenCalled(); expect(controller.getSnapshot().error?.code).toBe("configuration_migration_required");
  });
  it("rejects global online saves without calling a writer", async () => {
    const api = makeApi({ snapshot: vi.fn(async () => configuredSnapshot()), configurationSave: vi.fn() }); const controller = new DesktopController(api); await controller.refresh(); expect(await controller.saveConfiguration({ expected_revision: revision(), update: { kind: "global_defaults", global_defaults: configuration().saved.global_defaults } })).toBe(false); expect(api.configurationSave).not.toHaveBeenCalled(); expect(api.stop).not.toHaveBeenCalled();
  });
  it("never overwrites a newer model-configuration response with a late read", async () => {
    const old = deferred<ModelConfiguration>(); const latest = modelConfiguration(); latest.configuration_revision = revision("b"); latest.saved_effective.context_size = 8192; const api = makeApi({ snapshot: vi.fn(async () => configuredSnapshot()), configurationModelGet: vi.fn().mockReturnValueOnce(old.promise).mockResolvedValueOnce(latest) }); const controller = new DesktopController(api); await controller.refresh(); const pending = controller.refreshModelConfiguration(model.id); await controller.refreshModelConfiguration(model.id); old.resolve(modelConfiguration()); await pending; expect(controller.getSnapshot().model_configurations[model.id]).toEqual(latest);
  });
  it("chat uses running request defaults instead of newer saved-only values", async () => {
    const value = configuredSnapshot(); value.configuration!.saved.request_defaults.max_output_tokens = 1024; value.configuration!.pending_restart = true; const api = makeApi({ snapshot: vi.fn(async () => value) }); const controller = new DesktopController(api); await controller.refresh(); await controller.send("hello"); expect(api.chatStart).toHaveBeenCalledWith(expect.objectContaining({ max_output_tokens: 768 }));
  });
  it("native local token failures never leak secret-shaped messages into view or history", async () => {
    const api = makeApi({ copyToken: vi.fn(async () => { throw { code: "token_copy_failed", message: "Bearer SECRET" }; }) }); const controller = new DesktopController(api); await controller.refresh(); await controller.copyToken(); expect(JSON.stringify(controller.getSnapshot())).not.toContain("SECRET"); expect(localStorage.getItem("nexa.activity-summary.v1")).toBeNull();
  });
});

describe("configuration sequencing and degraded capability", () => {
  it("preserves the connected service on a configuration-route failure and never falls back to a legacy load", async () => {
    const value = configuredSnapshot(); value.configuration = null; value.configuration_error = { code: "configuration_unavailable", message: "old backend" };
    const { api, controller } = await setup(value, {}, "models"); expect(screen.getByRole("button", { name: "停止运行服务" })).toBeEnabled(); expect(screen.getByText("统一配置暂不可用")).toBeVisible(); await act(async () => controller.loadModel(model.id)); expect(api.loadModel).not.toHaveBeenCalled(); expect(api.start).not.toHaveBeenCalled();
  });
  it("allows profile saving while an external request is generating without cancellation or reload", async () => {
    const value = configuredSnapshot(); value.runtime!.state = "generating"; value.runtime!.active_request = "external-request";
    const { api } = await setup(value, {}, "models"); fireEvent.click(screen.getByText("运行档案与当前参数")); const input = await screen.findByRole("spinbutton", { name: /模型线程数/ }); fireEvent.change(input, { target: { value: "3" } }); const save = screen.getByRole("button", { name: "保存模型档案" }); expect(save).toBeEnabled(); fireEvent.click(save); await waitFor(() => expect(api.configurationSave).toHaveBeenCalledTimes(1)); expect(api.chatCancel).not.toHaveBeenCalled(); expect(api.loadModel).not.toHaveBeenCalled(); expect(screen.getByRole("button", { name: "按已保存档案加载并测试" })).toBeDisabled();
  });
  it("does not load after a failed save-and-reload confirmation", async () => {
    const { api } = await setup(configuredSnapshot(), { configurationSave: vi.fn(async () => { throw { code: "configuration_conflict", message: "changed" }; }), loadModelProfile: vi.fn(async () => runtime()) }, "models");
    fireEvent.click(screen.getByText("运行档案与当前参数")); fireEvent.change(await screen.findByRole("spinbutton", { name: /模型线程数/ }), { target: { value: "3" } }); fireEvent.click(screen.getByRole("button", { name: "保存并重新加载" })); fireEvent.click(screen.getByRole("button", { name: "加载并测试" })); await waitFor(() => expect(api.configurationSave).toHaveBeenCalledTimes(1)); expect(api.loadModelProfile).not.toHaveBeenCalled();
  });
  it("retains a successful save if the explicitly requested reload fails", async () => {
    const value = configuredSnapshot(); const config = modelConfiguration(); const api = makeApi({ snapshot: vi.fn(async () => value), configurationGet: vi.fn(async () => value.configuration!), configurationModelGet: vi.fn(async () => config),
      configurationSave: vi.fn(async (request) => { value.configuration!.revision = revision("b"); if (request.update.kind === "model_profile") { config.load_overrides = request.update.load_overrides; config.configuration_revision = revision("b"); } return value.configuration!; }),
      loadModelProfile: vi.fn(async () => { throw { code: "model_load_failed", message: "load failed" }; }) });
    const controller = new DesktopController(api); render(<App initialPage="models" controller={controller} />); fireEvent.click(await screen.findByText("运行档案与当前参数")); fireEvent.change(await screen.findByRole("spinbutton", { name: /模型线程数/ }), { target: { value: "3" } }); fireEvent.click(screen.getByRole("button", { name: "保存并重新加载" })); fireEvent.click(screen.getByRole("button", { name: "加载并测试" }));
    await waitFor(() => expect(api.loadModelProfile).toHaveBeenCalledExactlyOnceWith(model.id)); expect(config.load_overrides.threads).toBe(3); expect(controller.getSnapshot().snapshot?.configuration?.revision).toBe(revision("b")); expect(api.configurationSave).toHaveBeenCalledTimes(1);
  });
});

describe("window-owned unsaved configuration drafts", () => {
  it("preserves a dirty group while navigating and requires comparison after an external revision changed off-page", async () => {
    const { api, controller, setSnapshot } = await setup(); fireEvent.change(screen.getByRole("spinbutton", { name: "默认上下文长度" }), { target: { value: "8192" } }); fireEvent.click(screen.getByRole("button", { name: "活动" }));
    const next = configuredSnapshot(true); next.configuration!.revision = revision("b"); next.configuration!.saved.global_defaults.context_size = 16384; setSnapshot(next); await act(async () => controller.refresh()); fireEvent.click(screen.getByRole("button", { name: "设置" })); expect(screen.getByRole("spinbutton", { name: "默认上下文长度" })).toHaveValue(8192); expect(screen.getByRole("button", { name: "保存全局默认值" })).toBeDisabled();
    fireEvent.click(screen.getByText("比较草稿与最新已保存")); expect(screen.getByText(/最新已保存：.*16384/)).toBeVisible(); expect(api.configurationSave).not.toHaveBeenCalled(); fireEvent.click(screen.getByRole("button", { name: "确认保留草稿，下次保存覆盖此组" })); fireEvent.click(screen.getByRole("button", { name: "保存全局默认值" })); await waitFor(() => expect(api.configurationSave).toHaveBeenCalledWith(expect.objectContaining({ expected_revision: revision("b"), update: { kind: "global_defaults", global_defaults: { context_size: 8192, threads: null, batch_size: 512 } } })));
  });
  it("keeps model A and B drafts isolated across opening another model and leaving the page", async () => {
    const second = { ...model, id: "second", display_name: "Second model" }; const { api } = await setup(configuredSnapshot(), { modelsPage: vi.fn(async () => ({ data: [model, second], generation: "g", next_after: null })), configurationModelGet: vi.fn(async (id) => ({ ...modelConfiguration(), model_id: id })) }, "models");
    const row = (name: string) => within(screen.getByRole("heading", { name, level: 3 }).closest("article")!);
    fireEvent.click(row(model.display_name).getByText("运行档案与当前参数")); fireEvent.change(await row(model.display_name).findByRole("spinbutton", { name: /模型线程数/ }), { target: { value: "3" } });
    fireEvent.click(row(second.display_name).getByText("运行档案与当前参数")); const secondInput = await row(second.display_name).findByRole("spinbutton", { name: /模型线程数/ }); expect(secondInput).toHaveValue(null); fireEvent.change(secondInput, { target: { value: "5" } }); fireEvent.click(screen.getByRole("button", { name: "API 接入" })); fireEvent.click(screen.getByRole("button", { name: "模型库" }));
    fireEvent.click(row(model.display_name).getByText("运行档案与当前参数")); fireEvent.click(row(second.display_name).getByText("运行档案与当前参数")); expect(await row(model.display_name).findByRole("spinbutton", { name: /模型线程数/ })).toHaveValue(3); expect(await row(second.display_name).findByRole("spinbutton", { name: /模型线程数/ })).toHaveValue(5); expect(api.configurationSave).not.toHaveBeenCalled();
  });
});
