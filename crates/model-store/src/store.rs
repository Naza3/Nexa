use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{
    Arc, Mutex, MutexGuard,
    atomic::{AtomicBool, Ordering},
};

use cap_std::fs::{Dir, OpenOptions};
use fs2::FileExt;
use runtime_types::{ErrorCode, ModelId, ResolvedModel, RuntimeError};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::library::{
    LibraryDirectoryInfo, ModelLibrary, PreparedExternal, ScanControl, library_error,
};
use crate::manifest::validate_portable_id;
use crate::{ImportRequest, ModelManifest, ModelStorage, Result, gguf, invalid_manifest, io_error};

// A failed external cleanup permanently poisons this process's catalog owner.
// This bounds fail-closed retained leases instead of accumulating rebuilt stores.
static EXTERNAL_CLEANUP_UNCONFIRMED: AtomicBool = AtomicBool::new(false);
const COPY_BUFFER: usize = 64 * 1024;
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const SPACE_RESERVE: u64 = 1024 * 1024;

#[derive(Clone, Default, Debug)]
pub struct ImportCancellation(Arc<AtomicBool>);
impl ImportCancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
    fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(RuntimeError::new(
                ErrorCode::RequestCancelled,
                "model import cancelled",
            ))
        } else {
            Ok(())
        }
    }
}

/// Owns an exclusive process lock for the lifetime of the store. Share this
/// instance through Arc rather than opening the same data directory twice.
/// Methods are blocking; async callers must use a dedicated blocking executor.
/// No method downloads models or modifies/deletes the user's source file.
pub struct ModelStore {
    root_path: PathBuf,
    root: Dir,
    _process_lock: fs::File,
    gate: Mutex<()>,
    verified: Mutex<BTreeMap<ModelId, VerifiedModel>>,
    library: Mutex<Option<ModelLibrary>>,
    external_prepared: Mutex<BTreeMap<ModelId, PreparedExternal>>,
}
impl ModelStore {
    pub fn data_directory(&self) -> &Path {
        &self.root_path
    }
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self> {
        if EXTERNAL_CLEANUP_UNCONFIRMED.load(Ordering::Acquire) {
            return Err(library_error(ErrorCode::ExecutorCleanupUnconfirmed));
        }
        let root_path = prepare_root(data_dir.as_ref())?;
        let root =
            Dir::open_ambient_dir(&root_path, cap_std::ambient_authority()).map_err(io_error)?;
        ensure_directory(&root, Path::new("runtime"))?;
        let lock_path = Path::new("runtime/model-store.lock");
        if exists(&root, lock_path)? {
            ensure_regular(&root, lock_path)?;
        }
        let lock = root
            .open_with(
                lock_path,
                OpenOptions::new().read(true).write(true).create(true),
            )
            .map_err(io_error)?
            .into_std();
        lock.try_lock_exclusive().map_err(|error| {
            if error.raw_os_error() == fs2::lock_contended_error().raw_os_error()
                || error.kind() == std::io::ErrorKind::WouldBlock
            {
                RuntimeError::new(
                    ErrorCode::RuntimeBusy,
                    "model data directory is already in use",
                )
            } else {
                io_error(error)
            }
        })?;
        ensure_directory(&root, Path::new("models"))?;
        ensure_directory(&root, Path::new("imports"))?;
        let library = ModelLibrary::read(&root_path)?;
        let store = Self {
            root_path,
            root,
            _process_lock: lock,
            gate: Mutex::new(()),
            verified: Mutex::new(BTreeMap::new()),
            library: Mutex::new(library),
            external_prepared: Mutex::new(BTreeMap::new()),
        };
        store.recover_imports()?;
        for manifest in store.list_managed()? {
            if store.is_external(&manifest.id) {
                return Err(library_error(ErrorCode::ModelLibraryChanged));
            }
            store.verify(&manifest.id)?;
        }
        Ok(store)
    }

