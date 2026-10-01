//! Observation-only Windows console registration probe. It loads no model,
//! initializes no token, and does not change the runtime or fallback flags.
use super::report::{LaunchProbeReport, LaunchStrategy};
#[cfg(any(windows, test))]
use super::report::{MAX_REPORT_BYTES, SIGNAL_STATES};
#[cfg(any(windows, test))]
use serde::{Deserialize, Serialize};
#[cfg(windows)]
use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

#[cfg(any(windows, test))]
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignalObservation {
    signal_state: String,
    signal_os_error: Option<i32>,
    child_in_job: Option<bool>,
    child_job_os_error: Option<i32>,
}
#[cfg(any(windows, test))]
impl SignalObservation {
    fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > MAX_REPORT_BYTES {
            return None;
        }
        let value = runtime_api::dto::parse_json(bytes).ok()?;
        let keys = [
            "signal_state",
            "signal_os_error",
            "child_in_job",
            "child_job_os_error",
        ];
        let object = value.as_object()?;
        if object.len() != keys.len() || !object.keys().all(|key| keys.contains(&key.as_str())) {
            return None;
        }
        let observation: Self = serde_json::from_value(value).ok()?;
        (SIGNAL_STATES.contains(&observation.signal_state.as_str())
            && observation.signal_state != "not_observed"
            && (observation.signal_state == "error" || observation.signal_os_error.is_none())
            && !(observation.child_in_job.is_some() && observation.child_job_os_error.is_some()))
        .then_some(observation)
    }
}
#[cfg(windows)]
fn current_job() -> (Option<bool>, Option<i32>) {
    use windows_sys::Win32::System::{JobObjects::IsProcessInJob, Threading::GetCurrentProcess};
    let mut in_job = 0;
    // SAFETY: the pseudo handle names this process, NULL asks about any Job,
    // and the output points to a live BOOL. This query mutates no Job state.
    let result = unsafe { IsProcessInJob(GetCurrentProcess(), std::ptr::null_mut(), &mut in_job) };
    if result != 0 {
        (Some(in_job != 0), None)
    } else {
        (None, std::io::Error::last_os_error().raw_os_error())
    }
}

