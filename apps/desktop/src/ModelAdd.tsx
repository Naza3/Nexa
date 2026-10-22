import { errorText } from "./errorPresentation";
import { followOnLoadAction, followOnLoadLabel } from "./modelLoad";
import type { ModelFileResult } from "./types";
import type { DesktopController, ViewState } from "./controller";
import { canDismissAddResult } from "./controller";
import { LocalValidationFeedback } from "./ModelTestFeedback";

function size(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 ** 2) return `${(bytes / 1024).toFixed(1)} KiB`;
  if (bytes < 1024 ** 3) return `${(bytes / 1024 ** 2).toFixed(1)} MiB`;
  return `${(bytes / 1024 ** 3).toFixed(2)} GiB`;
}
export function ModelSelectionPanel({ state, controller, onStop }: {
  state: ViewState; controller: DesktopController; onStop: () => void;
}) {
  const selection = state.model_selection;
  if (!selection) return null;
  const busy = !!state.operation || state.library_phase !== "idle" || state.download_phase !== "idle" || state.chat_phase !== "idle";
  const stopped = state.snapshot?.connection === "stopped";
  const running = state.snapshot?.connection === "connected" || state.snapshot?.connection === "connecting";
  const multiple = selection.files.length > 1;
  return <section className="settings-card model-add-selection" aria-labelledby="model-add-title">
    <div className="card-heading"><div><h2 id="model-add-title">添加选中的模型</h2><p>已选择 {selection.files.length} 个 GGUF 文件</p></div><span className="subtle-pill">零复制 · 保留现有索引</span></div>
    <ol className="selected-model-files">{selection.files.map((file) => <li key={file.selection_index}><strong>{file.file_name}</strong><span>{size(file.size_bytes)}</span></li>)}</ol>
    <p>只校验并登记这些文件，不扫描父目录、不复制到 AppData、不移动源文件。已有模型与下载目录保持不变。</p>
    <p className="small-note">选择在 {Math.ceil(selection.expires_in_seconds / 60)} 分钟内有效；若已过期，请重新选择。同名文件按选择序号区分。</p>
    <label className="auto-test-option"><input type="checkbox" checked={state.add_auto_test} disabled={busy || multiple} onChange={(event) => controller.setAddAutoTest(event.target.checked)} />添加后加载并基础测试（仅单个模型）</label>
    <p className="small-note">{multiple ? "批量添加仅登记，不会轮流加载、卸载或测试模型；完成后可逐个显式加载。" : "默认仅登记。勾选后会在空闲时启动本机服务并测试此模型；服务忙碌时暂缓，不切换其他模型。登记成功不代表加载或测试通过。"}</p>
    {!stopped && <div className="directory-stop"><p>添加前须显式停止运行服务，会终止所有客户端任务并卸载模型。不会自动停服；所选文件保留，停止后请再次点击添加。</p><button className="danger-outline" disabled={busy || !running} onClick={onStop}>停止服务以添加</button></div>}
    <div className="directory-actions"><button className="primary" disabled={busy || !stopped} onClick={() => void controller.addModels()}>确认添加 {selection.files.length} 个模型</button><button disabled={busy} onClick={() => void controller.discardModelSelection()}>取消文件选择</button><button disabled={busy} onClick={() => void controller.pickModels()}>重新选择文件</button></div>
  </section>;
}
export function AddModelProgress({ state, controller }: { state: ViewState; controller: DesktopController }) {
  if (state.library_kind !== "add" || (!state.library && state.library_phase === "idle")) return null;
  const operation = state.library;
  const active = state.library_phase !== "idle";
  const phases = { checking: "检查所选文件", enumerating: "检查所选文件", verifying: "校验所选 GGUF", committing: "保存新增索引", testing: `登记已完成 · ${followOnLoadLabel(operation ?? {})}`, finished: "添加操作已结束" };
  const successes = operation?.files?.filter((file) => file.status === "registered" || file.status === "already_registered").length ?? 0;
  const already = operation?.files?.filter((file) => file.status === "already_registered").length ?? 0;
  const durabilityUnconfirmed = operation?.error?.code === "settings_durability_unconfirmed" && !!operation.result;
  const title = durabilityUnconfirmed ? "登记已发布，写入持久性待确认" : state.library_phase === "recovery" ? "添加状态尚未确认" : state.library_phase === "stopping" ? "正在取消，等待实际结果" : active ? operation ? phases[operation.phase] : "正在提交所选文件" : operation?.status === "completed" ? `添加完成：${successes} 个已登记${already ? `（其中 ${already} 个已存在）` : ""}` : operation?.status === "partial" ? `部分添加完成：${successes} 个已登记` : operation?.status === "cancelled" ? "添加已取消" : "添加未完成";
  const files: ModelFileResult[] = operation?.files?.length ? operation.files : state.library_selection?.files.map((file) => ({ ...file, status: "not_processed" as const })) ?? [];
  const labels = { registered: "已登记", already_registered: "已存在 · 保留原登记", rejected: "未登记", not_committed: "尚未确认登记", not_processed: "尚未处理" };
  return <section className="settings-card model-add-progress" aria-label="添加模型结果">
    <div className="card-heading"><div role="status"><h2>{title}</h2>{operation && <p>已检查 {operation.examined_entries} / {operation.candidate_files} 个所选文件 · 校验通过 {operation.verified_files} 个</p>}</div>
      {active && (state.library_phase === "recovery" ? <button onClick={() => void controller.recoverLibrary()}>重新确认添加结果</button> : <button disabled={state.library_phase === "stopping" && !state.error} onClick={() => void controller.cancelLibrary()}>{state.library_phase === "stopping" ? "等待取消确认" : followOnLoadAction(operation, "取消添加")}</button>)}
      {operation && canDismissAddResult(state) && <button onClick={() => controller.dismissAddResult(operation)}>关闭添加结果</button>}
    </div>
    <p>{state.library_phase === "recovery" ? "连接或状态读取中断，尚未确认结果。不会重放添加，请重新确认后再操作。" : "只处理所选文件，零复制，保留现有索引。登记结果与加载、测试结果分别记录。"}</p>
    {operation?.phase === "testing" && <p>加载最多 300 秒，基础短测最多 30 秒。取消或测试失败不撤销已经登记的模型。</p>}
    {durabilityUnconfirmed && <p className="warning-text">已发布的新索引需要刷新核对，不能假定已回滚。请勿重复添加。</p>}
    {operation?.error && <p className="warning-text">{errorText(operation.error)}</p>}
    <ol className="selected-model-files add-file-results">{files.map((file) => <li key={file.selection_index}>
      <div><strong>{file.file_name}</strong><p>{labels[file.status]}{file.error_code ? `（${file.error_code}）` : ""}</p>
        {file.local_validation ? <LocalValidationFeedback value={file.local_validation} /> : (file.status === "registered" || file.status === "already_registered") && <p className="small-note">本次未取得加载或测试通过记录，可在模型列表显式加载与测试。</p>}
      </div>
    </li>)}</ol>
    {!active && <p className="small-note">关闭仅收起本次结果，不影响已登记模型。源文件未移动或删除；已有本机测试历史以模型列表为准。</p>}
  </section>;
}
