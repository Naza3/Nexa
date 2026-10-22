import { describe, expect, it } from "vitest";
import { DesktopError, safeError } from "../src/adapter";
import { errorText, presentError } from "../src/errorPresentation";

describe("controlled error presentation", () => {
  it.each(["worker_lost", "configuration_invalid", "future_error"])("keeps %s but never echoes native paths, tokens or arbitrary content", (code) => {
    const error = { code, message: "Bearer PRIVATE_TOKEN C:\\Users\\Alice\\private https://host/?secret=x private prompt generated answer" };
    expect(safeError(error).code).toBe(code);
    expect(errorText(error)).not.toMatch(/PRIVATE_TOKEN|Alice|https|private|generated/);
  });
  it.each([null, "raw", new Error("private"), { code: "../private", message: "bad" }, { code: "__proto__", message: "private" }])("handles unknown shapes and dictionary prototype names", (error) => {
    expect(presentError(error).message).toContain("请先检查运行服务");
  });
  it("preserves safe developer-authored validation detail but blocks unsafe local content", () => {
    expect(safeError(new DesktopError("ocr_image_invalid", "图片文件不能超过 4 MiB。")).message).toBe("图片文件不能超过 4 MiB。");
    expect(safeError(new DesktopError("ocr_image_invalid", "Bearer PRIVATE")).message).not.toContain("PRIVATE");
  });
  it("preserves only numeric OS facts and fixed configuration field labels", () => {
    expect(safeError({ code: "runtime_start_failed", message: "private executable (OS error 5)" }).message).toContain("OS 错误 5");
    const error = safeError({ code: "configuration_invalid", message: "private（字段：模型上下文上限）" });
    expect(error.message).toContain("字段：模型上下文上限");
    expect(error.message).not.toContain("private");
    expect(safeError(error)).toEqual(error);
  });
  it("preserves exact controlled configuration causes through repeated normalization", () => {
    const error = safeError({ code: "configuration_invalid", message: "The configuration file contains invalid TOML syntax. Correct the document before retrying." });
    expect(error.message).toContain("TOML 语法无效");
    expect(safeError(error)).toEqual(error);
    expect(safeError({ code: error.code, message: `${error.message} private` }).message).not.toContain("private");
  });
  it("retains bounded sidecar exit diagnostics and a cleanup warning without echoing raw text", () => {
    const error = safeError({ code: "model_download_network_failed", message: "private（下载进程退出码 19）。未切换下载源，未发布模型文件。 临时文件清理未确认，请勿自动重试。" });
    expect(error.message).toContain("域名解析失败");
    expect(error.message).toContain("退出码 19");
    expect(error.message).toContain("清理未确认");
    expect(error.message).not.toContain("private");
    expect(safeError(error)).toEqual(error);
  });
  it.each(["worker_lost", "runtime_faulted", "model_not_loaded", "load_timeout", "queue_timeout", "runtime_loopback_bind_failed", "runtime_security_invalid", "model_download_outcome_unknown"])("provides a concrete Chinese next step for %s", (code) => {
    expect(presentError({ code }).message).not.toBe(presentError(null).message);
    expect(presentError({ code }).message).toMatch(/[\u4e00-\u9fff]/);
  });
});

it("keeps a typed configuration cause with an approved field label through both adapter passes", () => {
  const value = safeError({ code: "configuration_invalid", message: "Configuration fields or values are unsupported. Check the schema and allowed values before retrying.（字段：模型上下文上限）" });
  expect(value.message).toContain("配置字段或值不受支持");
  expect(value.message).toContain("模型上下文上限");
  expect(safeError(value)).toEqual(value);
});
