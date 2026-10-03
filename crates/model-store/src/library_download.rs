//! Destination transaction for explicitly requested catalog downloads.
//! The read-only scanner never calls this. Windows pins every ancestor and
//! prevents writes/deletion of the partial file until no-clobber publication.
use super::*;
use std::io::{Seek, SeekFrom, Write};

pub struct DownloadFile {
    directory: DirectoryGuard,
    file: Option<File>,
    temporary: PathBuf,
    destination: PathBuf,
    published: bool,
}
impl DownloadFile {
    pub fn create(library: &ModelLibrary, file_name: &str, operation_id: Uuid) -> Result<Self> {
        // Unix fixtures exercise byte transactions only, not Windows protection.
        #[cfg(not(any(windows, test)))]
        {
            let _ = (library, file_name, operation_id);
            return Err(library_error(ErrorCode::ModelDirectoryUnsupported));
        }
        #[allow(unreachable_code)]
        {
            if !valid_file_name(file_name) || !file_name.ends_with(".gguf") {
                return Err(library_error(ErrorCode::InvalidArgument));
            }
            validate_directory_syntax(&library.directory)?;
            let directory = DirectoryGuard::open(&library.directory)?;
            if !same_object(&directory.identity, &library.directory_identity) {
                return Err(library_error(ErrorCode::ModelFileChanged));
            }
            let destination = directory.path.join(file_name);
            match fs::symlink_metadata(&destination) {
                Ok(_) => return Err(library_error(ErrorCode::AlreadyExists)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(file_error(e)),
            }
            let temporary = directory
                .path
                .join(format!(".nexa-download-{operation_id}.part"));
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                use windows_sys::Win32::Storage::FileSystem::*;
                options
                    .access_mode(
                        windows_sys::Win32::Foundation::GENERIC_WRITE
                            | DELETE
                            | FILE_READ_ATTRIBUTES,
                    )
                    .share_mode(FILE_SHARE_READ)
                    .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
            }
            let file = options.open(&temporary).map_err(file_error)?;
            Ok(Self {
                directory,
                file: Some(file),
                temporary,
                destination,
                published: false,
            })
        }
    }
    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.file
            .as_mut()
            .unwrap()
            .write_all(bytes)
            .map_err(file_error)
    }
    /// Restart in-place using the same protected file and directory handles.
    /// Never reopens a path or releases Windows write/delete protection.
    pub fn reset_to_empty(&mut self) -> Result<()> {
        let file = self.file.as_mut().unwrap();
        file.set_len(0).map_err(file_error)?;
        file.seek(SeekFrom::Start(0)).map_err(file_error)?;
        Ok(())
    }
    /// Link publication is atomic and never replaces an existing destination.
    /// The protected live file stays open through publication, so the name
    /// cannot be swapped on Windows between hash verification and publication.
    pub fn publish(mut self) -> Result<bool> {
        self.file.as_ref().unwrap().sync_all().map_err(file_error)?;
        fs::hard_link(&self.temporary, &self.destination).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                library_error(ErrorCode::AlreadyExists)
            } else {
                file_error(e)
            }
        })?;
        self.published = true;
        let cleaned = self.cleanup();
        #[cfg(unix)]
        self.directory
            ._ancestors
            .last()
            .unwrap()
            .sync_all()
            .map_err(file_error)?;
        Ok(cleaned)
    }
    fn cleanup(&mut self) -> bool {
        let Some(file) = self.file.take() else {
            return true;
        };
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Storage::FileSystem::*;
            let info = FILE_DISPOSITION_INFO { DeleteFile: true };
            // SAFETY: the owned live file was opened with DELETE access. This
            // disposition addresses that handle, never a subsequently replaced path.
            let removed = unsafe {
                SetFileInformationByHandle(
                    file.as_raw_handle().cast(),
                    FileDispositionInfo,
                    (&info as *const FILE_DISPOSITION_INFO).cast(),
                    std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
                )
            } != 0;
            drop(file);
            removed
        }
        #[cfg(not(windows))]
        {
            // DownloadFile::create is unavailable outside Windows except unit fixtures.
            drop(file);
            fs::remove_file(&self.temporary).is_ok()
        }
    }
}
impl Drop for DownloadFile {
    fn drop(&mut self) {
        // Only our own open file is marked for deletion; never a replacement.
        let _ = self.cleanup();
        let _ = (&self.directory, self.published);
    }
}

