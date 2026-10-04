//! Closed startup diagnostics: no paths, error text, credentials or file contents.
use crate::layout::ProductLayout;
use serde::Serialize;

pub const PACKAGE_ERROR_CODES: &[&str] = &[
    "current_executable_unavailable",
    "desktop_executable_name_invalid",
    "package_root_unavailable",
    "selected_path_invalid",
    "selected_path_indirect",
    "selected_file_invalid",
    "selected_file_unavailable",
    "package_directory_unavailable",
    "package_file_unavailable",
    "package_manifest_unavailable",
    "package_manifest_too_large",
    "package_manifest_invalid",
    "package_identity_invalid",
    "package_inventory_invalid",
    "package_path_invalid",
    "package_indirect_path",
    "package_file_hash_mismatch",
    "package_checksum_invalid",
    "package_checksum_mismatch",
    "package_unlisted_file",
    "package_external_model_header_invalid",
    "runtime_source_identity_mismatch",
    "package_validation_failed",
];

pub fn safe_code(code: &str) -> &'static str {
    PACKAGE_ERROR_CODES
        .iter()
        .copied()
        .find(|candidate| *candidate == code)
        .unwrap_or("package_validation_failed")
}

fn package_reason(code: &str) -> &'static str {
    let code = safe_code(code);
    match code {
        "current_executable_unavailable" | "package_root_unavailable" => {
            "无法确定当前程序及其包目录。"
        }
        "desktop_executable_name_invalid" => "桌面程序名称与包内约定不一致。",
        "selected_path_invalid" | "package_path_invalid" => {
            "包路径不符合当前支持的本地磁盘路径规则。"
        }
        "selected_path_indirect" | "package_indirect_path" => {
            "包文件或其上级目录含链接、重解析点等间接路径，当前安全规则拒绝访问。"
        }
        "selected_file_invalid" => "包中预期的普通文件实际为其他类型。",
        "selected_file_unavailable" | "package_file_unavailable" => {
            "无法读取包内某个文件或其上级目录，可能是文件缺失或访问不可用。"
        }
        "package_directory_unavailable" => "无法枚举包目录中的文件。",
        "package_manifest_unavailable" => "无法读取包清单或校验表。",
        "package_manifest_too_large" => "包清单或校验表超过允许大小。",
        "package_manifest_invalid" => "包清单格式无效。",
        "package_identity_invalid" => "包清单的产品或来源标识无效。",
        "package_inventory_invalid" => "包清单包含无效、重复或冲突的文件记录。",
        "package_file_hash_mismatch" => "包内文件的大小或 SHA256 与清单不一致。",
        "package_checksum_invalid" => "SHA256SUMS 校验表格式无效。",
        "package_checksum_mismatch" => "SHA256SUMS 与包清单记录不一致。",
        "package_unlisted_file" => {
            "包目录中有未声明的文件或目录。程序根及 model、models 目录的直接普通 GGUF 可作为外置输入；其他附加文件或嵌套目录不在允许范围。"
        }
        "package_external_model_header_invalid" => {
            "包内额外 .gguf 文件不含完整 GGUF 文件头，无法识别为外置模型输入。"
        }
        "runtime_source_identity_mismatch" => "桌面与 runtime 清单的源码身份不一致。",
        _ => "包校验未能完成，原因尚未确定。",
    }
}

pub fn package_operation_message(code: &str) -> String {
    let code = safe_code(code);
    let reason = package_reason(code);
    format!("{reason}\n诊断代码：{code}\n本次模型目录操作未提交更改。")
}

pub fn package_message(code: &str) -> String {
    let code = safe_code(code);
    let reason = package_reason(code);
    format!("{reason}\n诊断代码：{code}\n请保留当前目录及此提示以便排查；本次未启动 runtime。")
}

#[derive(Serialize)]
pub struct StartupDiagnostic<'a> {
    schema_version: u32,
    package_verified: bool,
    package_error_code: Option<&'static str>,
    project_commit: Option<&'a str>,
    project_dirty: Option<bool>,
    webview2_version: Option<&'a str>,
    native_window_tested: bool,
}

impl<'a> StartupDiagnostic<'a> {
    pub fn new(
        layout: &'a Result<ProductLayout, &'static str>,
        webview2_version: Option<&'a str>,
    ) -> Self {
        Self {
            schema_version: 2,
            package_verified: layout.is_ok(),
            package_error_code: layout.as_ref().err().map(|code| safe_code(code)),
            project_commit: layout
                .as_ref()
                .ok()
                .map(|layout| layout.project_commit.as_str()),
            project_dirty: layout.as_ref().ok().map(|layout| layout.project_dirty),
            webview2_version,
            native_window_tested: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_code_has_a_bounded_message_without_dynamic_details() {
        for code in PACKAGE_ERROR_CODES {
            assert_eq!(safe_code(code), *code);
            let message = package_message(code);
            let operation_message = package_operation_message(code);
            assert!(operation_message.contains(code));
            assert!(operation_message.len() < 1024);
            assert!(!operation_message.contains("未启动 runtime"));
            assert!(message.contains(code));
            assert!(message.len() < 1024);
            if *code != "package_validation_failed" {
                assert!(!message.contains("原因尚未确定"));
            }
        }
        let private = "C:\\Users\\Private\\Bearer PRIVATE";
        assert_eq!(safe_code(private), "package_validation_failed");
        assert!(!package_message(private).contains("Private"));
    }

    #[test]
    fn report_preserves_failure_and_omits_unverified_identity() {
        let failed = Err("package_unlisted_file");
        let value =
            serde_json::to_value(StartupDiagnostic::new(&failed, Some("131.0.2903.86"))).unwrap();
        assert_eq!(value["schema_version"], 2);
        assert_eq!(value["package_error_code"], "package_unlisted_file");
        assert_eq!(value["package_verified"], false);
        assert!(value["project_commit"].is_null());
        assert!(value["project_dirty"].is_null());
        assert_eq!(value["native_window_tested"], false);
        assert_eq!(value.as_object().unwrap().len(), 7);
        let good = Ok(ProductLayout {
            package_root: "private package path".into(),
            runtime_executable: "private path".into(),
            project_commit: "a".repeat(40),
            project_dirty: false,
        });
        let value = serde_json::to_value(StartupDiagnostic::new(&good, None)).unwrap();
        assert_eq!(value["package_verified"], true);
        assert!(value["package_error_code"].is_null());
        assert!(value["webview2_version"].is_null());
        assert!(!value.to_string().contains("private"));
    }
}
