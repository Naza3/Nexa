#![cfg_attr(not(target_os = "android"), allow(dead_code))]
use crate::output::Operation;
use mnn_model_store::MnnModelStore;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write,
    os::{
        fd::{AsRawFd, FromRawFd, IntoRawFd, RawFd},
        unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
pub const PURPOSE: &str = "android_device_verification";
pub const MODEL: &str = "qwen3-0.6b-mnn";
pub const REPORT_LIMIT: usize = 2 * 1024 * 1024;
pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Clone, Serialize)]
pub struct Error {
    pub code: String,
    pub retry: &'static str,
}
pub fn failure(code: &str) -> Error {
    Error {
        code: code.into(),
        retry: match code {
            "stale_handle" => "reopen_required",
            "cleanup_unconfirmed" | "durability_unconfirmed" => "process_restart_required",
            _ => "allowed",
        },
    }
}
pub fn runtime_error(e: runtime_types::RuntimeError) -> Error {
    let code = serde_json::to_value(e.code).unwrap();
    failure(code.as_str().unwrap_or("native_failure"))
}
pub fn reply(result: Result<Value>) -> String {
    match result {
        Ok(value) => json!({"ok":true,"value":value}),
        Err(error) => json!({"ok":false,"error":error}),
    }
    .to_string()
}
pub fn timestamp() -> String {
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as libc::time_t;
    let mut tm = std::mem::MaybeUninit::<libc::tm>::uninit();
    unsafe {
        if libc::gmtime_r(&t, tm.as_mut_ptr()).is_null() {
            return "unknown".into();
        }
        let tm = tm.assume_init();
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec
        )
    }
}
pub fn digest(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}
pub fn private_dir(path: &Path) -> Result<()> {
    if !path.exists() {
        fs::create_dir(path).map_err(|_| failure("storage_failure"))?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| failure("storage_failure"))?;
    }
    let m = fs::symlink_metadata(path).map_err(|_| failure("storage_failure"))?;
    if !m.is_dir() || m.file_type().is_symlink() || m.mode() & 0o777 != 0o700 {
        return Err(failure("invalid_private_root"));
    }
    Ok(())
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Device {
    schema_version: u32,
    manufacturer: String,
    model: String,
    soc_manufacturer: Option<String>,
    soc_model: Option<String>,
    android_release: String,
    sdk_int: u32,
    security_patch: Option<String>,
    supported_abis: Vec<String>,
}
impl Device {
    pub fn parse(s: &str) -> Result<Value> {
        if s.len() > 16384 {
            return Err(failure("invalid_argument"));
        }
        let d: Self = serde_json::from_str(s).map_err(|_| failure("invalid_argument"))?;
        if d.schema_version != 1
            || !(28..=1000).contains(&d.sdk_int)
            || d.supported_abis.len() > 8
            || d.supported_abis.iter().any(|v| v.len() > 32)
            || [&d.manufacturer, &d.model, &d.android_release]
                .iter()
                .any(|v| v.len() > 128)
            || [&d.soc_manufacturer, &d.soc_model, &d.security_patch]
                .iter()
                .any(|v| v.as_ref().is_some_and(|v| v.len() > 128))
        {
            return Err(failure("invalid_argument"));
        }
        let mut v = serde_json::to_value(d).unwrap();
        v["page_size"] = json!(unsafe { libc::sysconf(libc::_SC_PAGESIZE) });
        v["memory_thermal"] = json!({"status":"not_run","reason":"sampling_not_implemented"});
        Ok(v)
    }
}
pub struct Selection {
    pub id: String,
    pub files: BTreeMap<String, File>,
    created: Instant,
}
#[derive(Clone)]
pub struct Report {
    pub operation_id: String,
    pub id: String,
    pub path: PathBuf,
    pub size: usize,
    pub sha: String,
}
struct ReportToken {
    id: String,
    created: Instant,
    report: Report,
}
pub struct State {
    visible: bool,
    lifecycle_sequence: i64,
    pub ready: bool,
    pub unavailable: bool,
    pub active: Option<Arc<Operation>>,
    selection: Option<Selection>,
    consumed: Option<String>,
    pub last_terminal: Option<Value>,
    pub report: Option<Report>,
    report_token: Option<ReportToken>,
}
pub struct Host {
    pub epoch: String,
    pub root: PathBuf,
    pub device: Value,
    pub store: Mutex<MnnModelStore>,
    pub state: Mutex<State>,
}
static HOST: OnceLock<Arc<Host>> = OnceLock::new();
fn model_id() -> runtime_types::ModelId {
    runtime_types::ModelId::new(MODEL).unwrap()
}
pub fn bootstrap(root: &str, device: &str) -> Result<Value> {
    if root.len() > 4096 {
        return Err(failure("invalid_private_root"));
    }
    let root = PathBuf::from(root);
    private_dir(&root)?;
    if !root.is_absolute()
        || fs::canonicalize(&root).map_err(|_| failure("invalid_private_root"))? != root
    {
        return Err(failure("invalid_private_root"));
    }
    if let Some(h) = HOST.get() {
        return if h.root == root {
            Ok(json!({"host_epoch":h.epoch}))
        } else {
            Err(failure("invalid_private_root"))
        };
    }
    let device = Device::parse(device)?;
    private_dir(&root.join("models"))?;
    private_dir(&root.join("reports"))?;
    private_dir(&root.join("inbox"))?;
    clean_inbox(&root.join("inbox"))?;
    let store = MnnModelStore::open(root.join("models")).map_err(runtime_error)?;
    let ready = store
        .snapshot()
        .map_err(runtime_error)?
        .resolve_candidate(&model_id())
        .is_ok();
    recover_report_temps(&root.join("reports"))?;
    let pending = root.join("reports/pending.json");
    if pending.exists() {
        let v = read_private_json(&pending, 4096)?;
        let id = v["operation_id"]
            .as_str()
            .ok_or_else(|| failure("report_unavailable"))?;
        if uuid::Uuid::parse_str(id).is_err() {
            return Err(failure("report_unavailable"));
        }
        let report_id = uuid::Uuid::new_v4().to_string();
        let terminal = json!({"operation_id":id,"outcome":"interrupted","cleanup":"process_ended_unknown","report_id":report_id,"error":{"code":"process_interrupted","retry":"allowed"}});
        let report = json!({"schema_version":1,"purpose":PURPOSE,"research_only":true,"production_admitted":false,"report_id":report_id,"operation_id":id,"suite_id":v["suite_id"],"suite_sha256":null,"started_at_utc":v["started_at_utc"],"finished_at_utc":timestamp(),"terminal":terminal,"build":build(),"device":device,"model":{"model_id":MODEL,"artifact_digest":mnn_model_store::candidate_digest(),"identity":report_model_identity()},"profile":null,"cases":[],"coverage":{"process_restart":"interrupted_record_recovered","previous_native_cleanup":"unknown","missing_case_records":"not_reconstructed"}});
        atomic_json(&root.join(format!("reports/{report_id}.json")), &report)?;
        atomic_json(
            &root.join("reports/latest-index.json"),
            &json!({"report_id":report_id}),
        )?;
        fs::remove_file(pending).map_err(|_| failure("storage_failure"))?;
        File::open(root.join("reports"))
            .and_then(|f| f.sync_all())
            .map_err(|_| failure("storage_failure"))?;
    }
    let (last_terminal, report) = restore_latest(&root.join("reports"))?;
    let h = Arc::new(Host {
        epoch: uuid::Uuid::new_v4().to_string(),
        root,
        device,
        store: Mutex::new(store),
        state: Mutex::new(State {
            visible: false,
            lifecycle_sequence: -1,
            ready,
            unavailable: false,
            active: None,
            selection: None,
            consumed: None,
            last_terminal,
            report,
            report_token: None,
        }),
    });
    HOST.set(h).map_err(|_| failure("busy"))?;
    Ok(json!({"host_epoch":HOST.get().unwrap().epoch}))
}
fn report_model_identity() -> Value {
    let mut identity = mnn_model_store::candidate_identity();
    identity["artifact_source"]
        .as_object_mut()
        .unwrap()
        .remove("source_uri");
    for file in identity["files"].as_array_mut().unwrap() {
        let name = file.as_object_mut().unwrap().remove("path").unwrap();
        file["filename"] = name;
    }
    identity
}
fn read_private_json(path: &Path, limit: usize) -> Result<Value> {
    use std::io::Read;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|_| failure("report_unavailable"))?;
    let m = file.metadata().map_err(|_| failure("report_unavailable"))?;
    if !m.is_file() || m.nlink() != 1 || m.len() > limit as u64 {
        return Err(failure("report_unavailable"));
    }
    let mut bytes = vec![];
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| failure("report_unavailable"))?;
    if bytes.len() > limit {
        return Err(failure("report_unavailable"));
    }
    serde_json::from_slice(&bytes).map_err(|_| failure("report_unavailable"))
}
fn recover_report_temps(root: &Path) -> Result<()> {
    for item in fs::read_dir(root).map_err(|_| failure("storage_failure"))? {
        let item = item.map_err(|_| failure("storage_failure"))?;
        let path = item.path();
        let name = item
            .file_name()
            .into_string()
            .map_err(|_| failure("report_unavailable"))?;
        let m = fs::symlink_metadata(&path).map_err(|_| failure("storage_failure"))?;
        if !m.is_file() || m.file_type().is_symlink() || m.nlink() != 1 || m.mode() & 0o777 != 0o600
        {
            return Err(failure("report_unavailable"));
        }
        if let Some(stem) = name.strip_suffix(".tmp") {
            if stem != "pending" && stem != "latest-index" && uuid::Uuid::parse_str(stem).is_err() {
                return Err(failure("report_unavailable"));
            }
            fs::remove_file(path).map_err(|_| failure("storage_failure"))?;
        } else if name != "pending.json"
            && name != "latest-index.json"
            && name
                .strip_suffix(".json")
                .is_none_or(|stem| uuid::Uuid::parse_str(stem).is_err())
        {
            return Err(failure("report_unavailable"));
        }
    }
    Ok(())
}
fn restore_latest(root: &Path) -> Result<(Option<Value>, Option<Report>)> {
    if !root.join("latest-index.json").exists() {
        return Ok((None, None));
    }
    let index = read_private_json(&root.join("latest-index.json"), 4096)?;
    let id = index["report_id"]
        .as_str()
        .ok_or_else(|| failure("report_unavailable"))?;
    if uuid::Uuid::parse_str(id).is_err() {
        return Err(failure("report_unavailable"));
    }
    let path = root.join(format!("{id}.json"));
    let v = read_private_json(&path, REPORT_LIMIT)?;
    let operation = v["operation_id"]
        .as_str()
        .ok_or_else(|| failure("report_unavailable"))?;
    if uuid::Uuid::parse_str(operation).is_err()
        || v["report_id"] != id
        || v["production_admitted"] != false
        || v["purpose"] != PURPOSE
    {
        return Err(failure("report_unavailable"));
    }
    let data = fs::read(&path).map_err(|_| failure("report_unavailable"))?;
    Ok((
        Some(v["terminal"].clone()),
        Some(Report {
            operation_id: operation.into(),
            id: id.into(),
            path,
            size: data.len(),
            sha: digest(&data),
        }),
    ))
}
pub fn host(epoch: &str) -> Result<Arc<Host>> {
    let h = HOST.get().ok_or_else(|| failure("unavailable"))?;
    if epoch != h.epoch {
        return Err(failure("stale_handle"));
    }
    Ok(h.clone())
}
pub fn open() -> Result<Value> {
    let h = HOST.get().ok_or_else(|| failure("unavailable"))?;
    snapshot(&h.epoch)
}
pub fn snapshot(epoch: &str) -> Result<Value> {
    let h = host(epoch)?;
    let s = h.state.lock().unwrap();
    let state = if s.unavailable {
        "cleanup_unconfirmed"
    } else if let Some(o) = &s.active {
        if o.stopped() {
            "stopping"
        } else if o.kind == "import" {
            "importing"
        } else {
            "running"
        }
    } else {
        "idle"
    };
    Ok(
        json!({"schema_version":1,"host_epoch":h.epoch,"purpose":PURPOSE,"research_only":true,"production_admitted":false,"build":build(),"device":h.device,"candidate":{"model_id":MODEL,"artifact_digest":mnn_model_store::candidate_digest(),"state":if s.unavailable{"durability_unconfirmed"}else if s.active.as_ref().is_some_and(|o|o.kind=="import"){"importing"}else if s.ready{"ready"}else{"absent"}},"host_state":state,"active_operation":s.active.as_ref().map(|o|operation_ref(o)),"latest_terminal":s.last_terminal,"last_error":null}),
    )
}
pub fn build() -> Value {
    let identity=mnn_adapter::build_identity().ok().map(|b|json!({"upstream_commit":b.upstream_commit,"patch_sha256":b.patch_sha256,"policy_sha256":b.policy_sha256,"artifact_manifest_sha256":b.artifact_manifest_sha256,"target":b.target,"compiler":b.compiler}));
    json!({"source_commit":env!("NEXA_SOURCE_COMMIT"),"source_dirty":env!("NEXA_SOURCE_DIRTY").parse::<bool>().ok(),"version":env!("NEXA_APP_VERSION"),"build_mode":env!("NEXA_BUILD_MODE"),"signing_kind":"internal_debug_key","application_id":"io.github.naza3.nexa.verifier","apk_sha256":null,"bridge_sha256":null,"hash_reason":"external_package_audit_required","frb":"2.13.0","flutter":"3.47.6","rust":"1.98.1","ndk":"30.0.16248370","native_identity":identity})
}
pub fn visibility(epoch: &str, visible: bool, sequence: i64) -> Result<Value> {
    let h = host(epoch)?;
    let mut s = h.state.lock().unwrap();
    if sequence < 0 {
        return Err(failure("invalid_argument"));
    }
    if sequence > s.lifecycle_sequence {
        s.lifecycle_sequence = sequence;
        s.visible = visible;
        if !visible {
            s.selection.take();
            if let Some(o) = &s.active {
                o.stop("backgrounded");
            }
        }
    }
    Ok(
        json!({"host_epoch":h.epoch,"lifecycle_sequence":s.lifecycle_sequence,"state":if s.unavailable{"cleanup_unconfirmed"}else if s.active.is_some(){"stopping"}else{"idle"}}),
    )
}
pub fn register(
    epoch: &str,
    names: Vec<String>,
    lengths: Vec<i64>,
    fds: Vec<RawFd>,
) -> Result<Value> {
    let h = host(epoch)?;
    let lifecycle_sequence = {
        let s = h.state.lock().unwrap();
        gate(&s)?;
        s.lifecycle_sequence
    };
    // Provider metadata calls below are deliberately outside the control lock.
    if names.len() != 5 || lengths.len() != 5 || fds.len() != 5 {
        return Err(failure("invalid_argument"));
    }
    let specs = mnn_model_store::candidate_identity()["files"]
        .as_array()
        .unwrap()
        .clone();
    let mut files = BTreeMap::new();
    for ((name, length), fd) in names.into_iter().zip(lengths).zip(fds) {
        let spec = specs
            .iter()
            .find(|v| v["path"] == name)
            .ok_or_else(|| failure("unsupported_source"))?;
        if length < 0
            || spec["size_bytes"].as_u64() != Some(length as u64)
            || files.contains_key(&name)
        {
            return Err(failure("unsupported_source"));
        }
        let duplicate = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0) };
        if duplicate < 0 {
            return Err(failure("unsupported_source"));
        }
        let file = unsafe { File::from_raw_fd(duplicate) };
        let flags = unsafe { libc::fcntl(duplicate, libc::F_GETFL) };
        let m = file.metadata().map_err(|_| failure("unsupported_source"))?;
        if flags < 0
            || flags & libc::O_ACCMODE != libc::O_RDONLY
            || !m.is_file()
            || m.len() != length as u64
            || unsafe { libc::lseek(duplicate, 0, libc::SEEK_CUR) } < 0
        {
            return Err(failure("unsupported_source"));
        }
        files.insert(name, file);
    }
    publish_selection(&h.state, lifecycle_sequence, files)
}
fn publish_selection(
    state: &Mutex<State>,
    observed_sequence: i64,
    files: BTreeMap<String, File>,
) -> Result<Value> {
    let id = uuid::Uuid::new_v4().to_string();
    let mut s = state.lock().unwrap();
    gate(&s)?;
    // Even a background->foreground round trip invalidates the captured grant.
    if s.lifecycle_sequence != observed_sequence {
        return Err(failure("selection_expired"));
    }
    s.selection = Some(Selection {
        id: id.clone(),
        files,
        created: Instant::now(),
    });
    Ok(json!({"selection_token":id}))
}

