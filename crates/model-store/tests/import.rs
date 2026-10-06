//! Synthetic files verify storage and structural checks, never model inference.
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Cursor, Read};
use std::sync::{Arc, Barrier, mpsc};

use model_store::{ImportCancellation, ImportRequest, ModelSource, ModelStore};
use runtime_types::{ErrorCode, ModelId};
use serde_json::json;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

fn string(bytes: &mut Vec<u8>, value: &str) {
    bytes.extend((value.len() as u64).to_le_bytes());
    bytes.extend(value.as_bytes());
}
fn fixture(payload_bytes: usize) -> Vec<u8> {
    assert!(payload_bytes > 0 && payload_bytes.is_multiple_of(4));
    let mut bytes = b"GGUF".to_vec();
    bytes.extend(3_u32.to_le_bytes());
    bytes.extend(1_u64.to_le_bytes());
    bytes.extend(4_u64.to_le_bytes());
    for (key, value) in [
        ("general.architecture", "qwen3"),
        ("tokenizer.chat_template", "synthetic template\n中文"),
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
    bytes.extend((payload_bytes as u64 / 4).to_le_bytes());
    bytes.extend(0_u32.to_le_bytes()); // F32
    bytes.extend(0_u64.to_le_bytes());
    bytes.resize(bytes.len().next_multiple_of(32), 0);
    bytes.resize(bytes.len() + payload_bytes, 0);
    bytes
}
fn id(value: &str) -> ModelId {
    ModelId::new(value).unwrap()
}
fn request(value: &str) -> ImportRequest {
    ImportRequest::new(
        id(value),
        "Synthetic storage fixture",
        ModelSource::local("test-fixture"),
    )
}
fn store() -> (TempDir, ModelStore) {
    let temp = tempfile::tempdir().unwrap();
    let store = ModelStore::open(temp.path()).unwrap();
    (temp, store)
}
fn assert_clean(root: &TempDir) {
    assert_eq!(
        fs::read_dir(root.path().join("imports")).unwrap().count(),
        0
    );
}
fn import(
    store: &ModelStore,
    bytes: &[u8],
    name: &str,
) -> model_store::Result<model_store::ModelManifest> {
    store.import_reader(
        Cursor::new(bytes),
        bytes.len() as u64,
        request(name),
        &ImportCancellation::default(),
    )
}

#[test]
fn copies_hashes_registers_resolves_and_removes_only_managed_copy() {
    let (root, store) = store();
    let source_dir = tempfile::tempdir().unwrap();
    let source = source_dir.path().join("original.gguf");
    let bytes = fixture(128);
    fs::write(&source, &bytes).unwrap();
    let manifest = store
        .import_file(
            &source,
            request("tiny.test-1"),
            &ImportCancellation::default(),
        )
        .unwrap();
    assert_eq!(manifest.sha256, format!("{:x}", Sha256::digest(&bytes)));
    assert_eq!(manifest.source.uri, "test-fixture");
    assert_eq!(manifest.relative_file, "model.gguf");
    assert!(!manifest.validated);
    assert!(manifest.validated_llama_commit.is_none());
    assert!(!manifest.capabilities.chat);
    assert_eq!(store.list().unwrap(), vec![manifest.clone()]);
    assert_eq!(store.get(&id("tiny.test-1")).unwrap(), manifest);
    let resolved = store.resolve(&id("tiny.test-1")).unwrap();
    assert!(resolved.loadable);
    assert_eq!(fs::read(resolved.path).unwrap(), bytes);
    assert_eq!(store.remove(&id("tiny.test-1")).unwrap(), manifest);
    assert!(store.list().unwrap().is_empty());
    assert_eq!(fs::read(source).unwrap(), bytes);
    assert_clean(&root);
}

#[test]
fn strict_id_and_portable_paths_reject_traversal_and_device_names() {
    for invalid in [
        "", ".", "..", "../evil", "a/b", "a\\b", "a:b", "/root", "A", "中文", "a\0b",
    ] {
        assert!(ModelId::new(invalid).is_err(), "{invalid:?}");
    }
    assert!(ModelId::new("a".repeat(65)).is_err());
    let (root, store) = store();
    for name in ["con", "prn.txt", "aux", "nul", "com1", "lpt9.gguf", "end."] {
        assert_eq!(
            import(&store, &fixture(4), name).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
    }
    assert_clean(&root);
    assert!(ModelStore::open(root.path().join("../uncontrolled")).is_err());
}

#[test]
fn corrupt_inputs_and_every_truncation_never_register() {
    let (root, store) = store();
    let bytes = fixture(4);
    for end in 0..bytes.len() {
        assert!(
            import(&store, &bytes[..end], "truncated").is_err(),
            "end={end}"
        );
        assert_clean(&root);
    }
    for (offset, replacement) in [
        (0, 0_u64),
        (4, 9),
        (8, u64::MAX),
        (16, u64::MAX),
        (24, u64::MAX),
    ] {
        let mut corrupt = bytes.clone();
        corrupt[offset..offset + 8].copy_from_slice(&replacement.to_le_bytes());
        assert!(import(&store, &corrupt, "corrupt").is_err());
    }
    assert!(store.list().unwrap().is_empty());
    assert_clean(&root);
}

#[test]
fn verifies_declared_size_expected_hash_and_space_before_registering() {
    let (root, store) = store();
    let bytes = fixture(128);
    for declared in [bytes.len() as u64 - 1, bytes.len() as u64 + 1] {
        assert!(
            store
                .import_reader(
                    Cursor::new(&bytes),
                    declared,
                    request("bad-size"),
                    &ImportCancellation::default()
                )
                .is_err()
        );
    }
    let mut wrong = request("wrong-hash");
    wrong.expected_sha256 = Some("0".repeat(64));
    assert_eq!(
        store
            .import_reader(
                Cursor::new(&bytes),
                bytes.len() as u64,
                wrong,
                &ImportCancellation::default()
            )
            .unwrap_err()
            .code,
        ErrorCode::IntegrityFailure
    );
    assert_eq!(
        store
            .import_reader(
                Cursor::new(&bytes),
                model_store::library::MAX_MODEL_BYTES + 1,
                request("oversized-file"),
                &ImportCancellation::default()
            )
            .unwrap_err()
            .code,
        ErrorCode::ModelLibraryLimit
    );
    assert!(store.list().unwrap().is_empty());
    assert_clean(&root);
}

struct BrokenReader {
    bytes: Cursor<Vec<u8>>,
    reads: usize,
}
impl Read for BrokenReader {
    fn read(&mut self, target: &mut [u8]) -> io::Result<usize> {
        self.reads += 1;
        if self.reads > 1 {
            return Err(io::Error::other("synthetic read failure"));
        }
        self.bytes.read(&mut target[..32])
    }
}
struct CancellingReader {
    bytes: Cursor<Vec<u8>>,
    cancel: ImportCancellation,
}
impl Read for CancellingReader {
    fn read(&mut self, target: &mut [u8]) -> io::Result<usize> {
        let result = self.bytes.read(target);
        self.cancel.cancel();
        result
    }
}
#[test]
fn read_failure_and_cancellation_clean_partial_files() {
    let (root, store) = store();
    let bytes = fixture(128 * 1024);
    let error = store
        .import_reader(
            BrokenReader {
                bytes: Cursor::new(bytes.clone()),
                reads: 0,
            },
            bytes.len() as u64,
            request("read-fail"),
            &ImportCancellation::default(),
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Io);
    assert_clean(&root);
    let cancel = ImportCancellation::default();
    let error = store
        .import_reader(
            CancellingReader {
                bytes: Cursor::new(bytes.clone()),
                cancel: cancel.clone(),
            },
            bytes.len() as u64,
            request("cancelled"),
            &cancel,
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::RequestCancelled);
    assert_clean(&root);
    assert!(store.list().unwrap().is_empty());
    assert_eq!(
        store
            .import_reader(Cursor::new(bytes), 1, request("pre-cancelled"), &cancel)
            .unwrap_err()
            .code,
        ErrorCode::RequestCancelled
    );
}

#[test]
fn duplicate_and_concurrent_ids_are_never_overwritten() {
    let (root, store) = store();
    let store = Arc::new(store);
    let barrier = Arc::new(Barrier::new(3));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let store = store.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                import(&store, &fixture(128), "same")
            })
        })
        .collect();
    barrier.wait();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results.iter().find_map(|r| r.as_ref().err()).unwrap().code,
        ErrorCode::AlreadyExists
    );
    let original = fs::read(root.path().join("models/same/model.gguf")).unwrap();
    assert_eq!(
        import(&store, &fixture(256), "same").unwrap_err().code,
        ErrorCode::AlreadyExists
    );
    assert_eq!(
        fs::read(root.path().join("models/same/model.gguf")).unwrap(),
        original
    );
    assert_eq!(store.list().unwrap().len(), 1);
    assert_clean(&root);
}

