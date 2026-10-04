//! Real external-catalog acceptance. Never prints paths, names, tokens or text.
#[cfg(windows)]
use super::consume;
use super::{Fault, Result, ensure, start};
#[cfg(windows)]
use desktop_bridge::{ChatStartRequest, LoadModelRequest, RuntimeState};
use desktop_bridge::{
    DesktopBridge, LibraryOperationResult, LibraryOperationStatus, ModelDirectoryState,
};
#[cfg(windows)]
use model_store::library::LIBRARY_FILE;
use model_store::{ModelStorage, library::ModelLibrary};
#[cfg(windows)]
use runtime_types::{Message, Role};
use serde_json::{Value, json};
use std::{collections::BTreeSet, fs, path::Path, sync::Arc, time::Duration};
use uuid::Uuid;

fn library(root: &Path) -> Result<ModelLibrary> {
    ModelLibrary::read(root)
        .map_err(|_| Fault::new("configuration_error"))?
        .ok_or_else(|| Fault::new("assertion_failed"))
}

fn managed_files(root: &Path) -> Result<BTreeSet<std::path::PathBuf>> {
    fn visit(path: &Path, found: &mut BTreeSet<std::path::PathBuf>) -> Result<()> {
        for entry in fs::read_dir(path)? {
            let path = entry?.path();
            let meta = fs::symlink_metadata(&path)?;
            ensure(!meta.file_type().is_symlink())?;
            if meta.is_dir() {
                visit(&path, found)?;
            } else if path
                .extension()
                .is_some_and(|value| value.eq_ignore_ascii_case("gguf"))
            {
                found.insert(path);
            }
        }
        Ok(())
    }
    let mut found = BTreeSet::new();
    visit(root, &mut found)?;
    Ok(found)
}

async fn completed(bridge: &DesktopBridge, id: Uuid) -> Result<LibraryOperationResult> {
    tokio::time::timeout(Duration::from_secs(310), async {
        loop {
            let state = bridge.library_next(id).await?;
            if state.terminal {
                if let Some(error) = state.error {
                    return Err(Fault::bridge(error));
                }
                ensure(state.status == LibraryOperationStatus::Completed)?;
                return state.result.ok_or_else(|| Fault::new("assertion_failed"));
            }
        }
    })
    .await
    .map_err(|_| Fault::new("timeout"))?
}

#[cfg(windows)]
fn load_request(id: &str) -> LoadModelRequest {
    LoadModelRequest {
        model_id: id.into(),
        context_size: 2048,
        threads: 2,
        batch_size: 128,
    }
}

#[cfg(windows)]
fn open_write(path: &Path) -> std::io::Result<fs::File> {
    fs::OpenOptions::new().write(true).open(path)
}

#[cfg(windows)]
fn open_delete_access(path: &Path) -> std::io::Result<fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    // DELETE access only; do not set disposition, delete-on-close or write bytes.
    fs::OpenOptions::new().access_mode(0x0001_0000).open(path)
}

#[cfg(windows)]
fn sharing_denied(result: std::io::Result<fs::File>) -> Result<()> {
    match result {
        Err(error) if matches!(error.raw_os_error(), Some(32 | 33)) => Ok(()),
        Err(error) => Err(Fault::io(error)),
        Ok(_) => Err(Fault::new("assertion_failed")),
    }
}

#[cfg(windows)]
struct OwnedLoad(tokio::task::JoinHandle<desktop_bridge::Result<desktop_bridge::RuntimeStatus>>);
#[cfg(windows)]
impl Drop for OwnedLoad {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(super) async fn run(
    root: &Path,
    runtime: &Path,
    model: &Path,
    stage: &mut &'static str,
) -> Result<Value> {
    *stage = "external_apply";
    let source_directory = model
        .parent()
        .ok_or_else(|| Fault::new("invalid_arguments"))?;
    let file_name = model
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| Fault::new("invalid_arguments"))?;
    let display_name = model
        .file_stem()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(file_name);
    let before_source = fs::metadata(model)?;
    let before_managed = managed_files(root)?;
    ensure(before_managed.len() == 1)?;
    let bridge = Arc::new(DesktopBridge::new(root.to_owned(), runtime.to_owned())?);
    let operation = bridge.directory_apply(source_directory.to_owned())?;
    let registered = completed(&bridge, operation.operation_id).await?;
    ensure(registered.registered_files >= 1)?;
    let initial = library(root)?;
    let selected = initial
        .models
        .iter()
        .find(|entry| entry.manifest.relative_file == file_name)
        .ok_or_else(|| Fault::new("assertion_failed"))?;
    ensure(
        selected.manifest.display_name == display_name
            && selected.manifest.id.as_str().starts_with("ext-"),
    )?;
    ensure(selected.manifest.size_bytes == before_source.len())?;
    let id = selected.manifest.id.clone();
    let hash = selected.manifest.sha256.clone();
    let identity = selected.identity.clone();
    ensure(managed_files(root)? == before_managed)?;

