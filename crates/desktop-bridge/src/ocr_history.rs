//! Private, bounded OCR result persistence. Images and prompts are never stored.
use crate::{BridgeError, DesktopBridge, Result};
use runtime_api::{performance::PerformanceSnapshot, token};
use runtime_types::{
    FinishReason, ModelId, PerformanceModality, PerformanceRecord, PerformanceStatus,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::File,
    io::{self, Read, Write},
    path::Path,
    sync::atomic::Ordering,
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

const CAPACITY: usize = 100;
const MAX_MARKDOWN_BYTES: usize = 256 * 1024;
// Both record count and encoded JSON bytes are bounded. Escaped content may
// reach the byte budget before the count budget; rejected writes preserve disk.
const MAX_FILE_BYTES: usize = 32 * 1024 * 1024;
const FILE_NAME: &str = "ocr-history.json";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum OcrHistorySaveMode {
    Create,
    UpdatePerformance,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OcrHistoryPerformance {
    pub instance_id: Uuid,
    pub record: PerformanceRecord,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OcrHistorySaveRequest {
    pub mode: OcrHistorySaveMode,
    pub id: Uuid,
    pub image_name: String,
    pub model_id: ModelId,
    pub status: PerformanceStatus,
    pub incomplete: bool,
    pub finish_reason: Option<FinishReason>,
    pub error_code: Option<String>,
    pub markdown: String,
    pub performance: Option<OcrHistoryPerformance>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OcrHistoryEntry {
    pub id: Uuid,
    pub image_name: String,
    pub model_id: ModelId,
    pub status: PerformanceStatus,
    pub incomplete: bool,
    pub finish_reason: Option<FinishReason>,
    pub error_code: Option<String>,
    pub markdown: String,
    pub performance: Option<OcrHistoryPerformance>,
    pub first_saved_at_unix_ms: u64,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OcrHistorySummary {
    pub id: Uuid,
    pub image_name: String,
    pub model_id: ModelId,
    pub status: PerformanceStatus,
    pub incomplete: bool,
    pub finish_reason: Option<FinishReason>,
    pub error_code: Option<String>,
    pub first_saved_at_unix_ms: u64,
    pub markdown_bytes: usize,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OcrHistoryList {
    pub capacity: usize,
    pub entries: Vec<OcrHistorySummary>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Store {
    schema: u32,
    entries: Vec<OcrHistoryEntry>,
}

impl DesktopBridge {
    pub async fn ocr_history_list(&self) -> Result<OcrHistoryList> {
        let root = self.root.clone();
        tokio::task::spawn_blocking(move || Ok(read(&root)?.list()))
            .await
            .map_err(|_| error("ocr_history_io"))?
    }
    pub async fn ocr_history_get(&self, id: Uuid) -> Result<OcrHistoryEntry> {
        if id.is_nil() {
            return Err(error("ocr_history_invalid"));
        }
        let root = self.root.clone();
        tokio::task::spawn_blocking(move || {
            read(&root)?
                .entries
                .into_iter()
                .find(|entry| entry.id == id)
                .ok_or_else(|| error("ocr_history_not_found"))
        })
        .await
        .map_err(|_| error("ocr_history_io"))?
    }
    pub async fn ocr_history_save(&self, request: OcrHistorySaveRequest) -> Result<OcrHistoryList> {
        let gate = self.persistence.clone();
        let guard = gate.lock_owned().await;
        if self.closing.load(Ordering::Acquire) {
            return Err(error("ocr_history_closing"));
        }
        let root = self.root.clone();
        tokio::task::spawn_blocking(move || {
            let _guard = guard;
            save(&root, request)
        })
        .await
        .map_err(|_| error("ocr_history_io"))?
    }
    pub async fn ocr_history_delete(&self, id: Uuid) -> Result<OcrHistoryList> {
        let gate = self.persistence.clone();
        let guard = gate.lock_owned().await;
        if self.closing.load(Ordering::Acquire) {
            return Err(error("ocr_history_closing"));
        }
        let root = self.root.clone();
        tokio::task::spawn_blocking(move || {
            let _guard = guard;
            delete(&root, id)
        })
        .await
        .map_err(|_| error("ocr_history_io"))?
    }
}
fn error(code: &str) -> BridgeError {
    BridgeError::new(code)
}
fn io_error(_: io::Error) -> BridgeError {
    error("ocr_history_io")
}
fn lock(root: &Path) -> Result<File> {
    lock_with_timeout(root, std::time::Duration::from_secs(5))
}
fn lock_with_timeout(root: &Path, timeout: std::time::Duration) -> Result<File> {
    token::create_private_dir(root).map_err(io_error)?;
    let path = root.join("ocr-history.lock");
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
                    return Err(error("ocr_history_busy"));
                }
                std::thread::sleep(remaining.min(std::time::Duration::from_millis(10)));
            }
            Err(std::fs::TryLockError::Error(e)) => return Err(io_error(e)),
        }
    }
}
fn read(root: &Path) -> Result<Store> {
    match std::fs::symlink_metadata(root.join(FILE_NAME)) {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Ok(Store {
                schema: 1,
                entries: vec![],
            });
        }
        Err(e) => return Err(io_error(e)),
    }
    let _lock = lock(root)?;
    read_unlocked(root)
}
fn read_unlocked(root: &Path) -> Result<Store> {
    let file = match token::open_private_file(&root.join(FILE_NAME)) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Ok(Store {
                schema: 1,
                entries: vec![],
            });
        }
        Err(e) => return Err(io_error(e)),
    };
    if file.metadata().map_err(io_error)?.len() > MAX_FILE_BYTES as u64 {
        return Err(error("ocr_history_corrupt"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(error("ocr_history_corrupt"));
    }
    let store: Store = serde_json::from_slice(&bytes).map_err(|_| error("ocr_history_corrupt"))?;
    let mut ids = HashSet::new();
    if store.schema != 1
        || store.entries.len() > CAPACITY
        || store
            .entries
            .iter()
            .any(|entry| !entry.valid() || !ids.insert(entry.id))
        || !store
            .entries
            .windows(2)
            .all(|pair| pair[0].first_saved_at_unix_ms >= pair[1].first_saved_at_unix_ms)
    {
        return Err(error("ocr_history_corrupt"));
    }
    Ok(store)
}
struct BoundedJson {
    bytes: Vec<u8>,
    exceeded: bool,
}
impl Write for BoundedJson {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_FILE_BYTES.saturating_sub(self.bytes.len()) {
            self.exceeded = true;
            return Err(io::Error::other("encoded history limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn write(root: &Path, store: &Store) -> Result<()> {
    let mut encoded = BoundedJson {
        bytes: Vec::new(),
        exceeded: false,
    };
    if serde_json::to_writer(&mut encoded, store).is_err() {
        return Err(error(if encoded.exceeded {
            "ocr_history_limit"
        } else {
            "ocr_history_invalid"
        }));
    }
    let bytes = encoded.bytes;
    token::atomic_replace_private(&root.join(FILE_NAME), &bytes).map_err(publication_error)
}
fn publication_error(source: io::Error) -> BridgeError {
    if matches!(
        token::PrivateFileError::from_io(&source),
        Some(token::PrivateFileError::PublishedDurabilityUnconfirmed { .. })
    ) {
        error("ocr_history_durability_unconfirmed")
    } else {
        io_error(source)
    }
}
impl Store {
    fn list(&self) -> OcrHistoryList {
        OcrHistoryList {
            capacity: CAPACITY,
            entries: self
                .entries
                .iter()
                .map(|entry| OcrHistorySummary {
                    id: entry.id,
                    image_name: entry.image_name.clone(),
                    model_id: entry.model_id.clone(),
                    status: entry.status,
                    incomplete: entry.incomplete,
                    finish_reason: entry.finish_reason,
                    error_code: entry.error_code.clone(),
                    first_saved_at_unix_ms: entry.first_saved_at_unix_ms,
                    markdown_bytes: entry.markdown.len(),
                })
                .collect(),
        }
    }
}
impl OcrHistoryEntry {
    fn valid(&self) -> bool {
        !self.id.is_nil()
            && self.image_name.len() <= 1024
            && !self.markdown.is_empty()
            && self.markdown.len() <= MAX_MARKDOWN_BYTES
            && self.first_saved_at_unix_ms <= 9_007_199_254_740_991
            && self.error_code.as_ref().is_none_or(|code| {
                !code.is_empty()
                    && code.len() <= 96
                    && code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            })
            && match self.status {
                PerformanceStatus::Completed => {
                    self.finish_reason.is_some() && self.error_code.is_none()
                }
                PerformanceStatus::Cancelled | PerformanceStatus::Failed => {
                    self.finish_reason.is_none() && self.error_code.is_some()
                }
            }
            && (self.incomplete
                || (self.status == PerformanceStatus::Completed
                    && self.finish_reason != Some(FinishReason::Length)))
            && self.performance.as_ref().is_none_or(|performance| {
                let record = &performance.record;
                record.request_id.to_string() == self.id.to_string()
                    && record.model_id == self.model_id
                    && record.modality == PerformanceModality::Image
                    && record.status == self.status
                    && record.finish_reason == self.finish_reason
                    && record.error_code.map(|code| code.as_str()) == self.error_code.as_deref()
                    && PerformanceSnapshot {
                        instance_id: performance.instance_id,
                        capacity: 1,
                        records: vec![record.clone()],
                    }
                    .is_valid()
            })
    }
    fn same_content(&self, other: &Self) -> bool {
        self.id == other.id
            && self.image_name == other.image_name
            && self.model_id == other.model_id
            && self.status == other.status
            && self.incomplete == other.incomplete
            && self.finish_reason == other.finish_reason
            && self.error_code == other.error_code
            && self.markdown == other.markdown
    }
}
fn save(root: &Path, request: OcrHistorySaveRequest) -> Result<OcrHistoryList> {
    let mode = request.mode;
    let mut entry = OcrHistoryEntry {
        id: request.id,
        image_name: request.image_name,
        model_id: request.model_id,
        status: request.status,
        incomplete: request.incomplete,
        finish_reason: request.finish_reason,
        error_code: request.error_code,
        markdown: request.markdown,
        performance: request.performance,
        first_saved_at_unix_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(9_007_199_254_740_991) as u64,
    };
    if !entry.valid()
        || (mode == OcrHistorySaveMode::UpdatePerformance && entry.performance.is_none())
    {
        return Err(error("ocr_history_invalid"));
    }
    let _lock = lock(root)?;
    let mut store = read_unlocked(root)?;
    match store
        .entries
        .iter_mut()
        .find(|existing| existing.id == entry.id)
    {
        Some(existing) => {
            if !existing.same_content(&entry) {
                return Err(error("ocr_history_conflict"));
            }
            if mode == OcrHistorySaveMode::Create || existing.performance == entry.performance {
                return Ok(store.list());
            }
            if existing.performance.is_some() {
                return Err(error("ocr_history_conflict"));
            }
            existing.performance = entry.performance;
        }
        None if mode == OcrHistorySaveMode::UpdatePerformance => {
            return Err(error("ocr_history_not_found"));
        }
        None => {
            // Clock rollback must not reorder previously saved entries.
            if let Some(newest) = store.entries.first() {
                entry.first_saved_at_unix_ms = entry
                    .first_saved_at_unix_ms
                    .max(newest.first_saved_at_unix_ms);
            }
            store.entries.insert(0, entry);
            store.entries.truncate(CAPACITY);
        }
    }
    write(root, &store)?;
    Ok(store.list())
}
fn delete(root: &Path, id: Uuid) -> Result<OcrHistoryList> {
    if id.is_nil() {
        return Err(error("ocr_history_invalid"));
    }
    let _lock = lock(root)?;
    let mut store = read_unlocked(root)?;
    let before = store.entries.len();
    store.entries.retain(|entry| entry.id != id);
    if store.entries.len() != before {
        write(root, &store)?;
    }
    Ok(store.list())
}

#[cfg(test)]
mod tests {
    use super::*;
    use runtime_types::{
        ErrorCode, InferenceTimings, LoadOptions, RequestPerformance, RequestTimings, Usage,
    };

    #[test]
    fn publication_state_requires_typed_error_not_display_text() {
        let published = publication_error(io::Error::other(
            token::PrivateFileError::PublishedDurabilityUnconfirmed {
                source: io::Error::other("private_sentinel_path"),
            },
        ));
        assert_eq!(published.code, "ocr_history_durability_unconfirmed");
        assert!(!format!("{published:?} {published}").contains("private_sentinel"));
        let spoof = publication_error(io::Error::other("configuration_durability_unconfirmed"));
        assert_eq!(spoof.code, "ocr_history_io");
    }
    fn request() -> OcrHistorySaveRequest {
        OcrHistorySaveRequest {
            mode: OcrHistorySaveMode::Create,
            id: Uuid::new_v4(),
            image_name: "页面.png".into(),
            model_id: ModelId::new("ocr-model").unwrap(),
            status: PerformanceStatus::Completed,
            incomplete: false,
            finish_reason: Some(FinishReason::Stop),
            error_code: None,
            markdown: "# 识别结果\n正文".into(),
            performance: None,
        }
    }
    fn measured(request: &OcrHistorySaveRequest) -> OcrHistoryPerformance {
        OcrHistoryPerformance {
            instance_id: Uuid::new_v4(),
            record: PerformanceRecord {
                sequence: 1,
                request_id: request.id.to_string().parse().unwrap(),
                model_id: request.model_id.clone(),
                modality: PerformanceModality::Image,
                status: request.status,
                accepted_at_unix_ms: 1,
                max_output_tokens: 10,
                usage: Usage {
                    prompt_tokens: 8,
                    completion_tokens: 2,
                },
                timings: RequestTimings::default(),
                performance: Some(RequestPerformance {
                    timings: InferenceTimings {
                        prepare_us: 1,
                        prefill_us: 2,
                        decode_us: 3,
                        output_callback_us: 4,
                    },
                    load_options: LoadOptions::default(),
                }),
                error_code: None,
                finish_reason: request.finish_reason,
            },
        }
    }
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
    #[tokio::test]
    async fn bounded_history_survives_new_bridge_and_reads_do_not_initialize_directory() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private");
        let first = bridge(&root);
        assert!(first.ocr_history_list().await.unwrap().entries.is_empty());
        assert_eq!(
            first
                .ocr_history_get(Uuid::new_v4())
                .await
                .err()
                .unwrap()
                .code,
            "ocr_history_not_found"
        );
        assert!(!root.exists());
        let mut ids = Vec::new();
        for _ in 0..101 {
            let request = request();
            ids.push(request.id);
            first.ocr_history_save(request).await.unwrap();
        }
        drop(first);
        let restarted = bridge(&root);
        let list = restarted.ocr_history_list().await.unwrap();
        assert_eq!(list.capacity, 100);
        assert_eq!(list.entries.len(), 100);
        assert_eq!(list.entries[0].id, ids[100]);
        assert_eq!(list.entries[99].id, ids[1]);
        assert_eq!(
            restarted.ocr_history_get(ids[0]).await.err().unwrap().code,
            "ocr_history_not_found"
        );
        assert_eq!(
            restarted.ocr_history_get(ids[100]).await.unwrap().markdown,
            request().markdown
        );
        let json = serde_json::to_string(&list).unwrap();
        assert!(!json.contains("识别结果"));
        assert!(!json.contains("performance"));
    }
    #[test]
    fn create_is_idempotent_update_keeps_order_and_deleted_update_cannot_resurrect() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private");
        let first = request();
        let before = save(&root, first.clone()).unwrap();
        assert_eq!(save(&root, first.clone()).unwrap(), before);
        let second = request();
        save(&root, second.clone()).unwrap();
        let mut update = first.clone();
        update.mode = OcrHistorySaveMode::UpdatePerformance;
        update.performance = Some(measured(&first));
        let list = save(&root, update.clone()).unwrap();
        assert_eq!(list.entries[0].id, second.id);
        assert_eq!(
            list.entries[1].first_saved_at_unix_ms,
            before.entries[0].first_saved_at_unix_ms
        );
        assert_eq!(save(&root, update.clone()).unwrap(), list);
        let mut conflict = first.clone();
        conflict.markdown.push('!');
        assert_eq!(
            save(&root, conflict).unwrap_err().code,
            "ocr_history_conflict"
        );
        delete(&root, first.id).unwrap();
        assert_eq!(
            save(&root, update).unwrap_err().code,
            "ocr_history_not_found"
        );
        assert_eq!(delete(&root, first.id).unwrap().entries.len(), 1);
    }
    #[test]
    fn text_limits_status_and_performance_identity_are_checked() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private");
        let mut req = request();
        req.markdown.clear();
        assert_eq!(
            save(&root, req.clone()).unwrap_err().code,
            "ocr_history_invalid"
        );
        req.markdown = "é".repeat(MAX_MARKDOWN_BYTES / 2);
        save(&root, req.clone()).unwrap();
        req.markdown.push('a');
        assert_eq!(save(&root, req).unwrap_err().code, "ocr_history_invalid");
        let mut req = request();
        req.image_name = "x".repeat(1025);
        assert!(save(&root, req).is_err());
        let mut req = request();
        req.id = Uuid::nil();
        assert!(save(&root, req).is_err());
        let mut req = request();
        req.status = PerformanceStatus::Failed;
        assert!(save(&root, req).is_err());
        for mutation in 0..7 {
            let mut req = request();
            let mut p = measured(&req);
            match mutation {
                0 => p.instance_id = Uuid::nil(),
                1 => p.record.modality = PerformanceModality::Text,
                2 => p.record.request_id = runtime_types::RequestId::new(),
                3 => p.record.model_id = ModelId::new("other").unwrap(),
                4 => p.record.status = PerformanceStatus::Cancelled,
                5 => p.record.performance.as_mut().unwrap().timings.decode_us = u64::MAX,
                _ => p.record.error_code = Some(ErrorCode::NativeFailure),
            }
            req.performance = Some(p);
            assert!(save(&root, req).is_err());
        }
        let mut failed = request();
        failed.status = PerformanceStatus::Failed;
        failed.incomplete = true;
        failed.finish_reason = None;
        failed.error_code = Some("native_failure".into());
        save(&root, failed).unwrap();
    }
    #[test]
    fn malformed_store_and_busy_lock_preserve_existing_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private");
        save(&root, request()).unwrap();
        let held = lock(&root).unwrap();
        assert_eq!(
            lock_with_timeout(&root, std::time::Duration::from_millis(20))
                .unwrap_err()
                .code,
            "ocr_history_busy"
        );
        drop(held);
        token::atomic_replace_private(&root.join(FILE_NAME), b"{broken").unwrap();
        assert_eq!(
            save(&root, request()).unwrap_err().code,
            "ocr_history_corrupt"
        );
        assert!(delete(&root, Uuid::new_v4()).is_err());
        assert_eq!(std::fs::read(root.join(FILE_NAME)).unwrap(), b"{broken");
    }
    #[test]
    fn encoded_file_budget_rejects_escaped_content_without_replacing_existing_file() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private");
        save(&root, request()).unwrap();
        let before = std::fs::read(root.join(FILE_NAME)).unwrap();
        let entry = read(&root).unwrap().entries.remove(0);
        let mut entries = Vec::new();
        for _ in 0..22 {
            let mut copy = entry.clone();
            copy.id = Uuid::new_v4();
            copy.markdown = "\0".repeat(MAX_MARKDOWN_BYTES);
            entries.push(copy);
        }
        assert_eq!(
            write(&root, &Store { schema: 1, entries })
                .unwrap_err()
                .code,
            "ocr_history_limit"
        );
        assert_eq!(std::fs::read(root.join(FILE_NAME)).unwrap(), before);
    }
    #[cfg(unix)]
    #[test]
    fn linked_history_and_lock_fail_closed_without_touching_target() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private");
        save(&root, request()).unwrap();
        let victim = root.join("victim");
        token::write_private_new(&victim, b"untouched").unwrap();
        std::fs::remove_file(root.join(FILE_NAME)).unwrap();
        symlink(&victim, root.join(FILE_NAME)).unwrap();
        assert!(save(&root, request()).is_err());
        assert!(read(&root).is_err());
        assert_eq!(std::fs::read(&victim).unwrap(), b"untouched");
        std::fs::remove_file(root.join(FILE_NAME)).unwrap();
        std::fs::remove_file(root.join("ocr-history.lock")).unwrap();
        symlink(&victim, root.join("ocr-history.lock")).unwrap();
        assert!(save(&root, request()).is_err());
        assert_eq!(std::fs::read(victim).unwrap(), b"untouched");
    }
    #[test]
    fn request_and_nested_performance_reject_unknown_fields() {
        let mut value = serde_json::to_value(request()).unwrap();
        value["prompt"] = serde_json::json!("forbidden");
        assert!(serde_json::from_value::<OcrHistorySaveRequest>(value).is_err());
        let mut req = request();
        req.performance = Some(measured(&req));
        let mut value = serde_json::to_value(req).unwrap();
        value["performance"]["extra"] = serde_json::json!(true);
        assert!(serde_json::from_value::<OcrHistorySaveRequest>(value).is_err());
    }
    #[tokio::test]
    async fn incomplete_flags_closing_and_generation_gate_are_independent() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private");
        let desktop = bridge(&root);
        let _generation = desktop.work.lock().await;
        let mut req = request();
        req.finish_reason = Some(FinishReason::Length);
        assert_eq!(
            desktop
                .ocr_history_save(req.clone())
                .await
                .unwrap_err()
                .code,
            "ocr_history_invalid"
        );
        req.incomplete = true;
        desktop.ocr_history_save(req.clone()).await.unwrap();
        assert!(desktop.ocr_history_get(req.id).await.unwrap().incomplete);
        desktop.closing.store(true, Ordering::Release);
        assert_eq!(
            desktop.ocr_history_save(request()).await.unwrap_err().code,
            "ocr_history_closing"
        );
        assert_eq!(
            desktop.ocr_history_delete(req.id).await.unwrap_err().code,
            "ocr_history_closing"
        );
        assert_eq!(desktop.ocr_history_list().await.unwrap().entries.len(), 1);
    }
    #[test]
    fn oversized_or_unknown_schema_files_are_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private");
        save(&root, request()).unwrap();
        let path = root.join(FILE_NAME);
        let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.set_len(MAX_FILE_BYTES as u64 + 1).unwrap();
        drop(file);
        assert_eq!(
            save(&root, request()).unwrap_err().code,
            "ocr_history_corrupt"
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().len(),
            MAX_FILE_BYTES as u64 + 1
        );
        let unknown = br#"{"schema":2,"entries":[]}"#;
        token::atomic_replace_private(&path, unknown).unwrap();
        assert_eq!(
            save(&root, request()).unwrap_err().code,
            "ocr_history_corrupt"
        );
        assert_eq!(std::fs::read(path).unwrap(), unknown);
    }
    #[test]
    fn readers_and_atomic_writers_share_the_same_file_lock() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private");
        save(&root, request()).unwrap();
        let held = lock(&root).unwrap();
        let open_reader = token::open_private_file(&root.join(FILE_NAME)).unwrap();
        let writer_root = root.clone();
        let (done, completed) = std::sync::mpsc::channel();
        let writer = std::thread::spawn(move || {
            for _ in 0..12 {
                save(&writer_root, request()).unwrap();
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
                let store = read(&reader_root).unwrap();
                assert!(!store.entries.is_empty());
                assert!(store.entries.iter().all(OcrHistoryEntry::valid));
            }
        });
        completed
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        writer.join().unwrap();
        reader.join().unwrap();
        assert_eq!(read(&root).unwrap().entries.len(), 13);
    }
    #[tokio::test]
    async fn writes_queued_before_closing_are_rejected_after_the_persistence_gate() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private");
        let desktop = std::sync::Arc::new(bridge(&root));
        let held = desktop.persistence.clone().lock_owned().await;
        let queued = {
            let desktop = desktop.clone();
            tokio::spawn(async move { desktop.ocr_history_save(request()).await })
        };
        tokio::task::yield_now().await;
        desktop.closing.store(true, Ordering::Release);
        drop(held);
        assert_eq!(
            queued.await.unwrap().err().unwrap().code,
            "ocr_history_closing"
        );
        assert!(!root.join(FILE_NAME).exists());
    }
}
