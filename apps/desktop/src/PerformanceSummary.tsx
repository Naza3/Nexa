import type { RequestPerformance } from "./performance";
import { rateText, seconds, tokenRate } from "./performance";
import type { Usage } from "./types";

/** Kept outside generated content so copying or saving the result stays verbatim. */
export function PerformanceSummary({ value, usage, image = false }: { value: RequestPerformance; usage?: Pick<Usage, "prompt_tokens" | "completion_tokens">; image?: boolean }) {
  const record = value.state === "ready" ? value.record : null;
  const counts = record?.usage ?? usage;
  return <section className="performance-summary" aria-label="本次推理性能">
    <p className="performance-summary-title">本次推理性能{value.state === "pending" ? " · 正在读取…" : ""}</p>
    <div className="performance-summary-metrics">
      <span>Prefill{image ? "（含图片编码）" : ""} <strong>{record ? rateText(tokenRate(record, "prefill")) : "不可用"}</strong> · {seconds(record?.performance?.timings.prefill_us || undefined)}</span>
      <span>Decode <strong>{record ? rateText(tokenRate(record, "decode")) : "不可用"}</strong> · {seconds(record?.performance?.timings.decode_us || undefined)}</span>
      <span>输入 / 输出 <strong>{counts ? `${counts.prompt_tokens} / ${counts.completion_tokens}` : "不可用"}</strong> token</span>
      <span>总执行耗时 <strong>{record ? seconds(record.timings.execution_ms * 1000) : "不可用"}</strong></span>
    </div>
    <p className="performance-summary-note">{value.state === "unavailable" ? "本次性能记录不可用。" : ""}Decode 已扣同步输出回调，包含采样、文本处理及清理；执行耗时不含排队和加载。</p>
  </section>;
}
