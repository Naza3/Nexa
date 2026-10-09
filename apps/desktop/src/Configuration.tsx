import { errorText } from "./errorPresentation";
import { useEffect, useState } from "react";
import type { DesktopController, ViewState } from "./controller";
import type { ConfigurationSnapshot, LoadOptions, LoadOverrides, ModelConfiguration, UiPreferences } from "./types";
import { useConfigDraft } from "./configDraft";
import { ownRecord } from "./records";
import { Modal } from "./Modal";

export function DraftConflict({ reset, rebase, draft, saved }: { reset: () => void; rebase?: () => void; draft?: unknown; saved?: unknown }) {
  return <div className="notice-band warning" role="alert"><div><strong>配置版本已变化</strong><p>未覆盖你的草稿，也未自动重试保存。请核对最新配置后重新编辑。</p>{draft !== undefined && <details><summary>比较草稿与最新已保存</summary><p className="hash">草稿：{JSON.stringify(draft)}</p><p className="hash">最新已保存：{JSON.stringify(saved)}</p></details>}<div className="workspace-actions"><button onClick={reset}>丢弃草稿并读取新配置</button>{rebase && <button onClick={rebase}>确认保留草稿，下次保存覆盖此组</button>}</div></div></div>;
}
export function MigrationNotice({ config, state, controller }: { config: ConfigurationSnapshot; state: ViewState; controller: DesktopController }) {
  const [choice, setChoice] = useState<"api" | "desktop" | null>(null);
  if (["not_needed", "complete"].includes(config.migration.state)) return null;
  return <section className="settings-card" aria-label="旧配置迁移"><h2>统一旧配置</h2><p>桌面推理偏好与 API 默认值来自旧文件。选择保留来源后一次迁移；不会重置凭据或模型索引。</p>
    {config.migration.differences.map((item) => <p key={item.field}>{item.field}：API {item.api ?? "自动线程"} / 桌面 {item.desktop}</p>)}
    <p>升级会保存原配置备份并写入新格式。旧版本可能拒绝新格式；回退须停止服务并恢复匹配备份。</p>
    <div className="workspace-actions"><button disabled={state.snapshot?.connection !== "stopped" || !!state.operation} onClick={() => setChoice("api")}>采用 API 默认值</button><button disabled={state.snapshot?.connection !== "stopped" || !!state.operation} onClick={() => setChoice("desktop")}>采用旧桌面值</button></div>
    {state.snapshot?.connection !== "stopped" && <p>请先显式停止服务后迁移。</p>}
    {choice && <Modal title="确认统一配置来源？" confirm="备份并迁移" onCancel={() => setChoice(null)} onConfirm={() => { const source = choice; setChoice(null); void controller.migrateConfiguration({ expected_revision: config.revision, expected_preferences_revision: config.migration.preferences_revision, choice: source, custom: null }); }}><p>采用{choice === "api" ? " API 默认值" : "旧桌面值"}，原配置由原生端备份。新格式可能无法在旧版本读取；不会自动启动服务。</p></Modal>}
  </section>;
}