    *stage = "external_rescan";
    let rescan = bridge.models_scan()?;
    let rescanned = completed(&bridge, rescan.operation_id).await?;
    let refreshed = library(root)?;
    let stable = refreshed
        .entry(&id)
        .ok_or_else(|| Fault::new("assertion_failed"))?;
    ensure(stable.manifest.sha256 == hash && stable.identity == identity)?;
    ensure(
        rescanned.directory_id == registered.directory_id
            && rescanned.library_generation != registered.library_generation,
    )?;

    let mut report = json!({
        "supported":false,"catalog_registered_without_copy":true,"source_unchanged":false,
        "automatic_display_name":true,"stable_id_rescan":true,"legacy_managed_preserved":false,
        "effective_directory_verified":false,"direct_chat_prepared":false,"owned_preparation_cancelled":false,
        "write_access_blocked_while_loaded":false,"delete_access_blocked_while_loaded":false,
        "guard_retained_after_unload":false,"guard_released_after_stop":false,"preexisting_writer_rejected":false,
        "failed_preparation_updates_list":false,"changed_identity_rejected":false
    });
    *stage = "external_start";
    let snapshot = start(&bridge).await?;
    ensure(snapshot.model_directory.state == ModelDirectoryState::Ready)?;
    ensure(snapshot.model_directory.configured == snapshot.model_directory.effective)?;
    ensure(
        snapshot
            .model_directory
            .effective
            .as_ref()
            .is_some_and(|value| {
                Some(value.directory_id) == rescanned.directory_id
                    && value.library_generation == rescanned.library_generation
            }),
    )?;
    report["effective_directory_verified"] = true.into();
    *stage = "external_list";
    let page = bridge.models_page(None, None).await?;
    ensure(
        page.data.iter().any(|entry| {
            entry.id.as_str() == "desktop-qa" && entry.storage == ModelStorage::Managed
        }),
    )?;
    ensure(page.data.iter().any(|entry| {
        entry.id == id
            && entry.storage == ModelStorage::External
            && entry.display_name == display_name
    }))?;
    report["legacy_managed_preserved"] = true.into();