struct PausedReader {
    bytes: Cursor<Vec<u8>>,
    started: Option<mpsc::Sender<()>>,
    resume: mpsc::Receiver<()>,
}
impl Read for PausedReader {
    fn read(&mut self, target: &mut [u8]) -> io::Result<usize> {
        if let Some(started) = self.started.take() {
            started.send(()).unwrap();
            self.resume.recv().unwrap();
        }
        self.bytes.read(target)
    }
}
#[test]
fn registration_is_invisible_until_both_files_are_complete() {
    let (root, store) = store();
    let store = Arc::new(store);
    let (started, waiting) = mpsc::channel();
    let (resume, resumed) = mpsc::channel();
    let worker = store.clone();
    let bytes = fixture(128);
    let task = std::thread::spawn(move || {
        worker.import_reader(
            PausedReader {
                bytes: Cursor::new(bytes.clone()),
                started: Some(started),
                resume: resumed,
            },
            bytes.len() as u64,
            request("atomic"),
            &ImportCancellation::default(),
        )
    });
    waiting.recv().unwrap();
    assert_eq!(
        store.resolve(&id("atomic")).unwrap_err().code,
        ErrorCode::RuntimeBusy
    );
    assert_eq!(fs::read_dir(root.path().join("models")).unwrap().count(), 0);
    assert_eq!(
        fs::read_dir(root.path().join("imports")).unwrap().count(),
        1
    );
    resume.send(()).unwrap();
    task.join().unwrap().unwrap();
    let manifest = store.get(&id("atomic")).unwrap();
    assert_eq!(
        fs::read(root.path().join("models/atomic/model.gguf"))
            .unwrap()
            .len() as u64,
        manifest.size_bytes
    );
    assert_clean(&root);
}