export function GlobalConfiguration({ state, controller }: { state: ViewState; controller: DesktopController }) {
  const config = state.snapshot!.configuration!;
  const source = { global_defaults: config.saved.global_defaults };
  const form = useConfigDraft(source, config.revision, "global_defaults");
  const busy = !!state.operation || state.library_phase !== "idle" || state.download_phase !== "idle" || state.chat_phase !== "idle";
  const editable = state.snapshot?.connection === "stopped" && state.snapshot.initialized && config.migration.state !== "required" && !busy;
  const { global_defaults: load } = form.draft;
  const valid = Number.isInteger(load.context_size) && load.context_size >= 32 && load.context_size <= 131072 &&
    (load.threads === null || Number.isInteger(load.threads) && load.threads >= 1 && load.threads <= 256) && Number.isInteger(load.batch_size) && load.batch_size >= 1 && load.batch_size <= Math.min(load.context_size, 4096);
  return <><MigrationNotice config={config} state={state} controller={controller} /><section className="settings-card" aria-label="全局默认配置"><h2>全局加载默认值</h2><p>模型档案未覆盖的字段继承这里。保存前须停止服务，下次显式启动时生效。</p>
    {form.conflict && <DraftConflict reset={form.reset} rebase={form.rebase} draft={form.draft} saved={source} />}
    <div className="settings-grid">{(["context_size", "threads", "batch_size"] as const).map((key) => <label className="setting-field" key={key}><span>{{ context_size: "默认上下文长度", threads: "默认推理线程", batch_size: "默认批次大小" }[key]}</span><input type="number" disabled={!editable} value={load[key] === null || Number.isNaN(load[key]) ? "" : load[key]} placeholder={key === "threads" ? "留空：自动" : ""} onChange={(event) => form.setDraft({ ...form.draft, global_defaults: { ...load, [key]: key === "threads" && event.target.value === "" ? null : event.target.valueAsNumber } })} /></label>)}
    </div>
    <p>自动线程由后端按设备解析。保存不会修改当前驻留、排队请求或历史测试条件。</p>{!valid && <p className="warning-text" role="alert">请检查字段范围和批次与上下文约束。</p>}
    <div className="save-row"><span>{config.runtime_effective ? config.pending_restart ? "已保存配置与运行中配置不同，需重启" : "运行中配置已报告" : "当前没有运行中配置快照"}</span><button disabled={!editable || !form.dirty || form.conflict || !valid} onClick={() => void controller.saveConfiguration({ expected_revision: form.baseRevision!, update: { kind: "global_defaults", global_defaults: form.draft.global_defaults } })}>保存全局默认值</button></div>
  </section></>;
}

export function UiPreferencesForm({ state, controller }: { state: ViewState; controller: DesktopController }) {
  const saved = state.snapshot!.ui_preferences!;
  const form = useConfigDraft(saved.preferences, saved.revision, "ui_preferences");
  const set = (patch: Partial<UiPreferences>) => form.setDraft({ ...form.draft, ...patch });
  return <section className="settings-card"><h2>界面偏好</h2>{form.conflict && <DraftConflict reset={form.reset} rebase={form.rebase} draft={form.draft} saved={saved.preferences} />}<label className="setting-field"><span>默认下载源</span><select value={form.draft.download_source} onChange={(event) => set({ download_source: event.target.value as UiPreferences["download_source"] })}><option value="modelscope">ModelScope</option><option value="huggingface">Hugging Face</option></select></label>
    <label className="auto-test-option"><input type="checkbox" checked={form.draft.close_runtime_on_exit} onChange={(event) => set({ close_runtime_on_exit: event.target.checked })} />关闭窗口时同时退出运行服务</label><p>{form.draft.close_runtime_on_exit ? "保存后，真正退出界面将停止所有客户端任务并释放模型。" : "真正退出界面取消本窗口任务并保留服务。重新打开 Nexa 可连接现有服务。"}关闭到托盘时只隐藏窗口，不执行此退出策略。</p><button disabled={!!state.operation || !form.dirty || form.conflict} onClick={() => void controller.saveUiPreferences(form.draft, form.baseRevision!)}>保存界面偏好</button></section>;
}

