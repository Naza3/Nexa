//! Read-only external GGUF catalog. Only its private metadata is persisted by
//! the owning application; this module never writes into a selected directory.
use crate::{ImportRequest, ModelManifest, ModelSource, ModelStorage, Result, gguf};
use runtime_types::{ErrorCode, ModelId, ResolvedModel, RuntimeError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Component, Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
    time::{Duration, Instant, UNIX_EPOCH},
};
use uuid::Uuid;

pub const LIBRARY_FILE: &str = "model-library.json";
pub const MAX_LIBRARY_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_DIRECTORY_ENTRIES: usize = 1024;
pub const MAX_EXTERNAL_MODELS: usize = 64;
pub const MAX_DIRECTORY_COMPONENTS: usize = 64;
pub const MAX_MODEL_BYTES: u64 = 16 * 1024 * 1024 * 1024;
pub const MAX_SCAN_BYTES: u64 = 32 * 1024 * 1024 * 1024;
pub const SCAN_TIMEOUT: Duration = Duration::from_secs(300);
const BLOCK: usize = 64 * 1024;

pub fn library_error(code: ErrorCode) -> RuntimeError {
    RuntimeError::new(code, "external model library operation failed safely")
}
fn file_error(error: std::io::Error) -> RuntimeError {
    let code = match error.kind() {
        std::io::ErrorKind::WouldBlock => ErrorCode::ModelFileInUse,
        _ if matches!(error.raw_os_error(), Some(32 | 33)) && cfg!(windows) => {
            ErrorCode::ModelFileInUse
        }
        _ => ErrorCode::ModelFileUnavailable,
    };
    library_error(code)
}

