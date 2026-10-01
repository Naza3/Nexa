//! Owned acceptance children publish one private regular file. Completion never
//! depends on stdout EOF: a surviving runtime may inherit unrelated handles.
use super::{Cleanup, FailureReport, Fault, Result, report};
use report::ReapObservation;
use runtime_api::token::{create_private_dir, write_private_new};
use std::{
    ffi::OsString,
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    time::Duration,
};
use uuid::Uuid;
const REPORT_FILE: &str = "report.json";
const DIRECTORY_PREFIX: &str = ".lifecycle-report-";

pub fn report_directory(args: &[OsString]) -> Option<PathBuf> {
    if args.len() != 8 || args[6] != "--report-dir" {
        return None;
    }
    let root = Path::new(&args[4]);
    let directory = Path::new(&args[7]);
    if !root.is_absolute() || directory.parent()? != root {
        return None;
    }
    Uuid::parse_str(
        directory
            .file_name()?
            .to_str()?
            .strip_prefix(DIRECTORY_PREFIX)?,
    )
    .ok()?;
    Some(directory.to_owned())
}
pub fn publish_report(
    directory: &Path,
    outcome: &std::result::Result<(), Box<FailureReport>>,
) -> std::io::Result<()> {
    create_private_dir(directory)?;
    let bytes = match outcome {
        Ok(()) => report::CHILD_SUCCESS.as_bytes().to_vec(),
        Err(failure) if failure.validate() => serde_json::to_vec(failure)
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidData))?,
        Err(_) => return Err(std::io::ErrorKind::InvalidData.into()),
    };
    if bytes.len() > report::MAX_REPORT_BYTES {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    write_private_new(&directory.join(REPORT_FILE), &bytes)
}
fn read_report(directory: &Path) -> Result<Vec<u8>> {
    let path = directory.join(REPORT_FILE);
    let metadata = std::fs::symlink_metadata(&path)?;
    if !metadata.file_type().is_file() {
        return Err(Fault::new("child_report_invalid"));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(Fault::new("child_report_invalid"));
        }
    }
    let mut bytes = Vec::new();
    // Unlike a pipe, a regular file reaches its current EOF even when another
    // process holds an open writer. Never wait for that writer to close.
    std::fs::File::open(path)?
        .take((report::MAX_REPORT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > report::MAX_REPORT_BYTES {
        return Err(Fault::new("child_report_invalid"));
    }
    Ok(bytes)
}
fn decode_report(directory: &Path, status: ExitStatus) -> Result<()> {
    let bytes = read_report(directory).map_err(|fault| Fault {
        child_exit_code: status.code(),
        ..fault
    })?;
    if status.success() {
        return if std::str::from_utf8(&bytes).ok() == Some(report::CHILD_SUCCESS) {
            Ok(())
        } else {
            Err(Fault {
                child_exit_code: status.code(),
                ..Fault::new("child_report_invalid")
            })
        };
    }
    let failure = FailureReport::parse(&bytes).map_err(|fault| Fault {
        child_exit_code: status.code(),
        ..fault
    })?;
    if !failure.stage.starts_with("child_")
        || failure.child_stage.is_some()
        || failure.cleanup.status != "not_needed"
    {
        return Err(Fault {
            child_exit_code: status.code(),
            ..Fault::new("child_report_invalid")
        });
    }
    Err(failure.into_child_fault(status.code()))
}
async fn reap_after_failure(child: &mut Child, fault: &mut Fault) {
    let kill_error = child.kill().err();
    let mut observation = ReapObservation {
        confirmed: false,
        kill_failed: kill_error.is_some(),
        os_error: kill_error.and_then(|error| error.raw_os_error()),
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                observation.confirmed = true;
                fault.child_exit_code = status.code();
                break;
            }
            Ok(None) => (),
            Err(error) => {
                if observation.os_error.is_none() {
                    observation.os_error = error.raw_os_error();
                }
                break;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    fault.child_reap = Some(observation);
}
async fn wait_report(child: &mut Child, directory: &Path, maximum: Duration) -> Result<()> {
    let deadline = tokio::time::Instant::now() + maximum;
    let failure = loop {
        match child.try_wait() {
            Ok(Some(status)) => return decode_report(directory, status),
            Ok(None) => (),
            Err(error) => break Fault::io(error),
        }
        if tokio::time::Instant::now() >= deadline {
            break Fault::new("timeout");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    };
    let mut failure = failure;
    reap_after_failure(child, &mut failure).await;
    Err(failure)
}
pub async fn lifecycle_child(runtime: &Path, root: &Path, stop: bool) -> Result<()> {
    let directory = root.join(format!("{DIRECTORY_PREFIX}{}", Uuid::new_v4()));
    create_private_dir(&directory)?;
    let mut child = Command::new(std::env::current_exe()?)
        .arg("--lifecycle-child")
        .arg("--runtime")
        .arg(runtime)
        .arg("--data-dir")
        .arg(root)
        .arg(if stop { "stop" } else { "keep" })
        .arg("--report-dir")
        .arg(&directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| Fault {
            code: "child_spawn_failed",
            ..Fault::io(error)
        })?;
    // Reports remain inside the private root until its final confirmed cleanup.
    // Failure retains that root, including the original single-use report.
    wait_report(&mut child, &directory, Duration::from_secs(90)).await
}
pub fn apply_reap_observation(fault: &Fault, cleanup: &mut Cleanup) {
    if let Some(observation) = &fault.child_reap {
        if !observation.confirmed {
            cleanup.status = "unconfirmed".into();
            cleanup.code = Some("cleanup_unconfirmed".into());
            cleanup.os_error = observation.os_error;
            cleanup.temporary_data_retained = true;
        } else if cleanup.code.is_none() && observation.kill_failed {
            // Even when a later wait confirms exit, retain a failed kill's
            // numeric observation separately from the original failure.
            cleanup.code = Some("io_error".into());
            cleanup.os_error = observation.os_error;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const FIXTURE_ROOT: &str = "NEXA_LIFECYCLE_REPORT_TEST_ROOT";
    fn fixture_command(test: &str, root: &Path) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", test, "--nocapture", "--test-threads=1"])
            .env(FIXTURE_ROOT, root)
            .stdin(Stdio::null())
            .stderr(Stdio::null());
        command
    }
    fn await_file(path: &Path, maximum: Duration) -> bool {
        let deadline = std::time::Instant::now() + maximum;
        while !path.exists() {
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        true
    }
    // Fixed test-binary children, inert during normal test discovery. No model,
    // credentials, shell commands, arbitrary executable, or production mode.
    #[test]
    fn fixture_writer() {
        let Some(root) = std::env::var_os(FIXTURE_ROOT).map(PathBuf::from) else {
            return;
        };
        std::fs::write(root.join("writer-ready"), b"ready").unwrap();
        let _ = await_file(&root.join("release-writer"), Duration::from_secs(3));
        std::fs::write(root.join("writer-done"), b"done").unwrap();
    }
    #[test]
    fn fixture_child() {
        let Some(root) = std::env::var_os(FIXTURE_ROOT).map(PathBuf::from) else {
            return;
        };
        let mut descendant = fixture_command("lifecycle::tests::fixture_writer", &root)
            .stdout(Stdio::inherit())
            .spawn()
            .unwrap();
        // Reap if it exits early; this short-lived reporting process otherwise
        // ends before its bounded writer fixture, as required by this test.
        std::thread::spawn(move || {
            let _ = descendant.wait();
        });
        assert!(await_file(
            &root.join("writer-ready"),
            Duration::from_secs(2)
        ));
        publish_report(&root, &Ok(())).unwrap();
        // The controlled descendant still owns stdout after this process exits.
    }
    #[tokio::test]
    async fn report_completes_after_child_exit_while_descendant_holds_stdout() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private-report");
        create_private_dir(&root).unwrap();
        let mut child = fixture_command("lifecycle::tests::fixture_child", &root)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let result = wait_report(&mut child, &root, Duration::from_secs(3)).await;
        let still_held = root.join("writer-ready").exists() && !root.join("writer-done").exists();
        // Release the fixture even if the assertion fails. No EOF read/join.
        std::fs::write(root.join("release-writer"), b"release").unwrap();
        assert!(await_file(
            &root.join("writer-done"),
            Duration::from_secs(2)
        ));
        assert!(
            result.is_ok(),
            "file report should finish without stdout EOF"
        );
        assert!(still_held, "writer must still be held at report completion");
        assert!(child.try_wait().unwrap().unwrap().success());
    }
    #[tokio::test]
    async fn child_timeout_keeps_original_fault_and_confirms_owned_reaping() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private-report");
        create_private_dir(&root).unwrap();
        let mut child = fixture_command("lifecycle::tests::fixture_writer", &root)
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let fault = wait_report(&mut child, &root, Duration::from_millis(50))
            .await
            .unwrap_err();
        assert_eq!(fault.code, "timeout");
        assert!(fault.child_reap.unwrap().confirmed);
        assert!(child.try_wait().unwrap().is_some());
        assert!(root.exists());
    }
    #[test]
    fn unconfirmed_owned_child_forces_retention_without_changing_original_fault() {
        let mut fault = Fault::new("timeout");
        fault.child_reap = Some(ReapObservation {
            confirmed: false,
            kill_failed: true,
            os_error: Some(5),
        });
        let mut cleanup = Cleanup::not_needed(true);
        cleanup.status = "confirmed".into();
        apply_reap_observation(&fault, &mut cleanup);
        assert_eq!(fault.code, "timeout");
        assert_eq!(cleanup.status, "unconfirmed");
        assert_eq!(cleanup.code.as_deref(), Some("cleanup_unconfirmed"));
        assert_eq!(cleanup.os_error, Some(5));
        assert!(cleanup.temporary_data_retained);
    }
    #[test]
    fn failed_kill_observation_survives_later_confirmed_exit() {
        let mut fault = Fault::new("timeout");
        fault.child_reap = Some(ReapObservation {
            confirmed: true,
            kill_failed: true,
            os_error: Some(5),
        });
        let mut cleanup = Cleanup::not_needed(true);
        cleanup.status = "confirmed".into();
        apply_reap_observation(&fault, &mut cleanup);
        assert_eq!(cleanup.status, "confirmed");
        assert_eq!(cleanup.code.as_deref(), Some("io_error"));
        assert_eq!(cleanup.os_error, Some(5));
        assert_eq!(fault.code, "timeout");
    }
    #[test]
    fn private_report_is_single_use_bounded_regular_and_strict() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private-report");
        create_private_dir(&root).unwrap();
        assert!(read_report(&root).is_err());
        publish_report(&root, &Ok(())).unwrap();
        assert_eq!(
            read_report(&root).unwrap(),
            report::CHILD_SUCCESS.as_bytes()
        );
        assert!(publish_report(&root, &Ok(())).is_err());
        std::fs::write(
            root.join(REPORT_FILE),
            vec![b' '; report::MAX_REPORT_BYTES + 1],
        )
        .unwrap();
        assert_eq!(read_report(&root).unwrap_err().code, "child_report_invalid");
        std::fs::remove_file(root.join(REPORT_FILE)).unwrap();
        std::fs::create_dir(root.join(REPORT_FILE)).unwrap();
        assert_eq!(read_report(&root).unwrap_err().code, "child_report_invalid");
        // Failure parsing still rejects duplicates, missing/extra keys and
        // unknown strings before any diagnostic can be forwarded.
        assert!(FailureReport::parse(br#"{"stage":"child_start","stage":"SECRET"}"#).is_err());
    }
    fn exit_status(code: u32) -> ExitStatus {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            ExitStatus::from_raw((code as i32) << 8)
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::ExitStatusExt;
            ExitStatus::from_raw(code)
        }
    }
    #[test]
    fn file_failure_preserves_child_stage_and_rejects_untrusted_fields() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private-report");
        let failure = FailureReport::new(
            "child_start",
            Fault::new("invalid_arguments"),
            Cleanup::not_needed(true),
        );
        publish_report(&root, &Err(Box::new(failure.clone()))).unwrap();
        let fault = decode_report(&root, exit_status(1)).unwrap_err();
        assert_eq!(fault.code, "invalid_arguments");
        assert_eq!(fault.child_stage.as_deref(), Some("child_start"));
        assert_eq!(fault.child_exit_code, Some(1));
        let original = serde_json::to_value(failure).unwrap();
        for (field, value) in [
            ("message", serde_json::json!("PRIVATE_FILE_CONTENT")),
            ("stage", serde_json::json!("unknown_stage")),
            ("code", serde_json::json!("unknown_code")),
        ] {
            let mut altered = original.clone();
            altered[field] = value;
            std::fs::write(
                root.join(REPORT_FILE),
                serde_json::to_vec(&altered).unwrap(),
            )
            .unwrap();
            assert_eq!(
                decode_report(&root, exit_status(1)).unwrap_err().code,
                "child_report_invalid"
            );
        }
        let mut missing = original;
        missing.as_object_mut().unwrap().remove("os_error");
        std::fs::write(
            root.join(REPORT_FILE),
            serde_json::to_vec(&missing).unwrap(),
        )
        .unwrap();
        assert_eq!(
            decode_report(&root, exit_status(1)).unwrap_err().code,
            "child_report_invalid"
        );
        std::fs::write(
            root.join(REPORT_FILE),
            b"{\"success\":true,\"message\":\"PRIVATE\"}",
        )
        .unwrap();
        assert_eq!(
            decode_report(&root, exit_status(0)).unwrap_err().code,
            "child_report_invalid"
        );
    }
    #[cfg(unix)]
    #[test]
    fn private_report_rejects_symlink() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private-report");
        create_private_dir(&root).unwrap();
        let other = temp.path().join("other");
        std::fs::write(&other, report::CHILD_SUCCESS).unwrap();
        std::os::unix::fs::symlink(other, root.join(REPORT_FILE)).unwrap();
        assert_eq!(read_report(&root).unwrap_err().code, "child_report_invalid");
    }
}
