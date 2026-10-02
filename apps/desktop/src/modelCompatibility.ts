import type { ModelSummary } from "./types";

const sourceErrors: Record<string, string> = {
  model_file_changed: "源文件已变动，请停止服务后重新扫描；重新登记不等于准入",
  model_file_unavailable: "源文件已失踪或无法读取，请恢复源文件后重试",
  model_file_in_use: "源文件被写入程序占用，请释放后重试",
  model_directory_unavailable: "源目录无法读取，请检查目录是否存在及读取权限",
  model_directory_unsupported: "当前平台或目录无法提供所需的源文件保护，请使用受支持的 Windows 本地目录",
  model_library_unsupported: "请停止服务后启动匹配版本",
};
const reasons: Record<string, string> = {
  architecture_unsupported:
    "登记的架构不在本版本引擎范围内（仅 qwen3）。请使用已准入资产，或等待对应架构接入并验证",
  quantization_unvalidated:
    "登记的量化未进入本版本验证矩阵（当前仅固定 Q8_0 资产）。请使用已准入资产，或等待该量化完成验证",
  template_unvalidated:
    "登记的模板指纹不匹配本版本验证矩阵。请使用已准入资产，或等待该模板完成验证",
  context_unvalidated:
    "登记的上下文配置不匹配本版本验证矩阵（模型上限 40960、默认 2048）。请使用已准入配置",
  artifact_unvalidated:
    "架构、量化、模板与上下文匹配，但文件 SHA-256 或长度不属于已准入的精确资产。请使用已准入资产，或等待此资产完成验证",
  unvalidated:
    "登记信息匹配已知资产，但缺少本版本准入记录。请停止服务后重新登记该资产；若仍未准入，请检查运行版本",
};

/** Preserve source failures ahead of historical metadata compatibility. */
export function modelCompatibility(model: ModelSummary) {
  const status = model.compatibility ?? "unknown";
  const detailed = status === "admitted" || status in reasons;
  const admitted = status === "admitted";
  const reason =
    (model.availability_error && sourceErrors[model.availability_error]) ||
    (model.availability_error && model.availability_error !== "unsupported_model"
      ? "运行服务报告模型不可用，请检查错误代码及运行状态"
      : reasons[status]) ||
    (!detailed
      ? "当前服务未提供兼容性详情，请停止服务后启动匹配版本；不能据此判断本版本支持范围"
      : !model.available
        ? "当前运行服务未确认模型可用，请检查运行状态"
        : null);
  return {
    admission: admitted
      ? "本版本精确矩阵已准入"
      : detailed
        ? "本版本精确矩阵未准入"
        : "未提供兼容性详情",
    architecture: !detailed
      ? "引擎架构范围：未提供详情"
      : status === "architecture_unsupported"
        ? "引擎架构范围：不支持此架构"
        : "引擎架构范围：qwen3",
    reason,
    unavailableLabel:
      model.availability_error && model.availability_error !== "unsupported_model"
        ? "当前不可用"
        : detailed && !admitted
          ? "未准入"
          : "当前不可用",
  };
}
