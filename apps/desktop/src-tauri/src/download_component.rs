//! Recheck the fixed component on each download; the bridge owns the live guard.
use std::path::{Path, PathBuf};
pub fn verify(
    root: &Path,
    commit: &str,
) -> Result<(PathBuf, download_engine::identity::ComponentGuard), desktop_bridge::BridgeError> {
    download_engine::identity::verify_component(&root.join("download"), Some(commit)).map_err(
        |_| desktop_bridge::BridgeError {
            code: "model_download_engine_unavailable".into(),
            message: "下载组件身份未通过校验，请使用完整安装包。".into(),
        },
    )
}
