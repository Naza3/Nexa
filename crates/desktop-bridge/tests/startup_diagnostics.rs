//! Keep process-spawning diagnostics separate from lock/atomic-write tests.
//! On Unix a concurrently forked child can briefly inherit another test's flock
//! before exec closes CLOEXEC descriptors; it must not distort a free-lock oracle.
#![cfg(unix)]
use desktop_bridge::DesktopBridge;
use runtime_api::{
    Config,
    token::{init_private_token, write_private_new},
};
use std::{path::PathBuf, sync::Arc};
static SPAWN_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
fn initialized() -> (tempfile::TempDir, PathBuf, Arc<DesktopBridge>) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("private");
    init_private_token(&root).unwrap();
    write_private_new(
        &root.join("config.toml"),
        Config::default().to_toml().unwrap().as_bytes(),
    )
    .unwrap();
    let bridge =
        Arc::new(DesktopBridge::new(root.clone(), temp.path().join("ai-runtime")).unwrap());
    (temp, root, bridge)
}
#[cfg(unix)]
#[tokio::test]
async fn startup_diagnostics_capture_spawn_error_and_clear_between_attempts() {
    use std::os::unix::fs::PermissionsExt;
    let _gate = SPAWN_GATE.lock().await;
    let (temp, _root, bridge) = initialized();
    let executable = temp.path().join("ai-runtime");
    let worker = temp.path().join("ai-runtime-worker");
    std::fs::write(&executable, b"non-executable controlled fixture").unwrap();
    std::fs::write(&worker, b"presence-only worker fixture").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        bridge.start(false).await.unwrap_err().code,
        "runtime_start_failed"
    );
    let diagnostic = bridge.startup_diagnostics();
    assert!(diagnostic.os_error.is_some());
    assert_eq!(diagnostic.process_exit_code, None);
    std::fs::remove_file(worker).unwrap();
    assert_eq!(
        bridge.start(false).await.unwrap_err().code,
        "packaged_runtime_missing"
    );
    assert_eq!(
        bridge.startup_diagnostics(),
        desktop_bridge::StartupDiagnostics::default()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn runtime_exit_zero_is_preserved_and_is_not_startup_success() {
    use std::os::unix::fs::PermissionsExt;
    let _gate = SPAWN_GATE.lock().await;
    let (temp, _root, bridge) = initialized();
    let executable = temp.path().join("ai-runtime");
    std::fs::write(&executable, b"#!/bin/sh\nexit 0\n").unwrap();
    std::fs::write(
        temp.path().join("ai-runtime-worker"),
        b"presence-only fixture",
    )
    .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        bridge.start(false).await.unwrap_err().code,
        "runtime_start_failed"
    );
    assert_eq!(
        bridge.startup_diagnostics(),
        desktop_bridge::StartupDiagnostics {
            os_error: None,
            process_exit_code: Some(0)
        }
    );
}

fn shell_fixture(temp: &tempfile::TempDir, script: &[u8]) {
    use std::os::unix::fs::PermissionsExt;
    let executable = temp.path().join("ai-runtime");
    std::fs::write(&executable, script).unwrap();
    std::fs::write(temp.path().join("ai-runtime-worker"), b"fixture").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
}

#[tokio::test]
async fn child_exit_reports_only_exact_startup_reason_and_clears_between_attempts() {
    let _gate = SPAWN_GATE.lock().await;
    let (temp, _root, bridge) = initialized();
    shell_fixture(
        &temp,
        b"#!/bin/sh\nprintf 'nexa-startup-v1:runtime_loopback_bind_failed\\n'\nexit 1\n",
    );
    let error = bridge.start(false).await.unwrap_err();
    assert_eq!(error.code, "runtime_loopback_bind_failed");
    assert_eq!(bridge.startup_diagnostics().process_exit_code, Some(1));
    shell_fixture(&temp, b"#!/bin/sh\nprintf 'nexa-startup-v1:configuration_invalid /private/secret token=secret\\n'\nexit 1\n");
    let error = bridge.start(false).await.unwrap_err();
    assert_eq!(error.code, "runtime_start_failed");
    assert!(!error.message.contains("secret"));
    assert!(!error.message.contains("/private"));
}

#[tokio::test]
async fn child_startup_channel_flood_is_drained_and_cannot_deadlock_startup() {
    let _gate = SPAWN_GATE.lock().await;
    let (temp, _root, bridge) = initialized();
    shell_fixture(&temp, b"#!/bin/sh\nprintf 'nexa-startup-v1:configuration_invalid\\n'\ni=0\nwhile [ $i -lt 5000 ]; do printf 'token=private-secret-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\\n'; i=$((i+1)); done\nexit 1\n");
    let error = tokio::time::timeout(std::time::Duration::from_secs(5), bridge.start(false))
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(error.code, "configuration_invalid");
    assert!(!error.message.contains("private-secret"));
    assert_eq!(bridge.startup_diagnostics().process_exit_code, Some(1));
}