    /// Opens a regular, readable source before checking space and importing.
    /// Symlink sources are refused; callers can explicitly select the target.
    pub fn import_file(
        &self,
        source: impl AsRef<Path>,
        request: ImportRequest,
        cancel: &ImportCancellation,
    ) -> Result<ModelManifest> {
        request.validate()?;
        cancel.check()?;
        if self.is_external(&request.id) {
            return Err(RuntimeError::new(
                ErrorCode::AlreadyExists,
                "model ID already registered externally",
            ));
        }
        #[cfg(windows)]
        validate_local_source_path(source.as_ref())?;
        let metadata = fs::symlink_metadata(source.as_ref()).map_err(io_error)?;
        if !metadata.file_type().is_file() {
            return Err(RuntimeError::invalid("model source must be a regular file"));
        }
        let mut options = fs::OpenOptions::new();
        options.read(true);
        // The preflight metadata check alone is insufficient: a source can be
        // replaced by a symlink/FIFO between metadata and open. Never block on
        // opening a special file or follow a final-component reparse point.
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(
                windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT,
            );
        }
        let source = options.open(source.as_ref()).map_err(io_error)?;
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Storage::FileSystem::{FILE_TYPE_DISK, GetFileType};
            // SAFETY: the owned File keeps this valid handle alive during the
            // query. Do not read a DOS character device or named pipe.
            if unsafe { GetFileType(source.as_raw_handle().cast()) } != FILE_TYPE_DISK {
                return Err(RuntimeError::invalid("model source must be a disk file"));
            }
        }
        let opened = source.metadata().map_err(io_error)?;
        if !opened.is_file() {
            return Err(RuntimeError::invalid("model source must be a regular file"));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if opened.file_attributes()
                & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
                != 0
            {
                return Err(RuntimeError::invalid(
                    "model source must not be a reparse point",
                ));
            }
        }
        self.import_reader(source, opened.len(), request, cancel)
    }

    /// Imports a stream whose exact byte length is known. Stream acquisition
    /// remains the caller's responsibility. Short and oversized streams
    /// fail, preserving the source and cleaning the incomplete destination.
    /// A blocking Read cannot itself be interrupted; adapters should arrange for
    /// their read to return when cancelled. Cancellation is checked per chunk and
    /// again at the atomic registration boundary.
    pub fn import_reader<R: Read>(
        &self,
        mut source: R,
        size_bytes: u64,
        request: ImportRequest,
        cancel: &ImportCancellation,
    ) -> Result<ModelManifest> {
        request.validate()?;
        cancel.check()?;
        if self.is_external(&request.id) {
            return Err(RuntimeError::new(
                ErrorCode::AlreadyExists,
                "model ID already registered externally",
            ));
        }
        let _gate = self.acquire()?;
        cancel.check()?;
        self.check_layout()?;
        let destination = model_directory(&request.id);
        if exists(&self.root, &destination)? {
            return Err(RuntimeError::new(
                ErrorCode::AlreadyExists,
                "model ID already exists",
            ));
        }
        if size_bytes == 0 {
            return Err(invalid_manifest("model source is empty"));
        }
        if size_bytes > crate::library::MAX_MODEL_BYTES {
            return Err(RuntimeError::new(
                ErrorCode::ModelLibraryLimit,
                "single GGUF exceeds the 16 GiB file budget",
            ));
        }
        let required = size_bytes.checked_add(SPACE_RESERVE).ok_or_else(|| {
            RuntimeError::new(
                ErrorCode::InsufficientSpace,
                "model size exceeds space-check bounds",
            )
        })?;
        let available = fs2::available_space(&self.root_path).map_err(io_error)?;
        if available < required {
            return Err(RuntimeError::new(
                ErrorCode::InsufficientSpace,
                "insufficient space for a private model copy",
            ));
        }
        let stem = format!("imports/import-{}", Uuid::new_v4().simple());
        let partial = PathBuf::from(format!("{stem}.partial"));
        let staged = PathBuf::from(format!("{stem}.staged"));
        let _cleanup = ImportCleanup {
            root: &self.root,
            partial: partial.clone(),
            staged: staged.clone(),
        };
        let mut output = self
            .root
            .open_with(
                &partial,
                OpenOptions::new().write(true).read(true).create_new(true),
            )
            .map_err(io_error)?;
        let mut hasher = Sha256::new();
        let mut copied = 0_u64;
        let mut buffer = [0_u8; COPY_BUFFER];
        loop {
            cancel.check()?;
            let count = match source.read(&mut buffer) {
                Ok(count) => count,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(io_error(error)),
            };
            if count == 0 {
                break;
            }
            copied = copied
                .checked_add(count as u64)
                .filter(|&n| n <= size_bytes)
                .ok_or_else(|| invalid_manifest("source stream exceeds its declared size"))?;
            output.write_all(&buffer[..count]).map_err(io_error)?;
            hasher.update(&buffer[..count]);
        }
        cancel.check()?;
        if copied != size_bytes {
            return Err(invalid_manifest(
                "source stream is shorter than its declared size",
            ));
        }
        output.sync_all().map_err(io_error)?;
        output.seek(SeekFrom::Start(0)).map_err(io_error)?;
        let metadata = gguf::read(&mut output)?;
        cancel.check()?;
        let manifest = ModelManifest::build(
            request,
            copied,
            format!("{:x}", hasher.finalize()),
            metadata,
        )?;
        let encoded = encode_manifest(&manifest)?;
        drop(output); // Windows rename must not retain an open writer.
        self.root.create_dir(&staged).map_err(io_error)?;
        self.root
            .rename(&partial, &self.root, staged.join("model.gguf"))
            .map_err(io_error)?;
        let mut file = self
            .root
            .open_with(
                staged.join("manifest.json"),
                OpenOptions::new().write(true).create_new(true),
            )
            .map_err(io_error)?;
        file.write_all(&encoded).map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        drop(file);
        sync_directory(&self.root, &staged)?;
        cancel.check()?;
        self.check_layout()?;
        if exists(&self.root, &destination)? {
            return Err(RuntimeError::new(
                ErrorCode::AlreadyExists,
                "model ID appeared before registration",
            ));
        }
        self.publish_import(&staged, &destination, &manifest, sync_committed_directory)?;
        Ok(manifest)
    }

    /// Returns a coherent manifest-derived index. This verifies metadata and file
    /// sizes, not all model hashes. Import/open/verify establish hash integrity;
    /// resolve checks that the cached verified identity has not visibly changed.
    fn list_managed(&self) -> Result<Vec<ModelManifest>> {
        let _gate = self.acquire()?;
        self.list_managed_locked()
    }
    fn list_managed_locked(&self) -> Result<Vec<ModelManifest>> {
        self.check_layout()?;
        let mut models = Vec::new();
        for entry in self.root.read_dir("models").map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            let name = entry.file_name();
            let name = name
                .to_str()
                .ok_or_else(|| invalid_manifest("non-UTF-8 model directory name"))?;
            let id = ModelId::new(name)
                .map_err(|_| invalid_manifest("invalid registered model directory name"))?;
            // Hidden managed copies still own their IDs; suppression must not
            // hide a corrupt external/managed namespace collision.
            if self.is_external(&id) {
                return Err(library_error(ErrorCode::ModelLibraryChanged));
            }
            match self.read_manifest(&id) {
                Ok(manifest) => models.push(manifest),
                Err(error) if error.code == ErrorCode::ModelNotFound => (),
                Err(error) => return Err(error),
            }
        }
        models.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(models)
    }

    pub fn list(&self) -> Result<Vec<ModelManifest>> {
        let _gate = self.acquire()?;
        let mut models = self.list_managed_locked()?;
        if let Some(library) = &*self.library.lock().map_err(|_| cache_error())? {
            models.extend(
                library
                    .models
                    .iter()
                    .filter(|m| !library.is_unregistered(&m.manifest))
                    .map(|m| m.manifest.clone()),
            );
        }
        models.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(models)
    }
    /// Trusted lifecycle owner only: call after Runtime shutdown and confirmed
    /// worker cleanup. Unload is deliberately insufficient.
    pub fn release_external_after_shutdown(&self) {
        self.external_prepared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }
    pub fn library_info(&self) -> Option<LibraryDirectoryInfo> {
        self.library
            .lock()
            .ok()?
            .as_ref()
            .and_then(ModelLibrary::info)
    }
    pub fn is_external(&self, id: &ModelId) -> bool {
        self.library
            .lock()
            .map(|l| l.as_ref().is_some_and(|l| l.entry(id).is_some()))
            .unwrap_or(true)
    }
    pub fn external_availability(&self, id: &ModelId) -> Option<ErrorCode> {
        let library_guard = self.library.lock().ok()?;
        let library = library_guard.as_ref()?;
        let entry = library.entry(id)?;
        if library.is_unregistered(&entry.manifest) {
            return Some(ErrorCode::ModelNotFound);
        }
        if !cfg!(windows) {
            return Some(ErrorCode::ModelDirectoryUnsupported);
        }
        library.availability(entry)
    }
    pub fn needs_external_preparation(&self, id: &ModelId) -> Result<bool> {
        self.check_registered(id)?;
        if !self.is_external(id) {
            return Ok(false);
        }
        let prepared = self.external_prepared.lock().map_err(|_| cache_error())?;
        match prepared.get(id) {
            Some(model) => {
                model.resolve()?;
                Ok(false)
            }
            None => Ok(true),
        }
    }
    pub fn prepare_external(&self, id: &ModelId, control: &ScanControl) -> Result<()> {
        let library_snapshot = self.library.lock().map_err(|_| cache_error())?.clone();
        let Some(library) = &library_snapshot else {
            return Ok(());
        };
        let Some(entry) = library.entry(id) else {
            return Ok(());
        };
        control.check()?;
        // Source identity errors take precedence over historical evidence.
        if let Some(code) = library.availability(entry) {
            return Err(library_error(code));
        }
        if !entry.manifest.load_candidate() {
            return Err(library_error(ErrorCode::UnsupportedModel));
        }
        let _gate = self
            .gate
            .try_lock()
            .map_err(|_| library_error(ErrorCode::RuntimeBusy))?;
        if !self.needs_external_preparation(id)? {
            return Ok(());
        }
        let prepared = PreparedExternal::prepare(library, entry, control)?;
        control.check()?;
        self.external_prepared
            .lock()
            .map_err(|_| cache_error())?
            .insert(id.clone(), prepared);
        Ok(())
    }
    pub fn get(&self, id: &ModelId) -> Result<ModelManifest> {
        self.check_registered(id)?;
        if let Some(entry) = self
            .library
            .lock()
            .map_err(|_| cache_error())?
            .as_ref()
            .and_then(|l| l.entry(id))
            .cloned()
        {
            return Ok(entry.manifest);
        }
        let _gate = self.acquire()?;
        self.check_layout()?;
        self.read_manifest(id)
    }

    /// Returns a cached, verified identity after bounded manifest and filesystem
    /// metadata checks. Full hashing happens at import/open/explicit verify, never
    /// inside this scheduler-facing call. Changes require explicit re-verification.
    /// The private directory must not be edited by other processes; metadata is a
    /// change detector, not authentication against a writer forging timestamps.
    /// Keep the store alive while using the result and coordinate remove/unload in
    /// the scheduler; a bare path is not a model lease.
    pub fn resolve(&self, id: &ModelId) -> Result<ResolvedModel> {
        self.check_registered(id)?;
        if self.is_external(id) {
            return self
                .external_prepared
                .lock()
                .map_err(|_| cache_error())?
                .get(id)
                .ok_or_else(|| library_error(ErrorCode::ModelFileUnavailable))?
                .resolve();
        }
        let _gate = self.gate.try_lock().map_err(|error| match error {
            std::sync::TryLockError::WouldBlock => RuntimeError::new(
                ErrorCode::RuntimeBusy,
                "model store is importing or verifying",
            ),
            std::sync::TryLockError::Poisoned(_) => {
                RuntimeError::new(ErrorCode::RuntimeFaulted, "model store lock was poisoned")
            }
        })?;
        self.check_layout()?;
        let manifest = self.read_manifest(id)?;
        let relative = model_directory(id).join("model.gguf");
        let current = fingerprint(&self.root, &relative)?;
        let verified = self.verified.lock().map_err(|_| cache_error())?;
        if !verified
            .get(id)
            .is_some_and(|cached| cached.manifest == manifest && cached.fingerprint == current)
        {
            return Err(RuntimeError::new(
                ErrorCode::IntegrityFailure,
                "registered model changed; explicit verification required",
            ));
        }
        Ok(ResolvedModel {
            id: id.clone(),
            path: self.root_path.join(relative),
            context_limit: manifest.executable_context_limit(),
            default_context: manifest.default_context,
            loadable: manifest.load_candidate(),
        })
    }

    /// Blocking full SHA-256 and structural verification of a registered copy.
    /// Run on a blocking executor before handing the store to the scheduler.
    pub fn verify(&self, id: &ModelId) -> Result<ModelManifest> {
        let _gate = self.acquire()?;
        self.check_layout()?;
        self.verified.lock().map_err(|_| cache_error())?.remove(id);
        let manifest = self.read_manifest(id)?;
        let relative = model_directory(id).join("model.gguf");
        let before = fingerprint(&self.root, &relative)?;
        let mut file = self.root.open(&relative).map_err(io_error)?;
        let mut hasher = Sha256::new();
        let mut buffer = [0_u8; COPY_BUFFER];
        loop {
            let count = file.read(&mut buffer).map_err(io_error)?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }
        if format!("{:x}", hasher.finalize()) != manifest.sha256 {
            return Err(RuntimeError::new(
                ErrorCode::IntegrityFailure,
                "registered model SHA-256 mismatch",
            ));
        }
        file.seek(SeekFrom::Start(0)).map_err(io_error)?;
        if !manifest.matches_metadata(&gguf::read(&mut file)?)
            || before != fingerprint(&self.root, &relative)?
        {
            return Err(RuntimeError::new(
                ErrorCode::IntegrityFailure,
                "registered model changed during verification or differs from manifest",
            ));
        }
        self.verified.lock().map_err(|_| cache_error())?.insert(
            id.clone(),
            VerifiedModel {
                manifest: manifest.clone(),
                fingerprint: before,
            },
        );
        Ok(manifest)
    }

    fn check_registered(&self, id: &ModelId) -> Result<()> {
        let library = self.library.lock().map_err(|_| cache_error())?;
        if let Some(library) = library.as_ref()
            && library
                .entry(id)
                .is_some_and(|entry| library.is_unregistered(&entry.manifest))
        {
            return Err(library_error(ErrorCode::ModelNotFound));
        }
        Ok(())
    }
    /// Unregister only. The actor lease must cover this call and its cache
    /// publication. Prepared source guards remain alive until confirmed shutdown.
    pub fn unregister(&self, id: &ModelId) -> Result<()> {
        self.unregister_with_commit(id, || {})
    }
    /// Publish dependent registry observations at the same known commit point.
    pub fn unregister_with_commit(&self, id: &ModelId, committed: impl FnOnce()) -> Result<()> {
        let _gate = self.acquire()?;
        self.check_layout()?;
        let (next, _) = crate::unregister::prepare(&self.root_path, id)?;
        let mut current = self.library.lock().map_err(|_| cache_error())?;
        crate::unregister::publish_with_commit(&self.root_path, &next, || {
            // The commit callback cannot fail and runs before durability sync.
            // A later read/sync failure must never revive the old resolver view.
            *current = Some(next.clone());
            committed();
        })
    }
    /// Removes only a complete, registered managed copy. The user-selected source
    /// is never consulted or deleted. Callers must unload it before removal.
    /// A crash after the atomic rename leaves an unregistered tombstone, removed
    /// by the next exclusive open; other registered models remain untouched.
    pub fn remove(&self, id: &ModelId) -> Result<ModelManifest> {
        let _gate = self.acquire()?;
        self.check_layout()?;
        let manifest = self.read_manifest(id)?;
        let directory = model_directory(id);
        for entry in self.root.read_dir(&directory).map_err(io_error)? {
            let name = entry.map_err(io_error)?.file_name();
            if name != "model.gguf" && name != "manifest.json" {
                return Err(invalid_manifest(
                    "registered model directory contains unmanaged entries",
                ));
            }
        }
        let tombstone = PathBuf::from(format!(
            "imports/import-{}.deleted",
            Uuid::new_v4().simple()
        ));
        self.root
            .rename(&directory, &self.root, &tombstone)
            .map_err(io_error)?;
        self.verified
            .lock()
            .map_err(|_| {
                RuntimeError::new(
                    ErrorCode::RuntimeFaulted,
                    "model unregistered but verification cache update failed",
                )
            })?
            .remove(id);
        sync_committed_directory(
            &self.root,
            Path::new("models"),
            "model unregistered but directory durability sync failed",
        )?;
        self.root.remove_dir_all(&tombstone).map_err(|_| {
            RuntimeError::new(
                ErrorCode::Io,
                "model unregistered; managed-copy cleanup will resume on next open",
            )
        })?;
        sync_committed_directory(
            &self.root,
            Path::new("imports"),
            "model removed but cleanup durability sync failed",
        )?;
        Ok(manifest)
    }

    // Kept as a distinct commit operation so failure-after-rename behavior can
    // be tested without pretending a real machine power loss was exercised.
    fn publish_import(
        &self,
        staged: &Path,
        destination: &Path,
        manifest: &ModelManifest,
        sync: impl Fn(&Dir, &Path, &str) -> Result<()>,
    ) -> Result<()> {
        let fingerprint = fingerprint(&self.root, &staged.join("model.gguf"))?;
        if exists(&self.root, destination)? {
            return Err(RuntimeError::new(
                ErrorCode::AlreadyExists,
                "model ID appeared before registration",
            ));
        }
        // Both files become registered in one same-filesystem directory rename.
        // The process lock and gate exclude all cooperating destination writers.
        self.root
            .rename(staged, &self.root, destination)
            .map_err(io_error)?;
        self.verified
            .lock()
            .map_err(|_| {
                RuntimeError::new(
                    ErrorCode::RuntimeFaulted,
                    "model registered but verification cache update failed",
                )
            })?
            .insert(
                manifest.id.clone(),
                VerifiedModel {
                    manifest: manifest.clone(),
                    fingerprint,
                },
            );
        sync(
            &self.root,
            Path::new("models"),
            "model registered but directory durability sync failed",
        )?;
        sync(
            &self.root,
            Path::new("imports"),
            "model registered but import durability sync failed",
        )?;
        Ok(())
    }

    fn acquire(&self) -> Result<MutexGuard<'_, ()>> {
        self.gate.lock().map_err(|_| {
            RuntimeError::new(ErrorCode::RuntimeFaulted, "model store lock was poisoned")
        })
    }
    fn check_layout(&self) -> Result<()> {
        reject_symlink_ancestors(&self.root_path)?;
        for path in ["models", "imports", "runtime"] {
            require_directory(&self.root, Path::new(path))?;
        }
        Ok(())
    }
    fn read_manifest(&self, id: &ModelId) -> Result<ModelManifest> {
        validate_portable_id(id)?;
        let directory = model_directory(id);
        if !exists(&self.root, &directory)? {
            return Err(RuntimeError::new(
                ErrorCode::ModelNotFound,
                "model ID is not registered",
            ));
        }
        require_directory(&self.root, &directory)?;
        let path = directory.join("manifest.json");
        let size = ensure_regular(&self.root, &path)?;
        if size == 0 || size > MAX_MANIFEST_BYTES {
            return Err(invalid_manifest("manifest exceeds its size bound"));
        }
        let mut bytes = Vec::with_capacity(size as usize);
        self.root
            .open(&path)
            .map_err(io_error)?
            .take(MAX_MANIFEST_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(io_error)?;
        if bytes.len() as u64 > MAX_MANIFEST_BYTES {
            return Err(invalid_manifest("manifest exceeds its size bound"));
        }
        let manifest: ModelManifest = serde_json::from_slice(&bytes)
            .map_err(|_| invalid_manifest("invalid model manifest JSON"))?;
        manifest.validate()?;
        if manifest.storage != ModelStorage::Managed {
            return Err(invalid_manifest(
                "external registrations cannot be stored as managed copies",
            ));
        }
        if manifest.id != *id {
            return Err(invalid_manifest("manifest ID differs from directory name"));
        }
        if self
            .library
            .lock()
            .map_err(|_| cache_error())?
            .as_ref()
            .is_some_and(|l| l.is_unregistered(&manifest))
        {
            return Err(library_error(ErrorCode::ModelNotFound));
        }
        if ensure_regular(&self.root, &directory.join("model.gguf"))? != manifest.size_bytes {
            return Err(RuntimeError::new(
                ErrorCode::IntegrityFailure,
                "registered model size differs from manifest",
            ));
        }
        Ok(manifest)
    }
    fn recover_imports(&self) -> Result<()> {
        for entry in self.root.read_dir("imports").map_err(io_error)? {
            let name = entry.map_err(io_error)?.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let Some((token, suffix)) = name
                .strip_prefix("import-")
                .and_then(|rest| rest.rsplit_once('.'))
            else {
                continue;
            };
            if token.len() != 32
                || !token
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                continue;
            }
            let path = Path::new("imports").join(name);
            match suffix {
                "partial" => {
                    ensure_regular(&self.root, &path)?;
                    self.root.remove_file(&path).map_err(io_error)?;
                }
                "staged" | "deleted" => {
                    require_directory(&self.root, &path)?;
                    self.root.remove_dir_all(&path).map_err(io_error)?;
                }
                _ => {}
            }
        }
        Ok(())
    }
}

