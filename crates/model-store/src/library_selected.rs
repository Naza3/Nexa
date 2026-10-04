//! Explicit, bounded file registrations. No directory enumeration and no copies.
use super::*;

pub const MAX_SELECTED_FILES: usize = 64;
/// Native-only selection lease. The original directory and file stay protected
/// through verification/publication; no path or handle crosses the WebView.
pub struct SelectedFile {
    directory: Arc<DirectoryGuard>,
    source: File,
    identity: FileIdentity,
    name: String,
}
impl SelectedFile {
    pub fn open(path: &Path) -> Result<Self> {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|n| valid_file_name(n))
            .ok_or_else(|| library_error(ErrorCode::ModelDirectoryUnsupported))?
            .to_owned();
        let parent = path
            .parent()
            .ok_or_else(|| library_error(ErrorCode::ModelDirectoryUnsupported))?;
        let directory = Arc::new(DirectoryGuard::open(parent)?);
        let source = open_read_file(&directory.path.join(&name), true)?;
        let identity = identity(&source)?;
        add_scan_bytes(0, identity.size)?;
        Ok(Self {
            directory,
            source,
            identity,
            name,
        })
    }
    pub fn open_configured(library: &ModelLibrary, name: &str) -> Result<Self> {
        if !valid_file_name(name) {
            return Err(library_error(ErrorCode::ModelDirectoryUnsupported));
        }
        let file = Self::open(&library.configured_directory()?.join(name))?;
        if !library
            .directory_identity
            .as_ref()
            .is_some_and(|saved| same_object(saved, &file.directory.identity))
        {
            return Err(library_error(ErrorCode::ModelFileChanged));
        }
        Ok(file)
    }
    pub fn file_name(&self) -> &str {
        &self.name
    }
    pub fn size_bytes(&self) -> u64 {
        self.identity.size
    }
    fn check(&self) -> Result<()> {
        let directory = DirectoryGuard::open(&self.directory.path)?;
        if !same_object(&directory.identity, &self.directory.identity)
            || identity(&self.source)? != self.identity
            || identity(&open_read_file(&directory.path.join(&self.name), false)?)? != self.identity
        {
            return Err(library_error(ErrorCode::ModelFileChanged));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SelectedStatus {
    Registered,
    AlreadyRegistered,
    Rejected,
    NotCommitted,
    NotProcessed,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SelectedResult {
    pub selection_index: usize,
    pub file_name: String,
    pub status: SelectedStatus,
    pub model_id: Option<ModelId>,
    pub error_code: Option<String>,
}
pub struct SelectedRegistration {
    pub library: ModelLibrary,
    pub files: Vec<SelectedResult>,
    pub available_files: usize,
    selected: Vec<SelectedFile>,
}
impl SelectedRegistration {
    /// Must run immediately before the commit decision while all leases are live.
    pub fn check(&self) -> Result<()> {
        for file in &self.selected {
            file.check()?;
        }
        Ok(())
    }
}
pub fn validate_selection(files: &[SelectedFile]) -> Result<()> {
    if files.is_empty() || files.len() > MAX_SELECTED_FILES {
        return Err(library_error(ErrorCode::ModelLibraryLimit));
    }
    let mut total = 0;
    for file in files {
        total = add_scan_bytes(total, file.identity.size)?;
    }
    Ok(())
}
/// Detect the precise managed source without opening/recovering a ModelStore or
/// inspecting any neighboring manifest/payload. Other hard-link aliases are not
/// inferred by scanning the managed catalog.
fn managed_target(
    root: &Path,
    file: &SelectedFile,
    hash: &str,
    metadata: &gguf::Metadata,
) -> Result<Option<ModelManifest>> {
    if file.name != "model.gguf" {
        return Ok(None);
    }
    let Some(parent) = file.directory.path.parent() else {
        return Ok(None);
    };
    let expected = root.join("models");
    match fs::symlink_metadata(&expected) {
        Ok(_) => (),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(library_error(ErrorCode::ModelLibraryChanged)),
    }
    let _managed_directory = DirectoryGuard::open(&expected)?;
    let canonical_expected = fs::canonicalize(&expected).map_err(file_error)?;
    if path_key(&fs::canonicalize(parent).map_err(file_error)?) != path_key(&canonical_expected) {
        return Ok(None);
    }
    let id = ModelId::new(
        file.directory
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| library_error(ErrorCode::ModelLibraryChanged))?,
    )?;
    let manifest_file = open_read_file(&file.directory.path.join("manifest.json"), false)?;
    if manifest_file.metadata().map_err(file_error)?.len() > 1024 * 1024 {
        return Err(library_error(ErrorCode::ModelLibraryLimit));
    }
    let mut bytes = Vec::new();
    manifest_file
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(file_error)?;
    if bytes.len() > 1024 * 1024 {
        return Err(library_error(ErrorCode::ModelLibraryLimit));
    }
    let manifest: ModelManifest = serde_json::from_slice(&bytes)
        .map_err(|_| library_error(ErrorCode::ModelLibraryChanged))?;
    manifest.validate()?;
    if manifest.storage != ModelStorage::Managed
        || manifest.id != id
        || manifest.relative_file != file.name
        || manifest.sha256 != hash
        || manifest.size_bytes != file.identity.size
        || !manifest.matches_metadata(metadata)
    {
        return Err(library_error(ErrorCode::ModelFileChanged));
    }
    Ok(Some(manifest))
}

/// Results are pending until the owner publishes library.encode() atomically.
/// Content rejection is per-file; path, identity, cancellation, and I/O abort.
pub fn register_selected(
    root: &Path,
    previous: Option<&ModelLibrary>,
    mut selected: Vec<SelectedFile>,
    control: &ScanControl,
    results: &Mutex<Vec<SelectedResult>>,
) -> Result<SelectedRegistration> {
    validate_selection(&selected)?;
    let mut library = previous.cloned().unwrap_or_else(|| ModelLibrary {
        schema_version: 2,
        directory_id: None,
        library_generation: Uuid::new_v4(),
        directory: None,
        directory_identity: None,
        models: vec![],
    });
    library.validate()?;
    library.schema_version = 2;
    {
        let mut progress = control.progress.lock().unwrap();
        progress.phase = "verifying";
        progress.candidate_files = selected.len();
    }
    *results.lock().unwrap() = selected
        .iter()
        .enumerate()
        .map(|(selection_index, file)| SelectedResult {
            selection_index,
            file_name: file.name.clone(),
            status: SelectedStatus::NotProcessed,
            model_id: None,
            error_code: None,
        })
        .collect();
    let mut already = BTreeSet::new();
    let mut available_files = 0;
    for (index, file) in selected.iter_mut().enumerate() {
        control.check()?;
        {
            let mut p = control.progress.lock().unwrap();
            p.examined_entries += 1;
            p.current_file_name = Some(file.name.clone());
        }
        file.check()?;
        file.source.seek(SeekFrom::Start(0)).map_err(file_error)?;
        let inspection = inspect_input(&mut file.source, file.identity.size, control, true);
        let (hash, metadata) =
            checked_inspection(&file.source, &file.identity, control, inspection)?;
        let metadata = match metadata {
            Ok(metadata) => metadata,
            Err(error) => {
                let reason = match error.code {
                    ErrorCode::InvalidManifest => ScanRejectionReason::InvalidManifest,
                    ErrorCode::UnsupportedModel => ScanRejectionReason::UnsupportedModel,
                    ErrorCode::UnsupportedChatTemplate => {
                        ScanRejectionReason::UnsupportedChatTemplate
                    }
                    _ => return Err(error),
                };
                control.reject(file.name.clone(), reason)?;
                let mut results = results.lock().unwrap();
                results[index].status = SelectedStatus::Rejected;
                results[index].error_code = Some(reason.as_str().into());
                continue;
            }
        };
        if let Some(manifest) = managed_target(root, file, &hash, &metadata)? {
            available_files += usize::from(manifest.load_candidate());
            let id = manifest.id;
            let mut results = results.lock().unwrap();
            results[index].status = SelectedStatus::NotCommitted;
            results[index].model_id = Some(id);
            already.insert(index);
            control.progress.lock().unwrap().verified_files += 1;
            continue;
        }
        let existing = library
            .models
            .iter()
            .position(|entry| {
                library.source(entry).is_ok_and(|(_, saved)| {
                    same_object(saved, &file.directory.identity)
                        && file_key(&entry.manifest.relative_file) == file_key(&file.name)
                })
            })
            .or_else(|| {
                library
                    .models
                    .iter()
                    .position(|entry| same_object(&entry.identity, &file.identity))
            });
        let same = existing.is_some_and(|i| {
            library.models[i].identity == file.identity && library.models[i].manifest.sha256 == hash
        });
        let id = match existing {
            Some(i) => library.models[i].manifest.id.clone(),
            None => fresh_id(root)?,
        };
        if root.join("models").join(id.as_str()).exists() {
            return Err(library_error(ErrorCode::ModelLibraryChanged));
        }
        {
            let stem = &file.name[..file.name.len() - 5];
            let name = if stem.trim().is_empty() {
                file.name.clone()
            } else {
                stem.to_owned()
            };
            let mut request = ImportRequest::new(
                id.clone(),
                name,
                ModelSource::local("user-selected read-only model file"),
            );
            request.source.file_name = Some(file.name.clone());
            request.default_context = request.default_context.min(metadata.context_length);
            let mut manifest = ModelManifest::build(request, file.identity.size, hash, metadata)?;
            manifest.storage = ModelStorage::External;
            manifest.relative_file = file.name.clone();
            manifest.validate()?;
            let entry = ExternalRegistration {
                manifest,
                identity: file.identity.clone(),
                source: Some(ExternalSource {
                    directory: file.directory.path.clone(),
                    directory_identity: file.directory.identity.clone(),
                }),
            };
            if let Some(i) = existing {
                library.models[i] = entry;
            } else {
                library.models.push(entry);
            }
        }
        // Adopt the explicitly selected spelling/source, even for a hard-link
        // alias. Keep the existing ID, never combine the new parent with an old basename.
        library.validate()?;
        available_files += usize::from(
            library
                .entry(&id)
                .is_some_and(|entry| entry.manifest.load_candidate()),
        );
        let mut results = results.lock().unwrap();
        results[index].status = SelectedStatus::NotCommitted;
        results[index].model_id = Some(id);
        if same {
            already.insert(index);
        }
        control.progress.lock().unwrap().verified_files += 1;
    }
    control.check()?;
    library.library_generation = Uuid::new_v4();
    library.encode()?;
    let mut files = results.lock().unwrap().clone();
    for file in &mut files {
        if file.status == SelectedStatus::NotCommitted {
            file.status = if already.contains(&file.selection_index) {
                SelectedStatus::AlreadyRegistered
            } else {
                SelectedStatus::Registered
            };
        }
    }
    let result = SelectedRegistration {
        library,
        files,
        available_files,
        selected,
    };
    result.check()?;
    Ok(result)
}

/// Only configure the default download/maintenance location. Does not enumerate
/// or inspect GGUFs, and keeps the directory guard alive through publication.
pub fn configure_directory(
    directory: &Path,
    previous: Option<&ModelLibrary>,
    control: &ScanControl,
) -> Result<ScannedLibrary> {
    control.check()?;
    let guard = Arc::new(DirectoryGuard::open(directory)?);
    let same = previous.is_some_and(|old| {
        old.directory_identity
            .as_ref()
            .is_some_and(|saved| same_object(saved, &guard.identity))
    });
    let mut models = previous.map_or_else(Vec::new, |old| old.models.clone());
    if !same && let Some(previous) = previous {
        for entry in &mut models {
            if entry.source.is_none() {
                entry.source = Some(ExternalSource {
                    directory: previous.configured_directory()?.to_owned(),
                    directory_identity: previous
                        .directory_identity
                        .clone()
                        .ok_or_else(|| library_error(ErrorCode::ModelLibraryChanged))?,
                });
            }
        }
    }
    let library = ModelLibrary {
        schema_version: 2,
        directory_id: if same {
            previous.and_then(|old| old.directory_id)
        } else {
            Some(Uuid::new_v4())
        },
        library_generation: Uuid::new_v4(),
        directory: Some(directory.to_owned()),
        directory_identity: Some(guard.identity.clone()),
        models,
    };
    library.encode()?;
    control.check()?;
    Ok(ScannedLibrary {
        library: Some(library),
        _directory: guard,
        _sources: Vec::new(),
    })
}
