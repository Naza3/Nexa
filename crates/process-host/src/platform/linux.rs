//! Linux development fallback. Parent death kills the worker itself; normal
//! teardown kills its process group. Arbitrary descendants after abrupt parent
//! exit are NOT a supported Linux guarantee (Windows Job objects provide it).
use std::{
    ffi::OsString,
    fs::File,
    io,
    os::{
        fd::{FromRawFd, IntoRawFd},
        unix::process::CommandExt,
    },
    path::Path,
    process::{Child as StdChild, Command, Stdio},
};
pub(crate) struct Child {
    process: StdChild,
    killed: bool,
    reaped: bool,
    pub stdin: Option<File>,
    pub stdout: Option<File>,
}
impl Child {
    pub fn spawn(path: &Path, args: &[OsString]) -> io::Result<Self> {
        // This thread stays alive until wait(): Linux PDEATHSIG is tied to the
        // creating thread, not merely its thread group. No detached spawn task.
        let parent = std::process::id() as libc::pid_t;
        let mut command = Command::new(path);
        command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        // SAFETY: only async-signal-safe syscalls and primitive copied values
        // run between fork and exec. No allocation, locks, or logging.
        unsafe {
            command.pre_exec(move || {
                if libc::setpgid(0, 0) != 0
                    || libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0
                {
                    return Err(io::Error::last_os_error());
                }
                if libc::getppid() != parent {
                    libc::_exit(127);
                }
                Ok(())
            });
        }
        let mut process = command.spawn()?;
        // SAFETY: move ownership of each Child pipe exactly once into File.
        let stdin = unsafe { File::from_raw_fd(process.stdin.take().unwrap().into_raw_fd()) };
        let stdout = unsafe { File::from_raw_fd(process.stdout.take().unwrap().into_raw_fd()) };
        Ok(Self {
            process,
            killed: false,
            reaped: false,
            stdin: Some(stdin),
            stdout: Some(stdout),
        })
    }
    pub fn id(&self) -> u32 {
        self.process.id()
    }
    pub fn try_wait(&mut self) -> io::Result<bool> {
        let done = self.process.try_wait()?.is_some();
        self.reaped |= done;
        Ok(done)
    }
    pub fn kill_tree(&mut self) -> io::Result<()> {
        if self.killed {
            return Ok(());
        }
        // SAFETY: child established its own pgid before exec; negative pid
        // targets that group. ESRCH means it is already gone.
        let result = unsafe { libc::kill(-(self.id() as libc::pid_t), libc::SIGKILL) };
        if result != 0 && io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH) {
            return Err(io::Error::last_os_error());
        }
        self.killed = true;
        Ok(())
    }
    pub fn wait(&mut self) -> io::Result<()> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !self.reaped {
            if self.try_wait()? {
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "worker did not exit after termination",
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        Ok(())
    }
}
impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.kill_tree();
        let _ = self.wait();
    }
}