#[cfg(windows)]
pub async fn child(root: &Path) -> bool {
    if !root.is_absolute() || runtime_api::token::create_private_dir(root).is_err() {
        return false;
    }
    let (child_in_job, child_job_os_error) = current_job();
    let result = tokio::time::timeout(Duration::from_millis(250), tokio::signal::ctrl_c()).await;
    let observation = match result {
        Err(_) => SignalObservation {
            signal_state: "pending".into(),
            signal_os_error: None,
            child_in_job,
            child_job_os_error,
        },
        Ok(Ok(())) => SignalObservation {
            signal_state: "received".into(),
            signal_os_error: None,
            child_in_job,
            child_job_os_error,
        },
        Ok(Err(error)) => SignalObservation {
            signal_state: "error".into(),
            signal_os_error: error.raw_os_error(),
            child_in_job,
            child_job_os_error,
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
    report.success = report.pending_confirmed();
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
async fn observe(
    root: &Path,
    strategy: LaunchStrategy,
    report: &mut LaunchProbeReport,
) -> ChildCleanup {
    use std::os::windows::process::CommandExt;
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
    command.creation_flags(strategy.creation_flags());
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
    let Some(observation) = SignalObservation::parse(&bytes) else {
        report.code = "invalid_report".into();
        return ChildCleanup::Reaped;
    };
    report.child_in_job = observation.child_in_job;
    report.child_job_os_error = observation.child_job_os_error;
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
pub async fn run(strategy: LaunchStrategy) -> LaunchProbeReport {
    let root = std::env::temp_dir().join(format!("Nexa launch probe {}", uuid::Uuid::new_v4()));
    let mut report = LaunchProbeReport::empty("probe_io_failed", strategy);
    (report.parent_in_job, report.parent_job_os_error) = current_job();
    if let Err(error) = runtime_api::token::create_private_dir(&root) {
        report.os_error = error.raw_os_error();
        report.cleanup_confirmed = !root.exists();
        return report;
    }
    let child = observe(&root, strategy, &mut report).await;
    finish_cleanup(&root, child, &mut report);
    report
}
#[cfg(not(windows))]
pub async fn run(strategy: LaunchStrategy) -> LaunchProbeReport {
    let mut report = LaunchProbeReport::empty("unsupported_platform", strategy);
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
        let mut report = LaunchProbeReport::empty("observed_pending", LaunchStrategy::Breakaway);
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
            let mut report = LaunchProbeReport::empty("spawn_failed", LaunchStrategy::Breakaway);
            finish_cleanup(&root, state, &mut report);
            assert!(report.cleanup_confirmed);
            assert!(!root.exists());
            assert!(!report.success);
        }
    }
    #[test]
    fn probe_schema_distinguishes_signal_error_spawn_error_and_pending() {
        let mut report =
            LaunchProbeReport::empty("signal_registration_failed", LaunchStrategy::Breakaway);
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
        report.parent_in_job = Some(true);
        report.child_in_job = Some(false);
        report.success = true;
        assert!(report.validate());
        report.child_exit_code = None;
        assert!(!report.validate());
    }
    #[test]
    fn strategies_have_explicit_bounded_v2_reports() {
        for strategy in [LaunchStrategy::Breakaway, LaunchStrategy::InheritJob] {
            assert_eq!(
                LaunchStrategy::parse(std::ffi::OsStr::new(strategy.as_str())),
                Some(strategy)
            );
            let mut report = LaunchProbeReport::empty("unsupported_platform", strategy);
            report.cleanup_confirmed = true;
            let bytes = report.encode().unwrap();
            let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(value["schema_version"], 2);
            assert_eq!(value["strategy"], strategy.as_str());
            assert_eq!(
                value.as_object().unwrap().len(),
                super::super::report::PROBE_KEYS.len()
            );
            assert!(bytes.len() <= MAX_REPORT_BYTES);
            assert!(!report.success);
            report.strategy = "arbitrary or secret strategy".into();
            assert!(report.encode().is_none());
        }
        assert!(LaunchStrategy::parse(std::ffi::OsStr::new("unknown")).is_none());
    }
    #[test]
    fn unknown_job_observation_cannot_claim_pending_success() {
        let mut report = LaunchProbeReport::empty("observed_pending", LaunchStrategy::InheritJob);
        report.signal_state = "pending".into();
        report.child_exit_code = Some(0);
        report.cleanup_confirmed = true;
        report.success = true;
        assert!(!report.validate());
        report.parent_in_job = Some(true);
        report.child_in_job = Some(true);
        assert!(report.validate());
        report.child_job_os_error = Some(5);
        assert!(!report.validate());
        report.child_in_job = None;
        report.success = false;
        assert!(report.validate());
        assert!(!report.pending_confirmed());
    }
    #[test]
    fn private_child_observation_is_closed_bounded_and_typed() {
        let original = serde_json::json!({
            "signal_state":"error", "signal_os_error":6,
            "child_in_job":true,"child_job_os_error":null
        });
        let observation =
            SignalObservation::parse(&serde_json::to_vec(&original).unwrap()).unwrap();
        assert_eq!(observation.child_in_job, Some(true));
        assert_eq!(observation.signal_os_error, Some(6));
        for (key, value) in [
            ("message", serde_json::json!("SECRET /private/path")),
            ("signal_state", serde_json::json!("not_observed")),
            ("signal_os_error", serde_json::json!(2147483648_u64)),
            ("child_in_job", serde_json::json!(1)),
            ("child_job_os_error", serde_json::json!(5)),
        ] {
            let mut altered = original.clone();
            altered[key] = value;
            assert!(SignalObservation::parse(&serde_json::to_vec(&altered).unwrap()).is_none());
        }
        let mut missing = original;
        missing
            .as_object_mut()
            .unwrap()
            .remove("child_job_os_error");
        assert!(SignalObservation::parse(&serde_json::to_vec(&missing).unwrap()).is_none());
        assert!(
            SignalObservation::parse(br#"{"signal_state":"error","signal_state":"pending"}"#)
                .is_none()
        );
        assert!(SignalObservation::parse(&vec![b' '; MAX_REPORT_BYTES + 1]).is_none());
    }
    #[cfg(windows)]
    #[test]
    fn comparison_changes_only_breakaway_flag() {
        use windows_sys::Win32::System::Threading::{
            CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS,
        };
        assert_eq!(
            LaunchStrategy::Breakaway.creation_flags(),
            CREATE_BREAKAWAY_FROM_JOB | CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS
        );
        assert_eq!(
            LaunchStrategy::InheritJob.creation_flags(),
            CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS
        );
        assert_eq!(
            LaunchStrategy::Breakaway.creation_flags()
                ^ LaunchStrategy::InheritJob.creation_flags(),
            CREATE_BREAKAWAY_FROM_JOB
        );
    }
}
