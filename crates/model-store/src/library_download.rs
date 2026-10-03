//! Destination transaction for explicitly requested catalog downloads.
//! The read-only scanner never calls this. Windows pins every ancestor and
//! prevents writes/deletion of the partial file until no-clobber publication.
use super::*;
use std::io::Write;

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
}
