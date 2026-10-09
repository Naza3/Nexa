import { followOnLoadAction } from "./modelLoad";
import { ModelLoadControl } from "./ModelLoadControl";
import { useState } from "react";
import type { DesktopController, ViewState } from "./controller";
import { runtimeView, localBaseUrl } from "./runtimeView";
import { ModelTestFeedback } from "./ModelTestFeedback";
import { LanApiSettings } from "./LanApiSettings";
import { LocalApiConfiguration } from "./Configuration";
import { Modal } from "./Modal";
import { DetailsGroup } from "./DetailsGroup";

export function OverviewPage({ state, controller, goModels, goApi, goActivity, goChat }: {
  state: ViewState; controller: DesktopController; goModels: () => void; goApi: () => void; goActivity: () => void; goChat: () => void;
}) {
  const view = runtimeView(state.snapshot);
  const active = state.activities.filter((item) => ["running", "stopping", "recovery"].includes(item.status));
  const settings = state.snapshot?.configuration?.saved.runtime ?? state.snapshot?.settings;
  const hasModels = state.models.data.length > 0;
  const transitioning = !!view.runtime && ["loading", "unloading"].includes(view.runtime.state);
  const unknown = !state.snapshot || ["error", "connecting"].includes(state.snapshot.connection);
  const faulted = view.runtime?.state === "faulted";
  return <>
    <div className="page-heading"><div><span className="eyebrow">本地模型服务</span><h1>概览</h1></div></div>
    <section className="overview-hero" aria-label="当前运行概况">
      <span className="overline">{view.resident ? "当前驻留" : "下一步"}</span>
      <h2>{view.residentName ?? (transitioning || faulted || unknown ? view.modelLabel : hasModels ? "选择要使用的模型" : "添加你的第一个模型")}</h2>
      <p>{view.resident ? view.readiness : transitioning ? "操作仍在进行，结果确认后再继续。" : unknown ? "先核对服务状态，再继续使用模型。" : faulted ? "请查看诊断，并在空闲时显式重新加载。" : hasModels ? "从模型库加载模型，准备好后即可接入应用。" : "选择本地 GGUF 文件，或下载一个模型。"}</p>
      <div className="workspace-actions"><button className="primary" onClick={unknown ? () => void controller.checkService() : transitioning ? goActivity : view.resident ? goApi : goModels} disabled={unknown && (!!state.operation || state.booting)}>{unknown ? "核对服务状态" : transitioning ? "查看当前任务" : view.resident ? "接入你的应用" : hasModels ? "前往模型库" : "添加或下载模型"}</button><button onClick={view.resident ? goChat : goApi}>{view.resident ? "聊天测试" : "查看 API 接入"}</button>{view.resident && <button onClick={goModels}>管理模型</button>}</div>
    </section>
    <DetailsGroup title="运行详情" description={view.modelLabel}>
      <section className="settings-card"><h2>{view.serviceLabel}</h2><p>{view.readiness}</p>
        <p>本机监听：{view.localListening ? "已连接" : "未确认"} · 局域网监听：{view.lanListening ? "已报告运行" : "未确认运行"}</p>
        {view.residentOptions && <p>上下文 {view.residentOptions.context_size} · {view.residentOptions.threads} 线程 · 批次 {view.residentOptions.batch_size}</p>}
        {!view.resident && view.lastSelectedName && <p>上次选择：{view.lastSelectedName}（不代表当前占用内存）</p>}
        {view.residentId && <div className="api-row"><span>当前模型 ID</span><output>{view.residentId}</output><button onClick={() => void controller.copyModelId(view.residentId!)}>复制模型 ID</button></div>}
        <p>实际空闲剩余时间：未报告；配置等待时间不是当前剩余时间。</p>
        <p>已保存空闲策略：{settings?.idle_unload_enabled === false ? "不自动卸载；驻留会继续占用内存" : `空闲 ${settings?.idle_unload_seconds ?? "未知"} 秒后卸载模型`}。服务在线不代表模型一直驻留；空 ID 与局域网请求需要驻留模型。</p>
      </section>
    </DetailsGroup>
    {active.length > 0 && <button className="overview-activity" onClick={goActivity}>{active.length} 项任务进行中 · 查看活动</button>}
  </>;
}

