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
    Foundation::{HANDLE as Handle, HANDLE_FLAG_INHERIT, WAIT_OBJECT_0, WAIT_TIMEOUT},
    Security::SECURITY_ATTRIBUTES as SecurityAttributes,
    System::{
        JobObjects::{
            CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION as ExtendedLimits,
            JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation,
            QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
        },
        Pipes::CreatePipe,
        Threading::{
            CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
            DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess,
            InitializeProcThreadAttributeList, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
            PROC_THREAD_ATTRIBUTE_JOB_LIST, PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY,
            PROCESS_INFORMATION as ProcessInformation, STARTF_USESTDHANDLES,
            STARTUPINFOEXW as StartupInfoEx, UpdateProcThreadAttribute, WaitForSingleObject,
        },
    },
};

// JOB_LIST, HANDLE_LIST and one DWORD64 MITIGATION_POLICY are submitted in the
// same CreateProcessW call. A host that rejects any attribute fails closed.
const ATTRIBUTE_COUNT: u32 = 3;
// WinSDK winbase.h and Microsoft's UpdateProcThreadAttribute reference define
// IMAGE_LOAD_PREFER_SYSTEM32_ALWAYS_ON as (0x00000001ui64 << 60). windows-sys
// 0.61.2 does not export that macro, so preserve its exact DWORD64 width here.
// This prefers System32 to the application directory; it is not system32-only.
const IMAGE_LOAD_PREFER_SYSTEM32_ALWAYS_ON: u64 = 1u64 << 60;

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
            InitializeProcThreadAttributeList(null_mut(), ATTRIBUTE_COUNT, 0, &mut size);
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
                ATTRIBUTE_COUNT,
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
    fn set_image_load_policy(&mut self, policy: &mut u64) -> io::Result<()> {
        // SAFETY: this is a live DWORD64, not a pointer-sized handle or the
        // two-word policy2 form. Its owner outlives this attribute list and
        // CreateProcessW. Never retry creation without this mitigation.
        unsafe {
            check(UpdateProcThreadAttribute(
                self.ptr(),
                0,
                PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY as usize,
                (policy as *mut u64).cast(),
                size_of::<u64>(),
                null_mut(),
                null_mut(),
            ))
        }
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
        let mut directory: Vec<u16> = cwd.as_os_str().encode_wide().collect();
        if directory.contains(&0) {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        directory.push(0);
        let mut env = Vec::<u16>::new();
        let mut sorted_environment = environment.to_vec();
        sorted_environment.sort_by_key(|(key, _)| key.to_string_lossy().to_ascii_uppercase());
        for (key, value) in &sorted_environment {
            let entry = key
                .encode_wide()
                .chain(std::iter::once(b'=' as u16))
                .chain(value.encode_wide())
                .collect::<Vec<_>>();
            if entry.contains(&0) {
                return Err(io::Error::from(io::ErrorKind::InvalidInput));
            }
            env.extend(entry);
            env.push(0);
        }
        if env.is_empty() {
            env.push(0);
        }
        env.push(0);
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
        let (parent_err, child_err) = pipe()?;
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
        unsafe {
            check(SetHandleInformation(
                parent_err.as_raw_handle(),
                HANDLE_FLAG_INHERIT,
                0,
            ))?;
        }
        let mut jobs = [job.as_raw_handle()];
        let mut inherited = [
            child_in.as_raw_handle(),
            child_out.as_raw_handle(),
            child_err.as_raw_handle(),
        ];
        let mut image_load_policy = IMAGE_LOAD_PREFER_SYSTEM32_ALWAYS_ON;
        // Attribute values must outlive DeleteProcThreadAttributeList. Declare
        // the owner last so its destructor runs before the borrowed arrays.
        let mut attributes = Attributes::new()?;
        attributes.set(PROC_THREAD_ATTRIBUTE_JOB_LIST as usize, &mut jobs)?; // PROC_THREAD_ATTRIBUTE_JOB_LIST
        attributes.set(PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize, &mut inherited)?; // HANDLE_LIST
        attributes.set_image_load_policy(&mut image_load_policy)?;
        let mut startup: StartupInfoEx = unsafe { zeroed() };
        startup.StartupInfo.cb = size_of::<StartupInfoEx>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES; // STARTF_USESTDHANDLES
        startup.StartupInfo.hStdInput = child_in.as_raw_handle();
        startup.StartupInfo.hStdOutput = child_out.as_raw_handle();
        startup.StartupInfo.hStdError = child_err.as_raw_handle();
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
                EXTENDED_STARTUPINFO_PRESENT | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
                env.as_ptr().cast(),
                directory.as_ptr(),
                &startup.StartupInfo,
                &mut information,
            ))?;
        }
        let process = own(information.hProcess)?;
        let _thread = own(information.hThread)?;
        // SAFETY: each pipe handle is transferred once into File.
        // No input file/RPC: closing the parent writer gives child stdin EOF.
        drop(parent_in);
        let stdout = unsafe { File::from_raw_handle(parent_out.into_raw_handle()) };
        let stderr = unsafe { File::from_raw_handle(parent_err.into_raw_handle()) };
        Ok(Self {
            process,
            job,
            stdout: Some(stdout),
            stderr: Some(stderr),
        })
    }
    pub fn try_wait(&mut self) -> io::Result<Option<u32>> {
        match unsafe { WaitForSingleObject(self.process.as_raw_handle(), 0) } {
            WAIT_OBJECT_0 => {
                let mut code = 0;
                unsafe {
                    check(GetExitCodeProcess(self.process.as_raw_handle(), &mut code))?;
                }
                Ok(Some(code))
            }
            WAIT_TIMEOUT => Ok(None),
            _ => Err(io::Error::last_os_error()),
        }
    }
    pub fn kill_tree(&mut self) -> io::Result<()> {
        unsafe { check(TerminateJobObject(self.job.as_raw_handle(), 127)) }
    }
    pub fn tree_stopped(&self) -> io::Result<bool> {
        let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { zeroed() };
        unsafe {
            check(QueryInformationJobObject(
                self.job.as_raw_handle(),
                JobObjectBasicAccountingInformation,
                (&mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                null_mut(),
            ))?;
        }
        Ok(info.ActiveProcesses == 0)
    }
    fn reap_tree(&mut self) {
        // No cleanup-complete assertion on a timeout. The caller's bounded
        // close wait may expire while this supervisor keeps all leases alive.
        loop {
            let _ = self.kill_tree();
            if matches!(self.try_wait(), Ok(Some(_))) && matches!(self.tree_stopped(), Ok(true)) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}
impl Drop for Child {
    fn drop(&mut self) {
        self.reap_tree();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn startup_mitigation_has_exact_dword64_layout_and_attribute_capacity() {
        assert_eq!(ATTRIBUTE_COUNT, 3);
        assert_eq!(size_of::<u64>(), 8);
        assert_eq!(IMAGE_LOAD_PREFER_SYSTEM32_ALWAYS_ON, 0x1000_0000_0000_0000);
        let mut policy = IMAGE_LOAD_PREFER_SYSTEM32_ALWAYS_ON;
        let mut attributes = Attributes::new().unwrap();
        attributes.set_image_load_policy(&mut policy).unwrap();
    }
    #[test]
    fn spawned_child_reports_prefer_system32_policy_enabled() {
        use windows_sys::Win32::System::Threading::{
            GetProcessMitigationPolicy, ProcessImageLoadPolicy,
        };
        // PROCESS_MITIGATION_IMAGE_LOAD_POLICY is a four-byte Flags union.
        // Mirror that documented ABI without enabling an unrelated SDK module.
        #[repr(C)]
        struct ImageLoadPolicy {
            flags: u32,
        }
        let mut policy = ImageLoadPolicy { flags: 0 };
        assert_eq!(size_of::<ImageLoadPolicy>(), 4);
        let environment = vec![("NEXA_DOWNLOAD_TEST_CASE".into(), "sleep".into())];
        let args = [
            "--exact",
            "supervisor::tests::child_entry",
            "--nocapture",
            "--test-threads=1",
        ]
        .map(OsString::from);
        let child = Child::spawn(
            &std::env::current_exe().unwrap(),
            &args,
            &std::env::current_dir().unwrap(),
            &environment,
        )
        .unwrap();
        // SAFETY: process is owned/alive; writable structure has the exact
        // documented four-byte image-load policy representation.
        unsafe {
            check(GetProcessMitigationPolicy(
                child.process.as_raw_handle(),
                ProcessImageLoadPolicy,
                (&mut policy as *mut ImageLoadPolicy).cast(),
                size_of::<ImageLoadPolicy>(),
            ))
            .unwrap();
        }
        assert_ne!(policy.flags & (1 << 2), 0, "PreferSystem32Images is absent");
        // Child's Drop terminates and confirms the entire Job before returning.
        drop(child);
    }
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
