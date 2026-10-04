//! Windows 10+ atomic job assignment. No spawn-then-AssignProcessToJobObject
//! window exists: both the non-inherited Job and exact pipe handle list are
//! supplied to CreateProcessW via STARTUPINFOEX. Unsupported hosts fail closed.
use std::{
    ffi::{OsStr, OsString, c_void},
    fs::File,
    io,
    mem::{size_of, zeroed},
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, IntoRawHandle, OwnedHandle},
    },
    path::Path,
    ptr::{null, null_mut},
};
use windows_sys::Win32::Foundation::SetHandleInformation;
use windows_sys::Win32::{
    Foundation::{
        DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE as Handle, HANDLE_FLAG_INHERIT,
        WAIT_OBJECT_0, WAIT_TIMEOUT,
    },
    Security::SECURITY_ATTRIBUTES as SecurityAttributes,
    System::{
        Console::{GetStdHandle, STD_ERROR_HANDLE},
        JobObjects::{
            CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION as ExtendedLimits,
            JobObjectExtendedLimitInformation, SetInformationJobObject, TerminateJobObject,
        },
        Pipes::CreatePipe,
        Threading::{
            CREATE_NO_WINDOW, CreateProcessW, DeleteProcThreadAttributeList,
            EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess, InitializeProcThreadAttributeList,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_JOB_LIST,
            PROCESS_INFORMATION as ProcessInformation, STARTF_USESTDHANDLES,
            STARTUPINFOEXW as StartupInfoEx, UpdateProcThreadAttribute, WaitForSingleObject,
        },
    },
};
const REAP_TIMEOUT_MS: u32 = 5000;
// SAFETY is restricted to these wrappers: each successful returned handle is
// immediately put in OwnedHandle; borrowed handles never outlive their owners.
fn own(raw: Handle) -> io::Result<OwnedHandle> {
    if raw.is_null() || raw as isize == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { OwnedHandle::from_raw_handle(raw) })
    }
}
fn check(result: i32) -> io::Result<()> {
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
fn pipe() -> io::Result<(OwnedHandle, OwnedHandle)> {
    let mut read = null_mut();
    let mut write = null_mut();
    let attributes = SecurityAttributes {
        nLength: size_of::<SecurityAttributes>() as u32,
        lpSecurityDescriptor: null_mut(),
        bInheritHandle: 1,
    };
    // SAFETY: output pointers and attributes refer to live stack values.
    unsafe {
        check(CreatePipe(&mut read, &mut write, &attributes, 8192))?;
    }
    Ok((own(read)?, own(write)?))
}
struct Attributes {
    words: Vec<usize>,
    initialized: bool,
}
impl Attributes {
    fn new() -> io::Result<Self> {
        let mut size = 0;
        // SAFETY: the first call requests the size, then aligned backing storage
        // stays fixed until DeleteProcThreadAttributeList, including on errors.
        unsafe {
            InitializeProcThreadAttributeList(null_mut(), 2, 0, &mut size);
        }
        if size == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut this = Self {
            words: vec![0; size.div_ceil(size_of::<usize>())],
            initialized: false,
        };
        unsafe {
            check(InitializeProcThreadAttributeList(
                this.ptr(),
                2,
                0,
                &mut size,
            ))?;
        }
        this.initialized = true;
        Ok(this)
    }
    fn ptr(&mut self) -> *mut c_void {
        self.words.as_mut_ptr().cast()
    }
    fn set(&mut self, attribute: usize, values: &mut [Handle]) -> io::Result<()> {
        // SAFETY: handles/slices stay alive through CreateProcessW; list valid.
        unsafe {
            check(UpdateProcThreadAttribute(
                self.ptr(),
                0,
                attribute,
                values.as_mut_ptr().cast(),
                size_of_val(values),
                null_mut(),
                null_mut(),
            ))
        }
    }
}
impl Drop for Attributes {
    fn drop(&mut self) {
        if self.initialized {
            unsafe {
                DeleteProcThreadAttributeList(self.ptr());
            }
        }
    }
}
/// Quote exactly as CommandLineToArgvW / the MS CRT expect. Always quote, and
/// double trailing backslashes before the closing quote. Embedded NUL rejected.
fn quote(value: &OsStr, output: &mut Vec<u16>) -> io::Result<()> {
    output.push(b'"' as u16);
    let mut slashes = 0;
    for unit in value.encode_wide() {
        if unit == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "NUL in worker argument",
            ));
        }
        if unit == b'\\' as u16 {
            slashes += 1;
            continue;
        }
        if unit == b'"' as u16 {
            output.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2 + 1));
        } else {
            output.extend(std::iter::repeat_n(b'\\' as u16, slashes));
        }
        slashes = 0;
        output.push(unit);
    }
    output.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
    output.push(b'"' as u16);
    Ok(())
}
pub(crate) struct Child {
    process: OwnedHandle,
    job: OwnedHandle,
    id: u32,
    pub stdin: Option<File>,
    pub stdout: Option<File>,
}
impl Child {
    pub fn spawn(path: &Path, args: &[OsString]) -> io::Result<Self> {
        let mut application: Vec<_> = path.as_os_str().encode_wide().collect();
        if application.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "NUL in worker path",
            ));
        }
        application.push(0);
        let mut command = Vec::new();
        quote(path.as_os_str(), &mut command)?;
        for arg in args {
            command.push(b' ' as u16);
            quote(arg, &mut command)?;
        }
        command.push(0);
        // SAFETY: null SA makes the Job non-inheritable. This owned Job is never
        // included in HANDLE_LIST. Descendants therefore cannot keep it alive.
        let job = own(unsafe { CreateJobObjectW(null(), null()) })?;
        let mut limits: ExtendedLimits = unsafe { zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE; // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        unsafe {
            check(SetInformationJobObject(
                job.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&limits as *const ExtendedLimits).cast(),
                size_of::<ExtendedLimits>() as u32,
            ))?;
        }
        let (child_in, parent_in) = pipe()?;
        let (parent_out, child_out) = pipe()?;
        unsafe {
            check(SetHandleInformation(
                parent_in.as_raw_handle(),
                HANDLE_FLAG_INHERIT,
                0,
            ))?;
            check(SetHandleInformation(
                parent_out.as_raw_handle(),
                HANDLE_FLAG_INHERIT,
                0,
            ))?;
        }
        let mut stderr = null_mut();
        let null_file;
        let stderr_source = unsafe { GetStdHandle(STD_ERROR_HANDLE) };
        let stderr_source = if stderr_source.is_null() || stderr_source as isize == -1 {
            null_file = File::options().write(true).open("NUL")?;
            null_file.as_raw_handle()
        } else {
            stderr_source
        };
        unsafe {
            check(DuplicateHandle(
                GetCurrentProcess(),
                stderr_source,
                GetCurrentProcess(),
                &mut stderr,
                0,
                1,
                DUPLICATE_SAME_ACCESS,
            ))?;
        }
        let stderr = own(stderr)?;
        let mut jobs = [job.as_raw_handle()];
        let mut inherited = [
            child_in.as_raw_handle(),
            child_out.as_raw_handle(),
            stderr.as_raw_handle(),
        ];
        // Attribute values must outlive DeleteProcThreadAttributeList. Declare
        // the owner last so its destructor runs before the borrowed arrays.
        let mut attributes = Attributes::new()?;
        attributes.set(PROC_THREAD_ATTRIBUTE_JOB_LIST as usize, &mut jobs)?; // PROC_THREAD_ATTRIBUTE_JOB_LIST
        attributes.set(PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize, &mut inherited)?; // HANDLE_LIST
        let mut startup: StartupInfoEx = unsafe { zeroed() };
        startup.StartupInfo.cb = size_of::<StartupInfoEx>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES; // STARTF_USESTDHANDLES
        startup.StartupInfo.hStdInput = child_in.as_raw_handle();
        startup.StartupInfo.hStdOutput = child_out.as_raw_handle();
        startup.StartupInfo.hStdError = stderr.as_raw_handle();
        startup.lpAttributeList = attributes.ptr();
        let mut information: ProcessInformation = unsafe { zeroed() };
        // SAFETY: all pointed-to buffers/attributes/handles remain live; mutable
        // command includes a terminator. Atomic job assignment, or no child.
        unsafe {
            check(CreateProcessW(
                application.as_ptr(),
                command.as_mut_ptr(),
                null(),
                null(),
                1,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_NO_WINDOW,
                null(),
                null(),
                &startup.StartupInfo,
                &mut information,
            ))?;
        }
        let process = own(information.hProcess)?;
        let _thread = own(information.hThread)?;
        // SAFETY: each pipe handle is transferred once into File.
        let stdin = unsafe { File::from_raw_handle(parent_in.into_raw_handle()) };
        let stdout = unsafe { File::from_raw_handle(parent_out.into_raw_handle()) };
        Ok(Self {
            process,
            job,
            id: information.dwProcessId,
            stdin: Some(stdin),
            stdout: Some(stdout),
        })
    }
    pub fn id(&self) -> u32 {
        self.id
    }
    pub fn try_wait(&mut self) -> io::Result<bool> {
        match unsafe { WaitForSingleObject(self.process.as_raw_handle(), 0) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            _ => Err(io::Error::last_os_error()),
        }
    }
    pub fn kill_tree(&mut self) -> io::Result<()> {
        unsafe { check(TerminateJobObject(self.job.as_raw_handle(), 127)) }
    }
    pub fn wait(&mut self) -> io::Result<()> {
        match unsafe { WaitForSingleObject(self.process.as_raw_handle(), REAP_TIMEOUT_MS) } {
            WAIT_OBJECT_0 => Ok(()),
            WAIT_TIMEOUT => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "contained worker did not exit after termination",
            )),
            _ => Err(io::Error::last_os_error()),
        }
    }
}
impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.kill_tree();
        let _ = self.wait();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quoting_handles_unicode_quotes_and_trailing_slashes() {
        for (input, expected) in [
            ("", "\"\""),
            ("a b", "\"a b\""),
            ("a\\", "\"a\\\\\""),
            ("a\"b", "\"a\\\"b\""),
        ] {
            let mut encoded = Vec::new();
            quote(OsStr::new(input), &mut encoded).unwrap();
            assert_eq!(String::from_utf16(&encoded).unwrap(), expected);
        }
        assert!(quote(OsStr::new("a\0b"), &mut Vec::new()).is_err());
    }
}
