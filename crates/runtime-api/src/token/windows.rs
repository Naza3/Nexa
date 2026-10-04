//! Windows credentials are born with a protected current-SID-only DACL.
//! Reparse points and unsafe preexisting ACLs fail closed; no post-write ACL fix.
use super::invalid;
use std::{
    ffi::c_void,
    fs::File,
    io::{self, Write},
    mem::{size_of, zeroed},
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::{Component, Path, PathBuf},
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE, LocalFree},
    Security::{Authorization::*, *},
    Storage::FileSystem::*,
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};
fn check(result: i32) -> io::Result<()> {
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
fn wide(path: &Path) -> io::Result<Vec<u16>> {
    let mut value: Vec<_> = path.as_os_str().encode_wide().collect();
    if value.contains(&0) {
        return Err(invalid());
    }
    value.push(0);
    Ok(value)
}
struct Local(*mut c_void);
impl Drop for Local {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}
struct Sid {
    _words: Vec<usize>,
    pointer: PSID,
}
impl Sid {
    fn current() -> io::Result<Self> {
        let mut handle = null_mut();
        unsafe {
            check(OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_QUERY,
                &mut handle,
            ))?;
        }
        let _handle = unsafe { OwnedHandle::from_raw_handle(handle) };
        let mut needed = 0;
        unsafe {
            GetTokenInformation(handle, TokenUser, null_mut(), 0, &mut needed);
        }
        if needed == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut words = vec![0usize; (needed as usize).div_ceil(size_of::<usize>())];
        unsafe {
            check(GetTokenInformation(
                handle,
                TokenUser,
                words.as_mut_ptr().cast(),
                needed,
                &mut needed,
            ))?;
        }
        let pointer = unsafe { (*(words.as_ptr().cast::<TOKEN_USER>())).User.Sid };
        Ok(Self {
            _words: words,
            pointer,
        })
    }
    fn text(&self) -> io::Result<String> {
        let mut text = null_mut();
        unsafe {
            check(ConvertSidToStringSidW(self.pointer, &mut text))?;
        }
        let _owner = Local(text.cast());
        let mut length = 0;
        unsafe {
            while *text.add(length) != 0 {
                length += 1;
            }
            String::from_utf16(std::slice::from_raw_parts(text, length)).map_err(|_| invalid())
        }
    }
}
struct Descriptor(Local);
impl Descriptor {
    fn private(directory: bool) -> io::Result<Self> {
        let sid = Sid::current()?.text()?;
        let sddl = format!(
            "O:{sid}D:P(A;{};FA;;;{sid})",
            if directory { "OICI" } else { "" }
        );
        let text: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
        let mut sd = null_mut();
        unsafe {
            check(ConvertStringSecurityDescriptorToSecurityDescriptorW(
                text.as_ptr(),
                1,
                &mut sd,
                null_mut(),
            ))?;
        }
        Ok(Self(Local(sd)))
    }
    fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0.0,
            bInheritHandle: 0,
        }
    }
}
fn open(path: &Path, directory: bool) -> io::Result<File> {
    let path = wide(path)?;
    let raw = unsafe {
        CreateFileW(
            path.as_ptr(),
            GENERIC_READ | READ_CONTROL,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            null(),
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT
                | if directory {
                    FILE_FLAG_BACKUP_SEMANTICS
                } else {
                    0
                },
            null_mut(),
        )
    };
    if raw == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let file = unsafe { File::from_raw_handle(raw) };
    let mut info = unsafe { zeroed::<BY_HANDLE_FILE_INFORMATION>() };
    unsafe {
        check(GetFileInformationByHandle(file.as_raw_handle(), &mut info))?;
    }
    if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || (info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0) != directory
        || (!directory && info.nNumberOfLinks != 1)
    {
        return Err(invalid());
    }
    Ok(file)
}
fn validate(file: &File) -> io::Result<()> {
    let current = Sid::current()?;
    let mut owner = null_mut();
    let mut dacl = null_mut();
    let mut sd = null_mut();
    let code = unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            &mut dacl,
            null_mut(),
            &mut sd,
        )
    };
    if code != 0 {
        return Err(io::Error::from_raw_os_error(code as i32));
    }
    let _sd = Local(sd);
    let mut control = 0;
    let mut revision = 0;
    unsafe {
        check(GetSecurityDescriptorControl(
            sd,
            &mut control,
            &mut revision,
        ))?;
    }
    if control & SE_DACL_PROTECTED == 0
        || dacl.is_null()
        || owner.is_null()
        || unsafe { EqualSid(owner, current.pointer) } == 0
        || unsafe { (*dacl).AceCount } != 1
    {
        return Err(invalid());
    }
    let mut ace = null_mut();
    unsafe {
        check(GetAce(dacl, 0, &mut ace))?;
    }
    let allow = unsafe { &*(ace.cast::<ACCESS_ALLOWED_ACE>()) };
    if allow.Header.AceType != 0
        || allow.Header.AceFlags & (INHERITED_ACE as u8) != 0
        || allow.Mask != FILE_ALL_ACCESS
        || unsafe {
            EqualSid(
                (&allow.SidStart as *const u32).cast_mut().cast(),
                current.pointer,
            )
        } == 0
    {
        return Err(invalid());
    }
    Ok(())
}
/// Keep every ancestor open without FILE_SHARE_DELETE while using the path.
/// Reparse-point opens are explicit and rejected before any descendant access.
fn directories(path: &Path, create: bool) -> io::Result<Vec<File>> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut prefix = PathBuf::new();
    let mut held = Vec::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => {
                prefix.push(component);
                continue;
            }
            Component::Normal(part) => prefix.push(part),
            Component::CurDir => continue,
            _ => return Err(invalid()),
        }
        let file = match open(&prefix, true) {
            Ok(file) => file,
            Err(e) if create && e.kind() == io::ErrorKind::NotFound => {
                let sd = Descriptor::private(true)?;
                let attributes = sd.attributes();
                let name = wide(&prefix)?;
                let made = unsafe { CreateDirectoryW(name.as_ptr(), &attributes) };
                if made == 0 && io::Error::last_os_error().kind() != io::ErrorKind::AlreadyExists {
                    return Err(io::Error::last_os_error());
                }
                open(&prefix, true)?
            }
            Err(error) => return Err(error),
        };
        held.push(file);
    }
    if held.is_empty() {
        return Err(invalid());
    }
    Ok(held)
}
pub fn create_private_dir(path: &Path) -> io::Result<()> {
    let held = directories(path, true)?;
    validate(held.last().unwrap())
}
pub fn validate_private_dir(path: &Path) -> io::Result<()> {
    let held = directories(path, false)?;
    validate(held.last().unwrap())
}
pub fn open_private_file(path: &Path) -> io::Result<File> {
    let held = directories(path.parent().ok_or_else(invalid)?, false)?;
    validate(held.last().unwrap())?;
    let file = open(path, false)?;
    validate(&file)?;
    Ok(file)
}
pub fn write_private_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(invalid)?;
    let held = directories(parent, false)?;
    validate(held.last().unwrap())?;
    let temporary = parent.join(format!(".nexa-private-{}", uuid::Uuid::new_v4()));
    let source = wide(&temporary)?;
    let target = wide(path)?;
    let sd = Descriptor::private(false)?;
    let attributes = sd.attributes();
    let raw = unsafe {
        CreateFileW(
            source.as_ptr(),
            GENERIC_WRITE | GENERIC_READ | READ_CONTROL,
            0,
            &attributes,
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
            null_mut(),
        )
    };
    if raw == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let mut file = unsafe { File::from_raw_handle(raw) };
    let result = (|| {
        validate(&file)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(())
    })();
    drop(file);
    if let Err(error) = result {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    // MOVEFILE_REPLACE_EXISTING is intentionally absent. The new protected DACL
    // moves with the inode; there is never a publicly readable destination.
    if unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), MOVEFILE_WRITE_THROUGH) } == 0 {
        let error = io::Error::last_os_error();
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(())
}

pub fn open_regular_file(path: &Path) -> io::Result<File> {
    let held = directories(path.parent().ok_or_else(invalid)?, false)?;
    validate(held.last().unwrap())?;
    open(path, false)
}

pub fn atomic_replace(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(invalid)?;
    let held = directories(parent, false)?;
    validate(held.last().unwrap())?;
    match open(path, false) {
        Ok(file) => drop(file),
        Err(e) if e.kind() == io::ErrorKind::NotFound => (),
        Err(e) => return Err(e),
    }
    let temporary = parent.join(format!(".nexa-config-{}", uuid::Uuid::new_v4()));
    write_private_new(&temporary, bytes)?;
    let source = wide(&temporary)?;
    let target = wide(path)?;
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        let error = io::Error::last_os_error();
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(())
}
