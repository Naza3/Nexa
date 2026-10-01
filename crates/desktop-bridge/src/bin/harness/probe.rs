//! Observation-only Windows console registration probe. It loads no model,
//! initializes no token, and does not change the runtime or fallback flags.
use super::report::LaunchProbeReport;
#[cfg(windows)]
use super::report::{MAX_REPORT_BYTES, SIGNAL_STATES};
#[cfg(windows)]
use serde::{Deserialize, Serialize};
#[cfg(windows)]
use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

#[cfg(windows)]
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignalObservation {
    signal_state: String,
    signal_os_error: Option<i32>,
}

#[cfg(windows)]
pub async fn child(root: &Path) -> bool {
    if !root.is_absolute() || runtime_api::token::create_private_dir(root).is_err() {
        return false;
    }
    let result = tokio::time::timeout(Duration::from_millis(250), tokio::signal::ctrl_c()).await;
    let observation = match result {
        Err(_) => SignalObservation {
            signal_state: "pending".into(),
            signal_os_error: None,
        },
        Ok(Ok(())) => SignalObservation {
            signal_state: "received".into(),
            signal_os_error: None,
        },
        Ok(Err(error)) => SignalObservation {
            signal_state: "error".into(),
            signal_os_error: error.raw_os_error(),
        },
    };
    let Ok(bytes) = serde_json::to_vec(&observation) else {
        return false;
    };
    runtime_api::token::write_private_new(&root.join("signal-probe.json"), &bytes).is_ok()
}
#[cfg(not(windows))]
pub async fn child(_root: &std::path::Path) -> bool {
    false
}

