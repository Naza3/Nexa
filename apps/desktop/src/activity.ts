import { presentError } from "./errorPresentation";
import { modelLoadLabel } from "./modelLoad";
import type { ViewState, ModelTestAttempt } from "./controller";
import type { DownloadOperation, LibraryOperation, SafeError } from "./types";
export const ACTIVITY_LIMIT = 32;
export interface Activity {
  id: string;
  kind: "model" | "download" | "library" | "chat" | "service" | "configuration";
  label: string;
  status: "running" | "stopping" | "recovery" | "completed" | "cancelled" | "failed" | "partial";
  updated_at: number;
  detail: string;
  error: SafeError | null;
  model?: ModelTestAttempt;
  download?: DownloadOperation;
  library?: LibraryOperation;
  library_kind?: ViewState["library_kind"];
}
export function recordActivity(records: Activity[], record: Activity): Activity[] {
  if (record.error) record = { ...record, error: presentError(record.error) };
  const existing = records.find((item) => item.id === record.id);
  if (existing && JSON.stringify({ ...existing, updated_at: 0 }) === JSON.stringify({ ...record, updated_at: 0 })) return records;
  const updated = [record, ...records.filter((item) => item.id !== record.id)];
  // Keep active work even if terminal history reaches the display limit.
  return updated.filter((item, index) => index < ACTIVITY_LIMIT || ["running", "stopping", "recovery"].includes(item.status));
}
export function projectActivities(previous: Activity[], state: ViewState): Activity[] {
  let records = previous;
  for (const attempt of Object.values(state.model_tests)) {
    const load = state.model_load?.attempt_id === attempt.id ? state.model_load : null;
    const status = attempt.phase === "running" ? load?.phase === "recovery" ? "recovery" : load?.phase === "stopping" ? "stopping" : "running" : attempt.outcome === "cancelled" ? "cancelled" : attempt.result?.error_code === "request_cancelled" ? "cancelled" : attempt.error || ["failed", "unavailable"].includes(attempt.result?.state ?? "") ? "failed" : "completed";
    records = recordActivity(records, { id: `${state.activity_session_id}:model:${attempt.id}`, kind: "model", label: `${attempt.mode === "load" ? "加载并短测" : "基础测试"} · ${attempt.model_id}`, status, updated_at: attempt.finished_at ?? attempt.started_at,
      detail: attempt.phase === "running" ? load ? `${modelLoadLabel(load)}；切换页面不影响任务。` : "此原生阶段暂不支持单独取消；切换页面不影响任务。" : "查看本次结果；历史证明与当前驻留分别判断。", error: attempt.error, model: attempt });
  }
  for (const kind of ["download", "library"] as const) {
    const id = kind === "download" ? state.download_task_id : state.library_task_id;
    const phase = kind === "download" ? state.download_phase : state.library_phase;
    const hasResult = kind === "download" ? state.download : state.library;
    const existing = records.find((item) => item.id === id);
    if (id && !hasResult && phase !== "idle" && (!existing || ["running", "stopping", "recovery"].includes(existing.status))) records = recordActivity(records, {
      id, kind, label: kind === "download" ? "模型下载" : "模型登记或维护", status: phase === "stopping" ? "stopping" : phase === "recovery" ? "recovery" : "running",
      updated_at: Date.now(), detail: "正在提交或等待原生阶段状态；取消须等待真实终态。", error: null,
    });
  }
  const download = state.download;
  if (download) records = recordActivity(records, { id: state.download_task_id ?? `download:${download.operation_id}`, kind: "download", label: `下载 · ${download.file_name}`, status: state.download_phase === "recovery" ? "recovery" : state.download_phase === "stopping" ? "stopping" : download.status,
    updated_at: Date.now(), detail: download.result?.saved ? download.result.registered ? "文件已保存并登记" : "文件已保存，登记未完成；请选择已保存文件添加，不要重复下载" : `阶段：${download.phase}`, error: download.error ?? download.result?.registration_error ?? null, download });
  const library = state.library;
  if (library) records = recordActivity(records, { id: state.library_task_id ?? `library:${library.operation_id}`, kind: "library", label: state.library_kind === "add" ? "添加模型" : state.library_kind === "configure" ? "设置下载目录" : "模型库维护", status: state.library_phase === "recovery" ? "recovery" : state.library_phase === "stopping" ? "stopping" : library.status,
    updated_at: Date.now(), detail: library.result ? `${library.result.registered_files} 个已登记；${library.result.rejected_files ?? 0} 个未登记` : `阶段：${library.phase}`, error: library.error, library, library_kind: state.library_kind });
  const chat = records.find((item) => item.kind === "chat" && !item.id.startsWith("previous:") && ["running", "stopping", "recovery"].includes(item.status));
  if (chat && ["stopping", "recovery"].includes(state.chat_phase)) records = recordActivity(records, { ...chat, status: state.chat_phase as "stopping" | "recovery", updated_at: Date.now() });
  return records;
}