#[test]
fn a_destination_created_during_copy_is_not_overwritten() {
    let (root, store) = store();
    let store = Arc::new(store);
    let (started, waiting) = mpsc::channel();
    let (resume, resumed) = mpsc::channel();
    let worker = store.clone();
    let bytes = fixture(128);
    let task = std::thread::spawn(move || {
        worker.import_reader(
            PausedReader {
                bytes: Cursor::new(bytes.clone()),
                started: Some(started),
                resume: resumed,
            },
            bytes.len() as u64,
            request("race"),
            &ImportCancellation::default(),
        )
    });
    waiting.recv().unwrap();
    let destination = root.path().join("models/race");
    fs::create_dir(&destination).unwrap();
    fs::write(destination.join("user-file"), "untouched").unwrap();
    resume.send(()).unwrap();
    assert_eq!(
        task.join().unwrap().unwrap_err().code,
        ErrorCode::AlreadyExists
    );
    assert_eq!(
        fs::read_to_string(destination.join("user-file")).unwrap(),
        "untouched"
    );
    assert_clean(&root);
}

#[test]
fn process_lock_is_exclusive_and_reopen_recovers_owned_temporaries_only() {
    let (root, store) = store();
    assert!(
        matches!(ModelStore::open(root.path()), Err(error) if error.code == ErrorCode::RuntimeBusy)
    );
    import(&store, &fixture(4), "registered").unwrap();
    let imports = root.path().join("imports");
    fs::write(
        imports.join("import-0123456789abcdef0123456789abcdef.partial"),
        "half",
    )
    .unwrap();
    let staged = imports.join("import-fedcba9876543210fedcba9876543210.staged");
    fs::create_dir(&staged).unwrap();
    fs::write(staged.join("model.gguf"), "half").unwrap();
    fs::write(imports.join("user.partial"), "preserve").unwrap();
    drop(store);
    let reopened = ModelStore::open(root.path()).unwrap();
    assert_eq!(reopened.list().unwrap().len(), 1);
    assert_eq!(fs::read_dir(imports).unwrap().count(), 1);
}