impl Drop for ModelStore {
    fn drop(&mut self) {
        let prepared = self
            .external_prepared
            .get_mut()
            .unwrap_or_else(|e| e.into_inner());
        if !prepared.is_empty() {
            EXTERNAL_CLEANUP_UNCONFIRMED.store(true, Ordering::Release);
            // Keep bounded file/directory guards alive until OS process teardown.
            // Never turn an unknown worker cleanup into a source-unlock claim.
            std::mem::forget(std::mem::take(prepared));
        }
        // Explicit unlock also releases the open-file-description lock if an
        // unrelated concurrent process spawn briefly inherited this descriptor.
        let _ = FileExt::unlock(&self._process_lock);
    }
}

struct ImportCleanup<'a> {
    root: &'a Dir,
    partial: PathBuf,
    staged: PathBuf,
}
impl Drop for ImportCleanup<'_> {
    fn drop(&mut self) {
        // Never touch the committed models/<id> directory. Failed cleanup leaves
        // an owned temporary name for the next exclusive open to recover.
        let _ = self.root.remove_file(&self.partial);
        let _ = self.root.remove_dir_all(&self.staged);
    }
}
fn model_directory(id: &ModelId) -> PathBuf {
    Path::new("models").join(id.as_str())
}
fn encode_manifest(manifest: &ModelManifest) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(manifest)
        .map_err(|_| invalid_manifest("manifest serialization failed"))?;
    bytes.push(b'\n');
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(invalid_manifest("manifest exceeds its size bound"));
    }
    Ok(bytes)
}
fn exists(root: &Dir, path: &Path) -> Result<bool> {
    match root.symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_error(error)),
    }
}
fn ensure_regular(root: &Dir, path: &Path) -> Result<u64> {
    let metadata = root.symlink_metadata(path).map_err(io_error)?;
    if !metadata.file_type().is_file() {
        return Err(invalid_manifest(
            "managed file is not a regular non-symlink file",
        ));
    }
    Ok(metadata.len())
}
fn require_directory(root: &Dir, path: &Path) -> Result<()> {
    if !root
        .symlink_metadata(path)
        .map_err(io_error)?
        .file_type()
        .is_dir()
    {
        return Err(invalid_manifest(
            "managed directory is not a non-symlink directory",
        ));
    }
    Ok(())
}
fn ensure_directory(root: &Dir, path: &Path) -> Result<()> {
    match root.create_dir(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            require_directory(root, path)
        }
        Err(error) => Err(io_error(error)),
    }
}
fn prepare_root(path: &Path) -> Result<PathBuf> {
    if path.as_os_str().is_empty() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(RuntimeError::invalid(
            "data directory must not contain parent traversal",
        ));
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_err(io_error)?.join(path)
    };
    reject_symlink_ancestors(&absolute)?;
    fs::create_dir_all(&absolute).map_err(io_error)?;
    reject_symlink_ancestors(&absolute)?;
    fs::canonicalize(absolute).map_err(io_error)
}
fn reject_symlink_ancestors(path: &Path) -> Result<()> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component);
        // A Windows drive/UNC prefix alone is not yet an absolute directory.
        if matches!(component, Component::Prefix(_)) {
            continue;
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(invalid_manifest("data directory path contains a symlink"));
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(invalid_manifest(
                    "data directory component is not a directory",
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error(error)),
        }
    }
    Ok(())
}
fn sync_committed_directory(root: &Dir, path: &Path, message: &str) -> Result<()> {
    sync_directory(root, path).map_err(|_| RuntimeError::new(ErrorCode::Io, message))
}
#[cfg(unix)]
fn sync_directory(root: &Dir, path: &Path) -> Result<()> {
    root.open(path)
        .map_err(io_error)?
        .sync_all()
        .map_err(io_error)
}
#[cfg(not(unix))]
fn sync_directory(_root: &Dir, _path: &Path) -> Result<()> {
    // Windows supports atomic directory rename but std has no portable directory
    // flush. Files are flushed before rename; power-loss durability is not claimed.
    Ok(())
}

