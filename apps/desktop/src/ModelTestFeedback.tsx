import { useEffect, useState } from "react";
import type { ModelTestAttempt } from "./controller";
import type { LocalValidation } from "./types";
import { modelTestStatus } from "./modelTestStatus";
import { localValidationLabel, localValidationReason, unavailableValidation, validLocalValidation, validationErrorReason } from "./localValidation";

function TestTime({ value, label }: { value: number; label: string }) {
  return <span>{label}<time dateTime={new Date(value).toISOString()}>{new Date(value).toLocaleString(undefined, { hour12: false })}.{String(value % 1000).padStart(3, "0")}</time></span>;
}

/** Shared by list history and add/download outcomes; these are separate facts. */
export function LocalValidationFeedback({ value, history = false }: { value: LocalValidation; history?: boolean }) {
  value = validLocalValidation(value) ? value : unavailableValidation();
  return <div className={`local-validation-record test-${modelTestStatus(value).tone}`}>
    {history && <p className="small-note">历史本机记录 · 不表示当前已加载或本次测试通过</p>}
    <strong>{localValidationLabel(value)}</strong>
    {localValidationReason(value) && <p className="small-note">{localValidationReason(value)}</p>}
    {value.checked_at_unix_ms !== null && <p className="small-note"><TestTime value={value.checked_at_unix_ms} label={history ? "记录时间：" : "本次检查时间："} /></p>}
    {value.error_code && <p className="small-note">本机测试诊断码：{value.error_code}</p>}
  </div>;
}

function resultTitle(attempt: ModelTestAttempt): string {
  const code = attempt.error?.code ?? attempt.result?.error_code;
  if (code === "validation_record_write_failed" || code === "validation_record_unavailable") return "本次测试记录无法保存";
  if (code === "validation_record_read_failed" || code === "validation_record_invalid") return "本次测试记录无法读取";
  if (attempt.error) return "本次操作未完成 · 请查看原因";
  switch (attempt.result?.state) {
    case "passed": return "本次基础测试通过";
    case "loaded": return "本次加载成功 · 短文本结果待确认";
    case "failed": return "本次基础测试失败";
    case "deferred": return "本次基础测试已暂缓";
    case "stale": return "本次结果已过期 · 待重测";
    case "unavailable": return "本次测试记录不可用";
    default: return "本次尚未取得基础测试结果";
  }
}
export function ModelTestFeedback({ attempt, currentEvidence }: { attempt: ModelTestAttempt; currentEvidence?: LocalValidation | null }) {
  const [now, setNow] = useState(attempt.started_at);
  const running = attempt.phase === "running";
  useEffect(() => {
    if (!running) return;
    const timer = setInterval(() => setNow(Date.now()), 250);
    return () => clearInterval(timer);
  }, [attempt.id, running]);
  const elapsed = Math.max(0, (attempt.finished_at ?? now) - attempt.started_at);
  const code = attempt.error?.code ?? attempt.result?.error_code;
  const explanation = code ? validationErrorReason(code) : attempt.result ? localValidationReason(attempt.result) : null;
  const original = modelTestStatus(null, attempt);
  const current = modelTestStatus(currentEvidence, attempt);
  const superseded = !running && (current.label !== original.label || current.tone !== original.tone);
  return <div className={`model-test-feedback ${current.tone}`} aria-label="本次模型测试">
    <strong role="status" aria-live="polite" aria-atomic="true">{running && <span className="spinner" aria-hidden="true" />}{running ? attempt.mode === "load" ? "本次正在加载与基础测试" : "本次正在进行基础测试" : superseded ? `${current.label} · 最新本机记录` : resultTitle(attempt)}</strong>
    {superseded && <p className="small-note">上次操作：{resultTitle(attempt).replace(/^本次/, "当时")}。历史结果不表示当前有效，请以下方最新本机记录为准。</p>}
    <p className="small-note">{running ? "已用时" : "本次耗时"} {(elapsed / 1000).toFixed(1)} 秒 · <TestTime value={attempt.finished_at ?? attempt.started_at} label={running ? "开始：" : "完成："} /></p>
    {!running && !superseded && explanation && <p>{explanation}</p>}
    {!running && !superseded && code && <p className="small-note">本次诊断码：{code}</p>}
    {!running && attempt.error && attempt.result?.state === "passed" && <p className="small-note">短文本调用返回了通过结果，但最新持久记录尚未确认。</p>}
    {!running && <p className="small-note">仅为本次加载与短文本测试结果，不保证回答质量、长上下文或工具调用能力。</p>}
  </div>;
}
