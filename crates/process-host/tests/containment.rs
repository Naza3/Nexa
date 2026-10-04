//! These execute an actual separate management process. Windows tests cover
//! arbitrary descendant cleanup on abrupt parent exit; Linux only claims the
//! worker PDEATHSIG guarantee and normal process-group descendant teardown.
use std::{
    path::Path,
    process::Command,
    thread,
    time::{Duration, Instant},
};
fn fixture() -> &'static str {
    env!("CARGO_BIN_EXE_nexa-fault-worker")
}
fn harness() -> &'static str {
    env!("CARGO_BIN_EXE_nexa-process-harness")
}
#[cfg(target_os = "linux")]
fn alive(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .is_some_and(|stat| {
            stat.rsplit_once(')')
                .is_some_and(|(_, rest)| !rest.trim_start().starts_with('Z'))
        })
}
#[cfg(windows)]
fn alive(pid: u32) -> bool {
    use std::{
        ffi::c_void,
        os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    };
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn WaitForSingleObject(handle: *mut c_void, timeout: u32) -> u32;
    }
    // SAFETY: query/synchronization handle only; always closed via OwnedHandle.
    let raw = unsafe { OpenProcess(0x0010_0000, 0, pid) };
    if raw.is_null() {
        assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(87));
        return false;
    }
    let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
    match unsafe { WaitForSingleObject(handle.as_raw_handle(), 0) } {
        0 => false,
        258 => true,
        other => panic!("unexpected process wait result {other}"),
    }
}
fn assert_gone(path: &Path) {
    let pid: u32 = std::fs::read_to_string(path).unwrap().parse().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while alive(pid) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(!alive(pid), "contained process {pid} remained alive");
}
#[test]
fn normal_parent_shutdown_reaps_worker_and_kills_descendant() {
    let temp = tempfile::tempdir().unwrap();
    let pid = temp.path().join("parent-normal");
    let status = Command::new(harness())
        .args(["parent-exit", fixture()])
        .arg(&pid)
        .arg("normal")
        .status()
        .unwrap();
    assert!(status.success());
    assert_gone(&pid);
    assert_gone(&temp.path().join("parent-normal.descendant"));
}
#[test]
fn abrupt_parent_exit_kills_worker() {
    let temp = tempfile::tempdir().unwrap();
    let pid = temp.path().join("parent-abrupt");
    let status = Command::new(harness())
        .args(["parent-exit", fixture()])
        .arg(&pid)
        .arg("abrupt")
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(73));
    assert_gone(&pid);
    #[cfg(windows)]
    assert_gone(&temp.path().join("parent-abrupt.descendant"));
}
#[test]
fn death_during_startup_never_leaves_a_worker() {
    for delay in [0, 1, 2, 5, 10, 20, 40] {
        let temp = tempfile::tempdir().unwrap();
        let pid = temp.path().join("startup");
        let status = Command::new(harness())
            .args(["startup-exit", fixture()])
            .arg(&pid)
            .arg(delay.to_string())
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(74));
        // A successfully spawned process may not have run user code before the
        // OS killed it. A present marker proves startup actually occurred.
        if pid.exists() {
            assert_gone(&pid);
        }
    }
}
#[test]
fn paths_and_arguments_with_spaces_and_unicode_are_preserved() {
    use process_host::{ProcessHost, ProcessHostConfig};
    use runtime_core::Runtime;
    use runtime_types::*;
    let temp = tempfile::tempdir().unwrap();
    let executable = temp.path().join(if cfg!(windows) {
        "worker 文本 with spaces.exe"
    } else {
        "worker 文本 with spaces"
    });
    std::fs::copy(fixture(), &executable).unwrap();
    let pid = temp.path().join("argument 文本 with spaces");
    let mut config = ProcessHostConfig::new(executable);
    config.worker_args = vec!["normal".into(), pid.clone().into_os_string()];
    let host = ProcessHost::new(config).unwrap();
    let runtime = Runtime::spawn(
        RuntimeConfig::default(),
        |id: &ModelId| {
            Ok(ResolvedModel {
                id: id.clone(),
                path: "unused.gguf".into(),
                context_limit: 4096,
                default_context: 4096,
                loadable: true,
            })
        },
        host,
    )
    .unwrap();
    runtime
        .handle()
        .load(ModelId::new("fixture").unwrap(), LoadOptions::default())
        .unwrap();
    runtime.shutdown().unwrap();
    assert_gone(&pid);
}