pub fn cancel_selection(epoch: &str, token: &str) -> Result<Value> {
    let h = host(epoch)?;
    let mut s = h.state.lock().unwrap();
    if s.selection.as_ref().is_some_and(|v| v.id == token) {
        s.selection.take();
    }
    Ok(json!({"state":if s.consumed.as_deref()==Some(token){"consumed"}else{"released"}}))
}
fn gate(s: &State) -> Result<()> {
    if s.unavailable {
        return Err(failure("cleanup_unconfirmed"));
    }
    if s.active.is_some() {
        return Err(failure("busy"));
    }
    if !s.visible {
        return Err(failure("backgrounded"));
    }
    Ok(())
}
pub fn operation_ref(o: &Operation) -> Value {
    json!({"operation_id":o.id,"kind":o.kind,"state":if o.stopped(){"stopping"}else{"running"}})
}
pub fn start_import(epoch: &str, token: &str) -> Result<Value> {
    let h = host(epoch)?;
    let mut s = h.state.lock().unwrap();
    gate(&s)?;
    if s.ready {
        return Err(failure("already_exists"));
    }
    if s.consumed.as_deref() == Some(token) {
        return Err(failure("selection_consumed"));
    }
    if !s
        .selection
        .as_ref()
        .is_some_and(|v| v.id == token && v.created.elapsed() < Duration::from_secs(600))
    {
        return Err(failure("selection_expired"));
    }
    let selection = s.selection.take().unwrap();
    s.consumed = Some(token.into());
    let o = Operation::new("import", None);
    s.active = Some(o.clone());
    drop(s);
    let result = operation_ref(&o);
    launch(h, o, move |h, o| import(h, o, selection));
    Ok(result)
}
pub fn start_suite(epoch: &str, suite: &str) -> Result<Value> {
    let h = host(epoch)?;
    if !matches!(suite, "b3a_smoke_v1" | "b3a_safety_v1") {
        return Err(failure("invalid_suite"));
    }
    let mut s = h.state.lock().unwrap();
    gate(&s)?;
    if !s.ready {
        return Err(failure("model_not_found"));
    }
    let o = Operation::new("suite", Some(suite.into()));
    s.active = Some(o.clone());
    drop(s);
    let result = operation_ref(&o);
    launch(h, o, crate::runner::run);
    Ok(result)
}
pub fn start_remove(epoch: &str) -> Result<Value> {
    let h = host(epoch)?;
    let mut s = h.state.lock().unwrap();
    gate(&s)?;
    let o = Operation::new("remove", None);
    s.active = Some(o.clone());
    drop(s);
    let result = operation_ref(&o);
    launch(h, o, |h, o| {
        o.emit(
            "progress",
            json!({"stage":"removing","completed_bytes":null,"total_bytes":null}),
        );
        h.store
            .lock()
            .unwrap()
            .remove_generation(&model_id())
            .map_err(runtime_error)?;
        Ok(vec![])
    });
    Ok(result)
}
pub fn poll(epoch: &str, id: &str, ack: Option<u64>) -> Result<Value> {
    let h = host(epoch)?;
    let o = {
        let s = h.state.lock().unwrap();
        s.active.clone().filter(|o| o.id == id)
    };
    if let Some(o) = o {
        let r = o.next(ack)?;
        o.heartbeat();
        if !r["terminal"].is_null() {
            let mut s = h.state.lock().unwrap();
            if s.active.as_ref().is_some_and(|active| active.id == id) {
                s.active = None;
            }
        }
        return Ok(r);
    }
    let s = h.state.lock().unwrap();
    if s.last_terminal
        .as_ref()
        .is_some_and(|t| t["operation_id"] == id)
    {
        return Ok(json!({"operation_id":id,"event":null,"terminal":s.last_terminal}));
    }
    Err(failure("stale_handle"))
}
pub fn cancel(epoch: &str, id: &str) -> Result<Value> {
    let h = host(epoch)?;
    let s = h.state.lock().unwrap();
    if let Some(o) = s.active.as_ref().filter(|o| o.id == id) {
        o.stop("request_cancelled");
        return Ok(operation_ref(o));
    }
    Err(failure("stale_handle"))
}
fn atomic_json(path: &Path, value: &Value) -> Result<()> {
    let data = serde_json::to_vec(value).map_err(|_| failure("report_unavailable"))?;
    if data.len() > REPORT_LIMIT {
        return Err(failure("report_limit"));
    }
    let tmp = path.with_extension("tmp");
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(|_| failure("storage_failure"))?;
    f.write_all(&data)
        .and_then(|_| f.sync_all())
        .map_err(|_| failure("storage_failure"))?;
    fs::rename(&tmp, path).map_err(|_| failure("storage_failure"))?;
    File::open(path.parent().unwrap())
        .and_then(|f| f.sync_all())
        .map_err(|_| failure("storage_failure"))?;
    Ok(())
}
pub fn is_cancellation(error: &Error) -> bool {
    is_cancellation_code(&error.code)
}
pub fn is_cancellation_code(code: &str) -> bool {
    matches!(code, "request_cancelled" | "backgrounded" | "slow_consumer")
}
fn terminal_error(o: &Operation, work_error: Option<&Error>) -> Option<Error> {
    // Cleanup failures and observed failures always dominate a concurrent stop.
    if work_error.is_some_and(|e| {
        matches!(
            e.code.as_str(),
            "cleanup_unconfirmed" | "executor_cleanup_unconfirmed"
        )
    }) {
        return work_error.cloned();
    }
    if let Some(e) = work_error.filter(|e| !is_cancellation(e)) {
        return Some(e.clone());
    }
    if let Some(case) = o
        .cases
        .lock()
        .unwrap()
        .iter()
        .find(|case| case["verdict"] == "failed")
    {
        return Some(failure(
            case["reason_code"].as_str().unwrap_or("unexpected_result"),
        ));
    }
    // Keep the bounded stop cause (first fault, otherwise first cancellation),
    // rather than a later generic checkpoint request_cancelled. No free text.
    o.stop_error().or_else(|| work_error.cloned())
}
fn summarize_outcome(o: &Operation, cleanup: &str, error: Option<&Error>) -> &'static str {
    if cleanup == "unconfirmed" {
        return "failed";
    }
    let cases = o.cases.lock().unwrap();
    if cases.iter().any(|c| c["verdict"] == "failed") {
        return "failed";
    }
    if let Some(error) = error {
        return if is_cancellation(error) {
            "cancelled"
        } else {
            "failed"
        };
    }
    if o.suite.as_deref() == Some("b3a_safety_v1")
        && cases.iter().any(|c| c["verdict"] == "not_run")
    {
        return "inconclusive";
    }
    "passed"
}