function ProfileEditor({ config, state, controller, readUnconfirmed = false }: { config: ModelConfiguration; state: ViewState; controller: DesktopController; readUnconfirmed?: boolean }) {
  const form = useConfigDraft(config.load_overrides, config.configuration_revision, `model_profile:${config.model_id}`);
  const [apply, setApply] = useState<"saved" | "save" | "temporary" | null>(null);
  const temporaryDraft = useConfigDraft<Partial<LoadOptions>>({}, undefined, `model_temporary:${config.model_id}`);
  const temporary = temporaryDraft.draft;
  const setTemporary = temporaryDraft.setDraft;
  const values = form.draft;
  const saved = config.saved_effective;
  const effectiveContext = values.context_size ?? state.snapshot?.configuration?.saved.global_defaults.context_size ?? saved.context_size;
  const valid = Object.entries(values).every(([key, value]) => value === null || Number.isInteger(value) && value >= (key === "context_size" ? 32 : 1) && value <= (key === "context_size" ? Math.min(config.context_limit ?? 131072, 131072) : key === "threads" ? 256 : Math.min(4096, effectiveContext)));
  const runtime = state.snapshot?.connection === "connected" ? state.snapshot.runtime : null;
  const resident = runtime?.selected_model === config.model_id && ["ready", "generating"].includes(runtime.state);
  const currentOptions = resident ? runtime?.load_options : null;
  const restoreOptions = !resident && runtime?.selected_model === config.model_id ? runtime.load_options : null;
  const busy = readUnconfirmed || !!state.operation || state.library_phase !== "idle" || state.download_phase !== "idle" || state.chat_phase !== "idle" || !!runtime?.active_request || !!runtime?.queued_jobs || !!runtime?.stopping || !!runtime?.registry_busy || ["loading", "generating", "unloading"].includes(runtime?.state ?? "");
  return <div className="profile-editor">{form.conflict && <DraftConflict reset={form.reset} rebase={form.rebase} draft={form.draft} saved={config.load_overrides} />}<p>空字段继承全局默认；仅保存本模型档案，不改变当前驻留。未保存草稿切页保留。恢复副本不是后端配置；重开后须确认恢复并重新核对版本。</p><p>继承来源：全局上下文 {state.snapshot?.configuration?.saved.global_defaults.context_size} · 线程 {state.snapshot?.configuration?.saved.global_defaults.threads ?? "自动"} · 批次 {state.snapshot?.configuration?.saved.global_defaults.batch_size}；草稿有效结果将在保存后由后端重新解析。</p><div className="settings-grid">{(["context_size", "threads", "batch_size"] as const).map((key) => <label className="setting-field" key={key}><span>{{ context_size: "模型上下文长度", threads: "模型线程数", batch_size: "模型批次大小" }[key]}</span><input type="number" placeholder="继承全局" value={values[key] === null || Number.isNaN(values[key]) ? "" : values[key]} onChange={(event) => form.setDraft({ ...values, [key]: event.target.value === "" ? null : event.target.valueAsNumber } as LoadOverrides)} /><small>已保存解析值 {saved[key]} · 来源 {config.saved_sources[key]}</small></label>)}</div>
    <p>模型上下文上限：{config.context_limit ?? "未知"}；上限不代表内存保证。</p><p>当前驻留参数：{currentOptions ? JSON.stringify(currentOptions) : "无"}</p>{restoreOptions && <p>上次会话恢复参数：{JSON.stringify(restoreOptions)}；本机同 ID 自动恢复沿用此值，主动加载采用已保存档案。</p>}
    <p>{config.pending_apply ? "新档案尚未应用到驻留模型" : "下次主动加载使用已保存解析值"}</p>{!valid && <p role="alert" className="warning-text">{values.context_size !== null && config.context_limit !== null && values.context_size > config.context_limit ? `该模型声明上限为 ${config.context_limit}，当前填写 ${values.context_size}，请调整后保存。` : "参数超出范围或批次大于上下文，请调整后保存。"}</p>}
    <div className="workspace-actions"><button disabled={readUnconfirmed || !!state.operation || state.library_phase !== "idle" || state.download_phase !== "idle" || !form.dirty || form.conflict || !valid || state.snapshot?.configuration?.migration.state === "required"} onClick={() => void controller.saveConfiguration({ expected_revision: form.baseRevision!, update: { kind: "model_profile", model_id: config.model_id, load_overrides: values } })}>保存模型档案</button><button disabled={busy || form.dirty || state.snapshot?.configuration?.pending_restart} onClick={() => setApply("saved")}>按已保存档案加载并测试</button><button disabled={busy || !form.dirty || form.conflict || !valid || state.snapshot?.configuration?.migration.state === "required" || state.snapshot?.configuration?.pending_restart} onClick={() => setApply("save")}>保存并重新加载</button><button onClick={() => form.setDraft({ context_size: null, threads: null, batch_size: null })}>恢复继承</button><button disabled={!form.dirty && !form.conflict} onClick={form.reset}>放弃档案更改</button></div>
    {state.snapshot?.configuration?.pending_restart && <p>磁盘配置尚未被当前服务采用，请先显式停止并重启服务。</p>}
    <details><summary>仅本次加载的临时覆盖</summary><p>留空使用后端档案。此处不会保存配置，主动重新加载或服务重启后结束；空闲恢复沿用本次会话参数。</p><div className="settings-grid">{(["context_size", "threads", "batch_size"] as const).map((key) => <label className="setting-field" key={key}><span>临时 {key}</span><input type="number" value={temporary[key] === undefined || Number.isNaN(temporary[key]) ? "" : temporary[key]} onChange={(event) => { const next = { ...temporary }; if (event.target.value === "") delete next[key]; else next[key] = event.target.valueAsNumber; setTemporary(next); }} /></label>)}</div><button disabled={busy || state.snapshot?.configuration?.schema_version !== 2 || state.snapshot?.configuration?.pending_restart || Object.entries(temporary).some(([key, value]) => !Number.isInteger(value) || value < (key === "context_size" ? 32 : 1) || value > (key === "context_size" ? Math.min(config.context_limit ?? 131072, 131072) : key === "threads" ? 256 : 4096))} onClick={() => setApply("temporary")}>按临时覆盖加载并测试</button></details>
    {apply && <Modal title={apply === "save" ? "保存档案并重新加载？" : "应用已保存的模型档案？"} confirm="加载并测试" onCancel={() => setApply(null)} onConfirm={() => {
      if (readUnconfirmed) { setApply(null); return; }
      const action = apply; setApply(null);
      void (async () => {
        if (action === "save" && !await controller.saveConfiguration({ expected_revision: form.baseRevision!, update: { kind: "model_profile", model_id: config.model_id, load_overrides: values } })) return;
        await controller.loadModel(config.model_id, action === "temporary" ? temporary : undefined);
      })();
    }}><p>将根据后端当前有效档案加载并短测。必要时释放旧驻留模型；忙时不抢占，失败不会自动恢复旧模型。</p></Modal>}
  </div>;
}
export function ModelProfile({ modelId, state, controller }: { modelId: string; state: ViewState; controller: DesktopController }) {
  const [open, setOpen] = useState(false);
  const [retrying, setRetrying] = useState(false);
  const revision = state.snapshot?.configuration?.revision;
  const runtimeScope = JSON.stringify([state.snapshot?.connection, state.snapshot?.runtime?.state, state.snapshot?.runtime?.selected_model, state.snapshot?.runtime?.load_options]);
  useEffect(() => { if (open) void controller.refreshModelConfiguration(modelId); }, [open, modelId, revision, runtimeScope, controller]);
  if (!state.snapshot?.configuration) return <p className="small-note">当前版本未提供逐模型运行档案。</p>;
  const config = ownRecord(state.model_configurations, modelId);
  const error = ownRecord(state.model_configuration_errors, modelId);
  const retry = async () => {
    if (retrying) return;
    setRetrying(true);
    try { await controller.refreshModelConfiguration(modelId); } finally { setRetrying(false); }
  };
  return <details onToggle={(event) => setOpen(event.currentTarget.open)}><summary>运行档案与当前参数</summary>{error && <div role="alert" className="warning-text"><p>{errorText(error)}</p><button disabled={retrying || state.closing} onClick={() => void retry()}>{retrying ? "正在重新读取档案…" : "重新读取模型档案"}</button>{config && <p>以下为上次读取的档案，当前状态尚未确认。</p>}</div>}{open && (config ? <ProfileEditor config={config} state={state} controller={controller} readUnconfirmed={!!error || retrying} /> : !error ? <p>正在读取后端解析结果…</p> : null)}</details>;
}

