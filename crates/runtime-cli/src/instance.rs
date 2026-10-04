use fs2::FileExt;
use runtime_api::token::{create_private_dir, write_private_new};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read},
    net::SocketAddr,
    path::{Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;

pub const DISCOVERY_FILE: &str = "instance.json";
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Discovery {
    pub schema_version: u32,
    pub protocol_version: u32,
    pub instance_id: Uuid,
    pub pid: u32,
    /// OS process creation identity is diagnostic; it never authorizes a kill.
    pub process_created: String,
    pub listen: SocketAddr,
}
impl Discovery {
    pub fn current(instance_id: Uuid, listen: SocketAddr) -> io::Result<Self> {
        Ok(Self {
            schema_version: 1,
            protocol_version: runtime_types::PROTOCOL_VERSION,
            instance_id,
            pid: std::process::id(),
            process_created: creation_identity()?,
            listen,
        })
    }
    pub fn read(data_dir: &Path) -> io::Result<Self> {
        let path = data_dir.join("runtime").join(DISCOVERY_FILE);
        ensure_regular(&path)?;
        let mut bytes = Vec::new();
        File::open(path)?.take(4097).read_to_end(&mut bytes)?;
        if bytes.len() > 4096 {
            return Err(invalid("oversized instance discovery"));
        }
        let value: Self =
            serde_json::from_slice(&bytes).map_err(|_| invalid("invalid instance discovery"))?;
        if value.schema_version != 1
            || value.protocol_version != runtime_types::PROTOCOL_VERSION
            || value.instance_id.is_nil()
            || value.pid == 0
            || value.process_created.is_empty()
            || value.process_created.len() > 256
            || !value.listen.ip().is_loopback()
            || value.listen.port() == 0
        {
            return Err(invalid("unsupported or unsafe instance discovery"));
        }
        Ok(value)
    }
}

pub enum InstanceObservation {
    Stopped(Option<InstanceLock>),
    Running,
}

pub struct InstanceLock {
    _file: File,
    root: PathBuf,
}
impl InstanceLock {
    /// Read-only observation: never creates/chmods state and never treats a
    /// failed HTTP proof as a stopped service. Keep the returned guard alive
    /// while reading an offline inventory. No lock file means no service has
    /// acquired ownership yet; callers must recheck before mutating anything.
    pub fn observe(data_dir: &Path) -> io::Result<InstanceObservation> {
        let root = data_dir.join("runtime");
        if data_dir.try_exists()? {
            model_store::inventory::validate_data_directory(data_dir)
                .map_err(|_| invalid("unsafe data directory"))?;
        }
        if root.try_exists()? {
            model_store::inventory::validate_data_directory(&root)
                .map_err(|_| invalid("unsafe runtime directory"))?;
        }
        let path = root.join("instance.lock");
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                if root.join(DISCOVERY_FILE).try_exists()? {
                    return Err(invalid("discovery without lock"));
                }
                return Ok(InstanceObservation::Stopped(None));
            }
            Err(error) => return Err(error),
            Ok(_) => ensure_regular(&path)?,
        }
        // Exclusive locking needs a write-capable handle on Windows, but this
        // operation never writes, creates, truncates, or changes permissions.
        let file = OpenOptions::new().read(true).write(true).open(&path)?;
        match file.try_lock_exclusive() {
            Ok(()) => {
                if root.join(DISCOVERY_FILE).try_exists()? {
                    return Err(invalid("cleanup unconfirmed"));
                }
                Ok(InstanceObservation::Stopped(Some(Self {
                    _file: file,
                    root,
                })))
            }
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock
                    || error.raw_os_error() == fs2::lock_contended_error().raw_os_error() =>
            {
                Ok(InstanceObservation::Running)
            }
            Err(error) => Err(error),
        }
    }

    /// Lock order: instance first, then model-store. A live service is never
    /// considered stale because of a failed network probe or an old PID.
    pub fn try_acquire(data_dir: &Path) -> io::Result<Option<Self>> {
        create_private_dir(data_dir)?;
        let root = data_dir.join("runtime");
        create_private_dir(&root)?;
        let path = root.join("instance.lock");
        match write_private_new(&path, b"") {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        ensure_regular(&path)?;
        let file = OpenOptions::new().read(true).write(true).open(&path)?;
        match file.try_lock_exclusive() {
            Ok(()) => Ok(Some(Self { _file: file, root })),
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock
                    || error.raw_os_error() == fs2::lock_contended_error().raw_os_error() =>
            {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }
    pub fn publish(&self, discovery: &Discovery) -> io::Result<()> {
        let target = self.root.join(DISCOVERY_FILE);
        let temporary = self
            .root
            .join(format!(".instance-{}.tmp", discovery.instance_id));
        let bytes = serde_json::to_vec(discovery)
            .map_err(|_| invalid("cannot encode instance discovery"))?;
        write_private_new(&temporary, &bytes)?;
        let result = (|| {
            // Only a free instance lock permits replacement of stale state.
            // The new complete file is published by rename after successful bind.
            match fs::symlink_metadata(&target) {
                Ok(metadata) if metadata.file_type().is_file() => {}
                Ok(_) => return Err(invalid("discovery is not a regular file")),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            fs::rename(&temporary, &target)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
    pub fn remove_own(&self, instance: Uuid) -> io::Result<()> {
        let data_dir = self
            .root
            .parent()
            .ok_or_else(|| invalid("invalid runtime directory"))?;
        match Discovery::read(data_dir) {
            Ok(value) if value.instance_id == instance => {
                fs::remove_file(self.root.join(DISCOVERY_FILE))
            }
            Ok(_) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
    /// A failed cleanup marker is retained so stop cannot report success merely
    /// because a process exited and released its OS lock.
    pub fn has_discovery(&self) -> bool {
        self.root.join(DISCOVERY_FILE).exists()
    }
}

pub async fn wait_stopped(data_dir: &Path, instance: Uuid, deadline: Duration) -> io::Result<()> {
    let start = tokio::time::Instant::now();
    loop {
        if let Some(lock) = InstanceLock::try_acquire(data_dir)? {
            match Discovery::read(data_dir) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
                Ok(record) if record.instance_id != instance => return Ok(()),
                _ => {
                    drop(lock);
                    return Err(invalid("cleanup could not be confirmed for this instance"));
                }
            }
        }
        if start.elapsed() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "service did not release its instance lock",
            ));
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
fn ensure_regular(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(invalid("runtime state must not be a reparse point"));
        }
    }
    if metadata.file_type().is_file() {
        Ok(())
    } else {
        Err(invalid("runtime state must be a regular file"))
    }
}
fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
#[cfg(target_os = "linux")]
fn creation_identity() -> io::Result<String> {
    let stat = fs::read_to_string("/proc/self/stat")?;
    let remainder = stat
        .rsplit_once(") ")
        .ok_or_else(|| invalid("invalid process creation identity"))?
        .1;
    let ticks = remainder
        .split_whitespace()
        .nth(19)
        .ok_or_else(|| invalid("missing process creation identity"))?;
    let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
    Ok(format!("linux:{}:{ticks}", boot.trim()))
}
#[cfg(windows)]
fn creation_identity() -> io::Result<String> {
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::Threading::{GetCurrentProcess, GetProcessTimes},
    };
    let mut creation: FILETIME = unsafe { std::mem::zeroed() };
    let mut exit = creation;
    let mut kernel = creation;
    let mut user = creation;
    if unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(format!(
        "windows:{}",
        ((creation.dwHighDateTime as u64) << 32) | creation.dwLowDateTime as u64
    ))
}
#[cfg(not(any(windows, target_os = "linux")))]
fn creation_identity() -> io::Result<String> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "process identity is unsupported on this build",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lock_and_nonce_protect_discovery() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("data");
        let lock = InstanceLock::try_acquire(&root).unwrap().unwrap();
        assert!(InstanceLock::try_acquire(&root).unwrap().is_none());
        let record =
            Discovery::current(Uuid::new_v4(), "127.0.0.1:43210".parse().unwrap()).unwrap();
        lock.publish(&record).unwrap();
        lock.remove_own(Uuid::new_v4()).unwrap();
        assert_eq!(Discovery::read(&root).unwrap(), record);
        lock.remove_own(record.instance_id).unwrap();
        assert!(!lock.has_discovery());
        drop(lock);
        assert!(InstanceLock::try_acquire(&root).unwrap().is_some());
    }
    #[tokio::test]
    async fn free_lock_with_unclean_marker_is_not_confirmed_stop() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("data");
        let lock = InstanceLock::try_acquire(&root).unwrap().unwrap();
        let record =
            Discovery::current(Uuid::new_v4(), "127.0.0.1:43210".parse().unwrap()).unwrap();
        lock.publish(&record).unwrap();
        drop(lock);
        assert!(
            wait_stopped(&root, record.instance_id, Duration::from_millis(100))
                .await
                .is_err()
        );
    }
    #[cfg(windows)]
    #[test]
    fn canonical_data_directory_observes_same_instance_without_creating_another() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("data");
        let lock = InstanceLock::try_acquire(&root).unwrap().unwrap();
        let canonical = fs::canonicalize(&root).unwrap();
        assert!(matches!(
            InstanceLock::observe(&canonical).unwrap(),
            InstanceObservation::Running
        ));
        drop(lock);
        assert!(matches!(
            InstanceLock::observe(&canonical).unwrap(),
            InstanceObservation::Stopped(Some(_))
        ));
    }
}