fn launch(
    h: Arc<Host>,
    o: Arc<Operation>,
    work: impl FnOnce(&Arc<Host>, &Arc<Operation>) -> Result<Vec<Value>> + Send + 'static,
) {
    o.watchdog();
    std::thread::spawn(move || {
        let pending = h.root.join("reports/pending.json");
        let result = atomic_json(
            &pending,
            &json!({"operation_id":o.id,"suite_id":o.suite,"started_at_utc":o.started}),
        )
        .and_then(|_| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&h, &o)))
                .unwrap_or_else(|_| Err(failure("cleanup_unconfirmed")))
        });
        crate::runner::complete_case_journal(&o);
        let cleanup = if result.as_ref().err().is_some_and(|e| {
            e.code == "cleanup_unconfirmed" || e.code == "executor_cleanup_unconfirmed"
        }) {
            "unconfirmed"
        } else {
            "confirmed"
        };
        let error = terminal_error(&o, result.as_ref().err());
        let outcome = summarize_outcome(&o, cleanup, error.as_ref());
        let report_id = uuid::Uuid::new_v4().to_string();
        let mut terminal = json!({"operation_id":o.id,"outcome":outcome,"cleanup":cleanup,"report_id":report_id,"error":error});
        let report = json!({"schema_version":1,"purpose":PURPOSE,"research_only":true,"production_admitted":false,"report_id":report_id,"operation_id":o.id,"suite_id":o.suite,"suite_sha256":o.suite.as_ref().map(|suite|digest(format!("{suite}\n{}",crate::runner::SUITE_SPEC).as_bytes())),"started_at_utc":o.started,"finished_at_utc":timestamp(),"terminal":terminal,"build":build(),"model":{"model_id":MODEL,"artifact_digest":mnn_model_store::candidate_digest(),"identity":report_model_identity()},"profile":{"backend":"cpu","context":2048,"threads":2,"batch_size":32,"max_tokens":256,"temperature":0,"top_p":1,"seed":0,"thinking":false},"device":h.device,"cases":*o.cases.lock().unwrap(),"coverage":{"executor":if o.kind=="suite"{"direct_public_sink"}else{"not_run"},"core":"not_run","adapter":if o.kind=="suite"{"separate_owner_after_executor_close"}else{"not_run"},"stability":"not_implemented_disabled","android_device_verdict":"self_report_requires_manual_review","logcat_canary":"not_run","memory_thermal":"not_run"}});
        let report_path = h.root.join(format!("reports/{report_id}.json"));
        let saved = atomic_json(&report_path, &report)
            .and_then(|_| {
                atomic_json(
                    &h.root.join("reports/latest-index.json"),
                    &json!({"report_id":report_id}),
                )
            })
            .and_then(|_| {
                fs::remove_file(&pending).map_err(|_| failure("storage_failure"))?;
                File::open(pending.parent().unwrap())
                    .and_then(|f| f.sync_all())
                    .map_err(|_| failure("storage_failure"))
            });
        let mut s = h.state.lock().unwrap();
        s.unavailable |= cleanup == "unconfirmed";
        match h.store.lock().unwrap().snapshot() {
            Ok(snapshot) => s.ready = snapshot.resolve_candidate(&model_id()).is_ok(),
            Err(_) => s.unavailable = true,
        }
        if saved.is_ok() {
            let bytes = serde_json::to_vec(&report).unwrap();
            s.report = Some(Report {
                operation_id: o.id.clone(),
                id: report_id,
                path: report_path,
                size: bytes.len(),
                sha: digest(&bytes),
            });
        } else {
            terminal["outcome"] = json!("failed");
            terminal["report_id"] = Value::Null;
            terminal["error"] = json!(failure("report_unavailable"));
            s.unavailable = true;
        }
        s.last_terminal = Some(terminal.clone());
        *o.terminal.lock().unwrap() = Some(terminal);
        o.changed.notify_all();
    });
}
pub fn report_prepare(epoch: &str, id: &str) -> Result<Value> {
    let h = host(epoch)?;
    let mut s = h.state.lock().unwrap();
    let r = s
        .report
        .as_ref()
        .filter(|r| r.operation_id == id)
        .ok_or_else(|| failure("report_unavailable"))?;
    let token = uuid::Uuid::new_v4().to_string();
    let value = json!({"report_token":token,"report_id":r.id,"schema_version":1,"sha256":r.sha,"size_bytes":r.size});
    let report = r.clone();
    s.report_token = Some(ReportToken {
        id: token,
        created: Instant::now(),
        report,
    });
    Ok(value)
}
pub fn open_report(epoch: &str, token: &str) -> Result<Value> {
    let h = host(epoch)?;
    let mut s = h.state.lock().unwrap();
    if !s
        .report_token
        .as_ref()
        .is_some_and(|t| t.id == token && t.created.elapsed() < Duration::from_secs(600))
    {
        return Err(failure("report_unavailable"));
    }
    let r = &s.report_token.as_ref().unwrap().report;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&r.path)
        .map_err(|_| failure("report_unavailable"))?;
    if file
        .metadata()
        .map_err(|_| failure("report_unavailable"))?
        .len()
        != r.size as u64
    {
        return Err(failure("report_unavailable"));
    }
    let value = json!({"fd":file.into_raw_fd(),"size_bytes":r.size,"sha256":r.sha});
    s.report_token = None;
    Ok(value)
}
fn clean_inbox(path: &Path) -> Result<()> {
    for entry in fs::read_dir(path).map_err(|_| failure("storage_failure"))? {
        let entry = entry.map_err(|_| failure("storage_failure"))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| failure("invalid_private_root"))?;
        let known = mnn_model_store::candidate_identity()["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["path"] == name);
        let m = fs::symlink_metadata(entry.path()).map_err(|_| failure("storage_failure"))?;
        if !known || !m.is_file() || m.file_type().is_symlink() || m.nlink() != 1 {
            return Err(failure("invalid_private_root"));
        }
        fs::remove_file(entry.path()).map_err(|_| failure("storage_failure"))?;
    }
    Ok(())
}
fn import(h: &Arc<Host>, o: &Arc<Operation>, selection: Selection) -> Result<Vec<Value>> {
    let inbox = h.root.join("inbox");
    let path = std::ffi::CString::new(h.root.as_os_str().as_encoded_bytes())
        .map_err(|_| failure("invalid_private_root"))?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return Err(failure("storage_failure"));
    }
    let stat = unsafe { stat.assume_init() };
    if (stat.f_bavail as u128) * (stat.f_frsize as u128) < 1043159148 {
        return Err(failure("space_insufficient"));
    }
    let result: Result<Vec<Value>> = (|| {
        let mut total = 0u64;
        let mut chunk = [0u8; 65536];
        for (name, file) in selection.files {
            let expected = mnn_model_store::candidate_identity()["files"]
                .as_array()
                .unwrap()
                .iter()
                .find(|v| v["path"] == name)
                .and_then(|v| v["size_bytes"].as_u64())
                .ok_or_else(|| failure("unsupported_source"))?;
            if file
                .metadata()
                .map_err(|_| failure("source_changed"))?
                .len()
                != expected
            {
                return Err(failure("source_changed"));
            }
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(inbox.join(name))
                .map_err(|_| failure("storage_failure"))?;
            let mut offset = 0u64;
            while offset < expected {
                if o.stopped() {
                    return Err(failure("request_cancelled"));
                }
                let want = (expected - offset).min(chunk.len() as u64) as usize;
                let n = unsafe {
                    libc::pread(
                        file.as_raw_fd(),
                        chunk.as_mut_ptr().cast(),
                        want,
                        offset as libc::off_t,
                    )
                };
                if n <= 0 {
                    return Err(failure("source_changed"));
                }
                output
                    .write_all(&chunk[..n as usize])
                    .map_err(|_| failure("storage_failure"))?;
                offset += n as u64;
                total += n as u64;
            }
            if file
                .metadata()
                .map_err(|_| failure("source_changed"))?
                .len()
                != expected
            {
                return Err(failure("source_changed"));
            }
            output.sync_all().map_err(|_| failure("storage_failure"))?;
            o.emit(
                "progress",
                json!({"stage":"source_copy","completed_bytes":total,"total_bytes":454470710}),
            );
        }
        o.emit(
            "progress",
            json!({"stage":"store_import","completed_bytes":null,"total_bytes":null}),
        );
        h.store
            .lock()
            .unwrap()
            .import_candidate(&inbox, || o.stopped())
            .map_err(runtime_error)?;
        Ok(vec![])
    })();
    let cleanup = clean_inbox(&inbox);
    result.and(cleanup.map(|_| vec![]))
}

