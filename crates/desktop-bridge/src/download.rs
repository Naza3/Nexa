//! A bounded, explicit, single catalog download. No executable content, URL input,
//! implicit source failover, automatic loading, or registration is supported.
use crate::*;
use model_store::library::{ModelLibrary, download::DownloadFile};
use reqwest::Url;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    sync::{Arc, atomic::AtomicU8},
    time::Instant,
};
use tokio::sync::{Notify, mpsc};
use uuid::Uuid;

const BLOCK: usize = 64 * 1024;
const WHOLE_TIMEOUT: Duration = Duration::from_secs(2 * 60 * 60);
const READ_TIMEOUT: Duration = Duration::from_secs(30);
#[derive(Default)]
pub(crate) struct DownloadSlot {
    current: Option<Arc<DownloadTask>>,
    previous: Option<Arc<DownloadTask>>,
}
struct DownloadTask {
    state: Mutex<DownloadOperationState>,
    control: AtomicU8, // 0 transferring, 1 cancelled, 2 committing
    changed: Notify,
    cancelled: Notify,
    deadline: Instant,
}
impl DownloadSlot {
    fn task(&self, id: Uuid) -> Result<Arc<DownloadTask>> {
        self.current
            .iter()
            .chain(self.previous.iter())
            .find(|task| task.state.lock().unwrap().operation_id == id)
            .cloned()
            .ok_or_else(|| BridgeError::new("request_not_owned"))
    }
}
impl DownloadTask {
    fn check(&self) -> Result<()> {
        if self.control.load(Ordering::Acquire) == 1 {
            return Err(BridgeError::new("model_download_cancelled"));
        }
        if Instant::now() >= self.deadline {
            return Err(BridgeError::new("model_download_timeout"));
        }
        Ok(())
    }
    fn cancel(&self) {
        if self
            .control
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.cancelled.notify_waiters();
        }
    }
    fn phase(&self, phase: DownloadPhase) {
        self.state.lock().unwrap().phase = phase;
    }
    fn finish(&self, result: Result<bool>) {
        let mut state = self.state.lock().unwrap();
        state.terminal = true;
        state.phase = DownloadPhase::Finished;
        match result {
            Ok(cleaned) => {
                state.status = DownloadStatus::Completed;
                state.result = Some(DownloadResult {
                    saved: true,
                    registered: false,
                    file_name: state.file_name.clone(),
                    cleanup_warning: (!cleaned).then(|| "partial_cleanup_unconfirmed".into()),
                });
            }
            Err(error) => {
                state.status = if error.code == "model_download_cancelled" {
                    DownloadStatus::Cancelled
                } else {
                    DownloadStatus::Failed
                };
                state.error = Some(error);
            }
        }
        drop(state);
        self.changed.notify_waiters();
    }
}
fn store_error(error: runtime_types::RuntimeError) -> BridgeError {
    BridgeError::new(error.code.as_str())
}
fn hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
/// Exact hosts only: a provider subdomain is not an unrestricted redirect grant.
fn allowed_url(source: DownloadSource, url: &Url) -> bool {
    if url.scheme() != "https"
        || url.port().is_some_and(|p| p != 443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return false;
    }
    match source {
        DownloadSource::Modelscope => matches!(url.host_str(), Some("modelscope.cn")),
        DownloadSource::Huggingface => matches!(
            url.host_str(),
            Some("huggingface.co" | "us.aws.cdn.hf.co" | "cas-bridge.xethub.hf.co")
        ),
    }
}
fn catalog() -> Result<ModelCatalog> {
    let catalog: ModelCatalog = serde_json::from_str(include_str!("model-catalog.json"))
        .map_err(|_| BridgeError::new("model_catalog_invalid"))?;
    let mut ids = BTreeSet::new();
    if catalog.entries.is_empty() || catalog.entries.len() > 64 {
        return Err(BridgeError::new("model_catalog_invalid"));
    }
    for entry in &catalog.entries {
        if runtime_types::ModelId::new(&entry.catalog_id).is_err()
            || !ids.insert(entry.catalog_id.clone())
            || entry.file_name.len() > 200
            || entry.file_name.contains(['/', '\\', ':', '\0'])
            || !entry.file_name.ends_with(".gguf")
            || entry.size_bytes < 4
            || entry.size_bytes > model_store::library::MAX_MODEL_BYTES
            || !hex(&entry.sha256, 64)
            || entry.sources.is_empty()
            || entry.sources.len() > 2
        {
            return Err(BridgeError::new("model_catalog_invalid"));
        }
        let mut seen = Vec::new();
        for source in &entry.sources {
            let url =
                Url::parse(&source.url).map_err(|_| BridgeError::new("model_catalog_invalid"))?;
            if !hex(&source.revision, 40)
                || !source.url.contains(&source.revision)
                || !allowed_url(source.source, &url)
                || seen.contains(&source.source)
            {
                return Err(BridgeError::new("model_catalog_invalid"));
            }
            seen.push(source.source);
        }
    }
    Ok(catalog)
}
impl DesktopBridge {
    pub fn model_catalog(&self) -> Result<ModelCatalog> {
        catalog()
    }
    pub(crate) fn download_active(&self) -> bool {
        self.downloads
            .lock()
            .unwrap()
            .current
            .as_ref()
            .is_some_and(|t| !t.state.lock().unwrap().terminal)
    }
    pub fn download_start(self: &Arc<Self>, catalog_id: String) -> Result<DownloadOperationHandle> {
        self.open()?;
        // Admission is short and fail-fast; no shared async mutex is held across HTTP.
        let _work = self
            .work
            .try_lock()
            .map_err(|_| BridgeError::new("desktop_busy"))?;
        self.open()?;
        let entry = catalog()?
            .entries
            .into_iter()
            .find(|entry| entry.catalog_id == catalog_id)
            .ok_or_else(|| BridgeError::new("model_catalog_not_found"))?;
        if !self.root.exists() {
            return Err(BridgeError::new("model_directory_required"));
        }
        let lock = InstanceLock::try_acquire(&self.root)
            .map_err(|_| BridgeError::new("instance_unavailable"))?
            .ok_or_else(|| BridgeError::new("runtime_running"))?;
        if lock.has_discovery() {
            return Err(BridgeError::new("runtime_stop_unconfirmed"));
        }
        let source = settings::preferences(&self.root)?.download_source;
        let source_entry = entry
            .sources
            .iter()
            .find(|s| s.source == source)
            .cloned()
            .ok_or_else(|| BridgeError::new("model_download_source_unavailable"))?;
        let library = ModelLibrary::read(&self.root)
            .map_err(store_error)?
            .ok_or_else(|| BridgeError::new("model_directory_required"))?;
        if let Some(validate) = &self.directory_validator {
            validate(&library.directory)?;
        }
        let id = Uuid::new_v4();
        let destination =
            DownloadFile::create(&library, &entry.file_name, id).map_err(store_error)?;
        let task = Arc::new(DownloadTask {
            state: Mutex::new(DownloadOperationState {
                operation_id: id,
                catalog_id,
                source,
                file_name: entry.file_name.clone(),
                directory_id: library.directory_id,
                target_display_path: library.directory.to_string_lossy().into_owned(),
                downloaded_bytes: 0,
                total_bytes: entry.size_bytes,
                phase: DownloadPhase::Connecting,
                status: DownloadStatus::Running,
                terminal: false,
                result: None,
                error: None,
            }),
            control: AtomicU8::new(0),
            changed: Notify::new(),
            cancelled: Notify::new(),
            deadline: Instant::now() + WHOLE_TIMEOUT,
        });
        self.register_download_task(task.clone())?;
        tokio::spawn(async move {
            let result = run_download(task.clone(), entry, source_entry, destination, lock).await;
            // Writer and all protected handles have exited before publishing terminal.
            task.finish(result);
        });
        Ok(DownloadOperationHandle { operation_id: id })
    }
    fn register_download_task(&self, task: Arc<DownloadTask>) -> Result<()> {
        let mut slot = self.downloads.lock().unwrap();
        // Synchronize final admission with close_download's read of this same
        // slot. Once close has observed an empty slot, no late task may appear.
        if self.closing.load(Ordering::Acquire) {
            return Err(BridgeError::new("desktop_closing"));
        }
        slot.previous = slot.current.take();
        slot.current = Some(task);
        Ok(())
    }
    pub async fn download_next(&self, id: Uuid) -> Result<DownloadOperationState> {
        let _poll = self
            .download_poll
            .try_lock()
            .map_err(|_| BridgeError::new("consumer_busy"))?;
        let task = self.downloads.lock().unwrap().task(id)?;
        let changed = task.changed.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        if !task.state.lock().unwrap().terminal {
            let _ = tokio::time::timeout(Duration::from_secs(1), changed).await;
        }
        Ok(task.state.lock().unwrap().clone())
    }
    pub async fn download_cancel(&self, id: Uuid) -> Result<DownloadStopping> {
        let task = self.downloads.lock().unwrap().task(id)?;
        task.cancel();
        Ok(DownloadStopping {
            stopping: !task.state.lock().unwrap().terminal,
        })
    }
    pub(crate) async fn close_download(&self) -> Result<()> {
        let task = self.downloads.lock().unwrap().current.clone();
        let Some(task) = task else {
            return Ok(());
        };
        task.cancel();
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let changed = task.changed.notified();
                tokio::pin!(changed);
                changed.as_mut().enable();
                if task.state.lock().unwrap().terminal {
                    break;
                }
                changed.await;
            }
        })
        .await
        .map_err(|_| BridgeError::new("model_download_cleanup_unconfirmed"))
    }
}
enum Block {
    Bytes(Vec<u8>),
    Finish,
}
async fn run_download(
    task: Arc<DownloadTask>,
    entry: CatalogEntry,
    source: CatalogSource,
    destination: DownloadFile,
    lock: InstanceLock,
) -> Result<bool> {
    let (send, receive) = mpsc::channel(2);
    let writer_task = task.clone();
    let writer_entry = entry.clone();
    let writer = tokio::task::spawn_blocking(move || {
        let _lock = lock;
        write_download(&writer_task, &writer_entry, destination, receive)
    });
    let cancel = task.cancelled.notified();
    tokio::pin!(cancel);
    cancel.as_mut().enable();
    let transfer = async {
        task.check()?;
        transfer(&task, &entry, &source, &send).await
    };
    let result = tokio::select! {
        result = tokio::time::timeout(WHOLE_TIMEOUT, transfer) => result.unwrap_or_else(|_| Err(BridgeError::new("model_download_timeout"))),
        _ = cancel => Err(BridgeError::new("model_download_cancelled")),
    };
    // Dropping the producer wakes the blocking writer even if HTTP is cancelled.
    drop(send);
    let written = writer
        .await
        .map_err(|_| BridgeError::new("model_download_write_failed"))?;
    resolve_transfer(result, written)
}
fn resolve_transfer(transfer: Result<()>, written: Result<bool>) -> Result<bool> {
    // Actual publication is authoritative even if cancellation/deadline raced
    // with receiving Finish. Never describe a saved file as rolled back.
    match written {
        Ok(cleaned) => Ok(cleaned),
        Err(write_error) if write_error.code != "model_download_incomplete" => Err(write_error),
        Err(write_error) => match transfer {
            Err(network_error) => Err(network_error),
            Ok(()) => Err(write_error),
        },
    }
}
fn write_download(
    task: &DownloadTask,
    entry: &CatalogEntry,
    mut destination: DownloadFile,
    mut receive: mpsc::Receiver<Block>,
) -> Result<bool> {
    let mut hasher = Sha256::new();
    let mut written = 0u64;
    while let Some(block) = receive.blocking_recv() {
        task.check()?;
        match block {
            Block::Bytes(bytes) => {
                if bytes.len() > BLOCK {
                    return Err(BridgeError::new("model_download_size_mismatch"));
                }
                written = written
                    .checked_add(bytes.len() as u64)
                    .filter(|n| *n <= entry.size_bytes)
                    .ok_or_else(|| BridgeError::new("model_download_size_mismatch"))?;
                destination.write(&bytes).map_err(store_error)?;
                hasher.update(&bytes);
                task.state.lock().unwrap().downloaded_bytes = written;
            }
            Block::Finish => {
                task.phase(DownloadPhase::Verifying);
                if written != entry.size_bytes || format!("{:x}", hasher.finalize()) != entry.sha256
                {
                    return Err(BridgeError::new("model_download_identity_mismatch"));
                }
                task.check()?;
                task.control
                    .compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire)
                    .map_err(|_| BridgeError::new("model_download_cancelled"))?;
                task.phase(DownloadPhase::Committing);
                return destination.publish().map_err(store_error);
            }
        }
    }
    task.check()?;
    Err(BridgeError::new("model_download_incomplete"))
}
async fn transfer(
    task: &DownloadTask,
    entry: &CatalogEntry,
    source: &CatalogSource,
    send: &mpsc::Sender<Block>,
) -> Result<()> {
    let client = reqwest::Client::builder()
        .https_only(true)
        .no_proxy()
        .referer(false)
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(READ_TIMEOUT)
        .timeout(WHOLE_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .build()
        .map_err(|_| BridgeError::new("model_download_network_failed"))?;
    let mut url = Url::parse(&source.url).map_err(|_| BridgeError::new("model_catalog_invalid"))?;
    for hop in 0..=5 {
        task.check()?;
        if !allowed_url(source.source, &url) {
            return Err(BridgeError::new("model_download_redirect_rejected"));
        }
        let response = client
            .get(url.clone())
            .header(reqwest::header::ACCEPT_ENCODING, "identity")
            .send()
            .await
            .map_err(|_| BridgeError::new("model_download_network_failed"))?;
        if response.status().is_redirection() {
            if hop == 5 {
                return Err(BridgeError::new("model_download_redirect_rejected"));
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .filter(|v| v.len() <= 16384)
                .ok_or_else(|| BridgeError::new("model_download_redirect_rejected"))?;
            url = url
                .join(location)
                .map_err(|_| BridgeError::new("model_download_redirect_rejected"))?;
            continue;
        }
        return transfer_response(task, entry, response, send).await;
    }
    Err(BridgeError::new("model_download_redirect_rejected"))
}

async fn transfer_response(
    task: &DownloadTask,
    entry: &CatalogEntry,
    response: reqwest::Response,
    send: &mpsc::Sender<Block>,
) -> Result<()> {
    if response.status() != reqwest::StatusCode::OK {
        return Err(BridgeError::new("model_download_http_failed"));
    }
    if response
        .content_length()
        .is_some_and(|n| n != entry.size_bytes)
        || response
            .headers()
            .get(reqwest::header::CONTENT_ENCODING)
            .is_some_and(|v| v != "identity")
    {
        return Err(BridgeError::new("model_download_size_mismatch"));
    }
    task.phase(DownloadPhase::Downloading);
    let mut response = response;
    let mut received = 0u64;
    while let Some(bytes) = response
        .chunk()
        .await
        .map_err(|_| BridgeError::new("model_download_network_failed"))?
    {
        task.check()?;
        received = received
            .checked_add(bytes.len() as u64)
            .filter(|n| *n <= entry.size_bytes)
            .ok_or_else(|| BridgeError::new("model_download_size_mismatch"))?;
        for bytes in bytes.chunks(BLOCK) {
            send.send(Block::Bytes(bytes.to_vec()))
                .await
                .map_err(|_| BridgeError::new("model_download_write_failed"))?;
        }
    }
    if received != entry.size_bytes {
        return Err(BridgeError::new("model_download_size_mismatch"));
    }
    send.send(Block::Finish)
        .await
        .map_err(|_| BridgeError::new("model_download_write_failed"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn task_for(entry: &CatalogEntry) -> Arc<DownloadTask> {
        Arc::new(DownloadTask {
            state: Mutex::new(DownloadOperationState {
                operation_id: Uuid::new_v4(),
                catalog_id: entry.catalog_id.clone(),
                source: DownloadSource::Modelscope,
                file_name: entry.file_name.clone(),
                directory_id: Uuid::new_v4(),
                target_display_path: "fixture".into(),
                downloaded_bytes: 0,
                total_bytes: entry.size_bytes,
                phase: DownloadPhase::Connecting,
                status: DownloadStatus::Running,
                terminal: false,
                result: None,
                error: None,
            }),
            control: AtomicU8::new(0),
            changed: Notify::new(),
            cancelled: Notify::new(),
            deadline: Instant::now() + Duration::from_secs(5),
        })
    }
    async fn fixture_response(status: &str, headers: &str, body: Vec<u8>) -> reqwest::Response {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let header = format!("HTTP/1.1 {status}\r\nConnection: close\r\n{headers}\r\n");
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = [0; 4096];
            let _ = stream.read(&mut request);
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(&body);
        });
        // Loopback HTTP exists only in this bounded fixture, never the production policy.
        reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(format!("http://{address}/fixture"))
            .send()
            .await
            .unwrap()
    }
    #[tokio::test]
    async fn bounded_http_fixture_checks_headers_lengths_and_backpressure_chunks() {
        for (status, headers, size, body, success) in [
            (
                "200 OK",
                "",
                (BLOCK * 3 + 7) as u64,
                vec![b'x'; BLOCK * 3 + 7],
                true,
            ),
            (
                "200 OK",
                "Content-Length: 5\r\n",
                4,
                b"12345".to_vec(),
                false,
            ),
            ("200 OK", "", 4, b"12345".to_vec(), false),
            ("200 OK", "", 5, b"1234".to_vec(), false),
            (
                "200 OK",
                "Content-Encoding: gzip\r\n",
                4,
                b"1234".to_vec(),
                false,
            ),
            ("403 Forbidden", "", 4, b"1234".to_vec(), false),
        ] {
            let response = fixture_response(status, headers, body).await;
            let mut entry = catalog().unwrap().entries.remove(0);
            entry.size_bytes = size;
            let task = task_for(&entry);
            let (send, mut receive) = mpsc::channel(2);
            let producer = async {
                let result = transfer_response(&task, &entry, response, &send).await;
                drop(send);
                result
            };
            let consumer = async {
                let mut count = 0;
                let mut finished = false;
                while let Some(block) = receive.recv().await {
                    match block {
                        Block::Bytes(bytes) => {
                            assert!(bytes.len() <= BLOCK);
                            count += bytes.len();
                        }
                        Block::Finish => {
                            assert!(!finished);
                            finished = true;
                        }
                    }
                }
                (count, finished)
            };
            let (result, (count, finished)) = tokio::join!(producer, consumer);
            assert_eq!(result.is_ok(), success);
            assert_eq!(finished, success);
            if success {
                assert_eq!(count as u64, size);
            }
        }
    }
    #[tokio::test]
    async fn active_task_keeps_snapshot_responsive_and_close_waits_for_terminal() {
        let root = tempfile::tempdir().unwrap();
        let bridge = DesktopBridge::new(
            root.path().join("private"),
            root.path().join(if cfg!(windows) {
                "ai-runtime.exe"
            } else {
                "ai-runtime"
            }),
        )
        .unwrap();
        let task = task_for(&catalog().unwrap().entries.remove(0));
        bridge.downloads.lock().unwrap().current = Some(task.clone());
        assert!(matches!(
            bridge.snapshot().await.unwrap().connection,
            ConnectionState::Stopped
        ));
        assert_eq!(
            bridge
                .settings_save(DesktopPreferences::default())
                .await
                .unwrap_err()
                .code,
            "model_download_active"
        );
        assert!(bridge.work.try_lock().is_ok());
        let task_copy = task.clone();
        let complete = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            assert_eq!(task_copy.control.load(Ordering::Acquire), 1);
            task_copy.finish(Err(BridgeError::new("model_download_cancelled")));
        });
        bridge.close().await.unwrap();
        complete.await.unwrap();
        assert!(task.state.lock().unwrap().terminal);
    }
    #[tokio::test]
    async fn close_before_final_admission_rejects_late_task_and_waits_for_work() {
        let temp = tempfile::tempdir().unwrap();
        let bridge = Arc::new(
            DesktopBridge::new(
                temp.path().join("private"),
                temp.path().join(if cfg!(windows) {
                    "ai-runtime.exe"
                } else {
                    "ai-runtime"
                }),
            )
            .unwrap(),
        );
        // Pause an admitted start after open/work acquisition but before its
        // final slot registration, exactly the slow-directory-validation gap.
        bridge.open().unwrap();
        let work = bridge.work.clone().try_lock_owned().unwrap();
        bridge.open().unwrap();
        let closing_bridge = bridge.clone();
        let close = tokio::spawn(async move { closing_bridge.close().await });
        while !bridge.closing.load(Ordering::Acquire) {
            tokio::task::yield_now().await;
        }
        assert!(!close.is_finished());
        let task = task_for(&catalog().unwrap().entries.remove(0));
        assert_eq!(
            bridge.register_download_task(task).unwrap_err().code,
            "desktop_closing"
        );
        drop(work);
        close.await.unwrap().unwrap();
        assert!(!bridge.download_active());
    }
    #[test]
    fn published_result_wins_racing_transfer_cancel_or_timeout() {
        for code in ["model_download_cancelled", "model_download_timeout"] {
            assert!(resolve_transfer(Err(BridgeError::new(code)), Ok(true)).unwrap());
            assert!(!resolve_transfer(Err(BridgeError::new(code)), Ok(false)).unwrap());
            assert_eq!(
                resolve_transfer(
                    Err(BridgeError::new(code)),
                    Err(BridgeError::new("model_download_incomplete"))
                )
                .unwrap_err()
                .code,
                code
            );
        }
    }
    #[test]
    fn commit_cancel_race_and_cleanup_warning_keep_saved_truthful() {
        let entry = catalog().unwrap().entries.remove(0);
        let task = task_for(&entry);
        task.control.store(2, Ordering::Release);
        task.cancel();
        assert_eq!(task.control.load(Ordering::Acquire), 2);
        task.finish(Ok(false));
        let state = task.state.lock().unwrap();
        assert_eq!(state.status, DownloadStatus::Completed);
        let result = state.result.as_ref().unwrap();
        assert!(result.saved);
        assert!(!result.registered);
        assert_eq!(
            result.cleanup_warning.as_deref(),
            Some("partial_cleanup_unconfirmed")
        );
        let cancelled = task_for(&entry);
        cancelled.cancel();
        assert_eq!(
            cancelled.check().unwrap_err().code,
            "model_download_cancelled"
        );
    }
    #[cfg(windows)]
    #[test]
    fn windows_fixture_bytes_verify_size_hash_cancel_and_publish_without_registration() {
        for mode in ["success", "hash", "short", "cancel", "oversize"] {
            let root = tempfile::tempdir().unwrap();
            let models = root.path().join("models");
            fs::create_dir(&models).unwrap();
            let scan = model_store::library::scan_directory(
                root.path(),
                &models,
                None,
                &model_store::library::ScanControl::default(),
            )
            .unwrap();
            let library = scan.library().unwrap().clone();
            drop(scan);
            let body = b"GGUF bounded fixture bytes";
            let mut entry = catalog().unwrap().entries.remove(0);
            entry.size_bytes = body.len() as u64;
            entry.sha256 = format!("{:x}", Sha256::digest(body));
            if mode == "hash" {
                entry.sha256 = "0".repeat(64);
            }
            if mode == "short" {
                entry.size_bytes += 1;
            }
            if mode == "oversize" {
                entry.size_bytes -= 1;
            }
            let task = task_for(&entry);
            if mode == "cancel" {
                task.cancel();
            }
            let id = task.state.lock().unwrap().operation_id;
            let destination = DownloadFile::create(&library, &entry.file_name, id).unwrap();
            let (send, receive) = mpsc::channel(2);
            send.try_send(Block::Bytes(body.to_vec())).unwrap();
            send.try_send(Block::Finish).unwrap();
            drop(send);
            let result = write_download(&task, &entry, destination, receive);
            assert_eq!(result.is_ok(), mode == "success", "{mode}");
            assert_eq!(
                models.join(&entry.file_name).exists(),
                mode == "success",
                "{mode}"
            );
            assert!(
                !models.join(format!(".nexa-download-{id}.part")).exists(),
                "{mode}"
            );
            assert!(
                !root
                    .path()
                    .join(model_store::library::LIBRARY_FILE)
                    .exists()
            );
            if mode == "success" {
                assert_eq!(
                    task.state.lock().unwrap().downloaded_bytes,
                    body.len() as u64
                );
            }
        }
    }
    #[test]
    fn pinned_catalog_has_both_sources_without_granting_validation() {
        let catalog = catalog().unwrap();
        assert!(catalog.entries.len() >= 2);
        for item in catalog.entries {
            assert_eq!(item.sources.len(), 2);
            assert!(
                item.sources
                    .iter()
                    .any(|s| s.source == DownloadSource::Modelscope)
            );
            assert!(
                item.sources
                    .iter()
                    .any(|s| s.source == DownloadSource::Huggingface)
            );
        }
    }
    #[test]
    fn redirects_are_exact_https_provider_hosts_without_credentials() {
        for bad in [
            "http://huggingface.co/file",
            "https://huggingface.co.evil.invalid/file",
            "https://127.0.0.1/file",
            "https://user:pass@huggingface.co/file",
            "https://huggingface.co:444/file",
            "https://evil.hf.co/file",
            "https://modelscope.cn/file",
        ] {
            assert!(
                !allowed_url(DownloadSource::Huggingface, &Url::parse(bad).unwrap()),
                "{bad}"
            );
        }
        assert!(allowed_url(
            DownloadSource::Huggingface,
            &Url::parse("https://us.aws.cdn.hf.co/file?signature=opaque").unwrap()
        ));
        assert!(!allowed_url(
            DownloadSource::Modelscope,
            &Url::parse("https://huggingface.co/file").unwrap()
        ));
    }
    #[test]
    fn old_preferences_default_to_modelscope_and_invalid_source_rejected() {
        let mut value = serde_json::to_value(DesktopPreferences::default()).unwrap();
        value.as_object_mut().unwrap().remove("download_source");
        assert_eq!(
            serde_json::from_value::<DesktopPreferences>(value.clone())
                .unwrap()
                .download_source,
            DownloadSource::Modelscope
        );
        value["download_source"] = json!("arbitrary-url");
        assert!(serde_json::from_value::<DesktopPreferences>(value).is_err());
    }
}