#[derive(Clone, Debug, Default)]
pub struct ScanProgress {
    pub phase: &'static str,
    pub examined_entries: usize,
    pub candidate_files: usize,
    pub verified_files: usize,
    pub current_file_name: Option<String>,
}
/// Cancellation wins until the explicit commit decision. Once committing,
/// publication finishes and a late cancellation cannot claim a rollback.
pub struct ScanControl {
    state: AtomicU8,
    deadline: Instant,
    progress: Mutex<ScanProgress>,
}
impl Default for ScanControl {
    fn default() -> Self {
        Self::with_timeout(SCAN_TIMEOUT)
    }
}
impl ScanControl {
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            state: AtomicU8::new(0),
            deadline: Instant::now() + timeout,
            progress: Mutex::new(ScanProgress {
                phase: "checking",
                ..Default::default()
            }),
        }
    }
    pub fn cancel(&self) {
        let _ = self
            .state
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire);
    }
    pub fn check(&self) -> Result<()> {
        if self.state.load(Ordering::Acquire) == 1 {
            return Err(library_error(ErrorCode::ModelScanCancelled));
        }
        if Instant::now() >= self.deadline {
            return Err(library_error(ErrorCode::ModelScanTimeout));
        }
        Ok(())
    }
    pub fn begin_commit(&self) -> Result<()> {
        self.check()?;
        self.state
            .compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| library_error(ErrorCode::ModelScanCancelled))?;
        let mut progress = self.progress.lock().unwrap();
        progress.phase = "committing";
        progress.current_file_name = None;
        Ok(())
    }
    pub fn progress(&self) -> ScanProgress {
        self.progress.lock().unwrap().clone()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FileIdentity {
    pub volume: u64,
    pub file: u64,
    pub size: u64,
    pub modified_seconds: u64,
    pub modified_nanos: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalRegistration {
    pub manifest: ModelManifest,
    pub identity: FileIdentity,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelLibrary {
    pub schema_version: u32,
    pub directory_id: Uuid,
    pub library_generation: Uuid,
    pub directory: PathBuf,
    pub directory_identity: FileIdentity,
    pub models: Vec<ExternalRegistration>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LibraryDirectoryInfo {
    pub directory_id: Uuid,
    pub display_path: String,
    pub library_generation: Uuid,
}
impl ModelLibrary {
    pub fn info(&self) -> LibraryDirectoryInfo {
        LibraryDirectoryInfo {
            directory_id: self.directory_id,
            display_path: self.directory.to_string_lossy().into_owned(),
            library_generation: self.library_generation,
        }
    }
    pub fn read(root: &Path) -> Result<Option<Self>> {
        let path = root.join(LIBRARY_FILE);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(library_error(ErrorCode::ModelLibraryChanged)),
        };
        if !metadata.is_file() || indirect(&metadata) || metadata.len() > MAX_LIBRARY_BYTES as u64 {
            return Err(library_error(ErrorCode::ModelLibraryChanged));
        }
        let mut bytes = Vec::new();
        open_read_file(&path, false)?
            .take(MAX_LIBRARY_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(file_error)?;
        if bytes.len() > MAX_LIBRARY_BYTES {
            return Err(library_error(ErrorCode::ModelLibraryLimit));
        }
        let library: Self = serde_json::from_slice(&bytes)
            .map_err(|_| library_error(ErrorCode::ModelLibraryChanged))?;
        library.validate()?;
        Ok(Some(library))
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let bytes = serde_json::to_vec(self)
            .map_err(|_| library_error(ErrorCode::ModelLibraryWriteFailed))?;
        if bytes.len() > MAX_LIBRARY_BYTES {
            return Err(library_error(ErrorCode::ModelLibraryLimit));
        }
        Ok(bytes)
    }
    pub fn validate(&self) -> Result<()> {
        validate_directory_syntax(&self.directory)?;
        if self.schema_version != 1
            || self.directory_id.is_nil()
            || self.library_generation.is_nil()
            || self.models.len() > MAX_EXTERNAL_MODELS
        {
            return Err(library_error(ErrorCode::ModelLibraryChanged));
        }
        let mut ids = BTreeSet::new();
        let mut names = BTreeSet::new();
        let mut total = 0_u64;
        for entry in &self.models {
            entry.manifest.validate()?;
            let m = &entry.manifest;
            total = total
                .checked_add(m.size_bytes)
                .ok_or_else(|| library_error(ErrorCode::ModelLibraryLimit))?;
            if m.storage != ModelStorage::External
                || m.size_bytes > MAX_MODEL_BYTES
                || total > MAX_SCAN_BYTES
                || m.size_bytes != entry.identity.size
                || !ids.insert(m.id.clone())
                || !names.insert(file_key(&m.relative_file))
            {
                return Err(library_error(ErrorCode::ModelLibraryChanged));
            }
        }
        Ok(())
    }
    pub fn directory_presence(&self) -> Result<bool> {
        if !directory_drive_present(&self.directory)? {
            return Ok(false);
        }
        match fs::symlink_metadata(&self.directory) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(_) => return Err(library_error(ErrorCode::ModelDirectoryUnavailable)),
            Ok(_) => (),
        }
        self.check_directory_identity()?;
        Ok(true)
    }
    pub fn check_directory_identity(&self) -> Result<()> {
        let directory = DirectoryGuard::open(&self.directory)?;
        if !same_object(&directory.identity, &self.directory_identity) {
            return Err(library_error(ErrorCode::ModelFileChanged));
        }
        Ok(())
    }
    pub fn entry(&self, id: &ModelId) -> Option<&ExternalRegistration> {
        self.models.iter().find(|m| &m.manifest.id == id)
    }
    /// Lightweight eligibility observation only. A real load still requires
    /// full hashing from an acquired source guard, regardless of this result.
    pub fn availability(&self, entry: &ExternalRegistration) -> Option<ErrorCode> {
        let check = || -> Result<()> {
            let directory = DirectoryGuard::open(&self.directory)?;
            if !same_object(&directory.identity, &self.directory_identity) {
                return Err(library_error(ErrorCode::ModelFileChanged));
            }
            let source =
                open_read_file(&self.directory.join(&entry.manifest.relative_file), false)?;
            if identity(&source)? != entry.identity {
                return Err(library_error(ErrorCode::ModelFileChanged));
            }
            Ok(())
        };
        check().err().map(|e| e.code)
    }
}

pub struct ScannedLibrary {
    library: ModelLibrary,
    _directory: Arc<DirectoryGuard>,
    _sources: Vec<File>,
}
impl ScannedLibrary {
    pub fn library(&self) -> &ModelLibrary {
        &self.library
    }
}

pub fn scan_directory(
    root: &Path,
    directory: &Path,
    previous: Option<&ModelLibrary>,
    control: &ScanControl,
) -> Result<ScannedLibrary> {
    control.check()?;
    let directory = Arc::new(DirectoryGuard::open(directory)?);
    let mut candidates = Vec::new();
    let mut total = 0_u64;
    control.progress.lock().unwrap().phase = "enumerating";
    for item in fs::read_dir(&directory.path)
        .map_err(|_| library_error(ErrorCode::ModelDirectoryUnavailable))?
    {
        control.check()?;
        control.progress.lock().unwrap().current_file_name = None;
        {
            let mut p = control.progress.lock().unwrap();
            p.examined_entries += 1;
            if p.examined_entries > MAX_DIRECTORY_ENTRIES {
                return Err(library_error(ErrorCode::ModelLibraryLimit));
            }
        }
        let item = item.map_err(|_| library_error(ErrorCode::ModelDirectoryUnavailable))?;
        let name = item
            .file_name()
            .into_string()
            .map_err(|_| library_error(ErrorCode::ModelDirectoryUnsupported))?;
        control.progress.lock().unwrap().current_file_name =
            (name.len() <= 1024).then(|| name.clone());
        let metadata = fs::symlink_metadata(item.path()).map_err(file_error)?;
        if indirect(&metadata) {
            return Err(library_error(ErrorCode::ModelDirectoryUnsupported));
        }
        if metadata.is_dir() {
            continue;
        }
        if !metadata.is_file() {
            return Err(library_error(ErrorCode::ModelDirectoryUnsupported));
        }
        if !name.to_ascii_lowercase().ends_with(".gguf") {
            continue;
        }
        if !valid_file_name(&name) {
            return Err(library_error(ErrorCode::ModelDirectoryUnsupported));
        }
        if metadata.len() == 0 || metadata.len() > MAX_MODEL_BYTES {
            return Err(library_error(ErrorCode::ModelLibraryLimit));
        }
        total = total
            .checked_add(metadata.len())
            .ok_or_else(|| library_error(ErrorCode::ModelLibraryLimit))?;
        candidates.push(name);
        if candidates.len() > MAX_EXTERNAL_MODELS || total > MAX_SCAN_BYTES {
            return Err(library_error(ErrorCode::ModelLibraryLimit));
        }
        control.progress.lock().unwrap().candidate_files = candidates.len();
    }
    candidates.sort_by_key(|name| file_key(name));
    let prior = previous.filter(|old| {
        same_object(&old.directory_identity, &directory.identity)
            && path_key(&old.directory) == path_key(&directory.path)
    });
    let directory_id = prior.map_or_else(Uuid::new_v4, |old| old.directory_id);
    let mut models = Vec::new();
    let mut sources = Vec::new();
    let mut verified_total = 0_u64;
    control.progress.lock().unwrap().phase = "verifying";
    for name in candidates {
        control.progress.lock().unwrap().current_file_name = Some(name.clone());
        control.check()?;
        let mut source = open_read_file(&directory.path.join(&name), true)?;
        let before = identity(&source)?;
        if before.size == 0 || before.size > MAX_MODEL_BYTES {
            return Err(library_error(ErrorCode::ModelLibraryLimit));
        }
        verified_total = verified_total
            .checked_add(before.size)
            .filter(|n| *n <= MAX_SCAN_BYTES)
            .ok_or_else(|| library_error(ErrorCode::ModelLibraryLimit))?;
        let (hash, metadata) = inspect(&mut source, before.size, control)?;
        if identity(&source)? != before {
            return Err(library_error(ErrorCode::ModelFileChanged));
        }
        let existing = prior.and_then(|old| {
            old.models.iter().find(|m| {
                file_key(&m.manifest.relative_file) == file_key(&name) && m.manifest.sha256 == hash
            })
        });
        let id = match existing {
            Some(entry)
                if !root
                    .join("models")
                    .join(entry.manifest.id.as_str())
                    .exists() =>
            {
                entry.manifest.id.clone()
            }
            _ => fresh_id(root)?,
        };
        let stem = &name[..name.len() - 5];
        let display_name = if stem.trim().is_empty() {
            name.clone()
        } else {
            stem.to_owned()
        };
        let mut request = ImportRequest::new(
            id,
            display_name,
            ModelSource::local("user-selected read-only model directory"),
        );
        request.source.file_name = Some(name.clone());
        let mut manifest = ModelManifest::build(request, before.size, hash, metadata)?;
        manifest.storage = ModelStorage::External;
        manifest.relative_file = name;
        manifest.validate()?;
        models.push(ExternalRegistration {
            manifest,
            identity: before,
        });
        sources.push(source);
        control.progress.lock().unwrap().verified_files += 1;
    }
    let library = ModelLibrary {
        schema_version: 1,
        directory_id,
        library_generation: Uuid::new_v4(),
        directory: directory.path.clone(),
        directory_identity: directory.identity.clone(),
        models,
    };
    library.encode()?;
    control.check()?;
    Ok(ScannedLibrary {
        library,
        _directory: directory,
        _sources: sources,
    })
}
fn fresh_id(root: &Path) -> Result<ModelId> {
    for _ in 0..4 {
        let id = ModelId::new(format!("ext-{}", Uuid::new_v4().simple()))?;
        match fs::symlink_metadata(root.join("models").join(id.as_str())) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(id),
            Ok(_) => (),
            Err(_) => return Err(library_error(ErrorCode::ModelLibraryChanged)),
        }
    }
    Err(library_error(ErrorCode::ModelLibraryChanged))
}
fn inspect(
    source: &mut File,
    size: u64,
    control: &ScanControl,
) -> Result<(String, gguf::Metadata)> {
    let mut buffer = [0_u8; BLOCK];
    let mut hasher = Sha256::new();
    let mut read = 0_u64;
    loop {
        control.check()?;
        let count = source.read(&mut buffer).map_err(file_error)?;
        if count == 0 {
            break;
        }
        read = read
            .checked_add(count as u64)
            .filter(|n| *n <= size)
            .ok_or_else(|| library_error(ErrorCode::ModelFileChanged))?;
        hasher.update(&buffer[..count]);
    }
    if read != size {
        return Err(library_error(ErrorCode::ModelFileChanged));
    }
    control.check()?;
    source.seek(SeekFrom::Start(0)).map_err(file_error)?;
    let metadata = gguf::read(ControlledRead { source, control });
    control.check()?;
    let metadata = metadata?;
    Ok((format!("{:x}", hasher.finalize()), metadata))
}

struct ControlledRead<'a> {
    source: &'a mut File,
    control: &'a ScanControl,
}
impl Read for ControlledRead<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.control
            .check()
            .map_err(|_| std::io::Error::other("model verification interrupted"))?;
        let limit = buffer.len().min(BLOCK);
        self.source.read(&mut buffer[..limit])
    }
}
impl Seek for ControlledRead<'_> {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        self.control
            .check()
            .map_err(|_| std::io::Error::other("model verification interrupted"))?;
        self.source.seek(position)
    }
}

