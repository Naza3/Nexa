//! Metadata-only unregister transactions. Payloads, manifests, profiles and
//! validation receipts are never removed. The caller also owns either the
//! runtime actor's registry lease or the stopped desktop instance lock.
use crate::{
    ModelManifest, Result, inventory,
    library::{self, ModelLibrary, UnregisteredModel},
};
use fs2::FileExt;
use runtime_types::{ErrorCode, ModelId};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub struct CatalogLock {
    _directory: library::DirectoryGuard,
    _runtime: library::DirectoryGuard,
    _lock: fs::File,
}
impl CatalogLock {
    /// Shares the store's exact OS lock, without opening/recovering a store or
    /// hashing any model. Never substitutes for the runtime instance lock.
    pub fn acquire(root: &Path) -> Result<Self> {
        let directory = library::DirectoryGuard::open_data_directory(root)?;
        let runtime = root.join("runtime");
        let _runtime = library::DirectoryGuard::open_data_directory(&runtime)?;
        let path = runtime.join("model-store.lock");
        if let Ok(metadata) = fs::symlink_metadata(&path)
            && (!metadata.is_file() || library::indirect(&metadata))
        {
            return Err(library::library_error(ErrorCode::ModelLibraryChanged));
        }
        let mut options = fs::OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .mode(0o600);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(
                windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT,
            );
        }
        let lock = options.open(path).map_err(crate::io_error)?;
        let opened = lock.metadata().map_err(crate::io_error)?;
        if !opened.is_file() || library::indirect(&opened) {
            return Err(library::library_error(ErrorCode::ModelLibraryChanged));
        }
        lock.try_lock_exclusive()
            .map_err(|_| library::library_error(ErrorCode::RuntimeBusy))?;
        Ok(Self {
            _directory: directory,
            _runtime,
            _lock: lock,
        })
    }
}
impl Drop for CatalogLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self._lock);
    }
}

/// Requires the shared CatalogLock (or the owning ModelStore) for the entire
/// read/prepare/publication. Keeping this separate permits no-hash offline use.
pub fn prepare(root: &Path, id: &ModelId) -> Result<(ModelLibrary, ModelManifest)> {
    let inventory = inventory::read(root)?;
    let entry = inventory
        .entries
        .into_iter()
        .find(|entry| &entry.manifest.id == id)
        .ok_or_else(|| library::library_error(ErrorCode::ModelNotFound))?;
    let mut library = ModelLibrary::read(root)?.unwrap_or_else(ModelLibrary::empty);
    if library.unregistered.len() >= inventory::MAX_INVENTORY_MODELS {
        return Err(library::library_error(ErrorCode::ModelLibraryLimit));
    }
    let registration_sha256 = library.registration_key(&entry.manifest)?;
    library.restore(id);
    library.unregistered.push(UnregisteredModel {
        model_id: id.clone(),
        registration_sha256,
    });
    library.schema_version = 3;
    library.library_generation = uuid::Uuid::new_v4();
    library.validate()?;
    Ok((library, entry.manifest))
}

/// Atomic same-directory publication. On a post-rename flush error, the new
/// document is already visible; callers must publish/re-read that observation.
pub fn publish(root: &Path, library: &ModelLibrary) -> Result<()> {
    publish_with_commit(root, library, || {})
}
/// `committed` runs immediately after the atomic replacement, before any
/// fallible durability observation. Its owner already holds all mutation locks.
pub fn publish_with_commit(
    root: &Path,
    library: &ModelLibrary,
    committed: impl FnOnce(),
) -> Result<()> {
    publish_with_sync(root, library, committed, || {
        #[cfg(unix)]
        fs::File::open(root)
            .and_then(|file| file.sync_all())
            .map_err(|_| library::library_error(ErrorCode::ModelLibraryWriteFailed))?;
        Ok(())
    })
}
fn publish_with_sync(
    root: &Path,
    library: &ModelLibrary,
    committed: impl FnOnce(),
    sync: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let _directory = library::DirectoryGuard::open_data_directory(root)?;
    ModelLibrary::read(root)?; // reject corrupt/symlinked targets before replacement
    let bytes = library.encode()?;
    let target = root.join(library::LIBRARY_FILE);
    let temporary = root.join(format!(".unregister-{}.tmp", uuid::Uuid::new_v4()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary).map_err(crate::io_error)?;
    let cleanup = Temporary(temporary.clone());
    file.write_all(&bytes).map_err(crate::io_error)?;
    file.sync_all().map_err(crate::io_error)?;
    drop(file);
    crate::local_validation::replace(&temporary, &target).map_err(crate::io_error)?;
    committed();
    drop(cleanup);
    sync()
}
struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rename_commit_is_published_even_if_durability_confirmation_fails() {
        let root = tempfile::tempdir().unwrap();
        let library = ModelLibrary::empty();
        let mut committed = false;
        let error = publish_with_sync(
            root.path(),
            &library,
            || committed = true,
            || Err(library::library_error(ErrorCode::ModelLibraryWriteFailed)),
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::ModelLibraryWriteFailed);
        assert!(committed);
        assert_eq!(
            ModelLibrary::read(root.path())
                .unwrap()
                .unwrap()
                .library_generation,
            library.library_generation
        );
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }
    #[test]
    fn schema3_is_strict_bounded_and_older_documents_remain_readable_without_writes() {
        let root = tempfile::tempdir().unwrap();
        let library = ModelLibrary::empty();
        let mut value = serde_json::to_value(&library).unwrap();
        value["schema_version"] = 2.into();
        let bytes = serde_json::to_vec(&value).unwrap();
        fs::write(root.path().join(library::LIBRARY_FILE), &bytes).unwrap();
        assert_eq!(
            ModelLibrary::read(root.path())
                .unwrap()
                .unwrap()
                .schema_version,
            2
        );
        assert_eq!(
            fs::read(root.path().join(library::LIBRARY_FILE)).unwrap(),
            bytes
        );
        value["unknown"] = true.into();
        fs::write(root.path().join(library::LIBRARY_FILE), value.to_string()).unwrap();
        assert!(ModelLibrary::read(root.path()).is_err());
        let mut bounded = ModelLibrary::empty();
        bounded.unregistered = (0..=inventory::MAX_INVENTORY_MODELS)
            .map(|i| UnregisteredModel {
                model_id: ModelId::new(format!("hidden-{i}")).unwrap(),
                registration_sha256: "a".repeat(64),
            })
            .collect();
        assert!(bounded.encode().is_err());
        bounded.unregistered.truncate(1);
        bounded.unregistered.push(bounded.unregistered[0].clone());
        assert!(bounded.encode().is_err());
    }
    #[test]
    #[cfg(unix)]
    fn indirect_catalog_and_lock_are_rejected_without_touching_targets() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("runtime")).unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        fs::write(outside.path(), b"sentinel").unwrap();
        symlink(outside.path(), root.path().join("runtime/model-store.lock")).unwrap();
        assert!(CatalogLock::acquire(root.path()).is_err());
        symlink(outside.path(), root.path().join(library::LIBRARY_FILE)).unwrap();
        let mut committed = false;
        assert!(
            publish_with_commit(root.path(), &ModelLibrary::empty(), || committed = true).is_err()
        );
        assert!(!committed);
        assert_eq!(fs::read(outside.path()).unwrap(), b"sentinel");
    }
}
