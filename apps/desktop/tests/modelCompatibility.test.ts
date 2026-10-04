import { describe, expect, it } from "vitest";
import { modelCompatibility } from "../src/modelCompatibility";
import { model } from "./fixtures";

describe("registered model compatibility", () => {
  it("separates exact evidence from current source availability", () => {
    expect(modelCompatibility(model)).toMatchObject({
      admission: "有精确模型验证记录",
      reason: null,
    });
    const unavailable = modelCompatibility({ ...model, available: false, availability_error: "model_file_changed" });
    expect(unavailable.admission).toBe("有精确模型验证记录");
    expect(unavailable.reason).toContain("源文件已变动");
    expect(unavailable.unavailableLabel).toBe("当前不可用");
  });
  it("permits a candidate without claiming native support or validation", () => {
    const details = modelCompatibility({ ...model, architecture: "future_arch", compatibility: "unvalidated", validated: false, loadable: true });
    expect(details.admission).toBe("未实测");
    expect(details.reason).toContain("可尝试加载");
    expect(details.architecture).toContain("实际加载为准");
  });
  it.each([
    ["model_file_changed", "源文件已变动"],
    ["model_file_unavailable", "源文件已失踪"],
    ["model_file_in_use", "源文件被写入程序占用"],
    ["model_directory_unavailable", "源目录无法读取"],
    ["model_directory_unsupported", "源文件保护"],
  ])("prioritizes %s over historical evidence", (code, message) => {
    const details = modelCompatibility({ ...model, compatibility: "unvalidated", validated: false, available: false, availability_error: code });
    expect(details.reason).toContain(message);
    expect(details.admission).toBe("未实测");
  });
  it("does not infer load eligibility from old validation fields", () => {
    const details = modelCompatibility({ ...model, loadable: undefined });
    expect(details.reason).toContain("未提供独立加载资格");
  });
  it("treats missing and unknown status as unavailable detail", () => {
    for (const compatibility of [undefined, "unknown"] as const) {
      const details = modelCompatibility({ ...model, compatibility });
      expect(details.admission).toBe("未提供兼容性详情");
      expect(details.reason).toContain("不能据此判断本版本支持范围");
    }
  });
});