/// Guards are intentionally native-only and never serialized into IPC. They
/// remain owned by the service's catalog through worker shutdown and reaping.
pub struct PreparedExternal {
    pub(crate) model: ResolvedModel,
    source: File,
    identity: FileIdentity,
    _directory: Arc<DirectoryGuard>,
}
impl PreparedExternal {
    pub fn prepare(
        library: &ModelLibrary,
        entry: &ExternalRegistration,
        control: &ScanControl,
    ) -> Result<Self> {
        #[cfg(not(windows))]
        {
            let _ = (library, entry, control);
            Err(library_error(ErrorCode::ModelDirectoryUnsupported))
        }
        #[cfg(windows)]
        {
            control.check()?;
            let directory = Arc::new(DirectoryGuard::open(&library.directory)?);
            if !same_object(&directory.identity, &library.directory_identity) {
                return Err(library_error(ErrorCode::ModelFileChanged));
            }
            let path = directory.path.join(&entry.manifest.relative_file);
            let mut source = open_read_file(&path, true)?;
            let observed = identity(&source)?;
            if observed != entry.identity {
                return Err(library_error(ErrorCode::ModelFileChanged));
            }
            let (hash, metadata) = inspect(&mut source, observed.size, control)?;
            if hash != entry.manifest.sha256
                || identity(&source)? != observed
                || !entry.manifest.matches_metadata(&metadata)
            {
                return Err(library_error(ErrorCode::ModelFileChanged));
            }
            let mut request = ImportRequest::new(
                entry.manifest.id.clone(),
                entry.manifest.display_name.clone(),
                entry.manifest.source.clone(),
            );
            request.default_context = entry.manifest.default_context;
            let inspected = ModelManifest::build(request, observed.size, hash, metadata)?;
            if !inspected.load_candidate()
                || inspected.template_sha256 != entry.manifest.template_sha256
            {
                return Err(library_error(ErrorCode::UnsupportedModel));
            }
            control.check()?;
            Ok(Self {
                model: ResolvedModel {
                    id: entry.manifest.id.clone(),
                    path,
                    context_limit: entry.manifest.executable_context_limit(),
                    default_context: entry.manifest.default_context,
                    loadable: true,
                },
                source,
                identity: observed,
                _directory: directory,
            })
        }
    }
    pub fn resolve(&self) -> Result<ResolvedModel> {
        if identity(&self.source)? != self.identity {
            return Err(library_error(ErrorCode::ModelFileChanged));
        }
        Ok(self.model.clone())
    }
}