/// Paths are produced only by an admitted, pinned destination transaction.
/// The engine receives the directory and the fixed `payload.part` basename.
#[derive(Clone, Debug)]
pub struct SidecarPaths {
    pub directory: PathBuf,
    pub payload: PathBuf,
}
struct SidecarResources {
    _directory: DirectoryGuard,
    task_pin: Option<File>,
    part_pin: Option<File>,
    task_path: PathBuf,
    part_path: PathBuf,
    destination: PathBuf,
    task_identity: FileIdentity,
    part_identity: FileIdentity,
}
/// Filesystem transaction only: no HTTP, process supervision, registration or
/// resume metadata trust. Windows guards deny namespace replacement while the
/// external writer is allowed to write the fixed partial object.
///
/// A caller must keep this object AND its instance lock in the supervisor/reaper
/// until the OS process/job and all pipe drainers have actually stopped. Merely
/// observing an exit message is not sufficient. Dropping an active transaction
/// is a contract error: guards are retained until process exit, not cleaned up.
pub struct SidecarDownloadFile {
    resources: Option<SidecarResources>,
    writer_active: bool,
}
/// Verification owns an externally write-protected handle until publication.
/// The caller must arbitrate cancellation immediately before calling publish.
pub struct VerifiedSidecarFile {
    // Drop this handle before transaction cleanup (declaration order matters).
    file: File,
    transaction: SidecarDownloadFile,
}
#[derive(Clone, Copy)]
enum CreationStep {
    BeforeTaskPin,
    TaskPinned,
    PartPinned,
}
/// Roll back only objects with owned handles. An object we could not pin is
/// unconfirmed and is deliberately left behind rather than deleting by path.
struct SidecarCreation {
    task_path: PathBuf,
    part_path: PathBuf,
    task_pin: Option<File>,
    part_pin: Option<File>,
}
impl Drop for SidecarCreation {
    fn drop(&mut self) {
        if let Some(file) = self.part_pin.take() {
            let _ = remove_pinned(file, &self.part_path);
        }
        if let Some(file) = self.task_pin.take() {
            let _ = remove_pinned(file, &self.task_path);
        }
    }
}
impl SidecarDownloadFile {
    pub fn create(library: &ModelLibrary, file_name: &str, operation_id: Uuid) -> Result<Self> {
        Self::create_checked(library, file_name, operation_id, |_, _| Ok(()))
    }
    fn create_checked(
        library: &ModelLibrary,
        file_name: &str,
        operation_id: Uuid,
        #[allow(unused_mut)] mut _checkpoint: impl FnMut(CreationStep, &Path) -> Result<()>,
    ) -> Result<Self> {
        #[cfg(not(any(windows, test)))]
        {
            let _ = (library, file_name, operation_id);
            return Err(library_error(ErrorCode::ModelDirectoryUnsupported));
        }
        #[allow(unreachable_code)]
        {
            if !valid_file_name(file_name) || !file_name.ends_with(".gguf") {
                return Err(library_error(ErrorCode::InvalidArgument));
            }
            let directory = DirectoryGuard::open(&library.directory)?;
            if !same_object(&directory.identity, &library.directory_identity) {
                return Err(library_error(ErrorCode::ModelFileChanged));
            }
            let destination = directory.path.join(file_name);
            match fs::symlink_metadata(&destination) {
                Ok(_) => return Err(library_error(ErrorCode::AlreadyExists)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                Err(error) => return Err(file_error(error)),
            }
            // Never adopt a pre-existing task directory, including after restart.
            let task_path = directory
                .path
                .join(format!(".nexa-download-{operation_id}"));
            #[allow(unused_mut)]
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(&task_path).map_err(file_error)?;
            let part_path = task_path.join("payload.part");
            let mut creation = SidecarCreation {
                task_path: task_path.clone(),
                part_path: part_path.clone(),
                task_pin: None,
                part_pin: None,
            };
            _checkpoint(CreationStep::BeforeTaskPin, &task_path)?;
            creation.task_pin = Some(sidecar_pin(&task_path, true, false)?);
            _checkpoint(CreationStep::TaskPinned, &task_path)?;
            let task_identity = identity(creation.task_pin.as_ref().unwrap())?;
            creation.part_pin = Some(sidecar_pin(&part_path, false, true)?);
            _checkpoint(CreationStep::PartPinned, &task_path)?;
            let part_identity = identity(creation.part_pin.as_ref().unwrap())?;
            Ok(Self {
                resources: Some(SidecarResources {
                    _directory: directory,
                    task_pin: creation.task_pin.take(),
                    part_pin: creation.part_pin.take(),
                    task_path,
                    part_path,
                    destination,
                    task_identity,
                    part_identity,
                }),
                writer_active: false,
            })
        }
    }
    pub fn begin_attempt(&mut self) -> Result<SidecarPaths> {
        self.stopped()?;
        let resources = self.resources.as_ref().unwrap();
        resources.check_part()?;
        self.writer_active = true;
        Ok(SidecarPaths {
            directory: resources.task_path.clone(),
            payload: resources.part_path.clone(),
        })
    }
    /// Caller contract: process handles, the entire Job and pipe drainers are
    /// confirmed stopped, or process creation was confirmed never to occur.
    /// This method does not infer OS process state and must not be called on a
    /// timeout, an aria2 console message, or an unconfirmed kill request.
    pub fn confirm_writer_stopped(&mut self) {
        self.writer_active = false;
    }
    /// Best-effort supervisor input, not an exact per-write quota. Network/file
    /// growth can race polling; size and hash are strictly checked by verify.
    pub fn observed_size(&self) -> Result<u64> {
        let resources = self.resources.as_ref().unwrap();
        Ok(resources
            .part_pin
            .as_ref()
            .unwrap()
            .metadata()
            .map_err(file_error)?
            .len())
    }
    fn stopped(&self) -> Result<()> {
        if self.writer_active {
            return Err(library_error(ErrorCode::ExecutorCleanupUnconfirmed));
        }
        Ok(())
    }
    /// One-shot recovery policy belongs to the caller (e.g. only aria2 exit 8).
    /// The operation identity and protected partial object are preserved.
    pub fn reset_for_retry(&mut self) -> Result<()> {
        self.stopped()?;
        let resources = self.resources.as_ref().unwrap();
        resources.check_part()?;
        if !resources.only_known_entries()? || !resources.cleanup_control_files() {
            return Err(library_error(ErrorCode::ModelFileChanged));
        }
        let file = resources.open_exclusive_part()?;
        file.set_len(0).map_err(file_error)?;
        file.sync_all().map_err(file_error)?;
        Ok(())
    }
    pub fn verify(
        self,
        expected_size: u64,
        expected_sha256: [u8; 32],
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<VerifiedSidecarFile> {
        self.stopped()?;
        if !(4..=MAX_MODEL_BYTES).contains(&expected_size) {
            return Err(library_error(ErrorCode::InvalidArgument));
        }
        let resources = self.resources.as_ref().unwrap();
        let mut file = resources.open_exclusive_part()?;
        if file.metadata().map_err(file_error)?.len() != expected_size {
            return Err(library_error(ErrorCode::IntegrityFailure));
        }
        let mut hasher = Sha256::new();
        let mut count = 0u64;
        let mut bytes = [0u8; BLOCK];
        loop {
            if cancelled() {
                return Err(library_error(ErrorCode::RequestCancelled));
            }
            let read = file.read(&mut bytes).map_err(file_error)?;
            if read == 0 {
                break;
            }
            count = count
                .checked_add(read as u64)
                .filter(|n| *n <= expected_size)
                .ok_or_else(|| library_error(ErrorCode::IntegrityFailure))?;
            hasher.update(&bytes[..read]);
        }
        let actual_sha256: [u8; 32] = hasher.finalize().into();
        if count != expected_size || actual_sha256 != expected_sha256 {
            return Err(library_error(ErrorCode::IntegrityFailure));
        }
        if cancelled() {
            return Err(library_error(ErrorCode::RequestCancelled));
        }
        file.sync_all().map_err(file_error)?;
        Ok(VerifiedSidecarFile {
            file,
            transaction: self,
        })
    }
    /// Active writers are never cleaned. False also means unknown objects or
    /// unconfirmed cleanup; no recursive deletion or reparse traversal occurs.
    pub fn cleanup(mut self) -> bool {
        if self.writer_active {
            return false;
        }
        self.resources.take().is_none_or(SidecarResources::cleanup)
    }
}
impl Drop for SidecarDownloadFile {
    fn drop(&mut self) {
        if let Some(resources) = self.resources.take() {
            if self.writer_active {
                // Fail closed on caller misuse. Normal cancellation transfers
                // the whole operation to a reaper instead of reaching this path.
                std::mem::forget(resources);
            } else {
                let _ = resources.cleanup();
            }
        }
    }
}
impl VerifiedSidecarFile {
    pub fn publish(self) -> Result<bool> {
        let resources = self.transaction.resources.as_ref().unwrap();
        resources.check_part()?;
        fs::hard_link(&resources.part_path, &resources.destination).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                library_error(ErrorCode::AlreadyExists)
            } else {
                file_error(error)
            }
        })?;
        // Publication is now authoritative. All remaining failures are warnings.
        #[cfg(unix)]
        let durable = resources
            ._directory
            ._ancestors
            .last()
            .unwrap()
            .sync_all()
            .is_ok();
        #[cfg(not(unix))]
        let durable = true;
        drop(self.file);
        Ok(self.transaction.cleanup() && durable)
    }
}
impl SidecarResources {
    fn check_part(&self) -> Result<()> {
        let task = fs::symlink_metadata(&self.task_path).map_err(file_error)?;
        let part = fs::symlink_metadata(&self.part_path).map_err(file_error)?;
        if !task.is_dir()
            || indirect(&task)
            || !part.is_file()
            || indirect(&part)
            || !same_object(
                &identity(self.task_pin.as_ref().unwrap())?,
                &self.task_identity,
            )
            || !same_object(
                &identity(self.part_pin.as_ref().unwrap())?,
                &self.part_identity,
            )
        {
            return Err(library_error(ErrorCode::ModelFileChanged));
        }
        // Windows pinning prevents namespace replacement. Unix is test-only;
        // verify path-to-object identity explicitly without claiming Windows guards.
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if task.ino() != self.task_identity.file
                || task.dev() != self.task_identity.volume
                || part.ino() != self.part_identity.file
                || part.dev() != self.part_identity.volume
            {
                return Err(library_error(ErrorCode::ModelFileChanged));
            }
        }
        Ok(())
    }
    fn open_exclusive_part(&self) -> Result<File> {
        self.check_part()?;
        let mut options = fs::OpenOptions::new();
        options.read(true).write(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            use windows_sys::Win32::Storage::FileSystem::*;
            // Existing identity pin owns DELETE access; allow that handle but
            // deny any existing or subsequent writer for verification/publish.
            options
                .share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        let file = options.open(&self.part_path).map_err(file_error)?;
        let metadata = file.metadata().map_err(file_error)?;
        if !metadata.is_file()
            || indirect(&metadata)
            || !same_object(&identity(&file)?, &self.part_identity)
        {
            return Err(library_error(ErrorCode::ModelFileChanged));
        }
        Ok(file)
    }
    fn only_known_entries(&self) -> Result<bool> {
        for (index, entry) in fs::read_dir(&self.task_path)
            .map_err(file_error)?
            .enumerate()
        {
            let entry = entry.map_err(file_error)?;
            if index >= 3
                || !matches!(
                    entry.file_name().to_str(),
                    Some("payload.part" | "payload.part.aria2" | "payload.part.aria2__temp")
                )
            {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn cleanup_control_files(&self) -> bool {
        let mut cleaned = true;
        for name in ["payload.part.aria2", "payload.part.aria2__temp"] {
            let path = self.task_path.join(name);
            match fs::symlink_metadata(&path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                Ok(metadata) if metadata.is_file() && !indirect(&metadata) => {
                    cleaned &= sidecar_pin(&path, false, false)
                        .is_ok_and(|file| remove_pinned(file, &path));
                }
                _ => cleaned = false,
            }
        }
        cleaned
    }
    fn cleanup(mut self) -> bool {
        // Unknown objects are preserved. All deletions address checked handles.
        if self.check_part().is_err() {
            return false;
        }
        let mut cleaned = self.only_known_entries().unwrap_or(false);
        cleaned &= self.cleanup_control_files();
        cleaned &= remove_pinned(self.part_pin.take().unwrap(), &self.part_path);
        cleaned &= remove_pinned(self.task_pin.take().unwrap(), &self.task_path);
        cleaned
    }
}
fn sidecar_pin(path: &Path, directory: bool, create: bool) -> Result<File> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    if create {
        options.write(true).create_new(true);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::*;
        options
            .access_mode(FILE_READ_ATTRIBUTES | DELETE)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(
                FILE_FLAG_OPEN_REPARSE_POINT
                    | if directory {
                        FILE_FLAG_BACKUP_SEMANTICS
                    } else {
                        0
                    },
            );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | if directory { libc::O_DIRECTORY } else { 0 });
    }
    let file = options.open(path).map_err(file_error)?;
    let metadata = file.metadata().map_err(file_error)?;
    if metadata.is_dir() != directory || (!directory && !metadata.is_file()) || indirect(&metadata)
    {
        return Err(library_error(ErrorCode::ModelFileChanged));
    }
    Ok(file)
}
fn remove_pinned(file: File, path: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::*;
        let _ = path;
        let info = FILE_DISPOSITION_INFO { DeleteFile: true };
        // SAFETY: live owned handle has DELETE access; no path is reopened.
        let removed = unsafe {
            SetFileInformationByHandle(
                file.as_raw_handle().cast(),
                FileDispositionInfo,
                (&info as *const FILE_DISPOSITION_INFO).cast(),
                std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
            )
        } != 0;
        drop(file);
        removed
    }
    #[cfg(not(windows))]
    {
        let Ok(metadata) = file.metadata() else {
            return false;
        };
        let Ok(current) = fs::symlink_metadata(path) else {
            return false;
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.ino() != current.ino() || metadata.dev() != current.dev() {
                return false;
            }
        }
        if indirect(&current) {
            return false;
        }
        drop(file);
        if metadata.is_dir() {
            fs::remove_dir(path).is_ok()
        } else {
            fs::remove_file(path).is_ok()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, ModelLibrary) {
        let root = tempfile::tempdir().unwrap();
        let model = root.path().join("models");
        fs::create_dir(&model).unwrap();
        let scan = scan_directory(root.path(), &model, None, &ScanControl::default()).unwrap();
        let library = scan.library().unwrap().clone();
        (root, library)
    }
    #[cfg(windows)]
    #[test]
    fn windows_protects_partial_and_ancestors_until_atomic_publication() {
        let (_root, library) = fixture();
        let id = Uuid::new_v4();
        let mut file = DownloadFile::create(&library, "protected.gguf", id).unwrap();
        let part = library.directory.join(format!(".nexa-download-{id}.part"));
        file.write(b"GGUFprotected").unwrap();
        assert!(fs::write(&part, b"tampered").is_err());
        assert!(fs::remove_file(&part).is_err());
        assert!(fs::rename(&part, library.directory.join("replacement.part")).is_err());
        assert!(
            fs::rename(
                &library.directory,
                library.directory.with_file_name("moved")
            )
            .is_err()
        );
        file.reset_to_empty().unwrap();
        assert!(fs::write(&part, b"tampered after reset").is_err());
        assert!(fs::remove_file(&part).is_err());
        assert!(fs::rename(&part, library.directory.join("replacement.part")).is_err());
        file.write(b"GGUFprotected").unwrap();
        assert!(file.publish().unwrap());
        assert_eq!(
            fs::read(library.directory.join("protected.gguf")).unwrap(),
            b"GGUFprotected"
        );
        assert!(!part.exists());
        let open_target = File::open(library.directory.join("protected.gguf")).unwrap();
        assert!(DownloadFile::create(&library, "protected.gguf", Uuid::new_v4()).is_err());
        drop(open_target);
    }
    #[test]
    fn publication_is_no_clobber_and_drop_removes_only_own_partial() {
        let (_root, library) = fixture();
        let id = Uuid::new_v4();
        let mut file = DownloadFile::create(&library, "test.gguf", id).unwrap();
        file.write(b"GGUFtest").unwrap();
        assert!(!library.directory.join("test.gguf").exists());
        file.publish().unwrap();
        assert_eq!(
            fs::read(library.directory.join("test.gguf")).unwrap(),
            b"GGUFtest"
        );
        assert!(
            !library
                .directory
                .join(format!(".nexa-download-{id}.part"))
                .exists()
        );
        assert!(DownloadFile::create(&library, "test.gguf", Uuid::new_v4()).is_err());
        let cancelled = Uuid::new_v4();
        drop(DownloadFile::create(&library, "other.gguf", cancelled).unwrap());
        assert!(
            !library
                .directory
                .join(format!(".nexa-download-{cancelled}.part"))
                .exists()
        );
        assert!(!library.directory.join("other.gguf").exists());
    }
    #[test]
    fn restart_truncates_and_rewinds_the_same_protected_file() {
        let (_root, library) = fixture();
        let id = Uuid::new_v4();
        let mut file = DownloadFile::create(&library, "reset.gguf", id).unwrap();
        file.write(b"GGUF old bytes that must disappear").unwrap();
        let before = identity(file.file.as_ref().unwrap()).unwrap();
        file.reset_to_empty().unwrap();
        let after = identity(file.file.as_ref().unwrap()).unwrap();
        assert!(same_object(&before, &after));
        assert_eq!(file.file.as_ref().unwrap().metadata().unwrap().len(), 0);
        file.write(b"GGUFnew").unwrap();
        file.publish().unwrap();
        assert_eq!(
            fs::read(library.directory.join("reset.gguf")).unwrap(),
            b"GGUFnew"
        );
        assert!(
            !library
                .directory
                .join(format!(".nexa-download-{id}.part"))
                .exists()
        );
    }
    #[test]
    fn target_created_during_transfer_is_never_replaced() {
        let (_root, library) = fixture();
        let mut file = DownloadFile::create(&library, "test.gguf", Uuid::new_v4()).unwrap();
        file.write(b"new").unwrap();
        fs::write(library.directory.join("test.gguf"), b"existing").unwrap();
        assert!(file.publish().is_err());
        assert_eq!(
            fs::read(library.directory.join("test.gguf")).unwrap(),
            b"existing"
        );
    }
    #[test]
    fn changed_directory_and_traversal_are_rejected() {
        let (_root, mut library) = fixture();
        assert!(DownloadFile::create(&library, "../bad.gguf", Uuid::new_v4()).is_err());
        library.directory_identity.file ^= 1;
        assert!(DownloadFile::create(&library, "test.gguf", Uuid::new_v4()).is_err());
    }
    fn sidecar_fixture() -> (
        tempfile::TempDir,
        ModelLibrary,
        SidecarDownloadFile,
        SidecarPaths,
    ) {
        let (root, library) = fixture();
        let mut transaction =
            SidecarDownloadFile::create(&library, "sidecar.gguf", Uuid::new_v4()).unwrap();
        let paths = transaction.begin_attempt().unwrap();
        (root, library, transaction, paths)
    }
    fn fixture_hash(bytes: &[u8]) -> [u8; 32] {
        Sha256::digest(bytes).into()
    }
    #[test]
    fn sidecar_requires_stop_before_reset_and_reuses_same_part_identity() {
        let (_root, _library, mut transaction, paths) = sidecar_fixture();
        let before = identity(
            transaction
                .resources
                .as_ref()
                .unwrap()
                .part_pin
                .as_ref()
                .unwrap(),
        )
        .unwrap();
        fs::write(&paths.payload, b"GGUF old partial bytes").unwrap();
        assert_eq!(
            transaction.reset_for_retry().unwrap_err().code,
            ErrorCode::ExecutorCleanupUnconfirmed
        );
        assert!(transaction.begin_attempt().is_err());
        transaction.confirm_writer_stopped();
        fs::write(paths.directory.join("payload.part.aria2"), b"control").unwrap();
        fs::write(
            paths.directory.join("payload.part.aria2__temp"),
            b"temporary",
        )
        .unwrap();
        transaction.reset_for_retry().unwrap();
        let after = identity(
            transaction
                .resources
                .as_ref()
                .unwrap()
                .part_pin
                .as_ref()
                .unwrap(),
        )
        .unwrap();
        assert!(same_object(&before, &after));
        assert_eq!(transaction.observed_size().unwrap(), 0);
        assert!(!paths.directory.join("payload.part.aria2").exists());
        let retry = transaction.begin_attempt().unwrap();
        assert_eq!(retry.payload, paths.payload);
        transaction.confirm_writer_stopped();
        assert!(transaction.cleanup());
        assert!(!paths.directory.exists());
    }
    #[test]
    fn sidecar_size_hash_cancel_and_no_clobber_are_independent_of_exit_status() {
        for mode in ["success", "short", "hash", "cancel", "existing"] {
            let (root, library, mut transaction, paths) = sidecar_fixture();
            let bytes = b"GGUF verified bytes";
            fs::write(&paths.payload, bytes).unwrap();
            transaction.confirm_writer_stopped();
            let size = bytes.len() as u64 + u64::from(mode == "short");
            let hash = if mode == "hash" {
                [0; 32]
            } else {
                fixture_hash(bytes)
            };
            let verified = transaction.verify(size, hash, || mode == "cancel");
            if matches!(mode, "short" | "hash" | "cancel") {
                assert!(verified.is_err(), "{mode}");
                assert!(!paths.directory.exists());
                assert!(!library.directory.join("sidecar.gguf").exists());
            } else {
                if mode == "existing" {
                    fs::write(library.directory.join("sidecar.gguf"), b"existing").unwrap();
                }
                let published = verified.unwrap().publish();
                assert_eq!(published.is_ok(), mode == "success");
                let expected: &[u8] = if mode == "success" {
                    bytes
                } else {
                    b"existing"
                };
                assert_eq!(
                    fs::read(library.directory.join("sidecar.gguf")).unwrap(),
                    expected
                );
                assert!(!paths.directory.exists());
            }
            assert!(!root.path().join(LIBRARY_FILE).exists());
        }
    }
    #[test]
    fn sidecar_unknown_objects_are_preserved_and_saved_result_has_cleanup_warning() {
        let (_root, library, mut transaction, paths) = sidecar_fixture();
        let bytes = b"GGUF valid";
        fs::write(&paths.payload, bytes).unwrap();
        let unknown = paths.directory.join("do-not-delete");
        fs::write(&unknown, b"unrelated").unwrap();
        transaction.confirm_writer_stopped();
        assert!(transaction.reset_for_retry().is_err());
        let verified = transaction
            .verify(bytes.len() as u64, fixture_hash(bytes), || false)
            .unwrap();
        assert!(!verified.publish().unwrap());
        assert_eq!(
            fs::read(library.directory.join("sidecar.gguf")).unwrap(),
            bytes
        );
        assert_eq!(fs::read(&unknown).unwrap(), b"unrelated");
        assert!(!paths.payload.exists());
    }
    #[test]
    fn sidecar_never_adopts_existing_task_directory_or_changed_library() {
        let (_root, mut library) = fixture();
        let id = Uuid::new_v4();
        let task = library.directory.join(format!(".nexa-download-{id}"));
        fs::create_dir(&task).unwrap();
        fs::write(task.join("preserve"), b"existing").unwrap();
        assert!(SidecarDownloadFile::create(&library, "sidecar.gguf", id).is_err());
        assert_eq!(fs::read(task.join("preserve")).unwrap(), b"existing");
        library.directory_identity.file ^= 1;
        assert!(SidecarDownloadFile::create(&library, "sidecar.gguf", Uuid::new_v4()).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn sidecar_cleanup_does_not_follow_control_symlinks_or_replaced_part() {
        use std::os::unix::fs::symlink;
        let (_root, library, mut transaction, paths) = sidecar_fixture();
        let outside = library.directory.join("outside");
        fs::write(&outside, b"preserve").unwrap();
        symlink(&outside, paths.directory.join("payload.part.aria2")).unwrap();
        transaction.confirm_writer_stopped();
        assert!(!transaction.cleanup());
        assert_eq!(fs::read(&outside).unwrap(), b"preserve");
        let mut transaction =
            SidecarDownloadFile::create(&library, "sidecar.gguf", Uuid::new_v4()).unwrap();
        let paths = transaction.begin_attempt().unwrap();
        fs::remove_file(&paths.payload).unwrap();
        symlink(&outside, &paths.payload).unwrap();
        transaction.confirm_writer_stopped();
        assert!(
            transaction
                .verify(8, fixture_hash(b"preserve"), || false)
                .is_err()
        );
        assert_eq!(fs::read(&outside).unwrap(), b"preserve");
    }
    #[cfg(windows)]
    #[test]
    fn windows_sidecar_pin_allows_writer_but_denies_replacement_then_verifier_denies_writers() {
        let (_root, library, mut transaction, paths) = sidecar_fixture();
        let bytes = b"GGUF Windows protected adoption";
        fs::write(&paths.payload, bytes).unwrap();
        assert!(fs::remove_file(&paths.payload).is_err());
        assert!(fs::rename(&paths.payload, paths.directory.join("swapped")).is_err());
        assert!(fs::rename(&paths.directory, library.directory.join("moved")).is_err());
        assert!(
            fs::rename(
                &library.directory,
                library.directory.with_file_name("moved-root")
            )
            .is_err()
        );
        let writer = fs::OpenOptions::new()
            .write(true)
            .open(&paths.payload)
            .unwrap();
        transaction.confirm_writer_stopped();
        assert!(
            transaction
                .resources
                .as_ref()
                .unwrap()
                .open_exclusive_part()
                .is_err()
        );
        drop(writer);
        let verified = transaction
            .verify(bytes.len() as u64, fixture_hash(bytes), || false)
            .unwrap();
        assert!(fs::write(&paths.payload, b"tamper").is_err());
        assert!(fs::remove_file(&paths.payload).is_err());
        assert!(verified.publish().unwrap());
        assert_eq!(
            fs::read(library.directory.join("sidecar.gguf")).unwrap(),
            bytes
        );
        assert!(!paths.directory.exists());
    }
    #[test]
    fn verified_drop_cleans_staging_without_publication() {
        let (_root, library, mut transaction, paths) = sidecar_fixture();
        let bytes = b"GGUF cancelled before publication";
        fs::write(&paths.payload, bytes).unwrap();
        transaction.confirm_writer_stopped();
        let verified = transaction
            .verify(bytes.len() as u64, fixture_hash(bytes), || false)
            .unwrap();
        drop(verified);
        assert!(!paths.directory.exists());
        assert!(!library.directory.join("sidecar.gguf").exists());
    }
    #[test]
    fn sidecar_initialization_rolls_back_only_owned_pinned_objects() {
        for fail_at in [CreationStep::TaskPinned, CreationStep::PartPinned] {
            let (_root, library) = fixture();
            let id = Uuid::new_v4();
            let result =
                SidecarDownloadFile::create_checked(&library, "sidecar.gguf", id, |step, _| {
                    if std::mem::discriminant(&step) == std::mem::discriminant(&fail_at) {
                        Err(library_error(ErrorCode::Io))
                    } else {
                        Ok(())
                    }
                });
            assert!(result.is_err());
            assert!(
                !library
                    .directory
                    .join(format!(".nexa-download-{id}"))
                    .exists()
            );
        }
        let (_root, library) = fixture();
        let id = Uuid::new_v4();
        let result =
            SidecarDownloadFile::create_checked(&library, "sidecar.gguf", id, |step, path| {
                if matches!(step, CreationStep::TaskPinned) {
                    fs::write(path.join("payload.part"), b"unowned collision").unwrap();
                }
                Ok(())
            });
        assert!(result.is_err());
        let path = library.directory.join(format!(".nexa-download-{id}"));
        assert_eq!(
            fs::read(path.join("payload.part")).unwrap(),
            b"unowned collision"
        );
        let id = Uuid::new_v4();
        let result =
            SidecarDownloadFile::create_checked(&library, "sidecar.gguf", id, |step, _| {
                if matches!(step, CreationStep::BeforeTaskPin) {
                    Err(library_error(ErrorCode::Io))
                } else {
                    Ok(())
                }
            });
        assert!(result.is_err());
        // Without a verified handle, do not claim ownership of the path.
        assert!(
            library
                .directory
                .join(format!(".nexa-download-{id}"))
                .exists()
        );
    }
}
