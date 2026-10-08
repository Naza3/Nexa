import type { PerformanceRecord, PerformanceSnapshot } from "./types";

const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === "object" && !Array.isArray(value);
const integer = (value: unknown): value is number => typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
const id = (value: unknown): value is string => typeof value === "string" && value.length > 0 && value.length <= 256 && !/[\r\n\t]/.test(value);
const numbers = (value: unknown, keys: string[]) => object(value) && keys.every((key) => integer(value[key]));

/** Fail closed on malformed measurements; missing native metrics remain null. */
export function validatePerformance(value: unknown): PerformanceSnapshot {
  if (!object(value) || !id(value.instance_id) || !integer(value.capacity) || value.capacity < 1 || value.capacity > 200 || !Array.isArray(value.records) || value.records.length > value.capacity) throw new Error("invalid_performance");
  let previous = Infinity;
  for (const row of value.records) {
    if (!object(row) || !integer(row.sequence) || row.sequence >= previous || !id(row.request_id) || !id(row.model_id) ||
      !["text", "image"].includes(row.modality as string) || !["completed", "cancelled", "failed"].includes(row.status as string) ||
      !integer(row.accepted_at_unix_ms) || !integer(row.max_output_tokens) ||
      !numbers(row.usage, ["prompt_tokens", "completion_tokens"]) || !numbers(row.timings, ["queue_ms", "load_ms", "execution_ms"]) ||
      !(row.error_code === null || typeof row.error_code === "string" && /^[a-z0-9_]{1,96}$/.test(row.error_code)) ||
      ![null, "stop", "length"].includes(row.finish_reason as null | string)) throw new Error("invalid_performance");
    if (row.performance !== null && (!object(row.performance) ||
      !numbers(row.performance.timings, ["prepare_us", "prefill_us", "decode_us", "output_callback_us"]) ||
      !numbers(row.performance.load_options, ["context_size", "threads", "batch_size"]))) throw new Error("invalid_performance");
    const record = row as unknown as PerformanceRecord;
    const p = record.performance;
    if (record.sequence === 0 || record.max_output_tokens === 0 || record.usage.completion_tokens > record.max_output_tokens ||
      !Number.isSafeInteger(record.timings.queue_ms + record.timings.load_ms + record.timings.execution_ms) ||
      (record.status === "completed" ? record.error_code !== null || record.finish_reason === null : record.performance !== null || record.finish_reason !== null || record.error_code === null) ||
      (p && (!Number.isSafeInteger(p.timings.prepare_us + p.timings.prefill_us + p.timings.decode_us + p.timings.output_callback_us) ||
        record.usage.prompt_tokens === 0 || p.load_options.context_size < 32 || p.load_options.context_size > 131072 ||
        p.load_options.threads < 1 || p.load_options.threads > 256 || p.load_options.batch_size < 1 || p.load_options.batch_size > Math.min(p.load_options.context_size, 4096)))) throw new Error("invalid_performance");
    previous = row.sequence;
  }
  return value as unknown as PerformanceSnapshot;
}
export function tokenRate(row: PerformanceRecord, phase: "prefill" | "decode"): number | null {
  const micros = row.performance?.timings[`${phase}_us`];
  if (row.status !== "completed" || !micros) return null;
  return row.usage[phase === "prefill" ? "prompt_tokens" : "completion_tokens"] / micros * 1e6;
}
export function filterPerformance(records: PerformanceRecord[], model: string, status: string, modality: string) {
  return records.filter((row) => (!model || row.model_id === model) && (!status || row.status === status) && (!modality || row.modality === modality));
}
export const performanceStatus = (row: PerformanceRecord) => row.status === "completed" ? row.finish_reason === "length" ? "成功 · 输出达上限" : "成功" : row.status === "failed" ? "失败" : "已取消";
export const seconds = (us: number | undefined) => us === undefined ? "不可用" : `${(us / 1e6).toFixed(us > 0 && us < 1000 ? 6 : 3)} s`;
export const rateText = (value: number | null) => value === null ? "不可用" : `${value.toFixed(2)} token/s`;

/** Quote every cell and neutralize spreadsheet formulas in identifiers. */
export function performanceCsv(snapshot: PerformanceSnapshot, records: PerformanceRecord[]): string {
  const header = ["instance_id", "sequence", "request_id", "model_id", "modality", "status", "finish_reason", "error_code", "accepted_at_unix_ms", "max_output_tokens", "prompt_tokens", "completion_tokens", "queue_ms", "load_ms", "execution_ms", "prepare_us", "prefill_us", "decode_us", "output_callback_us", "prefill_tokens_per_second", "decode_tokens_per_second", "context_size", "threads", "batch_size"];
  const rows = records.map((r) => [snapshot.instance_id, r.sequence, r.request_id, r.model_id, r.modality, r.status, r.finish_reason, r.error_code, r.accepted_at_unix_ms, r.max_output_tokens, r.usage.prompt_tokens, r.usage.completion_tokens, r.timings.queue_ms, r.timings.load_ms, r.timings.execution_ms, r.performance?.timings.prepare_us, r.performance?.timings.prefill_us, r.performance?.timings.decode_us, r.performance?.timings.output_callback_us, tokenRate(r, "prefill"), tokenRate(r, "decode"), r.performance?.load_options.context_size, r.performance?.load_options.threads, r.performance?.load_options.batch_size]);
  return [header, ...rows].map((row) => row.map((v) => {
    const text = v == null ? "" : String(v);
    return `"${(/^[=+@-]/.test(text) ? "'" : "") + text.replaceAll('"', '""')}"`;
  }).join(",")).join("\r\n");
}
