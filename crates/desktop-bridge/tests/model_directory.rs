use desktop_bridge::{
    DesktopBridge, LibraryOperationState, LibraryOperationStatus, ModelDirectoryState,
};
use model_store::library::{LIBRARY_FILE, ModelLibrary};
use runtime_cli::instance::InstanceLock;
use std::{
    fs,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use uuid::Uuid;
fn bridge(root: &Path) -> Arc<DesktopBridge> {
    Arc::new(
        DesktopBridge::new(
            root.to_owned(),
            root.with_file_name(if cfg!(windows) {
                "ai-runtime.exe"
            } else {
                "ai-runtime"
            }),
        )
        .unwrap(),
    )
}
async fn terminal(bridge: &DesktopBridge, id: Uuid) -> LibraryOperationState {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let state = bridge.library_next(id).await.unwrap();
            if state.terminal {
                return state;
            }
        }
    })
    .await
    .unwrap()
}
fn string(bytes: &mut Vec<u8>, value: &str) {
    bytes.extend((value.len() as u64).to_le_bytes());
    bytes.extend(value.as_bytes());
}
fn tiny_model(path: &Path) {
    let mut b = b"GGUF".to_vec();
    b.extend(3_u32.to_le_bytes());
    b.extend(1_u64.to_le_bytes());
    b.extend(4_u64.to_le_bytes());
    for (k, v) in [
        ("general.architecture", "qwen3"),
        ("tokenizer.chat_template", "test template"),
    ] {
        string(&mut b, k);
        b.extend(8_u32.to_le_bytes());
        string(&mut b, v);
    }
    for (k, v) in [
        ("general.file_type", 7_u32),
        ("qwen3.context_length", 40960),
    ] {
        string(&mut b, k);
        b.extend(4_u32.to_le_bytes());
        b.extend(v.to_le_bytes());
    }
    string(&mut b, "w");
    b.extend(1_u32.to_le_bytes());
    b.extend(32_u64.to_le_bytes());
    b.extend(0_u32.to_le_bytes());
    b.extend(0_u64.to_le_bytes());
    b.resize(b.len().next_multiple_of(32), 0);
    b.resize(b.len() + 128, 0);
    fs::write(path, b).unwrap();
}
#[tokio::test]
async fn selection_applies_without_copy_or_token_initialization_and_rescans_stably() {
    let temp = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let root = temp.path().join("private");
    let bridge = bridge(&root);
    let source_file = source.path().join("中文 模型.gguf");
    tiny_model(&source_file);
    let original = fs::read(&source_file).unwrap();
    let operation = bridge.directory_apply(source.path().to_owned()).unwrap();
    assert_eq!(bridge.library_active(), Some(operation.operation_id));
    assert_eq!(bridge.models_scan().unwrap_err().code, "desktop_busy");
    let result = terminal(&bridge, operation.operation_id).await;
    assert_eq!(result.status, LibraryOperationStatus::Completed);
    let first = ModelLibrary::read(&root).unwrap().unwrap();
    assert_eq!(first.models[0].manifest.display_name, "中文 模型");
    assert!(!root.join("secrets/api-token").exists());
    assert!(!root.join("models").exists());
    assert_eq!(fs::read(&source_file).unwrap(), original);
    assert_eq!(fs::read_dir(source.path()).unwrap().count(), 1);
    let snapshot = bridge.snapshot().await.unwrap();
    assert!(!snapshot.initialized);
    assert_eq!(snapshot.model_directory.state, ModelDirectoryState::Stopped);
    let scanned = bridge.models_scan().unwrap();
    assert_eq!(
        terminal(&bridge, scanned.operation_id).await.status,
        LibraryOperationStatus::Completed
    );
    let second = ModelLibrary::read(&root).unwrap().unwrap();
    assert_eq!(first.models[0].manifest.id, second.models[0].manifest.id);
    assert_ne!(first.library_generation, second.library_generation);
    bridge.library_cancel(operation.operation_id).await.unwrap();
    assert_eq!(
        bridge
            .library_next(operation.operation_id)
            .await
            .unwrap()
            .status,
        LibraryOperationStatus::Completed
    );
}
#[tokio::test]
async fn running_instance_preserves_config_and_mixed_scan_commits_valid_subset() {
    let temp = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let root = temp.path().join("private");
    let bridge = bridge(&root);
    tiny_model(&source.path().join("good.gguf"));
    let id = bridge
        .directory_apply(source.path().to_owned())
        .unwrap()
        .operation_id;
    terminal(&bridge, id).await;
    let original = fs::read(root.join(LIBRARY_FILE)).unwrap();
    let lock = InstanceLock::try_acquire(&root).unwrap().unwrap();
    let id = bridge.models_scan().unwrap().operation_id;
    let result = terminal(&bridge, id).await;
    assert_eq!(result.error.unwrap().code, "runtime_running");
    assert_eq!(fs::read(root.join(LIBRARY_FILE)).unwrap(), original);
    drop(lock);
    fs::write(source.path().join("坏 文件.gguf"), b"invalid").unwrap();
    let id = bridge.models_scan().unwrap().operation_id;
    let result = terminal(&bridge, id).await;
    assert_eq!(result.status, LibraryOperationStatus::Partial);
    assert_eq!(result.file_errors[0].file_name, "坏 文件.gguf");
    assert_eq!(result.file_errors[0].code, "invalid_manifest");
    assert!(result.failed_file_name.is_none());
    assert_eq!(result.result.as_ref().unwrap().registered_files, 1);
    assert_eq!(result.result.as_ref().unwrap().rejected_files, 1);
    assert_ne!(fs::read(root.join(LIBRARY_FILE)).unwrap(), original);
    let repeated = bridge.library_next(id).await.unwrap();
    assert_eq!(
        serde_json::to_value(repeated).unwrap(),
        serde_json::to_value(result).unwrap()
    );
}
#[tokio::test]
async fn early_cancel_and_close_finish_without_hidden_publication() {
    let temp = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let root = temp.path().join("private");
    let bridge = bridge(&root);
    tiny_model(&source.path().join("model.gguf"));
    let id = bridge
        .directory_apply(source.path().to_owned())
        .unwrap()
        .operation_id;
    bridge.library_cancel(id).await.unwrap();
    let result = terminal(&bridge, id).await;
    assert_eq!(result.status, LibraryOperationStatus::Cancelled);
    assert!(!root.join(LIBRARY_FILE).exists());
    let id = bridge
        .directory_apply(source.path().to_owned())
        .unwrap()
        .operation_id;
    bridge.close_ui_only().await.unwrap();
    assert_eq!(
        bridge.library_next(id).await.unwrap().status,
        LibraryOperationStatus::Cancelled
    );
    assert!(!root.join(LIBRARY_FILE).exists());
}
#[tokio::test]
async fn native_policy_is_called_for_real_apply_and_saved_rescan_paths() {
    let temp = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let root = temp.path().join("private");
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let expected = source.path().to_owned();
    let bridge = Arc::new(
        DesktopBridge::new(
            root.clone(),
            root.with_file_name(if cfg!(windows) {
                "ai-runtime.exe"
            } else {
                "ai-runtime"
            }),
        )
        .unwrap()
        .with_directory_validator(move |path| {
            assert_eq!(path, expected);
            c.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }),
    );
    let id = bridge
        .directory_apply(source.path().to_owned())
        .unwrap()
        .operation_id;
    assert_eq!(
        terminal(&bridge, id).await.status,
        LibraryOperationStatus::Completed
    );
    let id = bridge.models_scan().unwrap().operation_id;
    assert_eq!(
        terminal(&bridge, id).await.status,
        LibraryOperationStatus::Completed
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}
#[tokio::test]
async fn selected_directory_missing_is_not_shown_as_ready() {
    let temp = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let root = temp.path().join("private");
    let bridge = bridge(&root);
    let path = source.path().join("models");
    fs::create_dir(&path).unwrap();
    let id = bridge.directory_apply(path.clone()).unwrap().operation_id;
    terminal(&bridge, id).await;
    fs::remove_dir(&path).unwrap();
    let snapshot = bridge.snapshot().await.unwrap();
    assert_eq!(snapshot.model_directory.state, ModelDirectoryState::Missing);
    assert!(snapshot.model_directory.configured.is_some());
}
#[tokio::test]
async fn foreign_operation_is_rejected_without_cancelling_owned_work() {
    let temp = tempfile::tempdir().unwrap();
    let bridge = bridge(&temp.path().join("private"));
    assert_eq!(
        bridge
            .library_cancel(Uuid::new_v4())
            .await
            .unwrap_err()
            .code,
        "request_not_owned"
    );
    assert_eq!(
        bridge.library_next(Uuid::new_v4()).await.unwrap_err().code,
        "request_not_owned"
    );
    let id = bridge.models_scan().unwrap().operation_id;
    let result = terminal(&bridge, id).await;
    assert_eq!(result.error.unwrap().code, "model_directory_required");
}

#[tokio::test]
async fn all_rejected_keeps_old_index_and_empty_directory_can_replace_it() {
    let temp = tempfile::tempdir().unwrap();
    let original_source = tempfile::tempdir().unwrap();
    let rejected_source = tempfile::tempdir().unwrap();
    let empty_source = tempfile::tempdir().unwrap();
    let root = temp.path().join("private");
    let bridge = bridge(&root);
    tiny_model(&original_source.path().join("old.gguf"));
    let id = bridge
        .directory_apply(original_source.path().to_owned())
        .unwrap()
        .operation_id;
    assert_eq!(
        terminal(&bridge, id).await.status,
        LibraryOperationStatus::Completed
    );
    let original = fs::read(root.join(LIBRARY_FILE)).unwrap();
    fs::write(rejected_source.path().join("坏 中文.gguf"), []).unwrap();
    fs::write(rejected_source.path().join("bad.gguf"), b"bad").unwrap();
    let id = bridge
        .directory_apply(rejected_source.path().to_owned())
        .unwrap()
        .operation_id;
    let rejected = terminal(&bridge, id).await;
    assert_eq!(rejected.status, LibraryOperationStatus::Failed);
    assert_eq!(
        rejected.error.as_ref().unwrap().code,
        "model_scan_no_usable_files"
    );
    assert_eq!(rejected.file_errors.len(), 2);
    assert!(rejected.result.is_none());
    assert_eq!(fs::read(root.join(LIBRARY_FILE)).unwrap(), original);
    let id = bridge
        .directory_apply(empty_source.path().to_owned())
        .unwrap()
        .operation_id;
    let empty = terminal(&bridge, id).await;
    assert_eq!(empty.status, LibraryOperationStatus::Completed);
    assert_eq!(empty.result.unwrap().registered_files, 0);
    assert!(empty.file_errors.is_empty());
    assert_ne!(fs::read(root.join(LIBRARY_FILE)).unwrap(), original);
}

#[tokio::test]
async fn mixed_rescans_preserve_valid_ids_and_repair_rejected_files() {
    let temp = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let root = temp.path().join("private");
    let bridge = bridge(&root);
    tiny_model(&source.path().join("good.gguf"));
    fs::write(source.path().join("repair.gguf"), b"bad").unwrap();
    let id = bridge
        .directory_apply(source.path().to_owned())
        .unwrap()
        .operation_id;
    assert_eq!(
        terminal(&bridge, id).await.status,
        LibraryOperationStatus::Partial
    );
    let first = ModelLibrary::read(&root).unwrap().unwrap();
    let id = bridge.models_scan().unwrap().operation_id;
    assert_eq!(
        terminal(&bridge, id).await.status,
        LibraryOperationStatus::Partial
    );
    let second = ModelLibrary::read(&root).unwrap().unwrap();
    assert_eq!(first.models[0].manifest.id, second.models[0].manifest.id);
    assert_ne!(first.library_generation, second.library_generation);
    tiny_model(&source.path().join("repair.gguf"));
    let id = bridge.models_scan().unwrap().operation_id;
    assert_eq!(
        terminal(&bridge, id).await.status,
        LibraryOperationStatus::Completed
    );
    let repaired = ModelLibrary::read(&root).unwrap().unwrap();
    assert_eq!(repaired.models.len(), 2);
    assert_eq!(repaired.models[0].manifest.id, first.models[0].manifest.id);
    fs::write(source.path().join("good.gguf"), b"now bad").unwrap();
    let id = bridge.models_scan().unwrap().operation_id;
    assert_eq!(
        terminal(&bridge, id).await.status,
        LibraryOperationStatus::Partial
    );
    let last = ModelLibrary::read(&root).unwrap().unwrap();
    assert_eq!(last.models.len(), 1);
    assert_eq!(last.models[0].manifest.relative_file, "repair.gguf");
}

#[tokio::test]
async fn replaced_saved_directory_requires_explicit_apply_and_bounds_still_roll_back() {
    let temp = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let root = temp.path().join("private");
    let directory = source.path().join("selected");
    fs::create_dir(&directory).unwrap();
    tiny_model(&directory.join("good.gguf"));
    let bridge = bridge(&root);
    let id = bridge
        .directory_apply(directory.clone())
        .unwrap()
        .operation_id;
    terminal(&bridge, id).await;
    let original = fs::read(root.join(LIBRARY_FILE)).unwrap();
    fs::rename(&directory, source.path().join("old")).unwrap();
    fs::create_dir(&directory).unwrap();
    tiny_model(&directory.join("new.gguf"));
    let id = bridge.models_scan().unwrap().operation_id;
    assert_eq!(
        terminal(&bridge, id).await.error.unwrap().code,
        "model_file_changed"
    );
    assert_eq!(fs::read(root.join(LIBRARY_FILE)).unwrap(), original);
    let id = bridge
        .directory_apply(directory.clone())
        .unwrap()
        .operation_id;
    assert_eq!(
        terminal(&bridge, id).await.status,
        LibraryOperationStatus::Completed
    );
    let replaced = fs::read(root.join(LIBRARY_FILE)).unwrap();
    for index in 0..64 {
        fs::write(directory.join(format!("bad-{index}.gguf")), []).unwrap();
    }
    let id = bridge.models_scan().unwrap().operation_id;
    let bounded = terminal(&bridge, id).await;
    assert_eq!(bounded.status, LibraryOperationStatus::Failed);
    assert_eq!(bounded.error.unwrap().code, "model_library_limit");
    assert_eq!(fs::read(root.join(LIBRARY_FILE)).unwrap(), replaced);
}

#[tokio::test]
async fn startup_discovery_registers_existing_models_and_preserves_configured_directory() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("private");
    let models = temp.path().join("models");
    fs::create_dir(&models).unwrap();
    tiny_model(&models.join("local.gguf"));
    let bridge = Arc::new(
        DesktopBridge::new(
            root.clone(),
            temp.path().join(if cfg!(windows) {
                "ai-runtime.exe"
            } else {
                "ai-runtime"
            }),
        )
        .unwrap()
        .with_default_model_directory(models.clone()),
    );
    let handle = bridge.directory_discover().unwrap().unwrap();
    assert_eq!(
        terminal(&bridge, handle.operation_id).await.status,
        LibraryOperationStatus::Completed
    );
    let library = ModelLibrary::read(&root).unwrap().unwrap();
    assert_eq!(library.models.len(), 1);
    // Windows may preserve the selected DOS/8.3 spelling in the library while
    // canonicalization returns a verbatim long path. Compare both normalized
    // filesystem paths rather than treating display spelling as identity.
    assert_eq!(
        fs::canonicalize(&library.directory).unwrap(),
        fs::canonicalize(&models).unwrap()
    );
    assert!(bridge.directory_discover().unwrap().is_none());
    let saved = fs::read(root.join(LIBRARY_FILE)).unwrap();
    fs::rename(&models, temp.path().join("missing-models")).unwrap();
    assert!(bridge.directory_discover().unwrap().is_none());
    assert_eq!(fs::read(root.join(LIBRARY_FILE)).unwrap(), saved);
}

