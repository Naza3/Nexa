import type { LocalValidation } from "./types";

/** Reject contradictory evidence rather than displaying an invented pass. */
export function validLocalValidation(value: LocalValidation): boolean {
  if (!value || typeof value !== "object" ||
      !["untested", "loaded", "passed", "failed", "stale", "deferred", "unavailable"].includes(value.state) ||
      typeof value.load_success !== "boolean" || typeof value.generation_pass !== "boolean" ||
      (value.checked_at_unix_ms !== null && (!Number.isSafeInteger(value.checked_at_unix_ms) || value.checked_at_unix_ms < 0 || value.checked_at_unix_ms > 8640000000000000)) ||
      (value.error_code !== null && (typeof value.error_code !== "string" || !/^[a-z0-9_]{1,96}$/.test(value.error_code))) ||
      (value.generation_pass && !value.load_success)) return false;
  if (value.state === "passed") return value.load_success && value.generation_pass && value.checked_at_unix_ms !== null && value.error_code === null;
  if (value.state === "loaded") return value.load_success && !value.generation_pass;
  if (value.state === "untested") return !value.load_success && !value.generation_pass;
  if (value.state === "failed" || value.state === "deferred") return !value.generation_pass;
  if (value.state === "unavailable") return !value.load_success && !value.generation_pass && value.error_code !== null;
  return true;
}

export function localValidationLabel(value: LocalValidation): string {
  if (!validLocalValidation(value)) return "本机测试记录无效，待重测";
  if (["validation_record_write_failed", "validation_record_unavailable"].includes(value.error_code ?? "")) return value.load_success ? "本机加载通过 · 测试记录无法保存" : "本机测试记录无法保存";
  if (value.error_code === "request_cancelled") return value.load_success ? "本机加载通过 · 短文本测试已停止" : "本次加载或测试已停止";
  switch (value.state) {
    case "passed": return "本机基础测试通过";
    case "loaded": return "本机加载通过 · 短文本待测试";
    case "failed": return value.load_success ? "本机加载通过 · 短文本测试失败" : "本机加载测试失败";
    case "stale": return "本机记录已过期 · 待重测";
    case "deferred": return "本机测试已暂缓";
    case "unavailable": return "本机测试记录不可用";
    default: return "本机待测试";
  }
}

export function localValidationReason(value: LocalValidation): string | null {
  if (!validLocalValidation(value)) return "无法确认本机证明，请重新加载并测试。";
  if (value.error_code === "request_cancelled") return "本次加载或测试已停止，取消不算基础测试通过；已保存和登记的文件保留。";
  if (value.error_code?.startsWith("validation_")) return validationErrorReason(value.error_code);
  if (value.state === "stale") return "文件、引擎、设备或加载参数已变化；旧记录不能作为当前组合的通过证明。";
  if (value.state === "deferred") return "运行服务正在使用其他模型或处理任务，测试已暂缓。空闲后显式加载或重试；不会切换模型或中断其他客户端。";
  if (value.state === "failed") return value.load_success
    ? "加载已成功，但短文本生成未通过。取消、超时或失败均不算通过；可在空闲时重试。"
    : "本次未获得加载成功证明，请检查诊断并显式重试。";
  if (value.state === "loaded") return "本机已成功加载；尚未获得短文本生成通过证明。";
  if (value.state === "untested") return "已登记，尚无本机测试证明；可尝试加载并自动进行短文本测试。";
  return null;
}

/** Deliberately ignore raw native error text, paths, tokens and generated text. */
export function validationErrorReason(code: string): string {
  const reasons: Record<string, string> = {
    validation_engine_unavailable: "无法读取当前引擎身份，不能确认本机测试证明。请检查安装完整性后重试。",
    validation_scope_unavailable: "无法读取文件、设备或加载参数的验证条件，不能确认本机测试证明。",
    validation_scope_changed: "测试期间文件、引擎、设备或加载参数发生变化，本次结果不能用于当前组合，请重测。",
    validation_record_read_failed: "无法读取本机测试记录。记录可能损坏或暂不可读，不能视为未测试，也不能确认通过。",
    validation_record_invalid: "本机测试记录格式无效，不能视为未测试，也不能确认通过。",
    validation_record_write_failed: "本次测试记录无法保存，未取得可确认的持久证明。请检查应用数据目录的可写性后重试。",
    validation_record_unavailable: "本机测试记录无法保存或读取，未取得可确认的持久证明。",
    validation_result_unconfirmed: "本次返回结果与当前持久记录不一致，尚未确认本次通过。不能用此前记录代替本次结果，请刷新或重测。",
    validation_result_missing: "加载操作已结束，但未取得本次短文本生成结果。可在空闲时点击基础测试。",
    validation_refresh_failed: "本次操作后的状态或记录刷新失败，尚未确认最新持久证明。请刷新后重试。",
    invalid_local_validation: "本次测试返回的结果无效，未按通过处理。",
    runtime_busy: "当前有运行任务或模型操作，测试已暂缓。空闲后请再次点击，不会自动排队或中断其他客户端。",
    model_not_ready: "当前模型尚未就绪，请先加载此模型再测试。",
    request_cancelled: "本次短文本测试已取消，取消不算通过。",
    deadline_exceeded: "本次短文本测试超时，未按通过处理；请在空闲时重试。",
    worker_failed: "推理进程未完成本次测试，未按通过处理。请检查运行状态后重试。",
    configuration_migration_required: "请先在设置中确认旧配置迁移来源，再按统一档案加载。",
    configuration_restart_required: "已保存配置尚未被运行实例采用，请显式停止并重新启动服务后再加载。",
    load_timeout: "原生加载超时，请检查模型文件与加载参数。此错误不等同于上下文超限或内存不足。",
    model_load_interrupted: "本次加载连接已中断，实际结果尚未确认。请先核对运行状态，不自动重放。",
    context_length_exceeded: "输入、模板与输出预算超过上下文限制，请减少输入或输出预算，或使用模型支持的上下文配置。",
    context_too_large: "请求上下文超过模型或运行时上限，请降低该模型的上下文配置后重新加载。",
    model_load_failed: "模型加载失败，请检查文件、内存及当前加载参数；不能据此判断为上下文超限。",
    unsupported_model: "本次加载或测试未通过当前引擎检查，请查看模型与运行服务诊断。",
  };
  return reasons[code] ?? "本次操作未完成或结果尚未确认，请检查运行状态并在空闲时重试。";
}

export function unavailableValidation(error_code = "validation_record_invalid"): LocalValidation {
  return { state: "unavailable", load_success: false, generation_pass: false, checked_at_unix_ms: null, error_code };
}