#[derive(Eq, PartialEq)]
struct Fingerprint {
    length: u64,
    modified: std::time::SystemTime,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}
struct VerifiedModel {
    manifest: ModelManifest,
    fingerprint: Fingerprint,
}
fn fingerprint(root: &Dir, path: &Path) -> Result<Fingerprint> {
    ensure_regular(root, path)?;
    let metadata = root
        .open(path)
        .map_err(io_error)?
        .into_std()
        .metadata()
        .map_err(io_error)?;
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;
    Ok(Fingerprint {
        length: metadata.len(),
        modified: metadata.modified().map_err(io_error)?,
        #[cfg(unix)]
        device: metadata.dev(),
        #[cfg(unix)]
        inode: metadata.ino(),
    })
}
fn cache_error() -> RuntimeError {
    RuntimeError::new(
        ErrorCode::RuntimeFaulted,
        "model verification cache was poisoned",
    )
}

#[cfg(windows)]
fn validate_local_source_path(path: &Path) -> Result<()> {
    use std::path::Prefix;
    for component in path.components() {
        match component {
            Component::Prefix(prefix)
                if !matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)) =>
            {
                return Err(RuntimeError::invalid(
                    "model source must be a local disk path",
                ));
            }
            Component::Normal(name) => {
                let name = name.to_string_lossy();
                let stem = name
                    .split(['.', ':'])
                    .next()
                    .unwrap_or("")
                    .trim_end_matches(' ')
                    .to_ascii_uppercase();
                let device = matches!(
                    stem.as_str(),
                    "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
                ) || ["COM", "LPT"].iter().any(|prefix| {
                    stem.strip_prefix(prefix).is_some_and(|suffix| {
                        matches!(
                            suffix,
                            "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                        )
                    })
                });
                if device {
                    return Err(RuntimeError::invalid(
                        "model source must not be a DOS device path",
                    ));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ModelSource;

    #[test]
    fn post_commit_sync_failure_reports_registration_and_cleanup_keeps_it() {
        let temp = tempfile::tempdir().unwrap();
        let store = ModelStore::open(temp.path()).unwrap();
        let staged = PathBuf::from("imports/import-0123456789abcdef0123456789abcdef.staged");
        let partial = PathBuf::from("imports/import-0123456789abcdef0123456789abcdef.partial");
        // This exercises the storage commit unit after parsing, not GGUF inference.
        let payload = b"already verified staging payload";
        let manifest = ModelManifest::build(
            ImportRequest::new(
                ModelId::new("commit-test").unwrap(),
                "Commit fixture",
                ModelSource::local("synthetic"),
            ),
            payload.len() as u64,
            format!("{:x}", Sha256::digest(payload)),
            gguf::Metadata {
                architecture: "qwen3".into(),
                file_type: 7,
                template: "synthetic".into(),
                context_length: 40960,
            },
        )
        .unwrap();
        store.root.create_dir(&staged).unwrap();
        store
            .root
            .write(staged.join("model.gguf"), payload)
            .unwrap();
        store
            .root
            .write(
                staged.join("manifest.json"),
                encode_manifest(&manifest).unwrap(),
            )
            .unwrap();
        let cleanup = ImportCleanup {
            root: &store.root,
            partial,
            staged: staged.clone(),
        };
        let error = store
            .publish_import(
                &staged,
                &model_directory(&manifest.id),
                &manifest,
                |_, _, message| Err(RuntimeError::new(ErrorCode::Io, message)),
            )
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Io);
        assert!(error.message.contains("model registered"));
        drop(cleanup);
        assert_eq!(store.get(&manifest.id).unwrap(), manifest);
        assert_eq!(
            store
                .root
                .read(model_directory(&manifest.id).join("model.gguf"))
                .unwrap(),
            payload
        );
        assert!(store.root.read_dir("imports").unwrap().next().is_none());
    }
}

#[cfg(all(test, windows))]
mod external_guard_lifetime_tests {
    use super::*;
    #[test]
    fn probe_child() {
        let Some(root) = std::env::var_os("NEXA_EXTERNAL_GUARD_PROBE_ROOT").map(PathBuf::from)
        else {
            return;
        };
        let released =
            std::env::var_os("NEXA_EXTERNAL_GUARD_PROBE_MODE").is_some_and(|v| v == "released");
        let store = ModelStore::open(root.join("data")).unwrap();
        let file = root.join("source.gguf");
        let guard = crate::library::prepared_guard_fixture(&file).unwrap();
        store
            .external_prepared
            .lock()
            .unwrap()
            .insert(ModelId::new("fixture").unwrap(), guard);
        assert!(fs::OpenOptions::new().write(true).open(&file).is_err());
        if released {
            store.release_external_after_shutdown();
        }
        drop(store);
        if released {
            assert!(fs::OpenOptions::new().write(true).open(&file).is_ok());
            assert!(ModelStore::open(root.join("other")).is_ok());
        } else {
            assert!(fs::OpenOptions::new().write(true).open(&file).is_err());
            assert_eq!(
                ModelStore::open(root.join("other")).err().unwrap().code,
                ErrorCode::ExecutorCleanupUnconfirmed
            );
        }
    }
    #[test]
    fn windows_external_guard_release_and_unknown_cleanup_are_process_isolated() {
        for mode in ["released", "retained"] {
            let temp = tempfile::tempdir().unwrap();
            let source = temp.path().join("source.gguf");
            fs::write(&source, b"ownership fixture, not a model").unwrap();
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "store::external_guard_lifetime_tests::probe_child",
                    "--nocapture",
                ])
                .env("NEXA_EXTERNAL_GUARD_PROBE_ROOT", temp.path())
                .env("NEXA_EXTERNAL_GUARD_PROBE_MODE", mode)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let status = loop {
                if let Some(status) = child.try_wait().unwrap() {
                    break status;
                }
                if std::time::Instant::now() >= deadline {
                    child.kill().expect("owned fixture kill request failed");
                    let reap_deadline =
                        std::time::Instant::now() + std::time::Duration::from_secs(1);
                    while child.try_wait().unwrap().is_none()
                        && std::time::Instant::now() < reap_deadline
                    {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    panic!("owned guard fixture exceeded its deadline");
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            };
            assert!(status.success(), "owned guard fixture failed");
            assert!(fs::OpenOptions::new().write(true).open(&source).is_ok());
        }
    }
}