#[cfg(test)]
mod tests {
    use super::*;
    const DEVICE: &str = r#"{"schema_version":1,"manufacturer":"Linux-research","model":"host-test","soc_manufacturer":null,"soc_model":null,"android_release":"not-android","sdk_int":36,"security_patch":null,"supported_abis":["x86_64-host-test"]}"#;
    #[test]
    fn device_whitelist_rejects_duplicate_and_extra_fields() {
        assert!(Device::parse(DEVICE).is_ok());
        assert!(
            Device::parse(&DEVICE.replace("\"sdk_int\":36", "\"sdk_int\":36,\"sdk_int\":35"))
                .is_err()
        );
        assert!(
            Device::parse(
                &DEVICE.replace("\"sdk_int\":36", "\"sdk_int\":36,\"serial\":\"secret\"")
            )
            .is_err()
        );
    }
    #[test]
    fn safe_temporary_recovery_never_follows_links() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pending.tmp");
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .unwrap();
        recover_report_temps(dir.path()).unwrap();
        assert!(!path.exists());
        std::os::unix::fs::symlink("/no/such/private/target", &path).unwrap();
        assert!(recover_report_temps(dir.path()).is_err());
        assert!(fs::symlink_metadata(path).unwrap().file_type().is_symlink());
    }
    #[test]
    fn report_bound_and_immutable_names() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join(format!("{}.json", uuid::Uuid::new_v4()));
        let second = dir.path().join(format!("{}.json", uuid::Uuid::new_v4()));
        atomic_json(&first, &json!({"value":1})).unwrap();
        let mut fd = File::open(&first).unwrap();
        atomic_json(&second, &json!({"value":2})).unwrap();
        use std::io::Read;
        let mut data = String::new();
        fd.read_to_string(&mut data).unwrap();
        assert_eq!(data, "{\"value\":1}");
        assert!(
            atomic_json(
                &dir.path().join("huge.json"),
                &json!({"x":"a".repeat(REPORT_LIMIT)})
            )
            .is_err()
        );
    }
    #[test]
    fn selection_validation_does_not_hold_control_lock_or_survive_background() {
        let state = Arc::new(Mutex::new(State {
            visible: true,
            lifecycle_sequence: 1,
            ready: false,
            unavailable: false,
            active: None,
            selection: None,
            consumed: None,
            last_terminal: None,
            report: None,
            report_token: None,
        }));
        let (ready, validation_started) = std::sync::mpsc::sync_channel(1);
        let (resume, wait) = std::sync::mpsc::sync_channel(1);
        let worker_state = state.clone();
        let worker = std::thread::spawn(move || {
            let observed = {
                let s = worker_state.lock().unwrap();
                gate(&s).unwrap();
                s.lifecycle_sequence
            };
            ready.send(()).unwrap();
            wait.recv().unwrap(); // controlled provider delay, not a device latency claim
            publish_selection(&worker_state, observed, BTreeMap::new())
        });
        validation_started.recv().unwrap();
        {
            let mut s = state
                .try_lock()
                .expect("validation must not own control lock");
            s.visible = false;
            s.lifecycle_sequence = 2;
            s.visible = true;
            s.lifecycle_sequence = 3;
        }
        resume.send(()).unwrap();
        assert_eq!(
            worker.join().unwrap().unwrap_err().code,
            "selection_expired"
        );
        assert!(state.lock().unwrap().selection.is_none());
    }
    #[test]
    fn sealed_reason_preserves_cancellation_and_failure_precedence() {
        let op = Operation::new("suite", None);
        op.stop("backgrounded");
        for (work, code, outcome) in [
            ("request_cancelled", "backgrounded", "cancelled"),
            ("native_failure", "native_failure", "failed"),
            ("native_protocol", "native_protocol", "failed"),
            ("cleanup_unconfirmed", "cleanup_unconfirmed", "failed"),
        ] {
            let error = terminal_error(&op, Some(&failure(work))).unwrap();
            assert_eq!(error.code, code);
            assert_eq!(
                summarize_outcome(
                    &op,
                    if code == "cleanup_unconfirmed" {
                        "unconfirmed"
                    } else {
                        "confirmed"
                    },
                    Some(&error)
                ),
                outcome
            );
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("report.json");
            atomic_json(
                &path,
                &json!({"terminal":{"outcome":outcome,"error":error}}),
            )
            .unwrap();
            let sealed: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
            assert_eq!(sealed["terminal"]["error"]["code"], code);
        }
        op.cases.lock().unwrap().push(
            json!({"case_id":"english_stream","verdict":"failed","reason_code":"native_protocol"}),
        );
        assert_eq!(
            terminal_error(&op, Some(&failure("request_cancelled")))
                .unwrap()
                .code,
            "native_protocol"
        );
        assert_eq!(
            terminal_error(&op, Some(&failure("cleanup_unconfirmed")))
                .unwrap()
                .code,
            "cleanup_unconfirmed"
        );
        let guard = Operation::new("suite", None);
        guard.stop("output_limit");
        let error = terminal_error(&guard, Some(&failure("request_cancelled"))).unwrap();
        assert_eq!(
            summarize_outcome(&guard, "confirmed", Some(&error)),
            "failed"
        );
    }
    #[test]
    fn final_outcome_is_sticky_and_keeps_case_journal() {
        let op = Operation::new("suite", Some("b3a_smoke_v1".into()));
        op.cases
            .lock()
            .unwrap()
            .push(json!({"case_id":"load","layer":"executor","verdict":"failed"}));
        crate::runner::complete_case_journal(&op);
        assert_eq!(summarize_outcome(&op, "confirmed", None), "failed");
        assert!(
            op.cases
                .lock()
                .unwrap()
                .iter()
                .any(|c| c["case_id"] == "english_stream" && c["verdict"] == "not_run")
        );
        op.stop("request_cancelled");
        assert_eq!(
            summarize_outcome(&op, "unconfirmed", Some(&failure("cleanup_unconfirmed"))),
            "failed"
        );
        assert_eq!(
            summarize_outcome(&op, "confirmed", Some(&failure("request_cancelled"))),
            "failed"
        );
        let cancelled = Operation::new("suite", None);
        cancelled.stop("request_cancelled");
        assert_eq!(
            summarize_outcome(&cancelled, "confirmed", Some(&failure("request_cancelled"))),
            "cancelled"
        );
        let safety = Operation::new("suite", Some("b3a_safety_v1".into()));
        crate::runner::complete_case_journal(&safety);
        assert_eq!(
            summarize_outcome(&safety, "confirmed", None),
            "inconclusive"
        );
    }
    #[test]
    #[ignore = "requires exact real candidate directory and ~1GiB free temporary storage"]
    fn real_fd_import_executor_adapter_report_remove() {
        let source = PathBuf::from(
            std::env::var("NEXA_MNN_TEST_MODEL").expect("explicit real candidate required"),
        );
        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let epoch = bootstrap(dir.path().to_str().unwrap(), DEVICE).unwrap()["host_epoch"]
            .as_str()
            .unwrap()
            .to_string();
        visibility(&epoch, true, 1).unwrap();
        assert!(snapshot("stale").is_err());
        let files: Vec<_> = mnn_model_store::candidate_identity()["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| {
                let name = v["path"].as_str().unwrap().to_string();
                let file = File::open(source.join(&name)).unwrap();
                (name, v["size_bytes"].as_i64().unwrap(), file)
            })
            .collect();
        let mut duplicate_names: Vec<_> = files.iter().map(|v| v.0.clone()).collect();
        duplicate_names[1] = duplicate_names[0].clone();
        assert!(
            register(
                &epoch,
                duplicate_names,
                files.iter().map(|v| v.1).collect(),
                files.iter().map(|v| v.2.as_raw_fd()).collect()
            )
            .is_err()
        );
        let token = register(
            &epoch,
            files.iter().map(|v| v.0.clone()).collect(),
            files.iter().map(|v| v.1).collect(),
            files.iter().map(|v| v.2.as_raw_fd()).collect(),
        )
        .unwrap()["selection_token"]
            .as_str()
            .unwrap()
            .to_string();
        fn drain(epoch: &str, reference: Value) -> Value {
            let id = reference["operation_id"].as_str().unwrap();
            let mut ack = None;
            loop {
                let value = poll(epoch, id, ack).unwrap();
                if let Some(event) = value["event"].as_object() {
                    ack = event["sequence"].as_u64();
                }
                if !value["terminal"].is_null() {
                    return value["terminal"].clone();
                }
            }
        }
        // A later selection replaces a prior idle selection. Provider size
        // mutation is rejected before copying the oversized weight payload.
        let fake = tempfile::tempdir().unwrap();
        let mut fake_read = vec![];
        for (name, length, _) in &files {
            let writable = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(fake.path().join(name))
                .unwrap();
            writable.set_len(*length as u64).unwrap();
            fake_read.push(File::open(fake.path().join(name)).unwrap());
        }
        let bad = register(
            &epoch,
            files.iter().map(|v| v.0.clone()).collect(),
            files.iter().map(|v| v.1).collect(),
            fake_read.iter().map(AsRawFd::as_raw_fd).collect(),
        )
        .unwrap()["selection_token"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(
            start_import(&epoch, &token).unwrap_err().code,
            "selection_expired"
        );
        OpenOptions::new()
            .write(true)
            .open(fake.path().join("llm.mnn.weight"))
            .unwrap()
            .set_len(450810339)
            .unwrap();
        let rejected = drain(&epoch, start_import(&epoch, &bad).unwrap());
        assert_eq!(rejected["outcome"], "failed");
        assert_eq!(rejected["error"]["code"], "source_changed");
        assert_eq!(snapshot(&epoch).unwrap()["candidate"]["state"], "absent");
        drop(fake_read);
        drop(fake);
        let token = register(
            &epoch,
            files.iter().map(|v| v.0.clone()).collect(),
            files.iter().map(|v| v.1).collect(),
            files.iter().map(|v| v.2.as_raw_fd()).collect(),
        )
        .unwrap()["selection_token"]
            .as_str()
            .unwrap()
            .to_string();
        let imported = drain(&epoch, start_import(&epoch, &token).unwrap());
        assert_eq!(imported["outcome"], "passed", "{imported}");
        assert_eq!(snapshot(&epoch).unwrap()["candidate"]["state"], "ready");
        let suite = drain(&epoch, start_suite(&epoch, "b3a_smoke_v1").unwrap());
        let descriptor = report_prepare(&epoch, suite["operation_id"].as_str().unwrap()).unwrap();
        // The old report token must still identify its immutable report after
        // another operation seals a new, current report.
        let removed = drain(&epoch, start_remove(&epoch).unwrap());
        let report = open_report(&epoch, descriptor["report_token"].as_str().unwrap()).unwrap();
        assert_eq!(report["sha256"], descriptor["sha256"]);
        assert!(open_report(&epoch, descriptor["report_token"].as_str().unwrap()).is_err());
        let file = unsafe { File::from_raw_fd(report["fd"].as_i64().unwrap() as i32) };
        use std::io::Read;
        let mut bytes = vec![];
        file.take(REPORT_LIMIT as u64)
            .read_to_end(&mut bytes)
            .unwrap();
        assert_eq!(digest(&bytes), report["sha256"]);
        let result: Value = serde_json::from_slice(&bytes).unwrap();
        if let Ok(path) = std::env::var("NEXA_VERIFIER_TEST_REPORT") {
            fs::write(path, &bytes).unwrap();
        }
        assert_eq!(result["production_admitted"], false);
        assert_eq!(
            suite["outcome"],
            "passed",
            "{}",
            String::from_utf8(bytes).unwrap()
        );
        assert_eq!(
            bootstrap(dir.path().to_str().unwrap(), DEVICE).unwrap()["host_epoch"],
            epoch
        );
        assert_eq!(open().unwrap()["host_epoch"], epoch);
        assert_eq!(removed["cleanup"], "confirmed");
        assert_eq!(snapshot(&epoch).unwrap()["candidate"]["state"], "absent");
        // Inject only a control-state fault; this is not a native kernel claim.
        host(&epoch).unwrap().state.lock().unwrap().unavailable = true;
        visibility(&epoch, false, 2).unwrap();
        visibility(&epoch, true, 3).unwrap();
        assert_eq!(
            start_remove(&epoch).unwrap_err().code,
            "cleanup_unconfirmed"
        );
        assert_eq!(open().unwrap()["host_state"], "cleanup_unconfirmed");
        assert_eq!(
            bootstrap(dir.path().to_str().unwrap(), DEVICE).unwrap()["host_epoch"],
            epoch
        );
    }
}
