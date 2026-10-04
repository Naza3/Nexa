//! Explicit Windows CI acceptance probe. Never run implicitly at startup.
//! Fetches exactly the fixed catalog fixture once through the public desktop
//! bridge, then independently rehashes it. Stdout/report contain no user paths.
use desktop_bridge::{DesktopBridge, DownloadSource, DownloadStatus, LibraryOperationStatus};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use uuid::Uuid;

const CATALOG_ID: &str = "qwen3-0.6b-q8-0";
fn arguments(args: &[std::ffi::OsString]) -> Option<(PathBuf, PathBuf, PathBuf)> {
    if args.len() != 6
        || args[0] != "--directory"
        || args[2] != "--output"
        || args[4] != "--component-dir"
    {
        return None;
    }
    let directory = PathBuf::from(&args[1]);
    let output = PathBuf::from(&args[3]);
    let component = PathBuf::from(&args[5]);
    (directory.is_absolute() && output.is_absolute() && component.is_absolute())
        .then_some((directory, output, component))
}
#[derive(Debug)]
struct ProbeFailure {
    stage: &'static str,
    code: &'static str,
}
fn failure(stage: &'static str, code: &str) -> ProbeFailure {
    let code = match code {
        "unsupported_platform" => "unsupported_platform",
        "io" => "io",
        "already_exists" => "already_exists",
        "desktop_busy" => "desktop_busy",
        "desktop_closing" => "desktop_closing",
        "runtime_running" => "runtime_running",
        "runtime_stop_unconfirmed" => "runtime_stop_unconfirmed",
        "model_directory_required" => "model_directory_required",
        "model_directory_unavailable" => "model_directory_unavailable",
        "model_directory_unsupported" => "model_directory_unsupported",
        "model_file_changed" => "model_file_changed",
        "model_file_unavailable" => "model_file_unavailable",
        "model_file_in_use" => "model_file_in_use",
        "model_download_engine_unavailable" => "model_download_engine_unavailable",
        "model_download_network_failed" => "model_download_network_failed",
        "model_download_http_failed" => "model_download_http_failed",
        "model_download_redirect_rejected" => "model_download_redirect_rejected",
        "model_download_identity_mismatch" => "model_download_identity_mismatch",
        "model_download_size_mismatch" => "model_download_size_mismatch",
        "model_download_write_failed" => "model_download_write_failed",
        "model_download_incomplete" => "model_download_incomplete",
        "model_download_cancelled" => "model_download_cancelled",
        "model_download_timeout" => "model_download_timeout",
        "model_download_cleanup_unconfirmed" => "model_download_cleanup_unconfirmed",
        _ => "verification_failed",
    };
    ProbeFailure { stage, code }
}
fn bridge_failure(stage: &'static str, error: desktop_bridge::BridgeError) -> ProbeFailure {
    failure(stage, &error.code)
}
async fn verify(directory: PathBuf, component: PathBuf) -> Result<serde_json::Value, ProbeFailure> {
    let started = Instant::now();
    if !cfg!(windows) {
        return Err(failure("platform", "unsupported_platform"));
    }
    let private = std::env::temp_dir().join(format!("nexa-download-verify-{}", Uuid::new_v4()));
    runtime_api::token::create_private_dir(&private).map_err(|_| failure("prepare", "io"))?;
    let bridge = Arc::new(
        DesktopBridge::new(private.clone(), private.join("ai-runtime.exe"))
            .map_err(|e| bridge_failure("prepare", e))?
            .with_download_sidecar_verifier(move || {
                download_engine::identity::verify_component(&component, None).map_err(|_| {
                    desktop_bridge::BridgeError {
                        code: "model_download_engine_unavailable".into(),
                        message: "Download component verification failed".into(),
                    }
                })
            }),
    );
    let result = async {
        let item = bridge.model_catalog().map_err(|e| bridge_failure("prepare", e))?.entries.into_iter().find(|e| e.catalog_id == CATALOG_ID).ok_or_else(|| failure("prepare", "verification_failed"))?;
        let source = item.sources.iter().find(|s| s.source == DownloadSource::Modelscope).ok_or_else(|| failure("prepare", "verification_failed"))?;
        let library = bridge.directory_apply(directory).map_err(|e| bridge_failure("admit_directory", e))?;
        loop {
            let state = bridge.library_next(library.operation_id).await.map_err(|e| bridge_failure("scan", e))?;
            if state.terminal {
                if state.status != LibraryOperationStatus::Completed { return Err(state.error.map(|e| bridge_failure("scan", e)).unwrap_or_else(|| failure("scan", "verification_failed"))); }
                break;
            }
        }
        let handle = bridge.download_start(CATALOG_ID.into()).map_err(|e| bridge_failure("download_start", e))?;
        let terminal = tokio::time::timeout(Duration::from_secs(30 * 60), async {
            loop {
                let state = bridge.download_next(handle.operation_id).await.map_err(|e| bridge_failure("download_poll", e))?;
                if state.terminal { return Ok::<_, ProbeFailure>(state); }
            }
        }).await.map_err(|_| failure("download_poll", "model_download_timeout"))??;
        if terminal.status != DownloadStatus::Completed {
            return Err(terminal.error.map(|e| bridge_failure("download_poll", e)).unwrap_or_else(|| failure("download_poll", "verification_failed")));
        }
        if terminal.source != DownloadSource::Modelscope || terminal.downloaded_bytes != item.size_bytes || !terminal.result.as_ref().is_some_and(|r| r.saved && r.registered && r.registration_error.is_none() && r.cleanup_warning.is_none()) {
            return Err(failure("download_poll", "verification_failed"));
        }
        let library = model_store::library::ModelLibrary::read(&private).map_err(|_| failure("verify_file", "verification_failed"))?.ok_or_else(|| failure("verify_file", "verification_failed"))?;
        let model = library.directory.as_ref().ok_or_else(|| failure("verification", "model_directory_required"))?.join(&item.file_name);
        // Independent second pass witnesses final bytes, not merely the task counter.
        let (size, sha256) = tokio::task::spawn_blocking(move || {
            let mut file = fs::File::open(model).map_err(|_| failure("verify_file", "io"))?;
            let mut hasher = Sha256::new();
            let mut size = 0u64;
            let mut buffer = [0; 64 * 1024];
            loop {
                let n = file.read(&mut buffer).map_err(|_| failure("verify_file", "io"))?;
                if n == 0 { break; }
                size += n as u64;
                if size > item.size_bytes { return Err(failure("verify_file", "model_download_size_mismatch")); }
                hasher.update(&buffer[..n]);
            }
            Ok::<_, ProbeFailure>((size, format!("{:x}", hasher.finalize())))
        }).await.map_err(|_| failure("verify_file", "verification_failed"))??;
        if size != item.size_bytes || sha256 != item.sha256 { return Err(failure("verify_file", "model_download_identity_mismatch")); }
        Ok(json!({"schema_version":1,"success":true,"source":"modelscope","catalog_id":CATALOG_ID,"source_revision":source.revision,"size_bytes":size,"sha256":sha256,"downloaded_bytes":terminal.downloaded_bytes,"published":true,"registered":true,"elapsed_ms":started.elapsed().as_millis() as u64}))
    }.await;
    // Cancellation/cleanup must complete before deleting this probe's metadata.
    bridge
        .close()
        .await
        .map_err(|e| bridge_failure("close", e))?;
    fs::remove_dir_all(private).map_err(|_| failure("cleanup", "io"))?;
    result
}
#[tokio::main]
async fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let Some((directory, output, component)) = arguments(&args) else {
        println!("{{\"schema_version\":1,\"success\":false,\"error\":\"invalid_arguments\"}}");
        std::process::exit(2);
    };
    // Reserve report first: a duplicate invocation must not redownload merely
    // to discover at the end that its evidence destination already exists.
    let Ok(mut report) = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
    else {
        println!("{{\"schema_version\":1,\"success\":false,\"error\":\"report_unavailable\"}}");
        std::process::exit(2);
    };
    let result = verify(directory, component).await;
    let success = result.is_ok();
    let value = result.unwrap_or_else(|fault| json!({"schema_version":1,"success":false,"source":"modelscope","catalog_id":CATALOG_ID,"error":"catalog_download_verification_failed","stage":fault.stage,"code":fault.code}));
    let bytes = serde_json::to_vec(&value).unwrap();
    if bytes.len() > 4096
        || report
            .write_all(&bytes)
            .and_then(|_| report.sync_all())
            .is_err()
    {
        println!("{{\"schema_version\":1,\"success\":false,\"error\":\"report_write_failed\"}}");
        std::process::exit(2);
    }
    println!("{}", String::from_utf8(bytes).unwrap());
    if !success {
        std::process::exit(1);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diagnostic_codes_never_echo_paths_urls_or_unknown_errors() {
        for text in [
            "https://secret.invalid/?token=hidden",
            "C:\\Users\\private",
            "unexpected upstream detail",
        ] {
            assert_eq!(failure("download_poll", text).code, "verification_failed");
        }
        assert_eq!(
            failure("download_poll", "model_download_network_failed").code,
            "model_download_network_failed"
        );
    }
    #[test]
    fn arguments_are_explicit_absolute_and_bounded_by_fixed_shape() {
        assert!(arguments(&[]).is_none());
        assert!(
            arguments(&[
                "--directory".into(),
                "relative".into(),
                "--output".into(),
                "report.json".into()
            ])
            .is_none()
        );
        let base = std::env::temp_dir();
        assert!(
            arguments(&[
                "--directory".into(),
                base.join("models").into_os_string(),
                "--output".into(),
                base.join("report.json").into_os_string(),
                "--component-dir".into(),
                base.join("download").into_os_string()
            ])
            .is_some()
        );
        assert!(
            arguments(&[
                "--url".into(),
                "https://untrusted.invalid".into(),
                "--output".into(),
                base.join("report.json").into_os_string(),
                "--component-dir".into(),
                base.join("download").into_os_string()
            ])
            .is_none()
        );
    }
}