#[tokio::test]
async fn startup_discovery_does_not_create_absent_folder_or_replace_running_instance() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("private");
    let models = temp.path().join("models");
    let bridge = Arc::new(
        DesktopBridge::new(
            root.clone(),
            temp.path().join(if cfg!(windows) {
                "ai-runtime.exe"
            } else {
                "ai-runtime"
            }),
        )
        .unwrap()
        .with_default_model_directory(models.clone()),
    );
    assert!(bridge.directory_discover().unwrap().is_none());
    assert!(!models.exists());
    fs::create_dir(&models).unwrap();
    runtime_api::token::create_private_dir(&root).unwrap();
    let _lock = InstanceLock::try_acquire(&root).unwrap().unwrap();
    let handle = bridge.directory_discover().unwrap().unwrap();
    let result = terminal(&bridge, handle.operation_id).await;
    assert_eq!(result.error.unwrap().code, "runtime_running");
    assert!(!root.join(LIBRARY_FILE).exists());
}

#[tokio::test]
async fn download_reads_and_validates_target_while_instance_is_exclusively_owned() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("private");
    let models = temp.path().join("models");
    fs::create_dir(&models).unwrap();
    let initial = bridge(&root);
    let handle = initial.directory_apply(models.clone()).unwrap();
    assert_eq!(
        terminal(&initial, handle.operation_id).await.status,
        LibraryOperationStatus::Completed
    );
    let lock_root = root.clone();
    let downloader = Arc::new(
        DesktopBridge::new(
            root.clone(),
            temp.path().join(if cfg!(windows) {
                "ai-runtime.exe"
            } else {
                "ai-runtime"
            }),
        )
        .unwrap()
        .with_directory_validator(move |_| {
            assert!(InstanceLock::try_acquire(&lock_root).unwrap().is_none());
            // Deliberately stop before any file creation or network request.
            Err(desktop_bridge::BridgeError {
                code: "fixture_admission_stop".into(),
                message: "fixture".into(),
            })
        }),
    );
    assert_eq!(
        downloader
            .download_start("qwen3-0.6b-q8-0".into())
            .unwrap_err()
            .code,
        "fixture_admission_stop"
    );
    assert!(InstanceLock::try_acquire(&root).unwrap().is_some());
    assert_eq!(fs::read_dir(models).unwrap().count(), 0);
}