#[test]
fn missing_and_unregistered_directories_cannot_be_removed() {
    let (root, store) = store();
    assert_eq!(
        store.remove(&id("missing")).unwrap_err().code,
        ErrorCode::ModelNotFound
    );
    fs::create_dir(root.path().join("models/unregistered")).unwrap();
    fs::write(root.path().join("models/unregistered/precious"), "preserve").unwrap();
    assert!(store.remove(&id("unregistered")).is_err());
    assert!(root.path().join("models/unregistered/precious").is_file());
}

#[test]
fn unknown_manifest_fields_survive_but_claims_paths_and_tampering_fail_closed() {
    let (root, store) = store();
    let bytes = fixture(128);
    let mut request = request("extended");
    request.extra = BTreeMap::from([("application".into(), json!({"unknown": [1, 2, 3]}))]);
    let original = store
        .import_reader(
            Cursor::new(&bytes),
            bytes.len() as u64,
            request,
            &ImportCancellation::default(),
        )
        .unwrap();
    assert_eq!(store.get(&id("extended")).unwrap().extra, original.extra);
    let manifest_path = root.path().join("models/extended/manifest.json");
    let original_json: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    for (key, value) in [
        ("relative_file", json!("../../user.gguf")),
        ("validated", json!(true)),
        ("loadable", json!(true)),
        (
            "validated_llama_commit",
            json!("2149c00f4442dc59302e134a02e4c99d5f7ed9fc"),
        ),
        (
            "capabilities",
            json!({"chat":true,"streaming":false,"cancellation":false}),
        ),
    ] {
        let mut hostile = original_json.clone();
        hostile[key] = value;
        fs::write(&manifest_path, serde_json::to_vec(&hostile).unwrap()).unwrap();
        assert!(store.resolve(&id("extended")).is_err());
    }
    fs::write(&manifest_path, serde_json::to_vec(&original_json).unwrap()).unwrap();
    let mut altered = bytes;
    let last = altered.len() - 1;
    altered[last] ^= 1;
    fs::write(root.path().join("models/extended/model.gguf"), altered).unwrap();
    assert_eq!(
        store.resolve(&id("extended")).unwrap_err().code,
        ErrorCode::IntegrityFailure
    );
}

