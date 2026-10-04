use super::invalid;
use std::{
    ffi::CString,
    fs::File,
    io::{self, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Component, Path},
};
fn name(path: &std::ffi::OsStr) -> io::Result<CString> {
    CString::new(path.as_bytes()).map_err(|_| invalid())
}
fn check_private(file: &File, directory: bool) -> io::Result<()> {
    let m = file.metadata()?;
    // SAFETY: geteuid has no inputs and does not mutate caller memory.
    if m.uid() != unsafe { libc::geteuid() }
        || m.mode() & 0o077 != 0
        || (directory && !m.is_dir())
        || (!directory && (!m.is_file() || m.nlink() != 1))
    {
        return Err(invalid());
    }
    Ok(())
}
fn open_dir(path: &Path, create: bool) -> io::Result<File> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    // Walk descriptor-relative with NOFOLLOW at EVERY component. Lexical checks
    // alone would allow a symlink race in an ancestor directory.
    let mut dir = File::open("/")?;
    for component in path.components() {
        let Component::Normal(part) = component else {
            if matches!(component, Component::RootDir | Component::CurDir) {
                continue;
            }
            return Err(invalid());
        };
        let part = name(part)?;
        // SAFETY: owned dir descriptor and NUL-terminated component are live.
        let mut fd = unsafe {
            libc::openat(
                dir.as_raw_fd(),
                part.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_DIRECTORY,
            )
        };
        if fd < 0 && create && io::Error::last_os_error().kind() == io::ErrorKind::NotFound {
            let made = unsafe { libc::mkdirat(dir.as_raw_fd(), part.as_ptr(), 0o700) };
            if made < 0 && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists {
                return Err(io::Error::last_os_error());
            }
            fd = unsafe {
                libc::openat(
                    dir.as_raw_fd(),
                    part.as_ptr(),
                    libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_DIRECTORY,
                )
            };
        }
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        dir = unsafe { File::from_raw_fd(fd) };
    }
    check_private(&dir, true)?;
    Ok(dir)
}
pub fn create_private_dir(path: &Path) -> io::Result<()> {
    open_dir(path, true).map(|_| ())
}
pub fn validate_private_dir(path: &Path) -> io::Result<()> {
    open_dir(path, false).map(|_| ())
}
pub fn open_private_file(path: &Path) -> io::Result<File> {
    let parent = open_dir(path.parent().ok_or_else(invalid)?, false)?;
    let part = name(path.file_name().ok_or_else(invalid)?)?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            part.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let file = unsafe { File::from_raw_fd(fd) };
    check_private(&file, false)?;
    Ok(file)
}
pub fn write_private_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = open_dir(path.parent().ok_or_else(invalid)?, false)?;
    let target = name(path.file_name().ok_or_else(invalid)?)?;
    let temp = CString::new(format!(".nexa-private-{}", uuid::Uuid::new_v4())).unwrap();
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            temp.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut file = unsafe { File::from_raw_fd(fd) };
    let result = (|| {
        check_private(&file, false)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        // linkat is atomic and refuses to overwrite ANY existing entry, including
        // dangling symlinks. The temporary inode was already mode 0600 at birth.
        if unsafe {
            libc::linkat(
                parent.as_raw_fd(),
                temp.as_ptr(),
                parent.as_raw_fd(),
                target.as_ptr(),
                0,
            )
        } < 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    })();
    let unlink = unsafe { libc::unlinkat(parent.as_raw_fd(), temp.as_ptr(), 0) };
    result?;
    if unlink < 0 {
        return Err(io::Error::last_os_error());
    }
    parent.sync_all()
}

/// Configuration predates private-file ACL enforcement. Keep legacy readable
/// while rejecting links and pinning every path component and final descriptor.
pub fn open_regular_file(path: &Path) -> io::Result<File> {
    let parent = open_dir(path.parent().ok_or_else(invalid)?, false)?;
    let part = name(path.file_name().ok_or_else(invalid)?)?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            part.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let file = unsafe { File::from_raw_fd(fd) };
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.nlink() != 1 || metadata.uid() != unsafe { libc::geteuid() }
    {
        return Err(invalid());
    }
    Ok(file)
}

pub fn atomic_replace(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = open_dir(path.parent().ok_or_else(invalid)?, false)?;
    let target = name(path.file_name().ok_or_else(invalid)?)?;
    // Validation is descriptor based. The private parent remains pinned through
    // publication; rename replaces a directory entry and never follows a link.
    match open_regular_file(path) {
        Ok(file) => drop(file),
        Err(e) if e.kind() == io::ErrorKind::NotFound => (),
        Err(e) => return Err(e),
    }
    let temp = CString::new(format!(".nexa-config-{}", uuid::Uuid::new_v4())).unwrap();
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            temp.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut file = unsafe { File::from_raw_fd(fd) };
    let result = (|| {
        check_private(&file, false)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        if unsafe {
            libc::renameat(
                parent.as_raw_fd(),
                temp.as_ptr(),
                parent.as_raw_fd(),
                target.as_ptr(),
            )
        } < 0
        {
            return Err(io::Error::last_os_error());
        }
        parent
            .sync_all()
            .map_err(|_| io::Error::other("configuration_durability_unconfirmed"))
    })();
    if result.is_err() {
        unsafe {
            libc::unlinkat(parent.as_raw_fd(), temp.as_ptr(), 0);
        }
    }
    result
}