export const ACTIVITY_STORAGE_KEY = "nexa.activity-summary.v1";
export function createActivitySessionId(): string { return `session:${crypto.randomUUID()}`; }
const safeId = (value: unknown): value is string => typeof value === "string" && /^[a-zA-Z0-9_.:-]{1,160}$/.test(value);
const labels = { model: "模型加载或测试", download: "模型下载", library: "模型登记或维护", chat: "辅助聊天测试", service: "服务控制", configuration: "配置保存" };
const storageWarning = "活动摘要未能保存或清除。当前记录仅保留在本窗口，关闭后可能无法恢复；不会自动重放任务。";
/** Only bounded, allowlisted metadata survives reopening. No native result DTO is stored. */
export function persistActivitySummaries(records: Activity[]): string | null {
  const active = records.filter((item) => ["running", "stopping", "recovery"].includes(item.status));
  const inactive = records.filter((item) => !["running", "stopping", "recovery"].includes(item.status));
  const summaries = [...active, ...inactive].slice(0, ACTIVITY_LIMIT).map((item) => ({
    id: item.id, kind: item.kind, status: item.status, updated_at: item.updated_at,
    error_code: item.error?.code && /^[a-z0-9_]{1,96}$/.test(item.error.code) ? item.error.code : null,
  }));
  const value = JSON.stringify(summaries);
  if (summaries.some((item) => !safeId(item.id)) || new TextEncoder().encode(value).length > 32768) return storageWarning;
  try {
    if (summaries.length) localStorage.setItem(ACTIVITY_STORAGE_KEY, value);
    else localStorage.removeItem(ACTIVITY_STORAGE_KEY);
    return null;
  } catch { return storageWarning; }
}
export function readActivityHistory(): { records: Activity[]; warning: string | null } {
  try {
    const text = localStorage.getItem(ACTIVITY_STORAGE_KEY);
    if (!text) return { records: [], warning: null };
    if (new TextEncoder().encode(text).length > 32768) throw new Error("activity_limit");
    const values: unknown = JSON.parse(text);
    if (!Array.isArray(values) || values.length > ACTIVITY_LIMIT) throw new Error("activity_invalid");
    const identities = new Set<string>();
    const records = values.map((item, index): Activity => {
      if (!item || typeof item !== "object" || Object.keys(item).length !== 5 || !["id", "kind", "status", "updated_at", "error_code"].every((key) => Object.hasOwn(item, key)) ||
          !safeId(item.id) || !Object.hasOwn(labels, item.kind) || !["running", "stopping", "recovery", "completed", "cancelled", "failed", "partial"].includes(item.status) || !Number.isSafeInteger(item.updated_at) || item.updated_at < 0 || item.updated_at > 8640000000000000 ||
          (item.error_code !== null && (typeof item.error_code !== "string" || !/^[a-z0-9_]{1,96}$/.test(item.error_code)))) throw new Error("activity_invalid");
      const kind = item.kind as Activity["kind"];
      const interrupted = ["running", "stopping", "recovery"].includes(item.status);
      let id = item.id.startsWith("previous:") ? item.id : `previous:${item.id}:${item.updated_at}`;
      if (!safeId(id) || identities.has(id)) id = `previous:legacy:${item.updated_at}:${index}`;
      while (identities.has(id)) id += ":dup";
      identities.add(id);
      return { id, kind, label: labels[kind], status: interrupted ? "recovery" : item.status,
        updated_at: item.updated_at, detail: interrupted ? "上次窗口未记录终态；请核对当前服务与模型库。不会自动重放，也不把当前闲置当作上次成功。" : "上次窗口的脱敏摘要；详细结果仅保留在原会话。",
        error: item.error_code ? { code: item.error_code, message: "上次操作诊断码" } : null };
    });
    return { records, warning: null };
  } catch { return { records: [], warning: "未能读取或验证上次活动摘要，未采用缓存。当前活动仅能保证在本窗口查看。" }; }
}
export function readActivitySummaries(): Activity[] { return readActivityHistory().records; }
