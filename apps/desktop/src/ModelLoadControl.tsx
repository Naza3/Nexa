import { followOnLoadAction } from "./modelLoad";
import type { DesktopController, ModelLoadTaskView, ViewState } from "./controller";

/** Same owned task stays cancellable from the footer, model detail, and activity. */
export function ModelLoadControl({ task, controller }: { task: ModelLoadTaskView | null; controller: DesktopController }) {
  if (!task || task.progress?.terminal) return null;
  const recovery = task.phase === "recovery";
  const stopping = task.phase === "stopping" || (task.stop_requested && !task.cancel_error);
  const testing = task.progress?.phase === "testing";
  return <>
    <button type="button" className="model-load-stop" disabled={stopping}
      onClick={() => void controller.cancelModelLoad()}>
      {stopping ? "停止中…" : task.cancel_error ? "重试停止" : testing ? "停止本次测试" : "停止加载"}
    </button>
    {recovery && <button type="button" onClick={controller.recoverModelLoad}>重新确认加载任务</button>}
  </>;
}

export function FollowOnLoadControl({ state, controller }: { state: ViewState; controller: DesktopController }) {
  const kind = state.download_phase !== "idle" && state.download?.phase === "testing" ? "download"
    : state.library_phase !== "idle" && state.library?.phase === "testing" ? "library" : null;
  if (!kind) return null;
  const phase = kind === "download" ? state.download_phase : state.library_phase;
  const task = kind === "download" ? state.download : state.library;
  const recover = () => kind === "download" ? controller.recoverDownload() : controller.recoverLibrary();
  const cancel = () => kind === "download" ? controller.cancelDownload() : controller.cancelLibrary();
  return <button type="button" className="model-load-stop" disabled={phase === "stopping" && !state.error}
    onClick={() => void (phase === "recovery" ? recover() : cancel())}>
    {phase === "recovery" ? "重新确认加载任务" : phase === "stopping" ? state.error ? "重试停止" : "停止中…" : followOnLoadAction(task, "停止加载与测试")}
  </button>;
}
