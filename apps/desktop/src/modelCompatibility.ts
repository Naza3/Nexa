import type { ModelSummary } from "./types";
import { localValidationLabel, localValidationReason } from "./localValidation";

const sourceErrors: Record<string, string> = {
  model_file_changed: "源文件已变动，请停止服务后重新扫描；仍须通过加载时核验",
  model_file_unavailable: "源文件已失踪或无法读取，请恢复源文件后重试",
  model_file_in_use: "源文件被写入程序占用，请释放后重试",
  model_directory_unavailable: "源目录无法读取，请检查目录是否存在及读取权限",
  model_directory_unsupported: "当前平台或目录无法提供所需的源文件保护，请使用受支持的 Windows 本地目录",
  model_library_unsupported: "请停止服务后启动匹配版本",
};

/** Availability, historical evidence and actual native support are independent. */
export function modelCompatibility(model: ModelSummary) {
  const status = model.compatibility ?? "unknown";
  const detailed = status !== "unknown";
  const verified = detailed && status === "admitted" && model.validated;
  const candidate = model.loadable === true;
  const sourceFailure = model.availability_error && sourceErrors[model.availability_error];
  const reason = sourceFailure ||
    (model.availability_error
      ? model.availability_error === "unsupported_model"
        ? "当前引擎或文本适配器不能运行此模型；请查看加载错误，不会自动替换模板"
        : "运行服务报告模型不可用，请检查错误代码及运行状态"
      : !detailed
        ? "当前服务未提供兼容性详情；不能据此判断本版本支持范围"
        : !candidate
          ? "当前服务未提供独立加载资格，请重启匹配版本后检查"
          : model.local_validation
            ? localValidationReason(model.local_validation)
          : !verified
            ? "未实测，可尝试加载。引擎、原始模板与设备资源仍须在加载时通过检查，失败不会改用其他模板"
            : null);
  return {
    admission: model.local_validation ? localValidationLabel(model.local_validation) : verified ? "有精确模型验证记录" : detailed ? "未实测" : "未提供兼容性详情",
    architecture: status === "architecture_unsupported"
      ? "当前引擎报告不支持此架构"
      : "引擎兼容性以实际加载为准；验证记录不限制候选模型",
    reason,
    unavailableLabel: "当前不可用",
  };
}
