//! Synthetic catalog/ownership tests, not native-model compatibility evidence.
use model_store::library::{
    LIBRARY_FILE, MAX_MODEL_BYTES, ModelLibrary, ScanControl, scan_directory,
};
use model_store::{ImportCancellation, ImportRequest, ModelSource, ModelStorage, ModelStore};
use runtime_types::{ErrorCode, ModelId};
use std::{fs, io::Cursor, time::Duration};
fn string(bytes: &mut Vec<u8>, value: &str) {
    bytes.extend((value.len() as u64).to_le_bytes());
    bytes.extend(value.as_bytes());
}
fn gguf() -> Vec<u8> {
    let mut bytes = b"GGUF".to_vec();
    bytes.extend(3_u32.to_le_bytes());
    bytes.extend(1_u64.to_le_bytes());
    bytes.extend(4_u64.to_le_bytes());
    for (key, value) in [
        ("general.architecture", "qwen3"),
        ("tokenizer.chat_template", "synthetic 中文 template"),
    ] {
        string(&mut bytes, key);
        bytes.extend(8_u32.to_le_bytes());
        string(&mut bytes, value);
    }
    for (key, value) in [
        ("general.file_type", 7_u32),
        ("qwen3.context_length", 40960),
    ] {
        string(&mut bytes, key);
        bytes.extend(4_u32.to_le_bytes());
        bytes.extend(value.to_le_bytes());
    }
    string(&mut bytes, "synthetic.weight");
    bytes.extend(1_u32.to_le_bytes());
    bytes.extend(32_u64.to_le_bytes());
    bytes.extend(0_u32.to_le_bytes());
    bytes.extend(0_u64.to_le_bytes());
    bytes.resize(bytes.len().next_multiple_of(32), 0);
    bytes.resize(bytes.len() + 128, 0);
    bytes
}
fn scan(
    root: &std::path::Path,
    source: &std::path::Path,
    prior: Option<&ModelLibrary>,
) -> ModelLibrary {
    scan_directory(root, source, prior, &ScanControl::default())
        .unwrap()
        .library()
        .clone()
}
#[test]
fn scan_is_read_only_unicode_named_and_stable_without_copy() {
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let original = gguf();
    let path = source.path().join("中文 模型.GGUF");
    fs::write(&path, &original).unwrap();
    let first = scan(data.path(), source.path(), None);
    assert_eq!(first.models.len(), 1);
    let m = &first.models[0].manifest;
    assert_eq!(m.display_name, "中文 模型");
    assert!(m.id.as_str().starts_with("ext-"));
    assert_eq!(m.storage, ModelStorage::External);
    assert_eq!(m.relative_file, "中文 模型.GGUF");
    assert!(!m.validated);
    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(fs::read_dir(source.path()).unwrap().count(), 1);
    assert_eq!(fs::read_dir(data.path()).unwrap().count(), 0);
    let second = scan(data.path(), source.path(), Some(&first));
    assert_eq!(second.models[0].manifest.id, m.id);
    assert_eq!(second.directory_id, first.directory_id);
    assert_ne!(second.library_generation, first.library_generation);
}
#[test]
fn renamed_or_changed_files_get_new_ids_but_duplicate_hashes_are_not_deleted() {
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    fs::write(source.path().join("first.gguf"), gguf()).unwrap();
    let first = scan(data.path(), source.path(), None);
    fs::rename(
        source.path().join("first.gguf"),
        source.path().join("renamed.gguf"),
    )
    .unwrap();
    fs::write(source.path().join("duplicate.gguf"), gguf()).unwrap();
    let second = scan(data.path(), source.path(), Some(&first));
    assert_eq!(second.models.len(), 2);
    assert_ne!(second.models[0].manifest.id, second.models[1].manifest.id);
    assert_eq!(
        second.models[0].manifest.sha256,
        second.models[1].manifest.sha256
    );
    assert!(
        second
            .models
            .iter()
            .all(|m| m.manifest.id != first.models[0].manifest.id)
    );
    let mut modified = gguf();
    *modified.last_mut().unwrap() = 1;
    fs::write(source.path().join("renamed.gguf"), modified).unwrap();
    let third = scan(data.path(), source.path(), Some(&second));
    let old = second
        .models
        .iter()
        .find(|m| m.manifest.relative_file == "renamed.gguf")
        .unwrap();
    let new = third
        .models
        .iter()
        .find(|m| m.manifest.relative_file == "renamed.gguf")
        .unwrap();
    assert_ne!(old.manifest.id, new.manifest.id);
}
#[test]
fn managed_compatibility_and_external_id_conflict_do_not_write_source() {
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let bytes = gguf();
    let store = ModelStore::open(data.path()).unwrap();
    let old_id = ModelId::new("legacy.model").unwrap();
    store
        .import_reader(
            Cursor::new(&bytes),
            bytes.len() as u64,
            ImportRequest::new(
                old_id.clone(),
                "Existing model",
                ModelSource::local("fixture"),
            ),
            &ImportCancellation::default(),
        )
        .unwrap();
    drop(store);
    fs::write(source.path().join("外部 模型.gguf"), &bytes).unwrap();
    let library = scan(data.path(), source.path(), None);
    fs::write(data.path().join(LIBRARY_FILE), library.encode().unwrap()).unwrap();
    let store = ModelStore::open(data.path()).unwrap();
    assert_eq!(store.list().unwrap().len(), 2);
    assert_eq!(store.get(&old_id).unwrap().display_name, "Existing model");
    let id = library.models[0].manifest.id.clone();
    let error = store
        .import_reader(
            Cursor::new(&bytes),
            bytes.len() as u64,
            ImportRequest::new(id.clone(), "collision", ModelSource::local("fixture")),
            &ImportCancellation::default(),
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::AlreadyExists);
    let conflict = store
        .import_file(
            source.path().join("does-not-exist.gguf"),
            ImportRequest::new(id.clone(), "collision", ModelSource::local("fixture")),
            &ImportCancellation::default(),
        )
        .unwrap_err();
    assert_eq!(conflict.code, ErrorCode::AlreadyExists);
    assert!(store.remove(&id).is_err());
    assert!(!data.path().join("models").join(id.as_str()).exists());
    assert_eq!(
        fs::read(source.path().join("外部 模型.gguf")).unwrap(),
        bytes
    );
    assert_eq!(
        store.resolve(&id).unwrap_err().code,
        ErrorCode::ModelFileUnavailable
    );
}
#[test]
fn external_manifest_cannot_be_smuggled_into_managed_layout() {
    let data = tempfile::tempdir().unwrap();
    let bytes = gguf();
    let store = ModelStore::open(data.path()).unwrap();
    let id = ModelId::new("managed").unwrap();
    let mut manifest = store
        .import_reader(
            Cursor::new(&bytes),
            bytes.len() as u64,
            ImportRequest::new(id, "model", ModelSource::local("fixture")),
            &ImportCancellation::default(),
        )
        .unwrap();
    drop(store);
    manifest.storage = ModelStorage::External;
    fs::write(
        data.path().join("models/managed/manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    assert_eq!(
        ModelStore::open(data.path()).err().unwrap().code,
        ErrorCode::InvalidManifest
    );
    assert_eq!(
        fs::read(data.path().join("models/managed/model.gguf")).unwrap(),
        bytes
    );
}
#[test]
fn candidate_and_size_limits_fail_before_any_registration() {
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    for n in 0..65 {
        fs::write(source.path().join(format!("{n}.gguf")), b"GGUF").unwrap();
    }
    assert_eq!(
        scan_directory(data.path(), source.path(), None, &ScanControl::default())
            .err()
            .unwrap()
            .code,
        ErrorCode::ModelLibraryLimit
    );
    assert!(!data.path().join(LIBRARY_FILE).exists());
    let other = tempfile::tempdir().unwrap();
    fs::File::create(other.path().join("huge.gguf"))
        .unwrap()
        .set_len(MAX_MODEL_BYTES + 1)
        .unwrap();
    assert_eq!(
        scan_directory(data.path(), other.path(), None, &ScanControl::default())
            .err()
            .unwrap()
            .code,
        ErrorCode::ModelLibraryLimit
    );
}
#[test]
fn cancellation_deadline_and_invalid_file_preserve_existing_index() {
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    fs::write(source.path().join("model.gguf"), gguf()).unwrap();
    let old = scan(data.path(), source.path(), None);
    let encoded = old.encode().unwrap();
    fs::write(data.path().join(LIBRARY_FILE), &encoded).unwrap();
    let cancel = ScanControl::default();
    cancel.cancel();
    assert_eq!(
        scan_directory(data.path(), source.path(), Some(&old), &cancel)
            .err()
            .unwrap()
            .code,
        ErrorCode::ModelScanCancelled
    );
    assert_eq!(
        scan_directory(
            data.path(),
            source.path(),
            Some(&old),
            &ScanControl::with_timeout(Duration::ZERO)
        )
        .err()
        .unwrap()
        .code,
        ErrorCode::ModelScanTimeout
    );
    fs::write(source.path().join("损坏 文件.gguf"), b"invalid").unwrap();
    let control = ScanControl::default();
    assert!(scan_directory(data.path(), source.path(), Some(&old), &control).is_err());
    assert_eq!(
        control.progress().current_file_name.as_deref(),
        Some("损坏 文件.gguf")
    );
    assert_eq!(fs::read(data.path().join(LIBRARY_FILE)).unwrap(), encoded);
}
#[test]
fn source_change_and_missing_are_observed_without_rewriting_catalog() {
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let file = source.path().join("model.gguf");
    fs::write(&file, gguf()).unwrap();
    let catalog = scan(data.path(), source.path(), None);
    assert_eq!(catalog.availability(&catalog.models[0]), None);
    fs::write(&file, b"changed").unwrap();
    assert_eq!(
        catalog.availability(&catalog.models[0]),
        Some(ErrorCode::ModelFileChanged)
    );
    fs::remove_file(file).unwrap();
    assert_eq!(
        catalog.availability(&catalog.models[0]),
        Some(ErrorCode::ModelFileUnavailable)
    );
}
#[cfg(unix)]
#[test]
fn directory_and_file_links_are_rejected() {
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    fs::write(elsewhere.path().join("model.gguf"), gguf()).unwrap();
    std::os::unix::fs::symlink(
        elsewhere.path().join("model.gguf"),
        source.path().join("model.gguf"),
    )
    .unwrap();
    assert_eq!(
        scan_directory(data.path(), source.path(), None, &ScanControl::default())
            .err()
            .unwrap()
            .code,
        ErrorCode::ModelDirectoryUnsupported
    );
    std::os::unix::fs::symlink(elsewhere.path(), data.path().join("link")).unwrap();
    assert!(
        scan_directory(
            data.path(),
            &data.path().join("link"),
            None,
            &ScanControl::default()
        )
        .is_err()
    );
}
#[cfg(windows)]
#[test]
fn windows_scan_guard_blocks_writers_and_replacements_then_releases() {
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let file = source.path().join("model.gguf");
    fs::write(&file, gguf()).unwrap();
    let scanned =
        scan_directory(data.path(), source.path(), None, &ScanControl::default()).unwrap();
    assert!(fs::OpenOptions::new().write(true).open(&file).is_err());
    assert!(fs::rename(&file, source.path().join("renamed.gguf")).is_err());
    drop(scanned);
    assert!(fs::OpenOptions::new().write(true).open(&file).is_ok());
    fs::rename(&file, source.path().join("renamed.gguf")).unwrap();
}
#[cfg(windows)]
#[test]
fn windows_preexisting_writer_prevents_registration_guard() {
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let file = source.path().join("model.gguf");
    fs::write(&file, gguf()).unwrap();
    let writer = fs::OpenOptions::new().write(true).open(&file).unwrap();
    assert_eq!(
        scan_directory(data.path(), source.path(), None, &ScanControl::default())
            .err()
            .unwrap()
            .code,
        ErrorCode::ModelFileInUse
    );
    drop(writer);
    assert!(scan_directory(data.path(), source.path(), None, &ScanControl::default()).is_ok());
}

#[test]
fn unvalidated_external_source_failure_precedes_load_attempt_without_rehashing() {
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let path = source.path().join("候选 模型.gguf");
    fs::write(&path, gguf()).unwrap();
    let library = scan(data.path(), source.path(), None);
    let manifest = library.models[0].manifest.clone();
    let encoded = library.encode().unwrap();
    fs::write(data.path().join(LIBRARY_FILE), &encoded).unwrap();
    let store = ModelStore::open(data.path()).unwrap();
    assert_eq!(
        manifest.compatibility(),
        runtime_types::ModelCompatibility::Unvalidated
    );
    assert!(manifest.load_candidate());
    assert!(!manifest.validated);
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(1)
        .unwrap();
    assert_eq!(
        store
            .prepare_external(&manifest.id, &ScanControl::default())
            .unwrap_err()
            .code,
        ErrorCode::ModelFileChanged
    );
    fs::remove_file(&path).unwrap();
    assert_eq!(
        store
            .prepare_external(&manifest.id, &ScanControl::default())
            .unwrap_err()
            .code,
        ErrorCode::ModelFileUnavailable
    );
    assert_eq!(store.get(&manifest.id).unwrap(), manifest);
    assert_eq!(fs::read(data.path().join(LIBRARY_FILE)).unwrap(), encoded);
}
