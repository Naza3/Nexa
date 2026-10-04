//! Read-only external GGUF catalog. Only its private metadata is persisted by
//! the owning application; this module never writes into a selected directory.
#[path = "library_download.rs"]
pub mod download;
#[path = "library_selected.rs"]
pub mod selected;

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
pub const MAX_SCAN_DIAGNOSTIC_BYTES: usize = 512 * 1024;
const BLOCK: usize = 64 * 1024;

fn add_scan_bytes(total: u64, size: u64) -> Result<u64> {
    if size > MAX_MODEL_BYTES {
        return Err(library_error(ErrorCode::ModelLibraryLimit));
    }
    total
        .checked_add(size)
        .filter(|total| *total <= MAX_SCAN_BYTES)
        .ok_or_else(|| library_error(ErrorCode::ModelLibraryLimit))
}

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
    pub file_errors: Vec<ScanFileFailure>,
}
/// Content rejection only; filesystem, resource and cancellation failures never
/// become one of these per-file diagnostics.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScanRejectionReason {
    InvalidManifest,
    UnsupportedModel,
    UnsupportedChatTemplate,
}
impl ScanRejectionReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidManifest => "invalid_manifest",
            Self::UnsupportedModel => "unsupported_model",
            Self::UnsupportedChatTemplate => "unsupported_chat_template",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScanFileFailure {
    pub file_name: String,
    pub reason: ScanRejectionReason,
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
    pub fn selected_count(&self, count: usize) {
        self.progress.lock().unwrap().candidate_files = count;
    }
    pub fn phase(&self, phase: &'static str) {
        self.progress.lock().unwrap().phase = phase;
    }
    pub fn progress(&self) -> ScanProgress {
        self.progress.lock().unwrap().clone()
    }
    fn reject(&self, name: String, reason: ScanRejectionReason) -> Result<()> {
        let mut progress = self.progress.lock().unwrap();
        if !valid_file_name(&name) || progress.file_errors.len() >= MAX_EXTERNAL_MODELS {
            return Err(library_error(ErrorCode::ModelLibraryLimit));
        }
        progress.file_errors.push(ScanFileFailure {
            file_name: name,
            reason,
        });
        if serde_json::to_vec(&progress.file_errors)
            .map_err(|_| library_error(ErrorCode::ModelLibraryLimit))?
            .len()
            > MAX_SCAN_DIAGNOSTIC_BYTES
        {
            return Err(library_error(ErrorCode::ModelLibraryLimit));
        }
        Ok(())
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
pub struct ExternalSource {
    pub directory: PathBuf,
    pub directory_identity: FileIdentity,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalRegistration {
    pub manifest: ModelManifest,
    pub identity: FileIdentity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<ExternalSource>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelLibrary {
    pub schema_version: u32,
    pub directory_id: Option<Uuid>,
    pub library_generation: Uuid,
    pub directory: Option<PathBuf>,
    pub directory_identity: Option<FileIdentity>,
    pub models: Vec<ExternalRegistration>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LibraryDirectoryInfo {
    pub directory_id: Uuid,
    pub display_path: String,
    pub library_generation: Uuid,
}
impl ModelLibrary {
    pub fn info(&self) -> Option<LibraryDirectoryInfo> {
        Some(LibraryDirectoryInfo {
            directory_id: self.directory_id?,
            display_path: self.directory.as_ref()?.to_string_lossy().into_owned(),
            library_generation: self.library_generation,
        })
    }
    pub fn configured_directory(&self) -> Result<&Path> {
        self.directory
            .as_deref()
            .ok_or_else(|| library_error(ErrorCode::ModelDirectoryUnavailable))
    }
    pub fn source<'a>(
        &'a self,
        entry: &'a ExternalRegistration,
    ) -> Result<(&'a Path, &'a FileIdentity)> {
        if let Some(source) = &entry.source {
            Ok((&source.directory, &source.directory_identity))
        } else {
            Ok((
                self.configured_directory()?,
                self.directory_identity
                    .as_ref()
                    .ok_or_else(|| library_error(ErrorCode::ModelLibraryChanged))?,
            ))
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
        match (&self.directory, &self.directory_identity, self.directory_id) {
            (Some(path), Some(_), Some(id)) if !id.is_nil() => validate_directory_syntax(path)?,
            (None, None, None) if self.schema_version == 2 => (),
            _ => return Err(library_error(ErrorCode::ModelLibraryChanged)),
        }
        if !matches!(self.schema_version, 1 | 2)
            || self.library_generation.is_nil()
            || self.models.len() > MAX_EXTERNAL_MODELS
        {
            return Err(library_error(ErrorCode::ModelLibraryChanged));
        }
        let mut ids = BTreeSet::new();
        let mut names = BTreeSet::new();
        for entry in &self.models {
            entry.manifest.validate()?;
            let m = &entry.manifest;
            let (directory, directory_identity) = self.source(entry)?;
            validate_directory_syntax(directory)?;
            if m.storage != ModelStorage::External
                || m.size_bytes > MAX_MODEL_BYTES
                || m.size_bytes != entry.identity.size
                || !ids.insert(m.id.clone())
                || (self.schema_version == 1 && entry.source.is_some())
                || !names.insert((
                    directory_identity.volume,
                    directory_identity.file,
                    file_key(&m.relative_file),
                ))
                || directory_identity.modified_nanos >= 1_000_000_000
                || entry.identity.modified_nanos >= 1_000_000_000
            {
                return Err(library_error(ErrorCode::ModelLibraryChanged));
            }
        }
        Ok(())
    }
    pub fn directory_presence(&self) -> Result<bool> {
        if !directory_drive_present(self.configured_directory()?)? {
            return Ok(false);
        }
        match fs::symlink_metadata(self.configured_directory()?) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(_) => return Err(library_error(ErrorCode::ModelDirectoryUnavailable)),
            Ok(_) => (),
        }
        self.check_directory_identity()?;
        Ok(true)
    }
    pub fn check_directory_identity(&self) -> Result<()> {
        let directory = DirectoryGuard::open(self.configured_directory()?)?;
        if !self
            .directory_identity
            .as_ref()
            .is_some_and(|saved| same_object(&directory.identity, saved))
        {
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
            let (path, saved) = self.source(entry)?;
            let directory = DirectoryGuard::open(path)?;
            if !same_object(&directory.identity, saved) {
                return Err(library_error(ErrorCode::ModelFileChanged));
            }
            let source =
                open_read_file(&directory.path.join(&entry.manifest.relative_file), false)?;
            if identity(&source)? != entry.identity {
                return Err(library_error(ErrorCode::ModelFileChanged));
            }
            Ok(())
        };
        check().err().map(|e| e.code)
    }
}

/// New regular GGUF files only. Partial/control neighbors and symlinks never
/// qualify; observations are metadata-only and bounded by the scan budget.
pub fn observe_candidates(library: &ModelLibrary) -> Result<Vec<(String, FileIdentity)>> {
    library.check_directory_identity()?;
    let mut result = Vec::new();
    for (count, entry) in fs::read_dir(library.configured_directory()?)
        .map_err(file_error)?
        .enumerate()
    {
        if count >= MAX_DIRECTORY_ENTRIES {
            return Err(library_error(ErrorCode::ModelLibraryLimit));
        }
        let entry = entry.map_err(file_error)?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !name.to_ascii_lowercase().ends_with(".gguf") || !valid_file_name(&name) {
            continue;
        }
        if ["aria2", "part", "crdownload", "download"]
            .iter()
            .any(|suffix| {
                library
                    .configured_directory()
                    .unwrap()
                    .join(format!("{name}.{suffix}"))
                    .exists()
            })
        {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path()).map_err(file_error)?;
        if !metadata.is_file() || indirect(&metadata) || metadata.len() == 0 {
            continue;
        }
        if metadata.len() > MAX_MODEL_BYTES || result.len() >= MAX_EXTERNAL_MODELS {
            return Err(library_error(ErrorCode::ModelLibraryLimit));
        }
        let file = open_read_file(&entry.path(), false)?;
        let observed = identity(&file)?;
        if library.models.iter().any(|entry| {
            file_key(&entry.manifest.relative_file) == file_key(&name) && entry.identity == observed
        }) {
            continue;
        }
        result.push((name, observed));
    }
    result.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(result)
}

pub struct ScannedLibrary {
    // None means every candidate was rejected. There is deliberately no empty
    // replacement library to accidentally publish in that case.
    library: Option<ModelLibrary>,
    _directory: Arc<DirectoryGuard>,
    _sources: Vec<File>,
}
impl ScannedLibrary {
    pub fn library(&self) -> Option<&ModelLibrary> {
        self.library.as_ref()
    }
}

pub fn scan_directory(
    root: &Path,
    directory: &Path,
    previous: Option<&ModelLibrary>,
    control: &ScanControl,
) -> Result<ScannedLibrary> {
    scan_directory_inner(root, directory, previous, None, control)
}

/// Rescanning must bind to the saved directory object, not merely its path.
/// Applying a newly selected directory deliberately uses `scan_directory`.
pub fn rescan_directory(
    root: &Path,
    previous: &ModelLibrary,
    control: &ScanControl,
) -> Result<ScannedLibrary> {
    scan_directory_inner(
        root,
        previous.configured_directory()?,
        Some(previous),
        previous.directory_identity.as_ref(),
        control,
    )
}

fn scan_directory_inner(
    root: &Path,
    directory: &Path,
    previous: Option<&ModelLibrary>,
    expected_directory: Option<&FileIdentity>,
    control: &ScanControl,
) -> Result<ScannedLibrary> {
    control.check()?;
    let directory = Arc::new(DirectoryGuard::open(directory)?);
    if expected_directory.is_some_and(|expected| !same_object(expected, &directory.identity)) {
        return Err(library_error(ErrorCode::ModelFileChanged));
    }
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
        if previous.is_some_and(|old| {
            old.models.iter().any(|entry| {
                entry.source.as_ref().is_some_and(|source| {
                    same_object(&source.directory_identity, &directory.identity)
                        && file_key(&entry.manifest.relative_file) == file_key(&name)
                })
            })
        }) {
            continue;
        }
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
        if ["aria2", "part", "crdownload", "download"]
            .iter()
            .any(|suffix| directory.path.join(format!("{name}.{suffix}")).exists())
        {
            continue;
        }
        total = add_scan_bytes(total, metadata.len())?;
        candidates.push(name);
        if candidates.len() > MAX_EXTERNAL_MODELS {
            return Err(library_error(ErrorCode::ModelLibraryLimit));
        }
        control.progress.lock().unwrap().candidate_files = candidates.len();
    }
    candidates.sort_by_key(|name| file_key(name));
    let prior = previous.filter(|old| {
        old.directory_identity
            .as_ref()
            .is_some_and(|saved| same_object(saved, &directory.identity))
            && old
                .directory
                .as_ref()
                .is_some_and(|path| path_key(path) == path_key(&directory.path))
    });
    let directory_id = prior
        .and_then(|old| old.directory_id)
        .unwrap_or_else(Uuid::new_v4);
    let mut models = Vec::new();
    let mut sources = Vec::new();
    let mut verified_total = 0_u64;
    control.progress.lock().unwrap().phase = "verifying";
    for name in candidates {
        control.progress.lock().unwrap().current_file_name = Some(name.clone());
        control.check()?;
        let mut source = open_read_file(&directory.path.join(&name), true)?;
        let before = identity(&source)?;
        verified_total = add_scan_bytes(verified_total, before.size)?;
        let inspected = inspect_input(&mut source, before.size, control, true);
        // Even a content rejection must not conceal cancellation, real I/O or
        // a changing source. Inspect the same held handle before classifying it.
        let (hash, metadata) = checked_inspection(&source, &before, control, inspected)?;
        let metadata = match metadata {
            Ok(metadata) => metadata,
            Err(failure) => {
                let reason = match failure.code {
                    ErrorCode::InvalidManifest => ScanRejectionReason::InvalidManifest,
                    ErrorCode::UnsupportedModel => ScanRejectionReason::UnsupportedModel,
                    ErrorCode::UnsupportedChatTemplate => {
                        ScanRejectionReason::UnsupportedChatTemplate
                    }
                    _ => return Err(failure),
                };
                control.reject(name, reason)?;
                sources.push(source);
                continue;
            }
        };
        let existing = previous.and_then(|old| {
            old.models.iter().find(|m| {
                old.source(m).is_ok_and(|(_, saved)| {
                    same_object(saved, &directory.identity)
                        && file_key(&m.manifest.relative_file) == file_key(&name)
                }) && m.manifest.sha256 == hash
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
        // Only directory discovery chooses this default. Explicit import/load
        // contexts retain their existing validation and are never clamped.
        request.default_context = request.default_context.min(metadata.context_length);
        let mut manifest = ModelManifest::build(request, before.size, hash, metadata)?;
        manifest.storage = ModelStorage::External;
        manifest.relative_file = name;
        manifest.validate()?;
        models.push(ExternalRegistration {
            manifest,
            identity: before,
            source: existing.and_then(|entry| entry.source.clone()),
        });
        sources.push(source);
        control.progress.lock().unwrap().verified_files += 1;
    }
    let rejected_all = models.is_empty() && !control.progress().file_errors.is_empty();
    // Directory maintenance only owns implicit entries in the configured directory.
    // Freeze an old configured directory before changing it, and preserve all links.
    if let Some(previous) = previous {
        for old in &previous.models {
            if prior.is_some() && old.source.is_none() {
                continue;
            }
            if models
                .iter()
                .any(|entry| entry.manifest.id == old.manifest.id)
            {
                continue;
            }
            let mut old = old.clone();
            if old.source.is_none() {
                let (path, saved) = previous.source(&old)?;
                old.source = Some(ExternalSource {
                    directory: path.to_owned(),
                    directory_identity: saved.clone(),
                });
            }
            models.push(old);
        }
    }
    let library = ModelLibrary {
        schema_version: 2,
        directory_id: Some(directory_id),
        library_generation: Uuid::new_v4(),
        directory: Some(directory.path.clone()),
        directory_identity: Some(directory.identity.clone()),
        models,
    };
    let library = if rejected_all {
        None
    } else {
        library.encode()?;
        Some(library)
    };
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
type ContentInspection = Result<(String, Result<gguf::Metadata>)>;
fn checked_inspection(
    source: &File,
    before: &FileIdentity,
    control: &ScanControl,
    inspected: ContentInspection,
) -> ContentInspection {
    control.check()?;
    if identity(source)? != *before {
        return Err(library_error(ErrorCode::ModelFileChanged));
    }
    inspected
}
#[cfg(windows)]
fn inspect(
    source: &mut File,
    size: u64,
    control: &ScanControl,
) -> Result<(String, gguf::Metadata)> {
    let (hash, metadata) = inspect_input(source, size, control, false)?;
    Ok((hash, metadata?))
}
/// The outer Result contains I/O/control failures. The inner Result contains
/// only the structural reader's content verdict, never an unobserved read error.
fn inspect_input(
    source: &mut File,
    size: u64,
    control: &ScanControl,
    scan: bool,
) -> ContentInspection {
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
    let metadata = inspect_metadata(source, control, scan)?;
    Ok((format!("{:x}", hasher.finalize()), metadata))
}
fn inspect_metadata<R: Read + Seek>(
    source: &mut R,
    control: &ScanControl,
    scan: bool,
) -> Result<Result<gguf::Metadata>> {
    let mut observed = ReadObservation::default();
    let reader = ControlledRead {
        source,
        control,
        observed: &mut observed,
    };
    let metadata = if scan {
        gguf::read_for_scan(reader)
    } else {
        gguf::read(reader)
    };
    control.check()?;
    if let Some(error) = observed.failure {
        return Err(error);
    }
    // read_exact produces UnexpectedEof after an otherwise successful zero-byte
    // read. Only that observed parser EOF is a content error; a syscall error,
    // including an actual UnexpectedEof error, remains fatal above.
    let metadata = match metadata {
        Err(error) if error.code == ErrorCode::Io && observed.eof => {
            Err(crate::invalid_manifest("truncated GGUF content"))
        }
        other => other,
    };
    Ok(metadata)
}

#[derive(Default)]
struct ReadObservation {
    failure: Option<RuntimeError>,
    eof: bool,
}
struct ControlledRead<'a, R> {
    source: &'a mut R,
    control: &'a ScanControl,
    observed: &'a mut ReadObservation,
}
impl<R: Read> Read for ControlledRead<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.control
            .check()
            .map_err(|_| std::io::Error::other("model verification interrupted"))?;
        let limit = buffer.len().min(BLOCK);
        let result = self.source.read(&mut buffer[..limit]);
        match &result {
            Ok(0) if limit != 0 => self.observed.eof = true,
            Err(error) => {
                self.observed.failure = Some(library_error(
                    if matches!(error.raw_os_error(), Some(32 | 33)) && cfg!(windows) {
                        ErrorCode::ModelFileInUse
                    } else {
                        ErrorCode::ModelFileUnavailable
                    },
                ))
            }
            _ => (),
        }
        result
    }
}
impl<R: Seek> Seek for ControlledRead<'_, R> {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        self.control
            .check()
            .map_err(|_| std::io::Error::other("model verification interrupted"))?;
        let result = self.source.seek(position);
        if result.is_err() {
            self.observed.failure = Some(library_error(ErrorCode::ModelFileUnavailable));
        }
        result
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
            let (path, saved) = library.source(entry)?;
            let directory = Arc::new(DirectoryGuard::open(path)?);
            if !same_object(&directory.identity, saved) {
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DirectoryPolicy {
    ExternalSelected,
    DataDirectory,
}
/// Lexical Windows policy, deliberately independent of the host's Path parser.
/// Returns only the drive letter for GetDriveTypeW; never a rewritten path.
#[cfg(any(test, windows))]
fn windows_directory_drive(text: &str, policy: DirectoryPolicy) -> Option<u8> {
    if text.len() > 32768 || text.contains('\0') || text.contains("://") {
        return None;
    }
    let (disk, verbatim) = match text.strip_prefix(r"\\?\") {
        Some(disk) if policy == DirectoryPolicy::DataDirectory => (disk, true),
        Some(_) => return None,
        None => (text, false),
    };
    let bytes = disk.as_bytes();
    if bytes.len() < 3
        || !bytes[0].is_ascii_alphabetic()
        || bytes[1] != b':'
        || !(bytes[2] == b'\\' || (!verbatim && bytes[2] == b'/'))
        || (verbatim && disk.contains('/'))
    {
        return None;
    }
    let components: Vec<_> = disk[3..]
        .split(['\\', '/'])
        .filter(|part| !part.is_empty())
        .collect();
    if components.contains(&"..") || components.len() + 2 > MAX_DIRECTORY_COMPONENTS {
        return None;
    }
    Some(bytes[0])
}
fn validate_directory_syntax(path: &Path) -> Result<()> {
    validate_directory_policy(path, DirectoryPolicy::ExternalSelected)
}
fn validate_directory_policy(path: &Path, policy: DirectoryPolicy) -> Result<()> {
    let text = path
        .to_str()
        .ok_or_else(|| library_error(ErrorCode::ModelDirectoryUnsupported))?;
    if !path.is_absolute()
        || text.len() > 32768
        || path.components().count() > MAX_DIRECTORY_COMPONENTS
        || text.contains("://")
        || text.contains('\0')
        || path.components().any(|c| matches!(c, Component::ParentDir))
    {
        return Err(library_error(ErrorCode::ModelDirectoryUnsupported));
    }
    #[cfg(windows)]
    {
        use std::path::Prefix;
        let valid_prefix = match path.components().next() {
            Some(Component::Prefix(prefix)) => match prefix.kind() {
                Prefix::Disk(_) => true,
                Prefix::VerbatimDisk(_) => policy == DirectoryPolicy::DataDirectory,
                _ => false,
            },
            _ => false,
        };
        if !valid_prefix || windows_directory_drive(text, policy).is_none() {
            return Err(library_error(ErrorCode::ModelDirectoryUnsupported));
        }
    }
    #[cfg(not(windows))]
    {
        let _ = policy;
        if text.starts_with("\\\\") || text.starts_with("//") {
            return Err(library_error(ErrorCode::ModelDirectoryUnsupported));
        }
    }
    Ok(())
}
fn directory_drive_present(path: &Path) -> Result<bool> {
    directory_drive_present_with_policy(path, DirectoryPolicy::ExternalSelected)
}
fn directory_drive_present_with_policy(path: &Path, policy: DirectoryPolicy) -> Result<bool> {
    validate_directory_policy(path, policy)?;
    #[cfg(windows)]
    {
        let letter =
            windows_directory_drive(path.to_str().unwrap(), policy).expect("validated disk prefix");
        let root: Vec<u16> = format!("{}:\\\0", char::from(letter))
            .encode_utf16()
            .collect();
        // Query only X:\. All actual opens retain the original path, including
        // canonical verbatim semantics and its long/trailing-space components.
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
pub(crate) fn data_directory_object_identity(path: &Path) -> Result<(u64, u64)> {
    let guard = DirectoryGuard::open_data_directory(path)?;
    Ok((guard.identity.volume, guard.identity.file))
}
pub(crate) struct DirectoryGuard {
    path: PathBuf,
    identity: FileIdentity,
    _ancestors: Vec<File>,
}
impl DirectoryGuard {
    fn open(path: &Path) -> Result<Self> {
        Self::open_with_policy(path, DirectoryPolicy::ExternalSelected)
    }
    pub(crate) fn open_data_directory(path: &Path) -> Result<Self> {
        Self::open_with_policy(path, DirectoryPolicy::DataDirectory)
    }
    fn open_with_policy(path: &Path, policy: DirectoryPolicy) -> Result<Self> {
        if !directory_drive_present_with_policy(path, policy)? {
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
pub(crate) fn identity(file: &File) -> Result<FileIdentity> {
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
pub(crate) fn open_read_file(path: &Path, protect: bool) -> Result<File> {
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
    fn windows_directory_policy_is_host_independent_and_never_rewrites() {
        use DirectoryPolicy::{DataDirectory, ExternalSelected};
        for (path, external, internal) in [
            (r"C:\", true, true),
            (r"c:\Nexa\模型", true, true),
            ("D:/Nexa/models", true, true),
            (r"\\?\C:\Nexa\models", false, true),
            (r"\\?\C:\Nexa\tail. ", false, true),
            (r"\\?\C:\", false, true),
            (r"\\server\share\models", false, false),
            (r"\\?\UNC\server\share", false, false),
            (r"\\.\C:\models", false, false),
            (r"\\?\Volume{123}\models", false, false),
            (r"\\?\GLOBALROOT\Device\HarddiskVolume1", false, false),
            (r"\\?\C:relative", false, false),
            (r"\\?\C:/models", false, false),
            (r"C:relative", false, false),
            (r"\root-relative", false, false),
            (r"models", false, false),
            (r"C:\models\..\elsewhere", false, false),
            (r"\\?\C:\models\..\elsewhere", false, false),
            (r"1:\models", false, false),
            ("C:\\models\0hidden", false, false),
        ] {
            let before = path.to_owned();
            assert_eq!(
                windows_directory_drive(path, ExternalSelected).is_some(),
                external,
                "{path:?}"
            );
            assert_eq!(
                windows_directory_drive(path, DataDirectory).is_some(),
                internal,
                "{path:?}"
            );
            assert_eq!(path, before);
        }
        let long = format!(r"\\?\C:\{}", "x".repeat(260));
        assert_eq!(windows_directory_drive(&long, DataDirectory), Some(b'C'));
        assert!(
            windows_directory_drive(&format!(r"C:\{}", "x".repeat(32769)), DataDirectory).is_none()
        );
        let deep = format!(r"C:\{}", vec!["x"; MAX_DIRECTORY_COMPONENTS].join("\\"));
        assert!(windows_directory_drive(&deep, DataDirectory).is_none());
    }
    #[cfg(windows)]
    #[test]
    fn canonical_data_directory_retains_exact_path_and_object_identity() {
        let temp = tempfile::tempdir().unwrap();
        let canonical = fs::canonicalize(temp.path()).unwrap();
        assert!(
            matches!(canonical.components().next(), Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), std::path::Prefix::VerbatimDisk(_)))
        );
        assert!(DirectoryGuard::open(&canonical).is_err());
        let normal = DirectoryGuard::open_data_directory(temp.path()).unwrap();
        let verbatim = DirectoryGuard::open_data_directory(&canonical).unwrap();
        assert_eq!(verbatim.path, canonical);
        assert!(same_object(&normal.identity, &verbatim.identity));
        let special = canonical.join("kept-trailing. ");
        fs::create_dir(&special).unwrap();
        let special_guard = DirectoryGuard::open_data_directory(&special).unwrap();
        assert_eq!(special_guard.path, special);
        drop(special_guard);
        fs::remove_dir(special).unwrap();
    }
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
        let mut library = scan.library().unwrap().clone();
        drop(scan);
        // No probe or filesystem access to this old letter is authorized by
        // deserialization. Current access is checked separately on use.
        library.directory = Some(PathBuf::from(r"Z:\external-model-fixture"));
        fs::write(root.path().join(LIBRARY_FILE), library.encode().unwrap()).unwrap();
        let previous = ModelLibrary::read(root.path()).unwrap().unwrap();
        let replacement = scan_directory(
            root.path(),
            source.path(),
            Some(&previous),
            &ScanControl::default(),
        )
        .unwrap();
        assert_eq!(
            replacement.library().unwrap().directory.as_deref(),
            Some(source.path())
        );
        assert_ne!(
            replacement.library().unwrap().directory_id,
            previous.directory_id
        );
    }
}

#[cfg(test)]
mod scan_rejection_tests {
    use super::*;
    use std::io::{Cursor, ErrorKind};

    #[test]
    fn byte_budget_boundaries_use_the_same_counter_on_every_platform() {
        assert_eq!(add_scan_bytes(0, 0).unwrap(), 0);
        assert_eq!(add_scan_bytes(0, MAX_MODEL_BYTES).unwrap(), MAX_MODEL_BYTES);
        assert_eq!(
            add_scan_bytes(MAX_MODEL_BYTES, MAX_MODEL_BYTES).unwrap(),
            MAX_SCAN_BYTES
        );
        assert_eq!(
            add_scan_bytes(MAX_SCAN_BYTES - 1, 1).unwrap(),
            MAX_SCAN_BYTES
        );
        for (total, size) in [
            (0, MAX_MODEL_BYTES + 1),
            (MAX_SCAN_BYTES, 1),
            (u64::MAX, 1),
            (u64::MAX, 0),
        ] {
            assert_eq!(
                add_scan_bytes(total, size).unwrap_err().code,
                ErrorCode::ModelLibraryLimit
            );
        }
    }

    struct FaultReader {
        cursor: Cursor<Vec<u8>>,
        read_error: bool,
        seek_error: bool,
        eof: bool,
    }
    impl Read for FaultReader {
        fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
            if self.read_error {
                return Err(std::io::Error::from(ErrorKind::UnexpectedEof));
            }
            if self.eof {
                return Ok(0);
            }
            self.cursor.read(bytes)
        }
    }
    impl Seek for FaultReader {
        fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
            if self.seek_error {
                return Err(std::io::Error::from(ErrorKind::PermissionDenied));
            }
            self.cursor.seek(position)
        }
    }
    #[test]
    fn parser_eof_is_distinct_from_actual_read_and_seek_errors() {
        for (read_error, seek_error, eof) in [
            (true, false, false),
            (false, true, false),
            (false, false, true),
        ] {
            let mut reader = FaultReader {
                cursor: Cursor::new(vec![0; 64]),
                read_error,
                seek_error,
                eof,
            };
            let result = inspect_metadata(&mut reader, &ScanControl::default(), true);
            if eof {
                assert_eq!(
                    result.unwrap().unwrap_err().code,
                    ErrorCode::InvalidManifest
                );
            } else {
                assert_eq!(result.unwrap_err().code, ErrorCode::ModelFileUnavailable);
            }
        }
    }
    #[test]
    fn cancellation_and_source_change_precede_content_rejection() {
        let source = tempfile::NamedTempFile::new().unwrap();
        fs::write(source.path(), b"bad").unwrap();
        let file = File::open(source.path()).unwrap();
        let before = identity(&file).unwrap();
        let rejected = || Ok(("hash".into(), Err(crate::invalid_manifest("bad content"))));
        let cancelled = ScanControl::default();
        cancelled.cancel();
        assert_eq!(
            checked_inspection(&file, &before, &cancelled, rejected())
                .unwrap_err()
                .code,
            ErrorCode::ModelScanCancelled
        );
        assert_eq!(
            checked_inspection(
                &file,
                &before,
                &ScanControl::with_timeout(Duration::ZERO),
                rejected()
            )
            .unwrap_err()
            .code,
            ErrorCode::ModelScanTimeout
        );
        fs::write(source.path(), b"changed").unwrap();
        assert_eq!(
            checked_inspection(&file, &before, &ScanControl::default(), rejected())
                .unwrap_err()
                .code,
            ErrorCode::ModelFileChanged
        );
    }
}