export function RequestDefaultsForm({ state, controller }: { state: ViewState; controller: DesktopController }) {
  const config = state.snapshot!.configuration!;
  const form = useConfigDraft(config.saved.request_defaults, config.revision, "request_defaults");
  const request = form.draft;
  const valid = Number.isInteger(request.max_output_tokens) && request.max_output_tokens >= 1 && request.max_output_tokens <= 4096 && Number.isFinite(request.temperature) && request.temperature >= 0 && request.temperature <= 2 && Number.isFinite(request.top_p) && request.top_p > 0 && request.top_p <= 1;
  return <section className="settings-card"><h2>请求默认值</h2><p>可在线保存，仅对后续新请求生效；当前模型和已经受理的请求保持原值。调用方显式参数优先。</p>{form.conflict && <DraftConflict reset={form.reset} rebase={form.rebase} draft={form.draft} saved={config.saved.request_defaults} />}<div className="settings-grid">{(["max_output_tokens", "temperature", "top_p"] as const).map((key) => <label className="setting-field" key={key}><span>{{ max_output_tokens: "默认输出预算", temperature: "默认 temperature", top_p: "默认 top_p" }[key]}</span><input type="number" step={key === "max_output_tokens" ? 1 : 0.1} value={Number.isNaN(request[key]) ? "" : request[key]} onChange={(event) => form.setDraft({ ...request, [key]: event.target.valueAsNumber })} /></label>)}</div>{!valid && <p role="alert">请检查输出预算、temperature 与 top_p 范围。</p>}<button disabled={!!state.operation || !form.dirty || form.conflict || !valid || config.migration.state === "required"} onClick={() => void controller.saveConfiguration({ expected_revision: form.baseRevision!, update: { kind: "request_defaults", request_defaults: request } })}>保存请求默认值</button></section>;
}

