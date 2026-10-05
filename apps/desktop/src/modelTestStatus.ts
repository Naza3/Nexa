import type { ModelTestAttempt } from "./controller";
import type { LocalValidation } from "./types";
import { validLocalValidation } from "./localValidation";

export type TestTone = "passed" | "failed" | "neutral";
export interface TestStatus { tone: TestTone; label: string; running: boolean }

/** Test evidence is independent of residency, selection and file availability. */
export function modelTestStatus(value?: LocalValidation | null, attempt?: ModelTestAttempt): TestStatus {
  const status = (label: string, tone: TestTone = "neutral", running = false): TestStatus => ({ label, tone, running });
  if (attempt?.phase === "running") return status(attempt.mode === "load" ? "加载与测试中" : "测试中", "neutral", true);
  if (attempt?.outcome === "cancelled") {
    if (value && validLocalValidation(value) && value.checked_at_unix_ms !== null && value.checked_at_unix_ms > (attempt.finished_at ?? attempt.started_at)) return modelTestStatus(value);
    return status("本次操作已停止");
  }
  // A finished attempt is historical. A subsequent native reread can invalidate
  // that evidence without changing the model's identity or original test time.
  if (attempt && value) {
    if (!validLocalValidation(value) || ["stale", "unavailable"].includes(value.state) || value.error_code?.startsWith("validation_")) {
      return modelTestStatus(value);
    }
    const attemptedAt = attempt.result?.checked_at_unix_ms ?? attempt.finished_at ?? attempt.started_at;
    if (value.checked_at_unix_ms !== null && value.checked_at_unix_ms > attemptedAt) return modelTestStatus(value);
  }
  if (attempt) {
    const code = attempt.error?.code ?? attempt.result?.error_code;
    if (code === "request_cancelled") return status("测试已取消");
    if (code === "runtime_busy" || attempt.result?.state === "deferred") return status("测试已暂缓");
    if (code?.startsWith("validation_") || code === "invalid_local_validation") return status("测试结果待确认");
    if (attempt.error) {
      return ["model_load_failed", "load_timeout", "context_too_large", "context_length_exceeded", "unsupported_model", "worker_failed", "deadline_exceeded"].includes(code ?? "")
        ? status("加载或测试失败", "failed") : status("测试结果待确认");
    }
    value = attempt.result;
  }
  if (!value) return status(attempt ? "测试结果待确认" : "未测试");
  if (!validLocalValidation(value)) return status("测试记录不可用");
  if (value.error_code?.startsWith("validation_")) return status("测试记录待确认");
  if (value.error_code === "request_cancelled") return status("测试已取消");
  switch (value.state) {
    case "passed": return status("基础测试通过", "passed");
    case "failed": return status("基础测试失败", "failed");
    case "loaded": return status("已加载 · 待短测");
    case "stale": return status("记录过期 · 待重测");
    case "deferred": return status("测试已暂缓");
    case "unavailable": return status("测试记录不可用");
    default: return status("未测试");
  }
}
