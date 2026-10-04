use crate::*;
#[cfg(any(windows, all(unix, test)))]
use crate::{
    output::{self, OutputState},
    platform::Child,
};
#[cfg(any(windows, all(unix, test)))]
use std::ffi::OsString;
use std::time::{Duration, Instant};
use tokio::sync::watch;

#[cfg(any(windows, all(unix, test)))]
struct Launch {
    executable: std::path::PathBuf,
    args: Vec<OsString>,
    directory: std::path::PathBuf,
    environment: Vec<(OsString, OsString)>,
}
fn validate(
    spec: &DownloadSpec,
    sidecar: &SidecarConfig,
    staging: &StagingPaths,
    control: &DownloadControl,
    options: TransferOptions,
) -> Result<(), DownloadError> {
    if !(4..=17_179_869_184).contains(&spec.expected_size)
        || spec.url.as_str().len() > 16384
        || spec.url.scheme() != "https"
        || spec.url.port_or_known_default() != Some(443)
        || !spec.url.username().is_empty()
        || spec.url.password().is_some()
        || spec.url.fragment().is_some()
        || spec.url.host_str().is_none()
        || spec.url.as_str().bytes().any(|b| b <= b' ' || b == b'\\')
    {
        return Err(DownloadError::InvalidSpec);
    }
    if !sidecar.executable.is_absolute()
        || !staging.directory.is_absolute()
        || staging.file_name != "payload.part"
        || !matches!(options.attempt, 1 | 2)
        || control.deadline() > Instant::now() + Duration::from_secs(7200)
    {
        return Err(DownloadError::InvalidOptions);
    }
    if control.is_cancelled() {
        return Err(DownloadError::Cancelled);
    }
    if control.is_expired() {
        return Err(DownloadError::Timeout);
    }
    Ok(())
}
#[cfg(any(windows, all(unix, test)))]
fn launch(
    spec: &DownloadSpec,
    sidecar: SidecarConfig,
    staging: StagingPaths,
) -> Result<Launch, DownloadError> {
    let mut args: Vec<OsString> = [
        "--no-conf=true",
        "--no-netrc=true",
        "--enable-rpc=false",
        "--check-certificate=true",
        "--split=1",
        "--max-connection-per-server=1",
        "--file-allocation=none",
        "--auto-file-renaming=false",
        "--allow-overwrite=false",
        "--continue=true",
        "--always-resume=true",
        "--max-tries=3",
        "--retry-wait=1",
        "--connect-timeout=15",
        "--timeout=30",
        "--auto-save-interval=1",
        "--human-readable=false",
        "--enable-color=false",
        "--truncate-console-readout=false",
        "--summary-interval=1",
        "--console-log-level=notice",
        "--download-result=hide",
        "--check-integrity=true",
        "--http-accept-gzip=false",
        "--enable-mmap=false",
        "--disk-cache=0",
        "--max-concurrent-downloads=1",
        "--max-file-not-found=1",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    let hash = spec
        .sha256
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    args.push(format!("--checksum=sha-256={hash}").into());
    let mut directory = OsString::from("--dir=");
    directory.push(&staging.directory);
    args.push(directory);
    args.push("--out=payload.part".into());
    // End option parsing even though source adapters only return HTTPS URLs.
    args.push("--".into());
    args.push(spec.url.as_str().into());
    let environment = environment(spec.expected_size)?;
    Ok(Launch {
        executable: sidecar.executable,
        args,
        directory: staging.directory,
        environment,
    })
}
#[cfg(any(windows, all(unix, test)))]
fn environment(expected_size: u64) -> Result<Vec<(OsString, OsString)>, DownloadError> {
    if !(4..=17_179_869_184).contains(&expected_size) {
        return Err(DownloadError::InvalidSpec);
    }
    // Explicitly construct this value from the trusted spec. Never read or
    // merge an ambient NEXA_PAYLOAD_MAX_BYTES value.
    let limit = (
        OsString::from("NEXA_PAYLOAD_MAX_BYTES"),
        OsString::from(expected_size.to_string()),
    );
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        let mut buffer = vec![0u16; 32768];
        // SAFETY: the writable buffer and its advertised length agree.
        let length = unsafe {
            windows_sys::Win32::System::SystemInformation::GetWindowsDirectoryW(
                buffer.as_mut_ptr(),
                buffer.len() as u32,
            )
        } as usize;
        if length == 0 || length >= buffer.len() {
            return Err(DownloadError::SpawnFailed);
        }
        Ok(vec![
            limit,
            ("SystemRoot".into(), OsString::from_wide(&buffer[..length])),
        ])
    }
    #[cfg(not(windows))]
    {
        Ok(vec![limit])
    }
}
/// Every normal return, including an error, follows confirmed process-tree exit
/// and both pipe drainers ending. CleanupUnconfirmed is the only exception and
/// must leave the caller's staging transaction marked writer-active.
///
/// The caller MUST keep its executable/staging/instance leases alive while
/// awaiting this future. Do not abort it when a UI close timeout expires.
pub async fn transfer(
    spec: DownloadSpec,
    sidecar: SidecarConfig,
    staging: StagingPaths,
    control: DownloadControl,
    options: TransferOptions,
    progress: watch::Sender<DownloadProgress>,
) -> Result<TransferReport, DownloadError> {
    validate(&spec, &sidecar, &staging, &control, options)?;
    progress.send_replace(DownloadProgress {
        attempt: options.attempt,
        ..DownloadProgress::initial(spec.expected_size)
    });
    #[cfg(any(windows, all(unix, test)))]
    {
        let launch = launch(&spec, sidecar, staging)?;
        tokio::task::spawn_blocking(move || supervise(launch, control, progress))
            .await
            .map_err(|_| DownloadError::CleanupUnconfirmed)?
    }
    #[cfg(not(any(windows, all(unix, test))))]
    {
        let _ = (sidecar, staging, control, progress);
        Err(DownloadError::UnsupportedPlatform)
    }
}
#[cfg(any(windows, all(unix, test)))]
struct Drainers {
    handles: Vec<std::thread::JoinHandle<std::io::Result<()>>>,
}
#[cfg(any(windows, all(unix, test)))]
impl Drainers {
    fn join(&mut self) -> bool {
        let mut ok = true;
        for thread in self.handles.drain(..) {
            ok &= matches!(thread.join(), Ok(Ok(())));
        }
        ok
    }
}
#[cfg(any(windows, all(unix, test)))]
impl Drop for Drainers {
    fn drop(&mut self) {
        let _ = self.join();
    }
}
#[cfg(any(windows, all(unix, test)))]
fn supervise(
    launch: Launch,
    control: DownloadControl,
    progress: watch::Sender<DownloadProgress>,
) -> Result<TransferReport, DownloadError> {
    // Reverse drop order is intentional: on unwind kill/reap the Job before
    // joining pipe readers. Otherwise a live writer could deadlock cleanup.
    let mut drainers = Drainers {
        handles: Vec::with_capacity(2),
    };
    let mut child = Child::spawn(
        &launch.executable,
        &launch.args,
        &launch.directory,
        &launch.environment,
    )
    .map_err(|_| DownloadError::SpawnFailed)?;
    let state = OutputState::default();
    let mut stop = None;
    for pipe in [child.stdout.take(), child.stderr.take()] {
        let Some(pipe) = pipe else {
            stop = Some(DownloadError::PipeFailed);
            break;
        };
        let state = state.clone();
        let progress = progress.clone();
        match std::thread::Builder::new()
            .name("download-output".into())
            .spawn(move || output::drain(pipe, state, progress))
        {
            Ok(handle) => drainers.handles.push(handle),
            Err(_) => {
                stop = Some(DownloadError::PipeFailed);
                break;
            }
        }
    }
    let exit_code = loop {
        if stop.is_none() {
            if control.is_cancelled() {
                stop = Some(DownloadError::Cancelled);
            } else if control.is_expired() {
                stop = Some(DownloadError::Timeout);
            }
        }
        if stop.is_some() {
            let _ = child.kill_tree();
        }
        match child.try_wait() {
            Ok(Some(code)) => break code,
            Ok(None) => (),
            Err(_) => {
                stop = Some(DownloadError::CleanupUnconfirmed);
                let _ = child.kill_tree();
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    // A nominally successful root cannot leave descendants writing the staging
    // directory or holding pipe handles. Job assignment was atomic at spawn.
    loop {
        let _ = child.kill_tree();
        if matches!(child.tree_stopped(), Ok(true)) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let pipes_ok = drainers.join();
    // Explicitly finish child ownership before claiming writer_stopped.
    drop(child);
    if let Some(error) = stop {
        return Err(if error == DownloadError::CleanupUnconfirmed {
            DownloadError::PipeFailed
        } else {
            error
        });
    }
    if !pipes_ok {
        return Err(DownloadError::PipeFailed);
    }
    if control.is_cancelled() {
        return Err(DownloadError::Cancelled);
    }
    if control.is_expired() {
        return Err(DownloadError::Timeout);
    }
    if exit_code != 0 {
        return Err(DownloadError::SidecarExit {
            exit_code,
            error_code: state.error_code(),
        });
    }
    Ok(TransferReport {
        exit_code,
        writer_stopped: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn spec() -> DownloadSpec {
        DownloadSpec {
            url: Url::parse("https://models.example.com/revision/model?signature=CANARY_SECRET")
                .unwrap(),
            expected_size: 100,
            sha256: [0xab; 32],
        }
    }
    fn paths() -> (SidecarConfig, StagingPaths) {
        (
            SidecarConfig {
                executable: std::env::current_exe().unwrap(),
            },
            StagingPaths {
                directory: std::env::current_dir().unwrap(),
                file_name: "payload.part".into(),
            },
        )
    }
    fn fake_launch(case: &str) -> Launch {
        let (binary, staging) = paths();
        let mut environment = environment(100).unwrap();
        environment.push(("NEXA_DOWNLOAD_TEST_CASE".into(), case.into()));
        Launch {
            executable: binary.executable,
            args: vec![
                "--exact".into(),
                "supervisor::tests::child_entry".into(),
                "--nocapture".into(),
                "--test-threads=1".into(),
            ],
            directory: staging.directory,
            environment,
        }
    }
    #[test]
    fn child_entry() {
        let Ok(case) = std::env::var("NEXA_DOWNLOAD_TEST_CASE") else {
            return;
        };
        match case.as_str() {
            "success" => {
                println!("\r[#abcdef 50B/100B(50%) CN:1]");
                std::process::exit(0);
            }
            "error8" => {
                eprintln!("Exception: [x:1] errorCode=8 https://CANARY_SECRET/path?secret=1");
                std::process::exit(8);
            }
            "flood" => {
                let data = vec![b'x'; 4096];
                for _ in 0..128 {
                    std::io::stdout().write_all(&data).unwrap();
                }
                for _ in 0..128 {
                    std::io::stderr().write_all(&data).unwrap();
                }
                println!("\r[#abcdef 75B/100B(75%) CN:1]\r");
                std::process::exit(0);
            }
            "environment" => {
                assert_eq!(std::env::var("NEXA_PAYLOAD_MAX_BYTES").unwrap(), "100");
                for key in [
                    "HTTP_PROXY",
                    "HTTPS_PROXY",
                    "ALL_PROXY",
                    "NO_PROXY",
                    "PATH",
                    "HOME",
                    "APPDATA",
                    "LD_PRELOAD",
                    "SSL_CERT_FILE",
                    "SSL_CERT_DIR",
                ] {
                    assert!(
                        std::env::var_os(key).is_none(),
                        "unexpected inherited variable"
                    );
                }
                std::process::exit(0);
            }
            "ambient" => {
                assert_eq!(
                    std::env::var("NEXA_PAYLOAD_MAX_BYTES").unwrap(),
                    "INVALID_AMBIENT"
                );
                let (tx, _) = watch::channel(DownloadProgress::initial(100));
                assert!(
                    supervise(fake_launch("environment"), DownloadControl::new(), tx)
                        .unwrap()
                        .writer_stopped
                );
                std::process::exit(0);
            }
            "sleep" => {
                std::thread::sleep(Duration::from_secs(30));
                std::process::exit(99);
            }
            #[cfg(windows)]
            "descendant" => {
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "supervisor::tests::child_entry",
                        "--nocapture",
                        "--test-threads=1",
                    ])
                    .env("NEXA_DOWNLOAD_TEST_CASE", "sleep")
                    .spawn()
                    .unwrap();
                std::process::exit(0);
            }
            _ => std::process::exit(98),
        }
    }
    #[test]
    fn argv_is_fixed_single_transfer_and_debug_is_private() {
        let spec = spec();
        let (binary, staging) = paths();
        let command = launch(&spec, binary.clone(), staging.clone()).unwrap();
        let args = command
            .args
            .iter()
            .map(|s| s.to_str().unwrap())
            .collect::<Vec<_>>();
        for option in [
            "--no-conf=true",
            "--no-netrc=true",
            "--enable-rpc=false",
            "--check-certificate=true",
            "--allow-overwrite=false",
            "--auto-file-renaming=false",
            "--always-resume=true",
            "--split=1",
            "--max-connection-per-server=1",
            "--max-tries=3",
            "--out=payload.part",
        ] {
            assert!(args.contains(&option), "missing fixed option");
        }
        assert_eq!(args[args.len() - 2], "--");
        assert_eq!(args.last().unwrap(), &spec.url.as_str());
        for prefix in [
            "--header=",
            "--ca-certificate=",
            "--on-download",
            "--input-file=",
            "--load-cookies=",
            "--save-cookies=",
            "--server-stat-if=",
            "--server-stat-of=",
            "--log=",
            "--save-session=",
            "--http-user=",
            "--all-proxy=",
        ] {
            assert!(!args.iter().any(|arg| arg.starts_with(prefix)));
        }
        for rendered in [
            format!("{spec:?}"),
            format!("{binary:?}"),
            format!("{staging:?}"),
            format!(
                "{:?}",
                DownloadError::SidecarExit {
                    exit_code: 8,
                    error_code: Some(8)
                }
            ),
        ] {
            assert!(!rendered.contains("CANARY_SECRET"));
            assert!(!rendered.contains("https://"));
        }
    }

    #[test]
    fn payload_limit_is_canonical_and_bad_sizes_never_spawn() {
        let (binary, staging) = paths();
        let control = DownloadControl::new();
        for value in [4, 100, 17_179_869_184] {
            let env = environment(value).unwrap();
            let values = env
                .iter()
                .filter(|(k, _)| k == "NEXA_PAYLOAD_MAX_BYTES")
                .collect::<Vec<_>>();
            assert_eq!(values.len(), 1);
            assert_eq!(values[0].1.to_str().unwrap(), value.to_string());
            let mut candidate = spec();
            candidate.expected_size = value;
            assert!(
                validate(
                    &candidate,
                    &binary,
                    &staging,
                    &control,
                    TransferOptions::default()
                )
                .is_ok()
            );
        }
        for value in [0, 1, 2, 3, 17_179_869_185, u64::MAX] {
            let mut candidate = spec();
            candidate.expected_size = value;
            assert_eq!(
                validate(
                    &candidate,
                    &binary,
                    &staging,
                    &control,
                    TransferOptions::default()
                ),
                Err(DownloadError::InvalidSpec)
            );
            assert_eq!(environment(value), Err(DownloadError::InvalidSpec));
        }
    }
    #[test]
    fn ambient_payload_limit_is_not_inherited_or_merged() {
        let mut launch = fake_launch("ambient");
        for (key, value) in &mut launch.environment {
            if key == "NEXA_PAYLOAD_MAX_BYTES" {
                *value = "INVALID_AMBIENT".into();
            }
        }
        let (tx, _) = watch::channel(DownloadProgress::initial(100));
        assert!(
            supervise(launch, DownloadControl::new(), tx)
                .unwrap()
                .writer_stopped
        );
    }
    #[test]
    fn only_one_actor_wins_cancellation() {
        let control = DownloadControl::new();
        assert!(control.try_cancel());
        assert!(!control.try_cancel());
        assert!(!control.begin_publish());
        let published = DownloadControl::new();
        assert!(published.begin_publish());
        assert!(!published.try_cancel());
    }
    #[test]
    fn invalid_inputs_never_start_a_process() {
        let (binary, mut staging) = paths();
        let control = DownloadControl::new();
        for url in [
            "http://models.example.com/a",
            "https://user:secret@models.example.com/a",
            "https://models.example.com:444/a",
            "https://models.example.com/a#secret",
        ] {
            let mut spec = spec();
            spec.url = Url::parse(url).unwrap();
            assert_eq!(
                validate(
                    &spec,
                    &binary,
                    &staging,
                    &control,
                    TransferOptions::default()
                ),
                Err(DownloadError::InvalidSpec)
            );
        }
        staging.file_name = "../escape".into();
        assert_eq!(
            validate(
                &spec(),
                &binary,
                &staging,
                &control,
                TransferOptions::default()
            ),
            Err(DownloadError::InvalidOptions)
        );
    }
    #[test]
    fn control_cancellation_publication_and_deadline_are_monotonic() {
        let cancelled = DownloadControl::new();
        cancelled.cancel();
        assert!(!cancelled.begin_publish());
        let published = DownloadControl::new();
        assert!(published.begin_publish());
        published.cancel();
        assert!(!published.is_cancelled());
        assert!(!published.begin_publish());
        let expired = DownloadControl::with_deadline(Instant::now());
        assert!(expired.is_expired());
        assert!(!expired.begin_publish());
    }
    #[test]
    fn success_waits_for_pipe_eof_and_does_not_claim_full_bytes_or_saved() {
        let (tx, rx) = watch::channel(DownloadProgress::initial(100));
        let result = supervise(fake_launch("success"), DownloadControl::new(), tx).unwrap();
        assert!(result.writer_stopped);
        assert_eq!(result.exit_code, 0);
        assert_eq!(rx.borrow().written_bytes, 50);
    }
    #[test]
    fn numeric_error_and_exit_are_separate_and_bounded() {
        let (tx, _) = watch::channel(DownloadProgress::initial(100));
        let error = supervise(fake_launch("error8"), DownloadControl::new(), tx).unwrap_err();
        assert_eq!(
            error,
            DownloadError::SidecarExit {
                exit_code: 8,
                error_code: Some(8)
            }
        );
        assert!(error.writer_stopped());
    }
    #[test]
    fn stdout_and_stderr_flood_are_drained_with_bounded_memory() {
        let (tx, rx) = watch::channel(DownloadProgress::initial(100));
        assert!(
            supervise(fake_launch("flood"), DownloadControl::new(), tx)
                .unwrap()
                .writer_stopped
        );
        assert_eq!(rx.borrow().written_bytes, 75);
    }
    #[test]
    fn child_does_not_inherit_proxy_credentials_or_loader_environment() {
        let (tx, _) = watch::channel(DownloadProgress::initial(100));
        assert!(
            supervise(fake_launch("environment"), DownloadControl::new(), tx)
                .unwrap()
                .writer_stopped
        );
    }
    #[test]
    fn deadline_kills_and_reaps_before_return() {
        let (tx, _) = watch::channel(DownloadProgress::initial(100));
        let start = Instant::now();
        let result = supervise(
            fake_launch("sleep"),
            DownloadControl::with_deadline(start + Duration::from_millis(50)),
            tx,
        );
        assert_eq!(result, Err(DownloadError::Timeout));
        assert!(start.elapsed() < Duration::from_secs(5));
    }
    #[test]
    fn cancellation_kills_and_reaps_before_return() {
        let (tx, _) = watch::channel(DownloadProgress::initial(100));
        let control = DownloadControl::new();
        let to_cancel = control.clone();
        let canceller = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            to_cancel.cancel();
        });
        let result = supervise(fake_launch("sleep"), control, tx);
        canceller.join().unwrap();
        assert_eq!(result, Err(DownloadError::Cancelled));
    }
    #[cfg(windows)]
    #[test]
    fn atomic_job_kills_descendant_and_closes_its_inherited_pipes() {
        let (tx, _) = watch::channel(DownloadProgress::initial(100));
        let start = Instant::now();
        assert!(
            supervise(fake_launch("descendant"), DownloadControl::new(), tx)
                .unwrap()
                .writer_stopped
        );
        assert!(start.elapsed() < Duration::from_secs(5));
    }
}
