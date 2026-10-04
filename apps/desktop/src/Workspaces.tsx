import { useState } from "react";
import type { DesktopController, ViewState } from "./controller";
import { runtimeView, localBaseUrl } from "./runtimeView";
import { ModelTestFeedback } from "./ModelTestFeedback";
import { LanApiSettings } from "./LanApiSettings";
import { LocalApiConfiguration } from "./Configuration";
import { Modal } from "./Modal";

export function OverviewPage({ state, controller, goModels, goApi, goActivity, goChat }: {
  state: ViewState; controller: DesktopController; goModels: () => void; goApi: () => void; goActivity: () => void; goChat: () => void;
}) {
  const view = runtimeView(state.snapshot);
  const active = state.activities.filter((item) => ["running", "stopping", "recovery"].includes(item.status));
  const settings = state.snapshot?.configuration?.saved.runtime ?? state.snapshot?.settings;
  return <>
    <div className="page-heading"><div><span className="eyebrow">本地模型服务</span><h1>概览</h1><p>查看真实运行状态，准备模型并接入你的应用。</p></div><button onClick={goApi}>查看 API 接入</button></div>
    <section className="overview-grid" aria-label="当前运行概况">
      <article className="settings-card"><span className="overline">运行服务</span><h2>{view.serviceLabel}</h2><p>{view.readiness}</p>{state.operation && <p>本窗口操作：{state.operation.label}</p>}<p>本机监听：{view.localListening ? "已连接" : "未确认"} · 局域网监听：{view.lanListening ? "已报告运行" : "未确认运行"}</p></article>
      <article className="settings-card"><span className="overline">驻留模型</span><h2>{view.residentName ?? view.modelLabel}</h2><p>{view.modelLabel}</p>{view.residentOptions && <p>上下文 {view.residentOptions.context_size} · {view.residentOptions.threads} 线程 · 批次 {view.residentOptions.batch_size}</p>}{!view.resident && view.lastSelectedName && <p>上次选择：{view.lastSelectedName}（不代表当前占用内存）</p>}<button className="primary" onClick={goModels}>{state.models.data.length ? "选择或管理模型" : "添加第一个模型"}</button></article>
    </section>
    <section className="settings-card"><div className="card-heading"><div><h2>接入下一步</h2><p>{view.resident ? "将 Base URL、独立凭据和模型 ID 填入受信任的客户端。" : "在模型库显式加载；加载会进行安全短文本测试，不会自动切换正在执行的请求。"}</p></div></div>
      {view.residentId && <div className="api-row"><span>当前模型 ID</span><output>{view.residentId}</output><button onClick={() => void controller.copyModelId(view.residentId!)}>复制模型 ID</button></div>}
      <p>实际空闲剩余时间：未报告；配置等待时间不是当前剩余时间。</p>
      <p>已保存空闲策略：{settings?.idle_unload_enabled === false ? "不自动卸载；驻留会继续占用内存" : `空闲 ${settings?.idle_unload_seconds ?? "未知"} 秒后卸载模型`}。服务在线不代表模型一直驻留；空 ID 与局域网请求需要驻留模型。</p>
      <div className="workspace-actions"><button onClick={goApi}>查看客户端连接信息</button><button onClick={goChat}>聊天测试</button></div>
    </section>
    <p className="small-note">实际空闲剩余时间：未报告。这里显示已保存策略，不根据页面打开时间推算倒计时。</p>
    <section className="settings-card"><div className="card-heading"><div><h2>当前任务</h2><p>{active.length ? `${active.length} 项操作进行中，切换页面不影响已确认操作。` : "当前没有本窗口后台任务"}</p></div><button onClick={goActivity}>查看活动</button></div>{active.map((item) => <p key={item.id}>{item.label} · {item.detail}</p>)}</section>
  </>;
}

export function ApiPage({ state, controller, goChat }: { state: ViewState; controller: DesktopController; goChat: () => void }) {
  const [copy, setCopy] = useState(false);
  const view = runtimeView(state.snapshot);
  const base = localBaseUrl(state.snapshot?.api_address);
  return <>
    <div className="page-heading"><div><span className="eyebrow">连接你的应用</span><h1>API 接入</h1><p>凭据由原生端复制，不会在页面或活动记录中显示。</p></div><button onClick={goChat}>聊天测试</button></div>
    <section className="settings-card" aria-label="本机 API 连接信息"><h2>本机 API</h2><p>{view.readiness}</p><div className="api-row"><span>客户端 Base URL</span><output aria-label="本机客户端 Base URL">{base ?? "尚未确认地址"}</output><button disabled={!base} onClick={() => void controller.copyLocalBaseUrl()}>复制本机 Base URL</button></div>
      <div className="api-row"><span>驻留模型 ID</span><output>{view.residentId ?? "当前无已确认的驻留模型"}</output><button disabled={!view.residentId} onClick={() => void controller.copyModelId(view.residentId!)}>复制模型 ID</button></div>
      <div className="token-row"><p>本机管理令牌仅交给可信的本机客户端。复制将写入系统剪贴板，请使用后清除。</p><button disabled={!state.snapshot?.initialized || !!state.operation} onClick={() => setCopy(true)}>复制 API 令牌</button></div>
      <p className="small-note">Base URL 包含 /v1。{view.localListening ? "当前本机连接已由原生管理端验证" : "本机连接尚未确认在线"}；其他客户端仍需提供正确凭据，请求也可能因队列或参数被拒绝。队列容量当前未报告，不能据此保证每次受理。</p>
      <p>空或缺省 model 在准入时绑定当时驻留的模型，不会自动加载。显式 ID 不会默默切换到另一个模型；本机允许的首次加载或同模型重载由服务判断。</p>
      <p>当前文本接口不提供工具执行或完整思考协议。客户端的能力要求须另行验证。</p>
      <p>已保存空闲策略：{(state.snapshot?.configuration?.saved.runtime.idle_unload_enabled ?? state.snapshot?.settings.idle_unload_enabled) === false ? "不自动卸载" : `空闲 ${state.snapshot?.configuration?.saved.runtime.idle_unload_seconds ?? state.snapshot?.settings.idle_unload_seconds ?? "未知"} 秒后卸载`}；配置已保存与运行实例实际采用的策略请分别核对。</p>
    </section>
    {state.snapshot?.configuration && <LocalApiConfiguration state={state} controller={controller} />}
    {state.snapshot?.configuration?.runtime_effective && <p>运行中空闲策略：{state.snapshot.configuration.runtime_effective.values.runtime.idle_unload_enabled ? `空闲 ${state.snapshot.configuration.runtime_effective.values.runtime.idle_unload_seconds} 秒后卸载` : "不自动卸载"}</p>}
    <LanApiSettings state={state} controller={controller} />
    {copy && <Modal title="将令牌复制到系统剪贴板？" confirm="确认复制" onCancel={() => setCopy(false)} onConfirm={() => { setCopy(false); void controller.copyToken(); }}><p>其他应用或剪贴板历史可能读取这份凭据。请仅粘贴到你信任的本机客户端，使用后及时清除。</p></Modal>}
  </>;
}

