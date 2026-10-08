//! Local workbench drafts and OCR defaults, independently persisted as TOML.
use crate::{BridgeError, DesktopBridge, Result};
use runtime_api::token;
use runtime_types::{LoadOptions, ModelId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, Read},
    path::Path,
    sync::atomic::Ordering,
};

const FILE_NAME: &str = "workbench-preferences.toml";
const MAX_FILE_BYTES: usize = 64 * 1024;

#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkbenchPreferences {
    #[serde(default)]
    pub close_to_tray: bool,
    pub ocr: OcrWorkbenchPreferences,
    pub chat: ChatWorkbenchPreferences,
}
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OcrWorkbenchPreferences {
    pub model_id: Option<ModelId>,
    #[serde(default)]
    pub model_drafts: BTreeMap<ModelId, OcrLoadDraft>,
    pub context_size: u32,
    pub threads: u32,
    pub batch_size: u32,
    pub max_output_tokens: u32,
    pub prompt: String,
    pub image_edge: u32,
    pub markdown: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OcrLoadDraft {
    pub base: LoadOptions,
    pub draft: LoadOptions,
}
#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatWorkbenchPreferences {
    pub draft: String,
}
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkbenchPreferencesSnapshot {
    pub revision: String,
    pub preferences: WorkbenchPreferences,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkbenchPreferencesSaveRequest {
    pub expected_revision: String,
    pub preferences: WorkbenchPreferences,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Store {
    schema_version: u32,
    #[serde(default)]
    close_to_tray: bool,
    ocr: OcrWorkbenchPreferences,
    chat: ChatWorkbenchPreferences,
}
impl Default for OcrWorkbenchPreferences {
    fn default() -> Self {
        Self {
            model_id: None,
            model_drafts: BTreeMap::new(),
            context_size: 8192,
            threads: 4,
            batch_size: 256,
            max_output_tokens: 2048,
            prompt: "Text Recognition:".into(),
            image_edge: 0,
            markdown: false,
        }
    }
}
impl WorkbenchPreferences {
    fn valid(&self) -> bool {
        let ocr = &self.ocr;
        ocr.model_drafts.len() <= 128
            && ocr
                .model_drafts
                .values()
                .all(|entry| entry.base.validate().is_ok() && entry.draft.validate().is_ok())
            && LoadOptions {
                context_size: ocr.context_size,
                threads: ocr.threads,
                batch_size: ocr.batch_size,
            }
            .validate()
            .is_ok()
            && (1..=4096).contains(&ocr.max_output_tokens)
            && matches!(ocr.image_edge, 0 | 1600 | 2048)
            && ocr.prompt.len() <= 4096
            && self.chat.draft.len() <= 16 * 1024
    }
}
impl DesktopBridge {
    pub async fn workbench_get(&self) -> Result<WorkbenchPreferencesSnapshot> {
        let root = self.root.clone();
        tokio::task::spawn_blocking(move || read(&root))
            .await
            .map_err(|_| error("workbench_io"))?
    }
    pub async fn workbench_save(
        &self,
        request: WorkbenchPreferencesSaveRequest,
    ) -> Result<WorkbenchPreferencesSnapshot> {
        let gate = self.persistence.clone();
        let guard = gate.lock_owned().await;
        if self.closing.load(Ordering::Acquire) {
            return Err(error("workbench_closing"));
        }
        let root = self.root.clone();
        tokio::task::spawn_blocking(move || {
            let _guard = guard;
            save(&root, request)
        })
        .await
        .map_err(|_| error("workbench_io"))?
    }
}
fn error(code: &str) -> BridgeError {
    BridgeError::new(code)
}
fn io_error(_: io::Error) -> BridgeError {
    error("workbench_io")
}
fn revision(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn read(root: &Path) -> Result<WorkbenchPreferencesSnapshot> {
    match std::fs::symlink_metadata(root.join(FILE_NAME)) {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Ok(WorkbenchPreferencesSnapshot {
                revision: revision(b""),
                preferences: WorkbenchPreferences::default(),
            });
        }
        Err(e) => return Err(io_error(e)),
    }
    let _lock = lock(root)?;
    read_unlocked(root)
}
fn read_unlocked(root: &Path) -> Result<WorkbenchPreferencesSnapshot> {
    let file = match token::open_private_file(&root.join(FILE_NAME)) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Ok(WorkbenchPreferencesSnapshot {
                revision: revision(b""),
                preferences: WorkbenchPreferences::default(),
            });
        }
        Err(e) => return Err(io_error(e)),
    };
    if file.metadata().map_err(io_error)?.len() > MAX_FILE_BYTES as u64 {
        return Err(error("workbench_corrupt"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(error("workbench_corrupt"));
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| error("workbench_corrupt"))?;
    let store: Store = toml::from_str(text).map_err(|_| error("workbench_corrupt"))?;
    if store.schema_version != 1 {
        return Err(error("workbench_corrupt"));
    }
    let preferences = WorkbenchPreferences {
        close_to_tray: store.close_to_tray,
        ocr: store.ocr,
        chat: store.chat,
    };
    if !preferences.valid() {
        return Err(error("workbench_corrupt"));
    }
    Ok(WorkbenchPreferencesSnapshot {
        revision: revision(&bytes),
        preferences,
    })
}
fn lock(root: &Path) -> Result<File> {
    lock_with_timeout(root, std::time::Duration::from_secs(5))
}
fn lock_with_timeout(root: &Path, timeout: std::time::Duration) -> Result<File> {
    token::create_private_dir(root).map_err(io_error)?;
    let path = root.join("workbench-preferences.lock");
    match token::write_private_new(&path, b"") {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(io_error(e)),
    }
    let file = token::open_private_file(&path).map_err(io_error)?;
    let started = std::time::Instant::now();
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) => {
                let remaining = timeout.saturating_sub(started.elapsed());
                if remaining.is_zero() {
                    return Err(error("workbench_busy"));
                }
                std::thread::sleep(remaining.min(std::time::Duration::from_millis(10)));
            }
            Err(std::fs::TryLockError::Error(e)) => return Err(io_error(e)),
        }
    }
}
fn save(
    root: &Path,
    request: WorkbenchPreferencesSaveRequest,
) -> Result<WorkbenchPreferencesSnapshot> {
    if !request.preferences.valid()
        || request.expected_revision.len() != 64
        || !request
            .expected_revision
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(error("workbench_invalid"));
    }
    let _lock = lock(root)?;
    let current = read_unlocked(root)?;
    if current.revision != request.expected_revision {
        return Err(error("workbench_conflict"));
    }
    let store = Store {
        schema_version: 1,
        close_to_tray: request.preferences.close_to_tray,
        ocr: request.preferences.ocr.clone(),
        chat: request.preferences.chat.clone(),
    };
    let bytes = toml::to_string(&store)
        .map_err(|_| error("workbench_invalid"))?
        .into_bytes();
    if bytes.len() > MAX_FILE_BYTES {
        return Err(error("workbench_limit"));
    }
    token::atomic_replace_private(&root.join(FILE_NAME), &bytes).map_err(|e| {
        if e.to_string() == "configuration_durability_unconfirmed" {
            error("workbench_durability_unconfirmed")
        } else {
            io_error(e)
        }
    })?;
    Ok(WorkbenchPreferencesSnapshot {
        revision: revision(&bytes),
        preferences: request.preferences,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bridge(root: &Path) -> DesktopBridge {
        DesktopBridge::new(
            root.to_path_buf(),
            root.join(if cfg!(windows) {
                "ai-runtime.exe"
            } else {
                "ai-runtime"
            }),
        )
        .unwrap()
    }
    fn request(snapshot: &WorkbenchPreferencesSnapshot) -> WorkbenchPreferencesSaveRequest {
        WorkbenchPreferencesSaveRequest {
            expected_revision: snapshot.revision.clone(),
            preferences: snapshot.preferences.clone(),
        }
    }
    #[tokio::test]
    async fn defaults_are_read_only_and_unicode_toml_survives_restart_without_touching_other_configuration()
     {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private");
        let first = bridge(&root);
        let initial = first.workbench_get().await.unwrap();
        assert!(initial.preferences == WorkbenchPreferences::default());
        assert!(!root.exists());
        token::create_private_dir(&root).unwrap();
        token::write_private_new(&root.join("config.toml"), b"preserved config").unwrap();
        token::write_private_new(&root.join("desktop-settings.json"), b"preserved settings")
            .unwrap();
        let mut req = request(&initial);
        req.preferences.ocr.prompt = "  中文\n```提示```\\\"🙂\r\n".into();
        req.preferences.chat.draft = "草稿\n\t\0原文🙂".into();
        req.preferences.ocr.model_id = Some(ModelId::new("ocr-model").unwrap());
        let saved = first.workbench_save(req.clone()).await.unwrap();
        drop(first);
        let second = bridge(&root);
        let reopened = second.workbench_get().await.unwrap();
        assert!(saved == reopened);
        assert!(reopened.preferences == req.preferences);
        let raw = std::fs::read(root.join(FILE_NAME)).unwrap();
        assert_eq!(reopened.revision, revision(&raw));
        assert!(toml::from_str::<toml::Value>(std::str::from_utf8(&raw).unwrap()).is_ok());
        assert_eq!(
            std::fs::read(root.join("config.toml")).unwrap(),
            b"preserved config"
        );
        assert_eq!(
            std::fs::read(root.join("desktop-settings.json")).unwrap(),
            b"preserved settings"
        );
    }
    #[test]
    fn cas_conflict_and_lock_contention_preserve_the_committed_file() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private");
        let initial = read(&root).unwrap();
        let mut req = request(&initial);
        req.preferences.chat.draft = "first".into();
        save(&root, req).unwrap();
        let before = std::fs::read(root.join(FILE_NAME)).unwrap();
        assert_eq!(
            save(&root, request(&initial)).err().unwrap().code,
            "workbench_conflict"
        );
        let held = lock(&root).unwrap();
        assert_eq!(
            lock_with_timeout(&root, std::time::Duration::from_millis(20))
                .err()
                .unwrap()
                .code,
            "workbench_busy"
        );
        drop(held);
        assert_eq!(std::fs::read(root.join(FILE_NAME)).unwrap(), before);
    }
    #[test]
    fn tray_preference_defaults_for_old_toml_and_persists_with_draft_cas() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private");
        let initial = read(&root).unwrap();
        assert!(!initial.preferences.close_to_tray);
        let mut req = request(&initial);
        req.preferences.chat.draft = "keep chat".into();
        req.preferences.ocr.prompt = "keep OCR".into();
        save(&root, req).unwrap();
        let old = std::fs::read_to_string(root.join(FILE_NAME))
            .unwrap()
            .replace("close_to_tray = false\n", "");
        token::atomic_replace_private(&root.join(FILE_NAME), old.as_bytes()).unwrap();
        let previous = read(&root).unwrap();
        assert!(!previous.preferences.close_to_tray);
        let mut enable = request(&previous);
        enable.preferences.close_to_tray = true;
        save(&root, enable).unwrap();
        let restored = read(&root).unwrap();
        assert!(restored.preferences.close_to_tray);
        assert_eq!(restored.preferences.chat.draft, "keep chat");
        assert_eq!(restored.preferences.ocr.prompt, "keep OCR");
        assert_eq!(
            save(&root, request(&previous)).err().unwrap().code,
            "workbench_conflict"
        );
        assert!(read(&root).unwrap().preferences.close_to_tray);
        let mut disable = request(&restored);
        disable.preferences.close_to_tray = false;
        save(&root, disable).unwrap();
        assert!(!read(&root).unwrap().preferences.close_to_tray);
    }
    #[test]
    fn invalid_tray_preference_is_preserved_as_corrupt_instead_of_reset() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private");
        let initial = read(&root).unwrap();
        save(&root, request(&initial)).unwrap();
        let invalid = std::fs::read_to_string(root.join(FILE_NAME))
            .unwrap()
            .replace("close_to_tray = false", "close_to_tray = 'yes'");
        token::atomic_replace_private(&root.join(FILE_NAME), invalid.as_bytes()).unwrap();
        assert_eq!(read(&root).err().unwrap().code, "workbench_corrupt");
        assert_eq!(
            save(&root, request(&initial)).err().unwrap().code,
            "workbench_corrupt"
        );
        assert_eq!(
            std::fs::read_to_string(root.join(FILE_NAME)).unwrap(),
            invalid
        );
    }
    #[test]
    fn invalid_boundaries_and_empty_drafts_are_checked_by_utf8_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private");
        let initial = read(&root).unwrap();
        for field in 0..8 {
            let mut req = request(&initial);
            match field {
                0 => req.preferences.ocr.context_size = 31,
                1 => req.preferences.ocr.threads = 0,
                2 => req.preferences.ocr.batch_size = 4097,
                3 => req.preferences.ocr.max_output_tokens = 0,
                4 => req.preferences.ocr.max_output_tokens = 4097,
                5 => req.preferences.ocr.image_edge = 1024,
                6 => req.preferences.ocr.prompt = "é".repeat(2049),
                _ => req.preferences.chat.draft = "é".repeat(8193),
            }
            assert_eq!(save(&root, req).err().unwrap().code, "workbench_invalid");
        }
        let mut req = request(&initial);
        req.preferences.ocr.prompt.clear();
        req.preferences.ocr.context_size = 32;
        req.preferences.ocr.batch_size = 32;
        req.preferences.ocr.threads = 256;
        req.preferences.ocr.max_output_tokens = 4096;
        let saved = save(&root, req).unwrap();
        assert_eq!(saved.preferences.ocr.context_size, 32);
        assert_eq!(saved.preferences.ocr.threads, 256);
        for invalid in [0, 1] {
            let mut req = request(&saved);
            if invalid == 0 {
                req.preferences.ocr.threads = 257;
            } else {
                req.preferences.ocr.batch_size = 33;
            }
            assert_eq!(save(&root, req).err().unwrap().code, "workbench_invalid");
        }
        let mut req = request(&saved);
        req.preferences.ocr.prompt = "é".repeat(2048);
        req.preferences.chat.draft = "é".repeat(8192);
        assert!(save(&root, req).is_ok());
    }
    #[test]
    fn malformed_duplicate_unknown_and_oversized_files_are_never_overwritten() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private");
        let initial = read(&root).unwrap();
        save(&root, request(&initial)).unwrap();
        let valid = std::fs::read_to_string(root.join(FILE_NAME)).unwrap();
        let cases = [
            "broken = [".to_string(),
            valid.replacen("schema_version = 1", "schema_version = 2", 1),
            format!("unknown = true\n{valid}"),
            format!("schema_version = 1\n{valid}"),
            format!("{}\n{valid}", "#".repeat(MAX_FILE_BYTES + 1)),
        ];
        for text in cases {
            token::atomic_replace_private(&root.join(FILE_NAME), text.as_bytes()).unwrap();
            assert_eq!(
                save(&root, request(&initial)).err().unwrap().code,
                "workbench_corrupt"
            );
            assert_eq!(
                std::fs::read(root.join(FILE_NAME)).unwrap(),
                text.as_bytes()
            );
        }
    }
    #[cfg(unix)]
    #[test]
    fn linked_store_and_lock_do_not_follow_or_replace_targets() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private");
        let initial = read(&root).unwrap();
        save(&root, request(&initial)).unwrap();
        let victim = root.join("victim");
        token::write_private_new(&victim, b"untouched").unwrap();
        std::fs::remove_file(root.join(FILE_NAME)).unwrap();
        symlink(&victim, root.join(FILE_NAME)).unwrap();
        assert!(read(&root).is_err());
        assert!(save(&root, request(&initial)).is_err());
        std::fs::remove_file(root.join(FILE_NAME)).unwrap();
        std::fs::remove_file(root.join("workbench-preferences.lock")).unwrap();
        symlink(&victim, root.join("workbench-preferences.lock")).unwrap();
        assert!(save(&root, request(&initial)).is_err());
        assert_eq!(std::fs::read(victim).unwrap(), b"untouched");
    }
    #[tokio::test]
    async fn generation_does_not_block_preferences_and_closing_only_rejects_writes() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private");
        let desktop = bridge(&root);
        let _generation = desktop.work.lock().await;
        let current = desktop.workbench_get().await.unwrap();
        let saved = desktop.workbench_save(request(&current)).await.unwrap();
        desktop.closing.store(true, Ordering::Release);
        assert!(desktop.workbench_get().await.unwrap() == saved);
        assert_eq!(
            desktop
                .workbench_save(request(&saved))
                .await
                .err()
                .unwrap()
                .code,
            "workbench_closing"
        );
    }
    #[test]
    fn model_drafts_are_bounded_validated_and_persist_without_changing_fallback() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private");
        let initial = read(&root).unwrap();
        let mut req = request(&initial);
        let draft = OcrLoadDraft {
            base: LoadOptions {
                context_size: 32,
                threads: 1,
                batch_size: 32,
            },
            draft: LoadOptions {
                context_size: 1024,
                threads: 2,
                batch_size: 64,
            },
        };
        for n in 0..128 {
            req.preferences
                .ocr
                .model_drafts
                .insert(ModelId::new(format!("model-{n}")).unwrap(), draft);
        }
        let saved = save(&root, req).unwrap();
        assert_eq!(saved.preferences.ocr.context_size, 8192);
        assert!(read(&root).unwrap().preferences == saved.preferences);
        let mut req = request(&saved);
        req.preferences
            .ocr
            .model_drafts
            .insert(ModelId::new("extra-model").unwrap(), draft);
        assert_eq!(save(&root, req).err().unwrap().code, "workbench_invalid");
        let mut req = request(&saved);
        req.preferences
            .ocr
            .model_drafts
            .values_mut()
            .next()
            .unwrap()
            .draft
            .threads = 0;
        assert_eq!(save(&root, req).err().unwrap().code, "workbench_invalid");
        let mut req = request(&saved);
        req.preferences.chat.draft = "\0".repeat(16 * 1024);
        assert_eq!(save(&root, req).err().unwrap().code, "workbench_limit");
        assert!(read(&root).unwrap() == saved);
    }
    #[test]
    fn readers_and_atomic_writers_share_the_same_file_lock() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private");
        save(&root, request(&read(&root).unwrap())).unwrap();
        let held = lock(&root).unwrap();
        let open_reader = token::open_private_file(&root.join(FILE_NAME)).unwrap();
        let writer_root = root.clone();
        let (done, completed) = std::sync::mpsc::channel();
        let writer = std::thread::spawn(move || {
            for n in 0..12 {
                let current = read(&writer_root).unwrap();
                let mut req = request(&current);
                req.preferences.chat.draft = format!("draft-{n}");
                save(&writer_root, req).unwrap();
            }
            done.send(()).unwrap();
        });
        assert!(
            completed
                .recv_timeout(std::time::Duration::from_millis(30))
                .is_err()
        );
        drop(open_reader);
        drop(held);
        let reader_root = root.clone();
        let reader = std::thread::spawn(move || {
            for _ in 0..12 {
                assert!(read(&reader_root).unwrap().preferences.valid());
            }
        });
        completed
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        writer.join().unwrap();
        reader.join().unwrap();
        assert_eq!(read(&root).unwrap().preferences.chat.draft, "draft-11");
    }
    #[tokio::test]
    async fn writes_queued_before_closing_are_rejected_after_the_persistence_gate() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private");
        let desktop = std::sync::Arc::new(bridge(&root));
        let held = desktop.persistence.clone().lock_owned().await;
        let queued = {
            let desktop = desktop.clone();
            tokio::spawn(async move {
                desktop
                    .workbench_save(request(&desktop.workbench_get().await.unwrap()))
                    .await
            })
        };
        tokio::task::yield_now().await;
        desktop.closing.store(true, Ordering::Release);
        drop(held);
        assert_eq!(
            queued.await.unwrap().err().unwrap().code,
            "workbench_closing"
        );
        assert!(!root.join(FILE_NAME).exists());
    }
}