#[cfg(any(windows, test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChildCleanup {
    NoChild,
    Reaped,
    Unconfirmed,
}
#[cfg(any(windows, test))]
fn finish_cleanup(root: &std::path::Path, child: ChildCleanup, report: &mut LaunchProbeReport) {
    // Deleting a report directory never proves a spawned process stopped.
    // Preserve the private directory whenever its owner's reaping is unknown.
    report.cleanup_confirmed =
        child != ChildCleanup::Unconfirmed && std::fs::remove_dir_all(root).is_ok();
    report.success = report.code == "observed_pending" && report.cleanup_confirmed;
}
#[cfg(windows)]
async fn reap_failed_probe(
    child: &mut std::process::Child,
    report: &mut LaunchProbeReport,
) -> ChildCleanup {
    if let Err(error) = child.kill()
        && report.os_error.is_none()
    {
        report.os_error = error.raw_os_error();
    }
    // Killing is a request, not confirmation. Never block forever in wait()
    // after a failed kill. Only an actual observed exit confirms cleanup.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                report.child_exit_code = status.code();
                return ChildCleanup::Reaped;
            }
            Ok(None) => (),
            Err(error) => {
                if report.os_error.is_none() {
                    report.os_error = error.raw_os_error();
                }
                return ChildCleanup::Unconfirmed;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return ChildCleanup::Unconfirmed;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[cfg(windows)]
async fn observe(root: &Path, report: &mut LaunchProbeReport) -> ChildCleanup {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::{
        CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS,
    };
    let executable = match std::env::current_exe() {
        Ok(executable) => executable,
        Err(error) => {
            report.code = "probe_io_failed".into();
            report.os_error = error.raw_os_error();
            return ChildCleanup::NoChild;
        }
    };
    let mut command = Command::new(executable);
    command
        .arg("--signal-probe-child")
        .arg(root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command.creation_flags(CREATE_BREAKAWAY_FROM_JOB | CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            report.code = "spawn_failed".into();
            report.spawn_os_error = error.raw_os_error();
            return ChildCleanup::NoChild;
        }
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => (),
            Err(error) => {
                report.code = "probe_io_failed".into();
                report.os_error = error.raw_os_error();
                break None;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            report.code = "timeout".into();
            break None;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let Some(status) = status else {
        return reap_failed_probe(&mut child, report).await;
    }; // owned probe child only
    report.child_exit_code = status.code();
    if !status.success() {
        report.code = "child_failed".into();
        return ChildCleanup::Reaped;
    }
    let path = root.join("signal-probe.json");
    let read = || -> std::io::Result<Vec<u8>> {
        if !std::fs::symlink_metadata(&path)?.file_type().is_file() {
            return Err(std::io::Error::from(std::io::ErrorKind::InvalidData));
        }
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take((MAX_REPORT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        Ok(bytes)
    };
    let bytes = match read() {
        Ok(bytes) => bytes,
        Err(error) => {
            report.code = "probe_io_failed".into();
            report.os_error = error.raw_os_error();
            return ChildCleanup::Reaped;
        }
    };
    let observation = if bytes.len() <= MAX_REPORT_BYTES {
        runtime_api::dto::parse_json(&bytes)
            .ok()
            .and_then(|value| serde_json::from_value::<SignalObservation>(value).ok())
    } else {
        None
    };
    let Some(observation) = observation.filter(|o| {
        SIGNAL_STATES.contains(&o.signal_state.as_str())
            && o.signal_state != "not_observed"
            && (o.signal_state == "error" || o.signal_os_error.is_none())
    }) else {
        report.code = "invalid_report".into();
        return ChildCleanup::Reaped;
    };
    report.signal_state = observation.signal_state;
    report.signal_os_error = observation.signal_os_error;
    report.code = match report.signal_state.as_str() {
        "pending" => "observed_pending",
        "received" => "observed_signal",
        _ => "signal_registration_failed",
    }
    .into();
    ChildCleanup::Reaped
}
#[cfg(windows)]
pub async fn run() -> LaunchProbeReport {
    let root = std::env::temp_dir().join(format!("Nexa launch probe {}", uuid::Uuid::new_v4()));
    let mut report = LaunchProbeReport::empty("probe_io_failed");
    if let Err(error) = runtime_api::token::create_private_dir(&root) {
        report.os_error = error.raw_os_error();
        report.cleanup_confirmed = !root.exists();
        return report;
    }
    let child = observe(&root, &mut report).await;
    finish_cleanup(&root, child, &mut report);
    report
}
#[cfg(not(windows))]
pub async fn run() -> LaunchProbeReport {
    let mut report = LaunchProbeReport::empty("unsupported_platform");
    report.cleanup_confirmed = true;
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unconfirmed_child_reaping_retains_directory_and_never_reports_clean() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private-probe");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("retained-evidence"), b"fixture").unwrap();
        let mut report = LaunchProbeReport::empty("observed_pending");
        report.signal_state = "pending".into();
        report.child_exit_code = Some(0);
        finish_cleanup(&root, ChildCleanup::Unconfirmed, &mut report);
        assert!(!report.cleanup_confirmed);
        assert!(!report.success);
        assert!(root.join("retained-evidence").exists());
        assert!(report.validate());
    }
    #[test]
    fn no_child_or_confirmed_reaping_can_remove_owned_directory() {
        for state in [ChildCleanup::NoChild, ChildCleanup::Reaped] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("private-probe");
            std::fs::create_dir(&root).unwrap();
            let mut report = LaunchProbeReport::empty("spawn_failed");
            finish_cleanup(&root, state, &mut report);
            assert!(report.cleanup_confirmed);
            assert!(!root.exists());
            assert!(!report.success);
        }
    }
    #[test]
    fn probe_schema_distinguishes_signal_error_spawn_error_and_pending() {
        let mut report = LaunchProbeReport::empty("signal_registration_failed");
        report.signal_state = "error".into();
        report.signal_os_error = Some(6);
        report.child_exit_code = Some(0);
        report.cleanup_confirmed = true;
        assert!(report.validate());
        assert!(!report.success);
        report.code = "spawn_failed".into();
        report.signal_state = "not_observed".into();
        report.signal_os_error = None;
        report.spawn_os_error = Some(5);
        report.child_exit_code = None;
        assert!(report.validate());
        report.code = "observed_pending".into();
        report.signal_state = "pending".into();
        report.spawn_os_error = None;
        report.child_exit_code = Some(0);
        report.success = true;
        assert!(report.validate());
        report.child_exit_code = None;
        assert!(!report.validate());
    }
}