export function ApiPage({ state, controller, goChat }: { state: ViewState; controller: DesktopController; goChat: () => void }) {
  const [copy, setCopy] = useState(false);
  const view = runtimeView(state.snapshot);
  const base = localBaseUrl(state.snapshot?.api_address);
  return <>
    <div className="page-heading"><div><span className="eyebrow">连接你的应用</span><h1>API 接入</h1></div><button onClick={goChat}>聊天测试</button></div>
    <section className="settings-card" aria-label="本机 API 连接信息"><h2>本机 API</h2><p>{view.readiness}</p><div className="api-row"><span>客户端 Base URL</span><output aria-label="本机客户端 Base URL">{base ?? "尚未确认地址"}</output><button disabled={!base} onClick={() => void controller.copyLocalBaseUrl()}>复制本机 Base URL</button></div>
      <div className="api-row"><span>驻留模型 ID</span><output>{view.residentId ?? "当前无已确认的驻留模型"}</output><button disabled={!view.residentId} onClick={() => void controller.copyModelId(view.residentId!)}>复制模型 ID</button></div>
      <div className="token-row"><p>API 凭据 · 由原生端安全复制</p><button disabled={!state.snapshot?.initialized || !!state.operation} onClick={() => setCopy(true)}>复制 API 令牌</button></div>
      <DetailsGroup title="调用规则与运行策略"><p className="small-note">Base URL 包含 /v1。{view.localListening ? "当前本机连接已由原生管理端验证" : "本机连接尚未确认在线"}；其他客户端仍需提供正确凭据，请求也可能因队列或参数被拒绝。队列容量当前未报告，不能据此保证每次受理。</p>
      <p>空或缺省 model 在准入时绑定当时驻留的模型，不会自动加载。显式 ID 不会默默切换到另一个模型；本机允许的首次加载或同模型重载由服务判断。</p>
      <p>当前文本接口不提供工具执行或完整思考协议。客户端的能力要求须另行验证。</p>
      <p>已保存空闲策略：{(state.snapshot?.configuration?.saved.runtime.idle_unload_enabled ?? state.snapshot?.settings.idle_unload_enabled) === false ? "不自动卸载" : `空闲 ${state.snapshot?.configuration?.saved.runtime.idle_unload_seconds ?? state.snapshot?.settings.idle_unload_seconds ?? "未知"} 秒后卸载`}；配置已保存与运行实例实际采用的策略请分别核对。</p></DetailsGroup>
    </section>
    {state.snapshot?.configuration && <DetailsGroup title="本机监听设置" description="地址与端口" status={state.snapshot.configuration.pending_restart ? "待重启" : state.snapshot.connection === "stopped" ? undefined : "修改前需停服"} draftKeys={["local_api"]}><LocalApiConfiguration state={state} controller={controller} /></DetailsGroup>}
    {state.snapshot?.configuration?.runtime_effective && <p>运行中空闲策略：{state.snapshot.configuration.runtime_effective.values.runtime.idle_unload_enabled ? `空闲 ${state.snapshot.configuration.runtime_effective.values.runtime.idle_unload_seconds} 秒后卸载` : "不自动卸载"}</p>}
    <DetailsGroup title="局域网接入" description={view.lanDegraded ? "启动失败 · 本机服务可用" : view.lanListening ? "运行中 · 仅受信任的设备" : "按需开启 · 仅受信任的设备"} status={state.snapshot?.configuration?.pending_restart ? "已保存配置待重启" : undefined} draftKeys={["lan_api"]}><LanApiSettings state={state} controller={controller} /></DetailsGroup>
    {copy && <Modal title="将令牌复制到系统剪贴板？" confirm="确认复制" onCancel={() => setCopy(false)} onConfirm={() => { setCopy(false); void controller.copyToken(); }}><p>其他应用或剪贴板历史可能读取这份凭据。请仅粘贴到你信任的本机客户端，使用后及时清除。</p></Modal>}
  </>;
}

