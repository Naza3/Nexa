import type { DesktopController, ViewState } from "./controller";

export function WorkbenchNotice({ state, controller }: { state: ViewState; controller: DesktopController }) {
  const prefs = state.workbench;
  if (!prefs.supported) return null;
  if (!prefs.error) return prefs.hydrated ? null : <p role="status" className="small-note">正在恢复 OCR 设置与聊天输入草稿…</p>;
  return <div className="notice-band warning" role="status" aria-label="工作区设置保存状态"><div><strong>工作区设置尚未保存</strong><p>{prefs.error}</p><div className="workspace-actions">
    <button disabled={prefs.saving || state.closing} onClick={() => void controller.reloadWorkbench()}>读取已保存工作区设置</button>
    {prefs.hydrated && (prefs.conflict ? <button disabled={prefs.saving || state.closing} onClick={() => void controller.overwriteWorkbench()}>保留当前输入并覆盖工作区设置</button> : <button disabled={prefs.saving || state.closing} onClick={() => void controller.saveWorkbench()}>重试保存工作区设置</button>)}
  </div><p className="small-note">这里只保存 OCR 界面选项和未发送的聊天草稿，不会发布模型运行配置。</p></div></div>;
}
