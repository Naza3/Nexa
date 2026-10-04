//! Test-only process group containment. Not a Linux product guarantee.
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
    time::Duration,
};
pub(crate) struct Child {
    process: StdChild,
    pub stdout: Option<File>,
    pub stderr: Option<File>,
}
impl Child {
    pub fn spawn(
        path: &Path,
        args: &[OsString],
        cwd: &Path,
        environment: &[(OsString, OsString)],
    ) -> io::Result<Self> {
        let parent = std::process::id() as libc::pid_t;
        let mut command = Command::new(path);
        command
            .args(args)
            .current_dir(cwd)
            .env_clear()
            .envs(environment.iter().cloned())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // SAFETY: only async-signal-safe syscalls and primitive captured values.
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
        // SAFETY: transfer each owned pipe exactly once.
        let stdout = unsafe { File::from_raw_fd(process.stdout.take().unwrap().into_raw_fd()) };
        let stderr = unsafe { File::from_raw_fd(process.stderr.take().unwrap().into_raw_fd()) };
        Ok(Self {
            process,
            stdout: Some(stdout),
            stderr: Some(stderr),
        })
    }
    pub fn try_wait(&mut self) -> io::Result<Option<u32>> {
        Ok(self
            .process
            .try_wait()?
            .map(|s| s.code().map(|v| v as u32).unwrap_or(127)))
    }
    pub fn kill_tree(&mut self) -> io::Result<()> {
        let result = unsafe { libc::kill(-(self.process.id() as libc::pid_t), libc::SIGKILL) };
        if result != 0 && io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH) {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    pub fn tree_stopped(&self) -> io::Result<bool> {
        let result = unsafe { libc::kill(-(self.process.id() as libc::pid_t), 0) };
        Ok(result != 0 && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH))
    }
}
impl Drop for Child {
    fn drop(&mut self) {
        loop {
            let _ = self.kill_tree();
            if matches!(self.try_wait(), Ok(Some(_))) && matches!(self.tree_stopped(), Ok(true)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