const statuses = { running: "进行中", stopping: "正在取消，等待终态", recovery: "结果待确认", completed: "已完成", cancelled: "已取消", failed: "未完成", partial: "部分完成" };
export function ActivityPage({ state, controller, goChat }: { state: ViewState; controller: DesktopController; goChat: () => void }) {
  const view = runtimeView(state.snapshot);
  const [clear, setClear] = useState(false);
  const clearable = state.activities.some((item) => item.id.startsWith("previous:") || !["running", "stopping", "recovery"].includes(item.status));
  return <><div className="page-heading"><div><span className="eyebrow">操作与诊断</span><h1>活动</h1><p>最近 32 项操作，展开查看结果与诊断。</p></div><button disabled={!clearable} onClick={() => setClear(true)}>清除活动历史</button></div>
    {clear && <Modal title="清除活动历史？" confirm="确认清除活动历史" danger onCancel={() => setClear(false)} onConfirm={() => { setClear(false); controller.clearActivityHistory(); }}><p>将清除已完成记录与上次窗口摘要。当前窗口尚在进行或等待终态的任务继续保留，不会取消请求或删除模型。</p></Modal>}
    <DetailsGroup title="运行请求"><section className="settings-card"><h2>运行请求</h2><p>活动请求：{view.runtime?.active_request ?? (view.runtime ? "无" : "未知")} · 排队：{view.runtime?.queued_jobs ?? "未知"}</p><p>这里只观察其他客户端，不获取其正文或取消其请求。</p></section></DetailsGroup>
    {!state.activities.length && <section className="settings-card"><h2>尚无本窗口活动</h2><p>模型加载、添加、下载和聊天测试结果会出现在这里。</p></section>}
    {state.activities.map((item) => <details key={item.id} className="details-group activity-card" aria-label={item.label}><summary><strong>{item.label}</strong><span>{statuses[item.status]}</span></summary><div className="details-group-content"><p>{item.detail}</p><p className="small-note">任务 {item.id} · <time dateTime={new Date(item.updated_at).toISOString()}>{new Date(item.updated_at).toLocaleTimeString()}</time></p>
      {item.error && <p className="warning-text">{item.error.message}（{item.error.code}）</p>}{item.model && <ModelTestFeedback attempt={item.model} loadTask={state.model_load?.attempt_id === item.model.id ? state.model_load : null} />}
      {item.model && state.model_load?.attempt_id === item.model.id && <ModelLoadControl task={state.model_load} controller={controller} />}
      {item.kind === "download" && state.download_task_id === item.id && state.download_phase !== "idle" && <button disabled={state.download_phase === "stopping"} onClick={() => void (state.download_phase === "recovery" ? controller.recoverDownload() : controller.cancelDownload())}>{state.download_phase === "recovery" ? "重新确认下载状态" : followOnLoadAction(state.download, "取消下载后续步骤")}</button>}
      {item.kind === "library" && state.library_task_id === item.id && state.library_phase !== "idle" && <button disabled={state.library_phase === "stopping"} onClick={() => void (state.library_phase === "recovery" ? controller.recoverLibrary() : controller.cancelLibrary())}>{state.library_phase === "recovery" ? "重新确认模型库操作" : followOnLoadAction(state.library, "取消模型库操作")}</button>}
      {item.kind === "chat" && !item.id.startsWith("previous:") && ["running", "stopping", "recovery"].includes(item.status) && state.chat_phase !== "idle" && <button disabled={state.chat_phase === "stopping"} onClick={() => void (state.chat_phase === "recovery" ? controller.recover() : controller.cancel())}>{state.chat_phase === "recovery" ? "重新确认生成终态" : "取消本窗口生成"}</button>}
      {item.status === "recovery" && (item.id.startsWith("previous:") || ["service", "configuration", "model"].includes(item.kind)) && <button onClick={() => void controller.checkService()}>核对当前服务状态</button>}
      {item.kind === "chat" && <button onClick={goChat}>查看辅助测试会话</button>}
      {(item.download || item.library) && !["running", "stopping", "recovery"].includes(item.status) && <button disabled={state.download_phase !== "idle" || state.library_phase !== "idle"} onClick={() => controller.restoreActivity(item.id)}>查看阶段结果</button>}
    </div></details>)}
  </>;
}