pub(crate) fn valid_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 1024
        && name.to_ascii_lowercase().ends_with(".gguf")
        && !name.contains(['/', '\\', ':', '\0'])
        && name != "."
        && name != ".."
        && !name.ends_with([' ', '.'])
        && !reserved_name(name)
}
fn reserved_name(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|prefix| {
        stem.strip_prefix(prefix).is_some_and(|n| {
            matches!(
                n,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    })
}
fn file_key(value: &str) -> String {
    if cfg!(windows) {
        value.to_lowercase()
    } else {
        value.to_owned()
    }
}
fn path_key(value: &Path) -> String {
    file_key(&value.to_string_lossy())
}
#[cfg(any(test, windows))]
fn local_drive_type(kind: u32) -> bool {
    // GetDriveTypeW: removable, fixed, CD-ROM, RAM disk. Remote/unknown/root
    // missing cannot establish the local-disk sharing contract.
    matches!(kind, 2 | 3 | 5 | 6)
}
fn validate_directory_syntax(path: &Path) -> Result<()> {
    let text = path
        .to_str()
        .ok_or_else(|| library_error(ErrorCode::ModelDirectoryUnsupported))?;
    if !path.is_absolute()
        || text.len() > 32768
        || path.components().count() > MAX_DIRECTORY_COMPONENTS
        || text.contains("://")
        || text.starts_with("\\\\")
        || text.starts_with("//")
        || text.contains('\0')
        || path.components().any(|c| matches!(c, Component::ParentDir))
    {
        return Err(library_error(ErrorCode::ModelDirectoryUnsupported));
    }
    #[cfg(windows)]
    {
        use std::path::Prefix;
        let Some(Component::Prefix(prefix)) = path.components().next() else {
            return Err(library_error(ErrorCode::ModelDirectoryUnsupported));
        };
        let Prefix::Disk(_) = prefix.kind() else {
            return Err(library_error(ErrorCode::ModelDirectoryUnsupported));
        };
    }
    Ok(())
}
fn directory_drive_present(path: &Path) -> Result<bool> {
    validate_directory_syntax(path)?;
    #[cfg(windows)]
    {
        use std::path::Prefix;
        let Some(Component::Prefix(prefix)) = path.components().next() else {
            unreachable!("validated disk prefix")
        };
        let Prefix::Disk(letter) = prefix.kind() else {
            unreachable!("validated disk prefix")
        };
        let root: Vec<u16> = format!("{}:\\\0", char::from(letter))
            .encode_utf16()
            .collect();
        // SAFETY: this terminated root string is live for a read-only OS query.
        let kind = unsafe { windows_sys::Win32::Storage::FileSystem::GetDriveTypeW(root.as_ptr()) };
        if kind == 1 {
            return Ok(false);
        }
        if !local_drive_type(kind) {
            return Err(library_error(ErrorCode::ModelDirectoryUnsupported));
        }
        Ok(true)
    }
    #[cfg(not(windows))]
    {
        Ok(true)
    }
}
fn indirect(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}
/// Native picker/host preflight; no scan, token initialization or source writes.
pub fn validate_directory_candidate(path: &Path) -> Result<()> {
    DirectoryGuard::open(path).map(|_| ())
}
struct DirectoryGuard {
    path: PathBuf,
    identity: FileIdentity,
    _ancestors: Vec<File>,
}
impl DirectoryGuard {
    fn open(path: &Path) -> Result<Self> {
        if !directory_drive_present(path)? {
            return Err(library_error(ErrorCode::ModelDirectoryUnavailable));
        }
        let mut current = PathBuf::new();
        let mut ancestors = Vec::new();
        for component in path.components() {
            current.push(component);
            if matches!(component, Component::Prefix(_)) {
                continue;
            }
            let metadata = fs::symlink_metadata(&current)
                .map_err(|_| library_error(ErrorCode::ModelDirectoryUnavailable))?;
            if !metadata.is_dir() || indirect(&metadata) {
                return Err(library_error(ErrorCode::ModelDirectoryUnsupported));
            }
            let mut options = fs::OpenOptions::new();
            options.read(true);
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                use windows_sys::Win32::Storage::FileSystem::*;
                options
                    .access_mode(FILE_READ_ATTRIBUTES)
                    .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                    .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_NONBLOCK);
            }
            let file = options
                .open(&current)
                .map_err(|_| library_error(ErrorCode::ModelDirectoryUnavailable))?;
            let opened = file.metadata().map_err(file_error)?;
            if !opened.is_dir() || indirect(&opened) {
                return Err(library_error(ErrorCode::ModelDirectoryUnsupported));
            }
            ancestors.push(file);
        }
        let last = ancestors
            .last()
            .ok_or_else(|| library_error(ErrorCode::ModelDirectoryUnsupported))?;
        Ok(Self {
            path: path.to_owned(),
            identity: identity(last)?,
            _ancestors: ancestors,
        })
    }
}
fn same_object(a: &FileIdentity, b: &FileIdentity) -> bool {
    a.volume == b.volume && a.file == b.file
}
fn identity(file: &File) -> Result<FileIdentity> {
    let metadata = file.metadata().map_err(file_error)?;
    let modified = metadata
        .modified()
        .map_err(file_error)?
        .duration_since(UNIX_EPOCH)
        .map_err(|_| library_error(ErrorCode::ModelFileUnavailable))?;
    #[cfg(unix)]
    let (volume, number) = {
        use std::os::unix::fs::MetadataExt;
        (metadata.dev(), metadata.ino())
    };
    #[cfg(windows)]
    let (volume, number) = {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
        };
        let mut information: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: File owns the handle and information is a live output buffer.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), &mut information) } == 0
        {
            return Err(file_error(std::io::Error::last_os_error()));
        }
        (
            information.dwVolumeSerialNumber as u64,
            ((information.nFileIndexHigh as u64) << 32) | information.nFileIndexLow as u64,
        )
    };
    Ok(FileIdentity {
        volume,
        file: number,
        size: metadata.len(),
        modified_seconds: modified.as_secs(),
        modified_nanos: modified.subsec_nanos(),
    })
}
fn open_read_file(path: &Path, protect: bool) -> Result<File> {
    let metadata = fs::symlink_metadata(path).map_err(file_error)?;
    if !metadata.is_file() || indirect(&metadata) {
        return Err(library_error(ErrorCode::ModelFileUnavailable));
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        let _ = protect;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::*;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
        if protect {
            options.share_mode(FILE_SHARE_READ);
        }
    }
    let file = options.open(path).map_err(file_error)?;
    let opened = file.metadata().map_err(file_error)?;
    if !opened.is_file() || indirect(&opened) {
        return Err(library_error(ErrorCode::ModelFileUnavailable));
    }
    Ok(file)
}

