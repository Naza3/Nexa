import { describe, expect, it } from "vitest";
import { modelCompatibility } from "../src/modelCompatibility";
import type { ModelCompatibility } from "../src/types";
import { model } from "./fixtures";

describe("registered model compatibility", () => {
  it("separates admitted metadata from current source availability", () => {
    expect(modelCompatibility(model)).toMatchObject({
      admission: "本版本精确矩阵已准入",
      architecture: "引擎架构范围：qwen3",
      reason: null,
    });
    const unavailable = modelCompatibility({
      ...model,
      available: false,
      availability_error: "model_file_changed",
    });
    expect(unavailable.admission).toBe("本版本精确矩阵已准入");
    expect(unavailable.reason).toContain("源文件已变动");
    expect(unavailable.unavailableLabel).toBe("当前不可用");
  });
  it.each<[ModelCompatibility, string]>([
    ["architecture_unsupported", "不在本版本引擎范围"],
    ["quantization_unvalidated", "量化未进入"],
    ["template_unvalidated", "模板指纹不匹配"],
    ["context_unvalidated", "上下文配置不匹配"],
    ["artifact_unvalidated", "SHA-256 或长度"],
    ["unvalidated", "缺少本版本准入记录"],
  ])("explains %s without promising a rescan admits candidates", (compatibility, text) => {
    const details = modelCompatibility({
      ...model,
      compatibility,
      validated: false,
      available: false,
      availability_error: "unsupported_model",
    });
    expect(details.admission).toBe("本版本精确矩阵未准入");
    expect(details.reason).toContain(text);
    expect(details.unavailableLabel).toBe("未准入");
    expect(details.reason).not.toContain("尚未校验");
  });
  it.each([
    ["model_file_changed", "源文件已变动"],
    ["model_file_unavailable", "源文件已失踪"],
    ["model_file_in_use", "源文件被写入程序占用"],
    ["model_directory_unavailable", "源目录无法读取"],
    ["model_directory_unsupported", "源文件保护"],
  ])("prioritizes %s over compatibility without changing the registered status", (code, message) => {
    const details = modelCompatibility({
      ...model,
      compatibility: "architecture_unsupported",
      architecture: "qwen35",
      validated: false,
      available: false,
      availability_error: code,
    });
    expect(details.reason).toContain(message);
    expect(details.reason).not.toContain("等待对应架构");
    expect(details.admission).toBe("本版本精确矩阵未准入");
    expect(details.architecture).toBe("引擎架构范围：不支持此架构");
  });
  it("treats missing and unknown statuses as unavailable detail, even with a legacy validated claim", () => {
    for (const compatibility of [undefined, "unknown"] as const) {
      const details = modelCompatibility({ ...model, compatibility });
      expect(details.admission).toBe("未提供兼容性详情");
      expect(details.architecture).toContain("未提供详情");
      expect(details.reason).toContain("不能据此判断本版本支持范围");
    }
  });
});
