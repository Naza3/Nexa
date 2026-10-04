//! Synthetic catalog/ownership tests, not native-model compatibility evidence.
#[cfg(unix)]
use model_store::library::MAX_MODEL_BYTES;
use model_store::library::{LIBRARY_FILE, ModelLibrary, ScanControl, scan_directory};
use model_store::{ImportCancellation, ImportRequest, ModelSource, ModelStorage, ModelStore};
use runtime_types::{ErrorCode, ModelId};
use std::{fs, io::Cursor, time::Duration};
fn string(bytes: &mut Vec<u8>, value: &str) {
    bytes.extend((value.len() as u64).to_le_bytes());
    bytes.extend(value.as_bytes());
}
fn gguf() -> Vec<u8> {
    fixture("qwen3", Some(40960), Some("synthetic 中文 template"), None)
}
fn fixture(
    architecture: &str,
    context: Option<u32>,
    template: Option<&str>,
    split: Option<u32>,
) -> Vec<u8> {
    let mut bytes = b"GGUF".to_vec();
    bytes.extend(3_u32.to_le_bytes());
    bytes.extend(1_u64.to_le_bytes());
    bytes.extend(
        (2 + u64::from(context.is_some())
            + u64::from(template.is_some())
            + u64::from(split.is_some()))
        .to_le_bytes(),
    );
    for (key, value) in std::iter::once(("general.architecture", architecture))
        .chain(template.map(|value| ("tokenizer.chat_template", value)))
    {
        string(&mut bytes, key);
        bytes.extend(8_u32.to_le_bytes());
        string(&mut bytes, value);
    }
    let context_key = format!("{architecture}.context_length");
    for (key, value) in std::iter::once(("general.file_type", 7_u32))
        .chain(context.map(|value| (context_key.as_str(), value)))
        .chain(split.map(|value| ("split.count", value)))
    {
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
        .unwrap()
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
    // Physical-size fixtures remain Unix-only; the shared byte counter has
    // allocation-free boundary tests on every platform.
    #[cfg(unix)]
    {
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
    let mixed = scan_directory(data.path(), source.path(), Some(&old), &control).unwrap();
    assert_eq!(mixed.library().unwrap().models.len(), 1);
    assert_eq!(control.progress().file_errors.len(), 1);
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

#[test]
fn mixed_content_rejections_are_bounded_and_only_valid_models_are_publishable() {
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let mut unknown_layout = gguf();
    let name = b"synthetic.weight";
    let descriptor = unknown_layout
        .windows(name.len())
        .position(|bytes| bytes == name)
        .unwrap();
    let kind = descriptor + name.len() + 4 + 8;
    unknown_layout[kind..kind + 4].copy_from_slice(&99_u32.to_le_bytes());
    let cases = [
        ("a 合法.gguf", gguf()),
        (
            "b 分片.gguf",
            fixture("qwen3", Some(40960), Some("template"), Some(2)),
        ),
        ("c 缺模板.gguf", fixture("qwen3", Some(40960), None, None)),
        ("d mmproj.gguf", fixture("clip", None, None, None)),
        ("e 损坏.gguf", b"invalid".to_vec()),
        ("f 空文件.gguf", Vec::new()),
        ("g 未知布局.gguf", unknown_layout),
    ];
    for (name, bytes) in &cases {
        fs::write(source.path().join(name), bytes).unwrap();
    }
    let control = ScanControl::default();
    let scanned = scan_directory(data.path(), source.path(), None, &control).unwrap();
    let library = scanned.library().unwrap();
    assert_eq!(library.models.len(), 1);
    assert_eq!(library.models[0].manifest.display_name, "a 合法");
    let progress = control.progress();
    assert_eq!(
        (
            progress.candidate_files,
            progress.verified_files,
            progress.file_errors.len()
        ),
        (7, 1, 6)
    );
    assert_eq!(
        progress
            .file_errors
            .iter()
            .map(|f| f.reason.as_str())
            .collect::<Vec<_>>(),
        [
            "unsupported_model",
            "unsupported_chat_template",
            "invalid_manifest",
            "invalid_manifest",
            "invalid_manifest",
            "invalid_manifest"
        ]
    );
    assert!(!data.path().join(LIBRARY_FILE).exists());
    for (name, bytes) in cases {
        assert_eq!(fs::read(source.path().join(name)).unwrap(), bytes);
    }
    assert_eq!(fs::read_dir(source.path()).unwrap().count(), 7);
}

#[test]
fn all_rejected_has_no_publishable_library_but_empty_directory_does() {
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let empty = scan_directory(data.path(), source.path(), None, &ScanControl::default()).unwrap();
    assert!(empty.library().unwrap().models.is_empty());
    drop(empty);
    fs::write(source.path().join("empty.gguf"), []).unwrap();
    let control = ScanControl::default();
    let rejected = scan_directory(data.path(), source.path(), None, &control).unwrap();
    assert!(rejected.library().is_none());
    assert_eq!(control.progress().file_errors.len(), 1);
    assert!(!data.path().join(LIBRARY_FILE).exists());
}

#[test]
fn automatic_short_context_does_not_clamp_explicit_imports() {
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    for context in [32, 1024, 2048, 40960] {
        fs::write(
            source.path().join(format!("{context}.gguf")),
            fixture("other_arch", Some(context), Some("template"), None),
        )
        .unwrap();
    }
    let library = scan(data.path(), source.path(), None);
    for entry in library.models {
        assert_eq!(
            entry.manifest.default_context,
            entry.manifest.context_limit.min(2048)
        );
        assert!(!entry.manifest.validated);
    }
    let managed = tempfile::tempdir().unwrap();
    let store = ModelStore::open(managed.path()).unwrap();
    let request = ImportRequest::new(
        ModelId::new("explicit").unwrap(),
        "explicit",
        ModelSource::local("test"),
    );
    let bytes = fixture("other_arch", Some(1024), Some("template"), None);
    assert_eq!(
        store
            .import_reader(
                Cursor::new(&bytes),
                bytes.len() as u64,
                request,
                &ImportCancellation::default()
            )
            .unwrap_err()
            .code,
        ErrorCode::InvalidManifest
    );
}

#[test]
fn rescan_binds_saved_directory_identity_while_apply_can_select_replacement() {
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let directory = source.path().join("selected");
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("good.gguf"), gguf()).unwrap();
    let previous = scan(data.path(), &directory, None);
    fs::rename(&directory, source.path().join("old")).unwrap();
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("good.gguf"), gguf()).unwrap();
    assert_eq!(
        model_store::library::rescan_directory(data.path(), &previous, &ScanControl::default())
            .err()
            .unwrap()
            .code,
        ErrorCode::ModelFileChanged
    );
    let replacement = scan(data.path(), &directory, Some(&previous));
    assert_ne!(replacement.directory_id, previous.directory_id);
}

#[test]
fn rejected_files_still_count_towards_candidate_and_byte_budgets() {
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    for index in 0..64 {
        fs::write(source.path().join(format!("{index}.gguf")), []).unwrap();
    }
    let control = ScanControl::default();
    let rejected = scan_directory(data.path(), source.path(), None, &control).unwrap();
    assert!(rejected.library().is_none());
    assert_eq!(control.progress().file_errors.len(), 64);
    drop(rejected);
    fs::write(source.path().join("65.gguf"), []).unwrap();
    assert_eq!(
        scan_directory(data.path(), source.path(), None, &ScanControl::default())
            .err()
            .unwrap()
            .code,
        ErrorCode::ModelLibraryLimit
    );
    // Unix set_len creates sparse fixtures. Do not introduce a 48 GiB physical
    // allocation on Windows filesystems without an explicit sparse-file API.
    #[cfg(unix)]
    {
        let large = tempfile::tempdir().unwrap();
        for index in 0..3 {
            fs::File::create(large.path().join(format!("{index}.gguf")))
                .unwrap()
                .set_len(MAX_MODEL_BYTES)
                .unwrap();
        }
        assert_eq!(
            scan_directory(data.path(), large.path(), None, &ScanControl::default())
                .err()
                .unwrap()
                .code,
            ErrorCode::ModelLibraryLimit
        );
    }
}

#[cfg(windows)]
#[test]
fn rejected_source_guard_survives_until_scan_result_is_dropped() {
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let path = source.path().join("bad.gguf");
    fs::write(&path, b"bad").unwrap();
    let scanned =
        scan_directory(data.path(), source.path(), None, &ScanControl::default()).unwrap();
    assert!(scanned.library().is_none());
    assert!(fs::OpenOptions::new().write(true).open(&path).is_err());
    assert!(fs::rename(&path, source.path().join("renamed.gguf")).is_err());
    drop(scanned);
    assert!(fs::OpenOptions::new().write(true).open(&path).is_ok());
}

#[test]
fn metadata_budget_failure_after_content_rejection_aborts_the_whole_scan() {
    let data = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    fs::write(source.path().join("a_bad.gguf"), b"bad").unwrap();
    fs::write(source.path().join("b_good.gguf"), gguf()).unwrap();
    let mut over_limit = b"GGUF".to_vec();
    over_limit.extend(3_u32.to_le_bytes());
    over_limit.extend(100_001_u64.to_le_bytes());
    over_limit.extend(0_u64.to_le_bytes());
    fs::write(source.path().join("c_limit.gguf"), over_limit).unwrap();
    let control = ScanControl::default();
    assert_eq!(
        scan_directory(data.path(), source.path(), None, &control)
            .err()
            .unwrap()
            .code,
        ErrorCode::ModelLibraryLimit
    );
    assert_eq!(control.progress().file_errors.len(), 1);
    assert_eq!(control.progress().verified_files, 1);
    assert!(!data.path().join(LIBRARY_FILE).exists());
}