#[cfg(unix)]
#[test]
fn symlink_escapes_are_rejected_and_external_targets_are_preserved() {
    use std::os::unix::fs::symlink;
    let (root, store) = store();
    let external = tempfile::tempdir().unwrap();
    fs::write(external.path().join("precious"), "preserve").unwrap();
    symlink(external.path(), root.path().join("models/escape")).unwrap();
    assert!(store.resolve(&id("escape")).is_err());
    assert!(store.remove(&id("escape")).is_err());
    assert_eq!(
        import(&store, &fixture(4), "escape").unwrap_err().code,
        ErrorCode::AlreadyExists
    );
    assert_eq!(
        fs::read_to_string(external.path().join("precious")).unwrap(),
        "preserve"
    );
    fs::remove_file(root.path().join("models/escape")).unwrap();
    import(&store, &fixture(4), "safe").unwrap();
    fs::remove_file(root.path().join("models/safe/model.gguf")).unwrap();
    symlink(
        external.path().join("precious"),
        root.path().join("models/safe/model.gguf"),
    )
    .unwrap();
    assert!(store.resolve(&id("safe")).is_err());
    assert!(store.remove(&id("safe")).is_err());
    let alias = root.path().join("root-alias");
    symlink(external.path(), &alias).unwrap();
    assert!(ModelStore::open(alias).is_err());
}

#[test]
fn diagnostics_do_not_include_original_paths() {
    let (_root, store) = store();
    let private = "/private/user/name/secret.gguf";
    let error = store
        .import_file(private, request("absent"), &ImportCancellation::default())
        .unwrap_err();
    assert!(!error.to_string().contains(private));
}

#[test]
fn lock_probe_child() {
    let Some(path) = std::env::var_os("NEXA_STORE_LOCK_TEST_DIR") else {
        return;
    };
    assert!(matches!(ModelStore::open(path), Err(error) if error.code == ErrorCode::RuntimeBusy));
}

#[test]
fn a_second_process_cannot_open_the_locked_registry() {
    let (root, _store) = store();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "lock_probe_child", "--nocapture"])
        .env("NEXA_STORE_LOCK_TEST_DIR", root.path())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn explicit_reverification_and_reopen_detect_corruption() {
    let (root, store) = store();
    let bytes = fixture(128);
    import(&store, &bytes, "verify").unwrap();
    assert!(store.verify(&id("verify")).is_ok());
    let mut corrupt = bytes.clone();
    let last = corrupt.len() - 1;
    corrupt[last] ^= 1;
    fs::write(root.path().join("models/verify/model.gguf"), &corrupt).unwrap();
    assert_eq!(
        store.verify(&id("verify")).unwrap_err().code,
        ErrorCode::IntegrityFailure
    );
    assert_eq!(
        store.resolve(&id("verify")).unwrap_err().code,
        ErrorCode::IntegrityFailure
    );
    drop(store);
    match ModelStore::open(root.path()) {
        Err(error) => assert_eq!(error.code, ErrorCode::IntegrityFailure, "{error}"),
        Ok(_) => panic!("corrupted registered model unexpectedly reopened"),
    }
    fs::write(root.path().join("models/verify/model.gguf"), bytes).unwrap();
    let repaired = ModelStore::open(root.path()).unwrap();
    assert!(repaired.resolve(&id("verify")).is_ok());
}

#[test]
fn failed_file_import_keeps_user_source_unchanged() {
    let (root, store) = store();
    let source = tempfile::tempdir().unwrap();
    let path = source.path().join("invalid.gguf");
    fs::write(&path, b"GGUFcorrupt but user-owned").unwrap();
    assert!(
        store
            .import_file(&path, request("invalid"), &ImportCancellation::default())
            .is_err()
    );
    assert_eq!(fs::read(path).unwrap(), b"GGUFcorrupt but user-owned");
    assert_clean(&root);
}

