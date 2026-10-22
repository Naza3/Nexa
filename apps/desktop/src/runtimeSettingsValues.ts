import type { Preferences } from "./types";

export const DEFAULT_VERIFICATION_SECONDS = 300;
export const MIN_VERIFICATION_SECONDS = 30;
export const MAX_VERIFICATION_SECONDS = 7200;

export function validateIdleSeconds(seconds: number): string | null {
  return Number.isSafeInteger(seconds) && seconds >= 1 && seconds <= 86400
    ? null : "空闲等待时间须为 1–86400 秒的整数。";
}
export function validateVerificationSeconds(seconds: number): string | null {
  return Number.isSafeInteger(seconds) && seconds >= MIN_VERIFICATION_SECONDS && seconds <= MAX_VERIFICATION_SECONDS
    ? null : "模型文件校验超时须为 30–7200 秒的整数。";
}
/** A preferences save must never send independent runtime configuration fields. */
export function preferencesOnly(settings: Preferences): Preferences {
  return {
    context_size: settings.context_size,
    threads: settings.threads,
    batch_size: settings.batch_size,
    max_output_tokens: settings.max_output_tokens,
    close_runtime_on_exit: settings.close_runtime_on_exit,
    download_source: settings.download_source,
  };
}

export function validateExecutionSeconds(seconds: number): string | null {
  return Number.isSafeInteger(seconds) && seconds >= 1 && seconds <= 86400
    ? null : "推理执行超时须为 1–86400 秒的整数。";
}
