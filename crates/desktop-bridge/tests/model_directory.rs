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
async fn running_instance_and_failed_scan_preserve_original_configuration() {
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
    assert_eq!(result.status, LibraryOperationStatus::Failed);
    assert_eq!(result.failed_file_name.as_deref(), Some("坏 文件.gguf"));
    assert_eq!(fs::read(root.join(LIBRARY_FILE)).unwrap(), original);
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