#[cfg(unix)]
#[test]
fn source_symlinks_and_special_files_are_refused_without_modifying_source() {
    use std::os::unix::fs::symlink;
    let (root, store) = store();
    let sources = tempfile::tempdir().unwrap();
    let regular = sources.path().join("source.gguf");
    let bytes = fixture(64);
    fs::write(&regular, &bytes).unwrap();
    let alias = sources.path().join("alias.gguf");
    symlink(&regular, &alias).unwrap();
    assert!(
        store
            .import_file(&alias, request("alias"), &ImportCancellation::default())
            .is_err()
    );
    let fifo = sources.path().join("input.pipe");
    let cpath = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
    // SAFETY: cpath is NUL-terminated and owned for this synchronous syscall.
    assert_eq!(unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) }, 0);
    let before = std::time::Instant::now();
    assert!(
        store
            .import_file(&fifo, request("pipe"), &ImportCancellation::default())
            .is_err()
    );
    assert!(before.elapsed() < std::time::Duration::from_secs(1));
    assert_eq!(fs::read(&regular).unwrap(), bytes);
    assert_clean(&root);
}

#[cfg(windows)]
#[test]
fn dos_device_and_network_source_names_are_rejected_before_open_or_read() {
    let (root, store) = store();
    for source in [
        r"NUL",
        r"C:\NUL",
        r"C:\NUL.gguf",
        r"C:\COM1",
        r"C:\CONIN$",
        r"\\.\pipe\nexa-unopened-fixture",
        r"\\?\GLOBALROOT\Device\Null",
        r"\\server\share\model.gguf",
    ] {
        let error = store
            .import_file(source, request("device"), &ImportCancellation::default())
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidArgument);
    }
    assert!(store.list().unwrap().is_empty());
    assert_clean(&root);
}

#[test]
fn offline_inventory_neither_locks_recovers_nor_hashes_model_payloads() {
    let (root, store) = store();
    let bytes = fixture(128);
    import(&store, &bytes, "offline").unwrap();
    // Holding the store lock does not stop the read-only view. An import debris
    // file must remain untouched, and invalid payload bytes are not inspected.
    let leftover = root.path().join("imports/import-debris.partial");
    fs::write(&leftover, b"keep").unwrap();
    let path = root.path().join("models/offline/model.gguf");
    fs::write(&path, vec![b'x'; bytes.len()]).unwrap();
    let inventory = model_store::inventory::read(root.path()).unwrap();
    assert_eq!(inventory.entries.len(), 1);
    assert!(inventory.entries[0].availability_error.is_none());
    assert_eq!(fs::read(&leftover).unwrap(), b"keep");
    assert!(!inventory.entries[0].manifest.validated);
    assert!(store.resolve(&id("offline")).is_err());
    let absent = root.path().join("does-not-exist");
    assert!(
        model_store::inventory::read(&absent)
            .unwrap()
            .entries
            .is_empty()
    );
    assert!(!absent.exists());
}
#[test]
fn offline_inventory_generation_tracks_metadata_and_refuses_symlinks() {
    let (root, store) = store();
    let bytes = fixture(128);
    import(&store, &bytes, "offline").unwrap();
    let before = model_store::inventory::read(root.path())
        .unwrap()
        .generation;
    let path = root.path().join("models/offline/model.gguf");
    fs::write(&path, b"short").unwrap();
    let after = model_store::inventory::read(root.path()).unwrap();
    assert_ne!(before, after.generation);
    assert_eq!(
        after.entries[0].availability_error,
        Some(ErrorCode::ModelFileChanged)
    );
    #[cfg(unix)]
    {
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink("/dev/zero", &path).unwrap();
        assert!(
            model_store::inventory::read(root.path()).unwrap().entries[0]
                .availability_error
                .is_some()
        );
        fs::remove_file(root.path().join("models/offline/manifest.json")).unwrap();
        std::os::unix::fs::symlink(
            "/dev/zero",
            root.path().join("models/offline/manifest.json"),
        )
        .unwrap();
        assert!(model_store::inventory::read(root.path()).is_err());
    }
}

#[cfg(windows)]
#[test]
fn canonical_windows_store_inventory_no_model_regression() {
    let (_root, store) = store();
    assert!(matches!(store.data_directory().components().next(),
        Some(std::path::Component::Prefix(prefix)) if matches!(prefix.kind(), std::path::Prefix::VerbatimDisk(_))));
    assert!(
        model_store::inventory::read(store.data_directory())
            .unwrap()
            .entries
            .is_empty()
    );
}