    #[cfg(windows)]
    {
        // Observe the real preparation lease before dropping only this UI's
        // load transport. A successful load or mere stopping response is not cancellation.
        *stage = "external_cancel_prepare";
        let observer = Arc::new(DesktopBridge::new(root.to_owned(), runtime.to_owned())?);
        let loading = bridge.clone();
        let request = load_request(id.as_str());
        let mut owned = OwnedLoad(tokio::spawn(
            async move { loading.load_model(request).await },
        ));
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let state = observer
                    .snapshot()
                    .await?
                    .runtime
                    .ok_or_else(|| Fault::new("assertion_failed"))?;
                if state.registry_busy {
                    ensure(state.worker.sessions_started == Some(0))?;
                    return Ok::<_, Fault>(());
                }
                ensure(!owned.0.is_finished())?;
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .map_err(|_| Fault::new("timeout"))??;
        bridge.close_ui_only().await?;
        let interrupted = tokio::time::timeout(Duration::from_secs(15), &mut owned.0)
            .await
            .map_err(|_| Fault::new("timeout"))?
            .map_err(|_| Fault::new("unexpected_failure"))?;
        match interrupted {
            Err(error) if error.code == "model_load_interrupted" => (),
            Err(error) => return Err(error.into()),
            Ok(_) => return Err(Fault::new("assertion_failed")),
        }
        let state = observer
            .snapshot()
            .await?
            .runtime
            .ok_or_else(|| Fault::new("assertion_failed"))?;
        ensure(
            !state.registry_busy
                && matches!(state.state, RuntimeState::Unloaded)
                && state.worker.sessions_started == Some(0),
        )?;
        report["owned_preparation_cancelled"] = true.into();

        *stage = "external_direct_chat";
        // Cancellation may arrive after preparation cached a guard but before
        // native loading. A new runtime guarantees this chat is a cold prepare.
        observer.stop().await?;
        let restarted = start(&observer).await?;
        ensure(restarted.runtime.is_some_and(|state| {
            !state.registry_busy
                && state.worker.sessions_started == Some(0)
                && matches!(state.state, RuntimeState::Unloaded)
        }))?;
        let request = observer.chat_start(ChatStartRequest {
            model_id: id.as_str().into(),
            messages: vec![Message::new(
                Role::User,
                "请用中文简短解释白天的天空为何呈蓝色。",
            )],
            max_output_tokens: 16,
        })?;
        let (bytes, usage) = consume(&observer, request.request_id, false).await?;
        ensure(bytes > 0 && usage > 0)?;
        let state = observer
            .snapshot()
            .await?
            .runtime
            .ok_or_else(|| Fault::new("assertion_failed"))?;
        ensure(
            state.selected_model.as_ref() == Some(&id)
                && state.selected_model_display_name.as_deref() == Some(display_name),
        )?;
        report["direct_chat_prepared"] = true.into();
        sharing_denied(open_write(model))?;
        report["write_access_blocked_while_loaded"] = true.into();
        sharing_denied(open_delete_access(model))?;
        report["delete_access_blocked_while_loaded"] = true.into();
        *stage = "external_unload";
        observer.unload_model().await?;
        sharing_denied(open_write(model))?;
        sharing_denied(open_delete_access(model))?;
        report["guard_retained_after_unload"] = true.into();
        *stage = "external_stop";
        observer.stop().await?;
        drop(open_write(model)?);
        drop(open_delete_access(model)?);
        report["guard_released_after_stop"] = true.into();

        *stage = "external_preexisting_writer";
        let writer = open_write(model)?;
        start(&observer).await?;
        let before = observer.models_page(None, None).await?;
        ensure(
            before
                .data
                .iter()
                .any(|entry| entry.id == id && entry.available),
        )?;
        match observer.load_model(load_request(id.as_str())).await {
            Err(error) if error.code == "model_file_in_use" => (),
            Err(error) => return Err(error.into()),
            Ok(_) => return Err(Fault::new("assertion_failed")),
        }
        report["preexisting_writer_rejected"] = true.into();
        let after = observer.models_page(None, None).await?;
        ensure(
            after.generation != before.generation
                && after.data.iter().any(|entry| {
                    entry.id == id
                        && !entry.available
                        && entry.availability_error.as_deref() == Some("model_file_in_use")
                }),
        )?;
        report["failed_preparation_updates_list"] = true.into();
        drop(writer);
        observer.stop().await?;

        *stage = "external_source_changed";
        let index_path = root.join(LIBRARY_FILE);
        let original = fs::read(&index_path)?;
        let mut changed = library(root)?;
        let entry = changed
            .models
            .iter_mut()
            .find(|entry| entry.manifest.id == id)
            .ok_or_else(|| Fault::new("assertion_failed"))?;
        entry.identity.modified_nanos = (entry.identity.modified_nanos + 1) % 1_000_000_000;
        {
            let _lock = runtime_cli::instance::InstanceLock::try_acquire(root)?
                .ok_or_else(|| Fault::new("assertion_failed"))?;
            fs::write(
                &index_path,
                changed
                    .encode()
                    .map_err(|_| Fault::new("configuration_error"))?,
            )?;
        }
        start(&observer).await?;
        match observer.load_model(load_request(id.as_str())).await {
            Err(error) if error.code == "model_file_changed" => (),
            Err(error) => return Err(error.into()),
            Ok(_) => return Err(Fault::new("assertion_failed")),
        }
        report["changed_identity_rejected"] = true.into();
        *stage = "external_final_stop";
        observer.stop().await?;
        {
            let _lock = runtime_cli::instance::InstanceLock::try_acquire(root)?
                .ok_or_else(|| Fault::new("assertion_failed"))?;
            fs::write(&index_path, original)?;
        }
        report["supported"] = true.into();
    }
    #[cfg(not(windows))]
    {
        *stage = "external_final_stop";
        bridge.stop().await?;
    }

    // The source itself was never written. Rehash via a real scan after stop,
    // while checking that registration/loading added no managed GGUF copies.
    *stage = "external_rescan";
    let final_bridge = Arc::new(DesktopBridge::new(root.to_owned(), runtime.to_owned())?);
    let operation = final_bridge.models_scan()?;
    completed(&final_bridge, operation.operation_id).await?;
    let final_library = library(root)?;
    let final_entry = final_library
        .entry(&id)
        .ok_or_else(|| Fault::new("assertion_failed"))?;
    ensure(final_entry.manifest.sha256 == hash && final_entry.identity == identity)?;
    ensure(managed_files(root)? == before_managed)?;
    let after_source = fs::metadata(model)?;
    ensure(
        before_source.len() == after_source.len()
            && before_source.modified()? == after_source.modified()?,
    )?;
    report["source_unchanged"] = true.into();
    Ok(report)
}