const statuses = { running: "进行中", stopping: "正在取消，等待终态", recovery: "结果待确认", completed: "已完成", cancelled: "已取消", failed: "未完成", partial: "部分完成" };
export function ActivityPage({ state, controller, goChat }: { state: ViewState; controller: DesktopController; goChat: () => void }) {
  const view = runtimeView(state.snapshot);
  const [clear, setClear] = useState(false);
  const clearable = state.activities.some((item) => item.id.startsWith("previous:") || !["running", "stopping", "recovery"].includes(item.status));
  return <><div className="page-heading"><div><span className="eyebrow">操作与诊断</span><h1>活动</h1><p>保留最近 32 项脱敏摘要；详细结果仅在本窗口保留。重开后核对当前服务，不自动重放上次操作；存储失败会明确提示。</p></div><button disabled={!clearable} onClick={() => setClear(true)}>清除活动历史</button></div>
    {clear && <Modal title="清除活动历史？" confirm="确认清除活动历史" danger onCancel={() => setClear(false)} onConfirm={() => { setClear(false); controller.clearActivityHistory(); }}><p>将清除已完成记录与上次窗口摘要。当前窗口尚在进行或等待终态的任务继续保留，不会取消请求或删除模型。</p></Modal>}
    <section className="settings-card"><h2>运行请求</h2><p>活动请求：{view.runtime?.active_request ?? (view.runtime ? "无" : "未知")} · 排队：{view.runtime?.queued_jobs ?? "未知"}</p><p>这里只观察其他客户端，不获取其正文或取消其请求。</p></section>
    {!state.activities.length && <section className="settings-card"><h2>尚无本窗口活动</h2><p>模型加载、添加、下载和聊天测试结果会出现在这里。</p></section>}
    {state.activities.map((item) => <section key={item.id} className="settings-card activity-card" aria-label={item.label}><div className="card-heading"><h2>{item.label}</h2><span className="subtle-pill">{statuses[item.status]}</span></div><p>{item.detail}</p><p className="small-note">任务 {item.id} · <time dateTime={new Date(item.updated_at).toISOString()}>{new Date(item.updated_at).toLocaleTimeString()}</time></p>
      {item.error && <p className="warning-text">{item.error.message}（{item.error.code}）</p>}{item.model && <ModelTestFeedback attempt={item.model} />}
      {item.kind === "download" && state.download_task_id === item.id && state.download_phase !== "idle" && <button disabled={state.download_phase === "stopping"} onClick={() => void (state.download_phase === "recovery" ? controller.recoverDownload() : controller.cancelDownload())}>{state.download_phase === "recovery" ? "重新确认下载状态" : "取消下载后续步骤"}</button>}
      {item.kind === "library" && state.library_task_id === item.id && state.library_phase !== "idle" && <button disabled={state.library_phase === "stopping"} onClick={() => void (state.library_phase === "recovery" ? controller.recoverLibrary() : controller.cancelLibrary())}>{state.library_phase === "recovery" ? "重新确认模型库操作" : "取消模型库操作"}</button>}
      {item.kind === "chat" && !item.id.startsWith("previous:") && ["running", "stopping", "recovery"].includes(item.status) && state.chat_phase !== "idle" && <button disabled={state.chat_phase === "stopping"} onClick={() => void (state.chat_phase === "recovery" ? controller.recover() : controller.cancel())}>{state.chat_phase === "recovery" ? "重新确认生成终态" : "取消本窗口生成"}</button>}
      {item.status === "recovery" && (item.id.startsWith("previous:") || ["service", "configuration", "model"].includes(item.kind)) && <button onClick={() => void controller.checkService()}>核对当前服务状态</button>}
      {item.kind === "chat" && <button onClick={goChat}>查看辅助测试会话</button>}
      {(item.download || item.library) && !["running", "stopping", "recovery"].includes(item.status) && <button disabled={state.download_phase !== "idle" || state.library_phase !== "idle"} onClick={() => controller.restoreActivity(item.id)}>查看阶段结果</button>}
    </section>)}
  </>;
}