#[test]
fn canonical_store_inventory_evidence_roundtrip_replace_offline_and_stale() {
    use model_store::{
        inventory,
        local_validation::{self, LocalValidation, Receipt, Receipts, ValidationState},
    };
    let (root, store) = store();
    let manifest = import(&store, &fixture(128), "evidence-fixture").unwrap();
    let canonical = store.data_directory().to_owned();
    let catalog = inventory::read(&canonical).unwrap();
    assert_eq!(catalog.entries.len(), 1);
    assert_eq!(catalog.entries[0].manifest, manifest);
    let options = runtime_types::LoadOptions::default();
    let scope = local_validation::scope(&canonical, &catalog.entries[0], options, "fixture-engine")
        .unwrap();
    let ordinary = inventory::read(root.path()).unwrap();
    assert_eq!(
        scope,
        local_validation::scope(root.path(), &ordinary.entries[0], options, "fixture-engine")
            .unwrap()
    );
    let mut receipts = Receipts::default();
    // Synthetic storage observations, not a real inference claim.
    for (state, generation_pass) in [
        (ValidationState::Loaded, false),
        (ValidationState::Passed, true),
    ] {
        receipts
            .record(
                &canonical,
                Receipt {
                    scope: scope.clone(),
                    observation: LocalValidation {
                        state: state.clone(),
                        checked_at_unix_ms: Some(local_validation::now_ms()),
                        error_code: None,
                        load_success: true,
                        generation_pass,
                    },
                },
            )
            .unwrap();
        let read = Receipts::read(root.path()).unwrap();
        assert_eq!(read.schema_version, 1);
        assert_eq!(read.entries.len(), 1);
        assert_eq!(read.observation(&scope, true).state, state);
    }
    drop(store);
    let offline = inventory::read(&canonical).unwrap();
    let actual =
        local_validation::scope(&canonical, &offline.entries[0], options, "fixture-engine")
            .unwrap();
    assert_eq!(actual, scope);
    let receipts = Receipts::read(&canonical).unwrap();
    assert_eq!(
        receipts.observation(&actual, true).state,
        ValidationState::Passed
    );
    let reopened = ModelStore::open(root.path()).unwrap();
    assert_eq!(
        inventory::read(reopened.data_directory())
            .unwrap()
            .generation,
        offline.generation
    );
    for changed in [
        local_validation::scope(
            &canonical,
            &offline.entries[0],
            runtime_types::LoadOptions {
                threads: options.threads + 1,
                ..options
            },
            "fixture-engine",
        )
        .unwrap(),
        local_validation::scope(&canonical, &offline.entries[0], options, "different-engine")
            .unwrap(),
    ] {
        assert_eq!(
            receipts.observation(&changed, true).state,
            ValidationState::Stale
        );
    }
    fs::write(
        canonical.join("models/evidence-fixture/model.gguf"),
        fixture(132),
    )
    .unwrap();
    let changed = inventory::read(&canonical).unwrap();
    assert_eq!(
        changed.entries[0].availability_error,
        Some(ErrorCode::ModelFileChanged)
    );
    assert_eq!(
        receipts.observation(&actual, false).state,
        ValidationState::Stale
    );
}

