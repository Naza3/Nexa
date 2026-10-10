//! Real CLI process tests for the private desktop-only diagnostic channel.
use runtime_cli::{DESKTOP_STARTUP_REPORT_ENV, instance::Discovery};
use std::{
    fs,
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

fn command(root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ai-runtime"));
    command
        .arg("--data-dir")
        .arg(root)
        .env_remove(DESKTOP_STARTUP_REPORT_ENV);
    command
}
fn invalid_config(root: &Path) {
    runtime_api::token::init_private_token(root).unwrap();
    runtime_api::token::write_private_new(&root.join("config.toml"), b"invalid=[").unwrap();
}

#[test]
fn diagnostic_is_opt_in_and_never_changes_other_command_output() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    invalid_config(&root);
    let ordinary = command(&root).arg("serve").output().unwrap();
    assert_eq!(ordinary.status.code(), Some(1));
    assert!(ordinary.stdout.is_empty());
    let desktop = command(&root)
        .env(DESKTOP_STARTUP_REPORT_ENV, "1")
        .arg("serve")
        .output()
        .unwrap();
    assert_eq!(desktop.status.code(), Some(1));
    assert_eq!(desktop.stdout, b"nexa-startup-v1:configuration_invalid\n");
    assert_eq!(ordinary.stderr, desktop.stderr);
    let version = command(&root)
        .env(DESKTOP_STARTUP_REPORT_ENV, "1")
        .args(["version", "--json"])
        .output()
        .unwrap();
    assert!(version.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&version.stdout).unwrap()["name"],
        "Nexa"
    );
}

#[test]
fn competing_instance_lock_reports_busy_without_bypassing_ownership() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    runtime_api::configuration::initialize(&root).unwrap();
    let lock = runtime_cli::instance::InstanceLock::try_acquire(&root)
        .unwrap()
        .unwrap();
    let observed = command(&root)
        .env(DESKTOP_STARTUP_REPORT_ENV, "1")
        .arg("serve")
        .output()
        .unwrap();
    assert_eq!(observed.status.code(), Some(1));
    assert_eq!(observed.stdout, b"nexa-startup-v1:runtime_instance_busy\n");
    assert!(!lock.has_discovery());
    assert!(
        runtime_cli::instance::InstanceLock::try_acquire(&root)
            .unwrap()
            .is_none()
    );
    drop(lock);
}

#[test]
fn missing_or_nonregular_worker_keeps_fixed_report_and_cli_message() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    runtime_api::configuration::initialize(&root).unwrap();
    let package = temp.path().join("package");
    fs::create_dir(&package).unwrap();
    let executable = package.join(if cfg!(windows) {
        "ai-runtime.exe"
    } else {
        "ai-runtime"
    });
    fs::copy(env!("CARGO_BIN_EXE_ai-runtime"), &executable).unwrap();
    let worker = package.join(if cfg!(windows) {
        "ai-runtime-worker.exe"
    } else {
        "ai-runtime-worker"
    });
    for nonregular in [false, true] {
        if nonregular {
            fs::create_dir(&worker).unwrap();
        }
        let observed = Command::new(&executable)
            .arg("--data-dir")
            .arg(&root)
            .arg("serve")
            .env(DESKTOP_STARTUP_REPORT_ENV, "1")
            .output()
            .unwrap();
        assert_eq!(observed.status.code(), Some(1));
        assert_eq!(
            observed.stdout,
            b"nexa-startup-v1:packaged_runtime_missing\n"
        );
        assert_eq!(
            observed.stderr,
            if nonregular {
                b"ai-runtime: packaged worker must be a regular file beside ai-runtime\n".as_slice()
            } else {
                b"ai-runtime: packaged worker is missing beside ai-runtime\n".as_slice()
            }
        );
        assert!(Discovery::read(&root).is_err());
    }
}

#[test]
fn closed_reader_does_not_change_cli_failure_exit() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    invalid_config(&root);
    let mut child = command(&root)
        .env(DESKTOP_STARTUP_REPORT_ENV, "1")
        .arg("serve")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    assert_eq!(child.wait().unwrap().code(), Some(1));
}

struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}
#[test]
fn service_remains_available_after_desktop_diagnostic_reader_closes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    runtime_api::token::init_private_token(&root).unwrap();
    let mut config = runtime_api::Config::default();
    config.api.listen.set_port(0);
    runtime_api::token::write_private_new(
        &root.join("config.toml"),
        config.to_toml().unwrap().as_bytes(),
    )
    .unwrap();
    let package = temp.path().join("package");
    fs::create_dir(&package).unwrap();
    let executable = package.join(if cfg!(windows) {
        "ai-runtime.exe"
    } else {
        "ai-runtime"
    });
    fs::copy(env!("CARGO_BIN_EXE_ai-runtime"), &executable).unwrap();
    // Empty-store lifecycle only: no worker is launched and no inference is claimed.
    fs::copy(
        env!("CARGO_BIN_EXE_ai-runtime"),
        package.join(if cfg!(windows) {
            "ai-runtime-worker.exe"
        } else {
            "ai-runtime-worker"
        }),
    )
    .unwrap();
    let mut child = Running(
        Command::new(executable)
            .arg("--data-dir")
            .arg(&root)
            .arg("serve")
            .env(DESKTOP_STARTUP_REPORT_ENV, "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    drop(child.0.stdout.take());
    let started = Instant::now();
    while Discovery::read(&root).is_err() {
        assert!(child.0.try_wait().unwrap().is_none());
        assert!(started.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        command(&root)
            .arg("status")
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(
        command(&root)
            .arg("stop")
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(child.0.wait().unwrap().success());
}