export function LocalApiConfiguration({ state, controller }: { state: ViewState; controller: DesktopController }) {
  const config = state.snapshot!.configuration!;
  const form = useConfigDraft(config.saved.local_api, config.revision, "local_api");
  const value = form.draft.listen;
  const match = /^(127(?:\.\d{1,3}){3}|\[::1\]):(\d{1,5})$/.exec(value);
  const valid = !!match && Number(match[2]) >= 1 && Number(match[2]) <= 65535 && (match[1] === "[::1]" || match[1].split(".").every((part) => Number(part) <= 255));
  const editable = state.snapshot?.connection === "stopped" && state.snapshot.initialized && !state.operation && config.migration.state !== "required";
  return <section className="settings-card"><h2>本机监听配置</h2><p>只接受回环地址。保存前须显式停止服务，下次启动使用新地址；不会自动中断其他客户端。</p>{form.conflict && <DraftConflict reset={form.reset} rebase={form.rebase} draft={form.draft} saved={config.saved.local_api} />}<label className="setting-field"><span>本机监听地址与端口</span><input value={value} disabled={!editable} onChange={(event) => form.setDraft({ listen: event.target.value })} placeholder="127.0.0.1:18181" /></label><p>已保存：{config.saved.local_api.listen} · 运行中：{config.runtime_effective?.values.local_api.listen ?? "尚未报告"}</p>{!valid && <p role="alert">请输入有效回环 IPv4 或 [::1] 地址及 1–65535 端口。</p>}<button disabled={!editable || !form.dirty || form.conflict || !valid} onClick={() => void controller.saveConfiguration({ expected_revision: form.baseRevision!, update: { kind: "local_api", local_api: form.draft } })}>保存本机监听配置</button></section>;
}