#[test]
fn unregister_preserves_managed_bytes_metadata_and_restart_then_explicit_selected_restore() {
    use model_store::{
        inventory,
        library::{
            self, ScanControl,
            selected::{SelectedFile, register_selected},
        },
    };
    let (root, store) = store();
    let bytes = fixture(128);
    import(&store, &bytes, "keep-file").unwrap();
    import(&store, &bytes, "other").unwrap();
    let payload = root.path().join("models/keep-file/model.gguf");
    let manifest = root.path().join("models/keep-file/manifest.json");
    let original_manifest = fs::read(&manifest).unwrap();
    let before = inventory::read(root.path()).unwrap().generation;
    fs::write(root.path().join("config.toml"), "sentinel config").unwrap();
    fs::write(
        root.path().join("local-validation.json"),
        "sentinel receipt",
    )
    .unwrap();
    store.unregister(&id("keep-file")).unwrap();
    assert_eq!(
        store.get(&id("keep-file")).unwrap_err().code,
        ErrorCode::ModelNotFound
    );
    assert_eq!(
        store.resolve(&id("keep-file")).unwrap_err().code,
        ErrorCode::ModelNotFound
    );
    assert_eq!(
        store.verify(&id("keep-file")).unwrap_err().code,
        ErrorCode::ModelNotFound
    );
    assert_eq!(
        store.unregister(&id("keep-file")).unwrap_err().code,
        ErrorCode::ModelNotFound
    );
    assert_eq!(store.list().unwrap().len(), 1);
    assert_eq!(fs::read(&payload).unwrap(), bytes);
    assert_eq!(fs::read(&manifest).unwrap(), original_manifest);
    assert_eq!(
        fs::read_to_string(root.path().join("config.toml")).unwrap(),
        "sentinel config"
    );
    assert_eq!(
        fs::read_to_string(root.path().join("local-validation.json")).unwrap(),
        "sentinel receipt"
    );
    assert_ne!(inventory::read(root.path()).unwrap().generation, before);
    assert!(model_store::unregister::CatalogLock::acquire(root.path()).is_err());
    drop(store);
    let reopened = ModelStore::open(root.path()).unwrap();
    assert_eq!(reopened.list().unwrap().len(), 1);
    drop(reopened);
    let prior = library::ModelLibrary::read(root.path()).unwrap().unwrap();
    let results = std::sync::Mutex::new(Vec::new());
    let selected = register_selected(
        root.path(),
        Some(&prior),
        vec![SelectedFile::open(&payload).unwrap()],
        &ScanControl::default(),
        &results,
    )
    .unwrap();
    assert_eq!(selected.files[0].model_id, Some(id("keep-file")));
    assert!(
        selected.library.models.is_empty(),
        "must not create an external alias for a managed copy"
    );
    assert!(selected.library.unregistered.is_empty());
    selected.check().unwrap();
    model_store::unregister::publish(root.path(), &selected.library).unwrap();
    drop(selected);
    assert_eq!(inventory::read(root.path()).unwrap().entries.len(), 2);
    assert_eq!(fs::read(&payload).unwrap(), bytes);
    assert_eq!(
        ModelStore::open(root.path()).unwrap().list().unwrap().len(),
        2
    );
}

#[test]
fn offline_unregister_never_hashes_recovers_or_deletes_models() {
    let (root, store) = store();
    import(&store, &fixture(128), "broken").unwrap();
    drop(store);
    // Invalid payload would fail a ModelStore::open full hash, but removal is
    // metadata-only even when the source is unavailable or damaged.
    fs::write(root.path().join("models/broken/model.gguf"), b"changed").unwrap();
    fs::write(
        root.path()
            .join("imports/import-11111111111111111111111111111111.partial"),
        b"keep",
    )
    .unwrap();
    let _guard = model_store::unregister::CatalogLock::acquire(root.path()).unwrap();
    let (next, _) = model_store::unregister::prepare(root.path(), &id("broken")).unwrap();
    model_store::unregister::publish(root.path(), &next).unwrap();
    assert!(
        model_store::inventory::read(root.path())
            .unwrap()
            .entries
            .is_empty()
    );
    assert_eq!(
        fs::read(root.path().join("models/broken/model.gguf")).unwrap(),
        b"changed"
    );
    assert_eq!(
        fs::read_dir(root.path().join("imports")).unwrap().count(),
        1
    );
}
