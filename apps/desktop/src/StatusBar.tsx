import type { DesktopController, ViewState } from "./controller";
import { FollowOnLoadControl, ModelLoadControl } from "./ModelLoadControl";
import { followOnLoadLabel, modelLoadLabel } from "./modelLoad";
import { runtimeView } from "./runtimeView";

function currentOperation(state: ViewState): string {
  if (state.booting) return "正在确认状态";
  if (state.model_load) return modelLoadLabel(state.model_load);
  if (state.operation) return state.operation.label;
  if (state.download_phase !== "idle") {
    if (state.download_phase === "recovery") return "下载结果待确认";
    if (state.download_phase === "stopping") return "等待下载取消确认";
    if (state.download?.phase === "testing") return followOnLoadLabel(state.download);
    return state.download ? ({ connecting: "正在连接下载源", downloading: "正在下载模型", verifying: "正在校验下载", committing: "正在保存文件", registering: "正在登记模型", testing: "正在加载与测试", finished: "正在确认下载结果" })[state.download.phase] : "正在提交下载";
  }
  if (state.library_phase !== "idle") {
    if (state.library_phase === "recovery") return "模型库操作结果待确认";
    if (state.library_phase === "stopping") return "等待模型库取消确认";
    if (state.library?.phase === "testing") return followOnLoadLabel(state.library);
    return state.library_kind === "configure" ? "正在保存下载目录" : state.library_kind === "add" ? "正在添加模型" : "正在扫描模型目录";
  }
  if (state.chat_phase !== "idle") return state.chat_phase === "recovery" ? "生成结果待确认" : state.chat_phase === "stopping" ? "等待生成取消确认" : "正在聊天测试";
  if (state.models_loading) return "正在读取模型列表";
  const view = runtimeView(state.snapshot);
  if (!view.runtime) return view.service === "stopped" ? "无进行中操作" : "运行任务待确认";
  if (view.runtime.stopping) return "等待服务清理确认";
  if (view.runtime.registry_busy) return "模型索引操作中";
  if (["loading", "generating", "unloading"].includes(view.runtime.state)) return view.modelLabel;
  if (view.runtime.active_request || view.runtime.queued_jobs > 0) return "运行请求处理中";
  return "无进行中操作";
}

export function StatusBar({ state, controller, goActivity }: { state: ViewState; controller?: DesktopController; goActivity: () => void }) {
  const view = runtimeView(state.snapshot);
  const operation = currentOperation(state);
  return <footer className="app-statusbar" aria-label="应用状态栏">
    <div className="statusbar-facts" role="status" aria-live="polite" aria-atomic="true">
      <span className="statusbar-service"><span className={`status-dot ${view.service === "connected" ? "online" : ""}`} aria-hidden="true" />{state.booting || state.operation?.kind === "check" ? "服务状态待确认" : state.operation?.kind === "start" ? "服务正在启动" : state.operation?.kind === "stop" ? "服务正在停止" : view.serviceLabel}</span>
      <span className="statusbar-model" title={view.residentName ?? view.modelLabel}>驻留：{view.residentName ?? view.modelLabel}</span>
      <span className="statusbar-operation" title={operation}>{operation}</span>
    </div>
    {controller && <><ModelLoadControl task={state.model_load} controller={controller} /><FollowOnLoadControl state={state} controller={controller} /></>}
    <button className="text-button" onClick={goActivity}>查看活动</button>
  </footer>;
}