#[cfg(all(test, windows))]
pub(crate) fn prepared_guard_fixture(path: &Path) -> Result<PreparedExternal> {
    // Ownership-only fixture: never presented as validated model/inference.
    let directory = Arc::new(DirectoryGuard::open(path.parent().unwrap())?);
    let source = open_read_file(path, true)?;
    let observed = identity(&source)?;
    Ok(PreparedExternal {
        model: ResolvedModel {
            id: ModelId::new("fixture")?,
            path: path.to_owned(),
            context_limit: 32,
            default_context: 32,
            loadable: false,
        },
        source,
        identity: observed,
        _directory: directory,
    })
}
#[cfg(test)]
mod control_tests {
    use super::*;
    #[test]
    fn directory_syntax_limits_do_not_probe_or_truncate_paths() {
        let mut path = PathBuf::from(if cfg!(windows) { r"C:\" } else { "/" });
        while path.components().count() < MAX_DIRECTORY_COMPONENTS {
            path.push("x");
        }
        assert!(validate_directory_syntax(&path).is_ok());
        path.push("x");
        assert_eq!(
            validate_directory_syntax(&path).unwrap_err().code,
            ErrorCode::ModelDirectoryUnsupported
        );
        let too_long =
            PathBuf::from(if cfg!(windows) { r"C:\" } else { "/" }).join("x".repeat(32769));
        assert_eq!(
            validate_directory_syntax(&too_long).unwrap_err().code,
            ErrorCode::ModelDirectoryUnsupported
        );
    }
    #[test]
    fn mapped_network_drive_class_and_unknown_roots_are_rejected() {
        for kind in [0, 1, 4, u32::MAX] {
            assert!(!local_drive_type(kind));
        }
        for kind in [2, 3, 5, 6] {
            assert!(local_drive_type(kind));
        }
    }
    #[test]
    fn commit_clears_file_attribution_and_late_cancel_does_not_claim_rollback() {
        let control = ScanControl::default();
        control.progress.lock().unwrap().current_file_name = Some("valid.gguf".into());
        control.begin_commit().unwrap();
        control.cancel();
        assert!(control.check().is_ok());
        assert_eq!(control.progress().current_file_name, None);
        let cancel = ScanControl::default();
        cancel.cancel();
        assert_eq!(
            cancel.begin_commit().unwrap_err().code,
            ErrorCode::ModelScanCancelled
        );
    }
}

#[cfg(all(test, windows))]
mod saved_directory_tests {
    use super::*;
    #[test]
    fn persisted_disk_path_does_not_require_that_drive_to_be_present() {
        let root = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        let scan =
            scan_directory(root.path(), source.path(), None, &ScanControl::default()).unwrap();
        let mut library = scan.library().clone();
        drop(scan);
        // No probe or filesystem access to this old letter is authorized by
        // deserialization. Current access is checked separately on use.
        library.directory = PathBuf::from(r"Z:\external-model-fixture");
        fs::write(root.path().join(LIBRARY_FILE), library.encode().unwrap()).unwrap();
        let previous = ModelLibrary::read(root.path()).unwrap().unwrap();
        let replacement = scan_directory(
            root.path(),
            source.path(),
            Some(&previous),
            &ScanControl::default(),
        )
        .unwrap();
        assert_eq!(replacement.library().directory, source.path());
        assert_ne!(replacement.library().directory_id, previous.directory_id);
    }
}
