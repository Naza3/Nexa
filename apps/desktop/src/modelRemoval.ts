import type { ViewState } from "./controller";
import type { SafeError } from "./types";
import { runtimeView } from "./runtimeView";

/** UI admission is advisory; the native actor rechecks ownership atomically. */
export function modelRemovalBlocker(state: ViewState, modelId: string): SafeError | null {
  if (state.booting || !state.snapshot || !["connected", "stopped"].includes(state.snapshot.connection))
    return { code: "runtime_unavailable", message: "请先重新检查服务状态。" };
  if (["stale", "unsupported"].includes(state.snapshot.model_directory.state))
    return { code: "model_directory_mismatch", message: "请先核对当前模型目录与服务状态。" };
  const view = runtimeView(state.snapshot);
  if (state.operation || state.library_phase !== "idle" || state.download_phase !== "idle" || state.chat_phase !== "idle" || state.testing_model || state.model_load || view.busy)
    return { code: "runtime_busy", message: "当前有任务进行中，请等待结束后移除。" };
  if (state.snapshot.connection === "connected" && !view.runtime)
    return { code: "runtime_unavailable", message: "请先重新检查服务状态。" };
  if (view.runtime?.state === "faulted")
    return { code: "runtime_faulted", message: "服务当前故障，请通过左侧服务按钮显式停止服务，再移除模型。" };
  if (view.runtime?.selected_model === modelId && view.runtime.state !== "unloaded")
    return { code: "model_unregister_loaded", message: "请先在模型详情中卸载，再从模型库移除。" };
  if (!state.models.data.some((model) => model.id === modelId))
    return { code: "model_not_found", message: "此模型已不在当前列表，请刷新后核对。" };
  if (!state.models.generation)
    return { code: "model_list_changed", message: "请先刷新模型列表，再确认移除。" };
  return null;
}
