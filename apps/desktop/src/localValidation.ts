import type { LocalValidation } from "./types";

/** Reject contradictory evidence rather than displaying an invented pass. */
export function validLocalValidation(value: LocalValidation): boolean {
  if (!value || typeof value !== "object" ||
      !["untested", "loaded", "passed", "failed", "stale", "deferred"].includes(value.state) ||
      typeof value.load_success !== "boolean" || typeof value.generation_pass !== "boolean" ||
      (value.checked_at_unix_ms !== null && (!Number.isSafeInteger(value.checked_at_unix_ms) || value.checked_at_unix_ms < 0)) ||
      (value.error_code !== null && (typeof value.error_code !== "string" || !/^[a-z0-9_]{1,80}$/.test(value.error_code))) ||
      (value.generation_pass && !value.load_success)) return false;
  if (value.state === "passed") return value.load_success && value.generation_pass && value.checked_at_unix_ms !== null && value.error_code === null;
  if (value.state === "loaded") return value.load_success && !value.generation_pass;
  if (value.state === "untested") return !value.load_success && !value.generation_pass;
  if (value.state === "failed") return !value.generation_pass;
  return true;
}

export function localValidationLabel(value: LocalValidation): string {
  if (!validLocalValidation(value)) return "本机测试记录无效，待重测";
  switch (value.state) {
    case "passed": return "本机基础测试通过";
    case "loaded": return "本机加载通过 · 短文本待测试";
    case "failed": return value.load_success ? "本机加载通过 · 短文本测试失败" : "本机加载测试失败";
    case "stale": return "本机记录已过期 · 待重测";
    case "deferred": return "本机测试已暂缓";
    default: return "本机待测试";
  }
}

export function localValidationReason(value: LocalValidation): string | null {
  if (!validLocalValidation(value)) return "无法确认本机证明，请重新加载并测试。";
  if (value.state === "stale") return "文件、引擎、设备或加载参数已变化；旧记录不能作为当前组合的通过证明。";
  if (value.state === "deferred") return "运行服务正在使用其他模型或处理任务，测试已暂缓。空闲后显式加载或重试；不会切换模型或中断其他客户端。";
  if (value.state === "failed") return value.load_success
    ? "加载已成功，但短文本生成未通过。取消、超时或失败均不算通过；可在空闲时重试。"
    : "本次未获得加载成功证明，请检查诊断并显式重试。";
  if (value.state === "loaded") return "本机已成功加载；尚未获得短文本生成通过证明。";
  if (value.state === "untested") return "已登记，尚无本机测试证明；可尝试加载并自动进行短文本测试。";
  return null;
}
