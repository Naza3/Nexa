import type { ModelLoadTaskView } from "./controller";
import type { ModelLoadOperation, RuntimeStatus, SafeError } from "./types";
import { validLocalValidation } from "./localValidation";

function validError(value: SafeError | null): boolean {
  return value === null || (!!value && typeof value === "object" && typeof value.code === "string" &&
    /^[a-z0-9_]{1,96}$/.test(value.code) && typeof value.message === "string" && value.message.length <= 500);
}
function validRuntime(value: RuntimeStatus | null): boolean {
  return value === null || (!!value && typeof value === "object" &&
    ["unloaded", "loading", "ready", "generating", "unloading", "faulted"].includes(value.state) &&
    (value.selected_model === null || typeof value.selected_model === "string") &&
    (value.selected_model_display_name === null || typeof value.selected_model_display_name === "string") &&
    (value.active_request === null || typeof value.active_request === "string") &&
    typeof value.stopping === "boolean" && typeof value.registry_busy === "boolean" &&
    Number.isSafeInteger(value.queued_jobs) && value.queued_jobs >= 0 &&
    (value.load_options === null || (!!value.load_options &&
      [value.load_options.context_size, value.load_options.threads, value.load_options.batch_size].every((field) => Number.isSafeInteger(field) && field > 0))));
}
/** Never release ownership from a foreign, contradictory, or incomplete receipt. */
export function validModelLoadOperation(value: ModelLoadOperation, operationId: string, modelId: string): boolean {
  if (!value || typeof value !== "object" || value.operation_id !== operationId || value.model_id !== modelId ||
      !["preparing", "loading", "testing", "finished"].includes(value.phase) ||
      !["running", "cancelling", "completed", "cancelled", "failed"].includes(value.status) ||
      !validRuntime(value.runtime) || !validError(value.error) ||
      (value.local_validation !== null && !validLocalValidation(value.local_validation))) return false;
  const terminal = ["completed", "cancelled", "failed"].includes(value.status);
  if (value.terminal !== terminal || (value.phase === "finished") !== terminal) return false;
  if (value.status === "failed" && !value.error) return false;
  if (value.status === "completed" && value.error) return false;
  if (value.status === "cancelled" && (value.local_validation?.generation_pass || (value.error && value.error.code !== "request_cancelled"))) return false;
  return true;
}
export function modelLoadLabel(task: ModelLoadTaskView): string {
  if (task.progress?.terminal) return "正在核对加载结果";
  if (task.phase === "recovery") return "加载任务结果待确认";
  if (task.phase === "stopping") return "停止中 · 等待任务清理确认";
  if (task.phase === "starting") return "正在准备加载任务";
  return task.progress?.phase === "testing" ? "加载完成 · 正在基础测试"
    : task.progress?.phase === "loading" ? "正在加载模型" : "正在准备并校验模型文件";
}

/** Nested add/download tasks expose their own phase, never another client's runtime. */
export function validOptionalLoadPhase(value: unknown): boolean {
  return value == null || ["preparing", "loading", "testing"].includes(value as string);
}
export function followOnLoadAction(task: { phase: string; load_phase?: string | null } | null, fallback: string): string {
  if (task?.phase !== "testing") return fallback;
  return task.load_phase === "testing" ? "停止本次测试" : task.load_phase ? "停止加载" : "停止加载与测试";
}
export function followOnLoadLabel(task: { load_phase?: string | null }): string {
  return task.load_phase === "preparing" ? "正在准备并校验模型文件" : task.load_phase === "loading" ? "正在加载模型"
    : task.load_phase === "testing" ? "加载完成 · 正在基础测试" : "正在加载与测试";
}
