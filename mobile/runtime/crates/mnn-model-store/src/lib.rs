//! Fixed Qwen3 CPU candidate store. Integrity is never production admission.
//! Private copies and leases defend against untrusted sources and cooperating
//! App threads/processes, not a hostile process with the same UID.
#![forbid(unsafe_code)]
#[cfg(not(unix))]
compile_error!("the private MNN store requires the audited Unix filesystem profile");
mod strict_json;
use fs2::FileExt;
use runtime_types::{ErrorCode, LoadOptions, ModelId, ResolvedModel, RuntimeError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Component, Path, PathBuf},
    sync::Arc,
};

pub const MODEL_ID: &str = "qwen3-0.6b-mnn";
pub const TEMPLATE_SHA256: &str =
    "87a2728cb8dc9fe424d624542f6060ec05a1d285ebbec578bb078900e33396b5";
pub const POLICY_SHA256: &str = "ea06621b78e67e58f97f98951b26db0a8a893ded4112da3e5a762b98566fa328";
const MAX_JSON: u64 = 65536;
fn err(code: ErrorCode, message: &str) -> RuntimeError {
    RuntimeError::new(code, message)
}
fn invalid() -> RuntimeError {
    err(
        ErrorCode::InvalidManifest,
        "MNN fixed package contract rejected",
    )
}
fn io(e: std::io::Error) -> RuntimeError {
    err(
        if e.raw_os_error() == Some(libc::ENOSPC) {
            ErrorCode::InsufficientSpace
        } else {
            ErrorCode::Io
        },
        "MNN private storage operation failed",
    )
}
fn hash(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}
fn checkpoint(cancel: &impl Fn() -> bool) -> Result<(), RuntimeError> {
    if cancel() {
        Err(err(
            ErrorCode::RequestCancelled,
            "MNN storage operation cancelled",
        ))
    } else {
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PackageFile {
    pub path: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub role: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MnnPackageManifestV1 {
    pub schema_version: u32,
    pub format: String,
    pub id: ModelId,
    pub display_name: String,
    pub identity: Value,
    pub artifact_digest: String,
}
fn files() -> Vec<PackageFile> {
    let v: Value = serde_json::from_str(include_str!(
        "../../../../../scripts/android_mnn/candidate-model.json"
    ))
    .expect("compiled candidate lock");
    v["files"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(path, v)| PackageFile {
            path: path.clone(),
            size_bytes: v["size"].as_u64().unwrap(),
            sha256: v["sha256"].as_str().unwrap().into(),
            role: match path.as_str() {
                "config.json" => "config",
                "llm_config.json" => "metadata",
                "llm.mnn" => "graph",
                "llm.mnn.weight" => "external_weight",
                _ => "tokenizer",
            }
            .into(),
        })
        .collect()
}
pub fn candidate_identity() -> Value {
    json!({"architecture":"qwen3", "context_limit":2048,"default_context":2048,
    "artifact_source":{"publisher":"taobao-mnn","repository":"taobao-mnn/Qwen3-0.6B-MNN","source_uri":"https://huggingface.co/taobao-mnn/Qwen3-0.6B-MNN","revision":"34dfccda1187ded6e07ea06426da576b0b793c6b","license":"Apache-2.0","provenance_kind":"preconverted"},
    "conversion_provenance":{"original_revision":null,"exporter_commit":null,"arguments":null},
    "files":files(),"template":{"source":"llm_config.json","sha256":TEMPLATE_SHA256,"policy":"cpu-text-qwen3-v1-nonthinking","policy_sha256":POLICY_SHA256,"enable_thinking":false},
    "variants":[{"id":"cpu-text-qwen3-v1","files":files().iter().map(|f|&f.path).collect::<Vec<_>>(),"policy_sha256":POLICY_SHA256}]})
}
pub fn candidate_digest() -> String {
    hash(&serde_json::to_vec(&candidate_identity()).unwrap())
}
impl MnnPackageManifestV1 {
    pub fn candidate() -> Self {
        Self {
            schema_version: 1,
            format: "mnn_package".into(),
            id: ModelId::new(MODEL_ID).unwrap(),
            display_name: "Qwen3-0.6B MNN (candidate)".into(),
            identity: candidate_identity(),
            artifact_digest: candidate_digest(),
        }
    }
    pub fn parse(bytes: &[u8]) -> Result<Self, RuntimeError> {
        if bytes.len() > MAX_JSON as usize {
            return Err(invalid());
        }
        let value = strict_json::parse(bytes).map_err(|_| invalid())?;
        let m: Self = serde_json::from_value(value).map_err(|_| invalid())?;
        if m.schema_version != 1
            || m.format != "mnn_package"
            || m.id.as_str() != MODEL_ID
            || m.display_name.is_empty()
            || m.display_name.len() > 256
            || m.identity != candidate_identity()
            || m.artifact_digest != candidate_digest()
        {
            return Err(invalid());
        }
        Ok(m)
    }
}

fn safe_directory(path: &Path) -> Result<(), RuntimeError> {
    let mut p = PathBuf::new();
    for part in path.components() {
        match part {
            Component::RootDir | Component::Normal(_) => p.push(part),
            _ => return Err(invalid()),
        };
        let m = fs::symlink_metadata(&p).map_err(io)?;
        if !m.is_dir() || m.file_type().is_symlink() {
            return Err(invalid());
        }
    }
    Ok(())
}
fn private_dir(path: &Path) -> Result<(), RuntimeError> {
    safe_directory(path)?;
    let m = fs::metadata(path).map_err(io)?;
    if m.mode() & 0o077 != 0 {
        return Err(err(
            ErrorCode::ModelDirectoryUnsupported,
            "MNN store directory must be private",
        ));
    }
    Ok(())
}
fn regular(path: &Path) -> Result<File, RuntimeError> {
    let f = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(io)?;
    let m = f.metadata().map_err(io)?;
    if !m.is_file() || m.nlink() != 1 {
        return Err(invalid());
    }
    Ok(f)
}
fn bounded_json(path: &Path) -> Result<Value, RuntimeError> {
    let f = regular(path)?;
    if f.metadata().map_err(io)?.len() > MAX_JSON {
        return Err(invalid());
    }
    let mut b = Vec::new();
    f.take(MAX_JSON + 1).read_to_end(&mut b).map_err(io)?;
    if b.len() > MAX_JSON as usize {
        return Err(invalid());
    }
    strict_json::parse(&b).map_err(|_| invalid())
}
fn check_names(path: &Path, manifest: bool) -> Result<(), RuntimeError> {
    safe_directory(path)?;
    let mut expected: Vec<String> = files().into_iter().map(|f| f.path).collect();
    if manifest {
        expected.push("manifest.json".into());
    }
    expected.sort();
    let mut actual = Vec::new();
    for ent in fs::read_dir(path).map_err(io)? {
        let e = ent.map_err(io)?;
        actual.push(e.file_name().into_string().map_err(|_| invalid())?);
        if actual.len() > expected.len() {
            return Err(invalid());
        }
        let _ = regular(&e.path())?;
    }
    actual.sort();
    if actual != expected {
        return Err(invalid());
    }
    Ok(())
}
fn stream_file(
    source: &Path,
    mut output: Option<&mut dyn Write>,
    spec: &PackageFile,
    cancel: &impl Fn() -> bool,
) -> Result<(), RuntimeError> {
    let mut f = regular(source)?;
    let before = f.metadata().map_err(io)?;
    if before.len() != spec.size_bytes {
        return Err(invalid());
    }
    let mut h = Sha256::new();
    let mut n = 0u64;
    let mut buf = [0u8; 65536];
    loop {
        checkpoint(cancel)?;
        let got = f.read(&mut buf).map_err(io)?;
        if got == 0 {
            break;
        }
        n = n.checked_add(got as u64).ok_or_else(invalid)?;
        if n > spec.size_bytes {
            return Err(invalid());
        }
        h.update(&buf[..got]);
        if let Some(o) = output.as_deref_mut() {
            o.write_all(&buf[..got]).map_err(io)?;
        }
    }
    let after = f.metadata().map_err(io)?;
    if n != spec.size_bytes
        || format!("{:x}", h.finalize()) != spec.sha256
        || after.nlink() != 1
        || before.ino() != after.ino()
        || before.len() != after.len()
        || before.mtime() != after.mtime()
        || before.mtime_nsec() != after.mtime_nsec()
    {
        return Err(err(
            ErrorCode::IntegrityFailure,
            "MNN asset integrity check failed",
        ));
    }
    Ok(())
}
fn validate_metadata(path: &Path) -> Result<Value, RuntimeError> {
    let config = bounded_json(&path.join("config.json"))?;
    let metadata = bounded_json(&path.join("llm_config.json"))?;
    let config_keys = [
        "llm_model",
        "llm_weight",
        "backend_type",
        "thread_num",
        "precision",
        "memory",
        "sampler_type",
        "mixed_samplers",
        "penalty",
        "temperature",
        "topP",
        "topK",
        "min_p",
    ];
    if config.as_object().is_none_or(|o| {
        o.len() != config_keys.len() || o.keys().any(|k| !config_keys.contains(&k.as_str()))
    }) || config["llm_model"] != "llm.mnn"
        || config["llm_weight"] != "llm.mnn.weight"
    {
        return Err(invalid());
    }
    let keys = [
        "hidden_size",
        "layer_nums",
        "attention_mask",
        "key_value_shape",
        "bos",
        "system_prompt_template",
        "user_prompt_template",
        "assistant_prompt_template",
        "is_visual",
        "jinja",
        "tie_embeddings",
    ];
    if metadata
        .as_object()
        .is_none_or(|o| o.len() != keys.len() || o.keys().any(|k| !keys.contains(&k.as_str())))
        || metadata["hidden_size"] != 1024
        || metadata["layer_nums"] != 28
        || metadata["is_visual"] != false
        || metadata["key_value_shape"] != json!([2, 1, 0, 8, 128])
        || metadata["tie_embeddings"] != json!([275780066u64, 431362530u64, 19447808u64, 8, 64])
    {
        return Err(invalid());
    }
    let j = &metadata["jinja"];
    if j.as_object()
        .is_none_or(|o| o.len() != 2 || !o.contains_key("chat_template") || !o.contains_key("eos"))
        || j["eos"] != "<|im_end|>"
        || j["chat_template"]
            .as_str()
            .is_none_or(|t| hash(t.as_bytes()) != TEMPLATE_SHA256)
    {
        return Err(invalid());
    }
    // Fixed packed embedding spans: checked addition cannot wrap into the weight.
    for (offset, length) in [(275780066u64, 155582464u64), (431362530, 19447808)] {
        if offset.checked_add(length).is_none_or(|v| v > 450810338) {
            return Err(invalid());
        }
    }
    Ok(metadata)
}
struct Root {
    path: PathBuf,
    _lock: File,
}
pub struct MnnModelStore {
    root: Arc<Root>,
    entries: BTreeMap<ModelId, Arc<Entry>>,
    poisoned: bool,
    #[cfg(test)]
    fault: Option<Fault>,
}
#[cfg(test)]
struct Fault {
    point: &'static str,
    errno: i32,
}
struct Entry {
    root: Arc<Root>,
    path: PathBuf,
    manifest: MnnPackageManifestV1,
}
/// Holding a snapshot pins its generations against deletion or replacement.
#[derive(Clone)]
pub struct MnnRegistrySnapshot {
    entries: BTreeMap<ModelId, Arc<Entry>>,
}
pub struct LoadLease {
    entry: Arc<Entry>,
    work: tempfile::TempDir,
    config: PathBuf,
}
fn controlled_name(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix).is_some_and(|suffix| {
        suffix.len() == 16 && suffix.bytes().all(|b| b.is_ascii_alphanumeric())
    })
}
fn require_absent(path: &Path) -> Result<(), RuntimeError> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io(e)),
        Ok(_) => Err(err(
            ErrorCode::AlreadyExists,
            "MNN transaction destination exists",
        )),
    }
}
impl MnnModelStore {
    fn healthy(&self) -> Result<(), RuntimeError> {
        if self.poisoned {
            Err(err(
                ErrorCode::ModelConflict,
                "MNN storage durability uncertain; reopen required",
            ))
        } else {
            Ok(())
        }
    }
    fn failpoint(&mut self, _point: &'static str) -> Result<(), RuntimeError> {
        #[cfg(test)]
        if self.fault.as_ref().is_some_and(|f| f.point == _point) {
            let fault = self.fault.take().unwrap();
            return Err(io(std::io::Error::from_raw_os_error(fault.errno)));
        }
        Ok(())
    }
    fn sync_root(&mut self, point: &'static str) -> Result<(), RuntimeError> {
        let result = self.failpoint(point).and_then(|()| {
            File::open(&self.root.path)
                .map_err(io)?
                .sync_all()
                .map_err(io)
        });
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }

    /// Root must already exist, be private (0700), and have no symlink ancestors.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, RuntimeError> {
        let path = path.as_ref();
        if !path.is_absolute() {
            return Err(invalid());
        }
        private_dir(path)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path.join(".store-lock"))
            .map_err(io)?;
        if !lock.metadata().map_err(io)?.is_file() || lock.metadata().map_err(io)?.nlink() != 1 {
            return Err(invalid());
        }
        lock.try_lock_exclusive()
            .map_err(|_| err(ErrorCode::ModelConflict, "MNN store already owned"))?;
        let root = Arc::new(Root {
            path: path.to_path_buf(),
            _lock: lock,
        });
        let mut entries = BTreeMap::new();
        for e in fs::read_dir(path).map_err(io)? {
            let e = e.map_err(io)?;
            let name = e.file_name();
            let name = name.to_str().ok_or_else(invalid)?;
            if name == ".store-lock" {
                continue;
            }
            if [".stage-", ".work-", ".trash-"]
                .iter()
                .any(|prefix| controlled_name(name, prefix))
            {
                private_dir(&e.path())?;
                fs::remove_dir_all(e.path()).map_err(io)?;
                continue;
            }
            if !controlled_name(name, "generation-") {
                return Err(invalid());
            }
            private_dir(&e.path())?;
            check_names(&e.path(), true)?;
            let mut b = Vec::new();
            regular(&e.path().join("manifest.json"))?
                .take(MAX_JSON + 1)
                .read_to_end(&mut b)
                .map_err(io)?;
            let manifest = MnnPackageManifestV1::parse(&b)?;
            let entry = Arc::new(Entry {
                root: root.clone(),
                path: e.path(),
                manifest,
            });
            if entries.insert(entry.manifest.id.clone(), entry).is_some() {
                return Err(invalid());
            }
        }
        File::open(path).map_err(io)?.sync_all().map_err(io)?;
        Ok(Self {
            root,
            entries,
            poisoned: false,
            #[cfg(test)]
            fault: None,
        })
    }
    pub fn import_candidate(
        &mut self,
        source: impl AsRef<Path>,
        cancel: impl Fn() -> bool,
    ) -> Result<(), RuntimeError> {
        self.healthy()?;
        if !self.entries.is_empty() {
            return Err(err(
                ErrorCode::AlreadyExists,
                "MNN candidate already installed",
            ));
        }
        let source = source.as_ref();
        check_names(source, false)?;
        checkpoint(&cancel)?;
        let stage = tempfile::Builder::new()
            .prefix(".stage-")
            .rand_bytes(16)
            .tempdir_in(&self.root.path)
            .map_err(io)?;
        fs::set_permissions(stage.path(), fs::Permissions::from_mode(0o700)).map_err(io)?;
        for spec in files() {
            let mut out = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(stage.path().join(&spec.path))
                .map_err(io)?;
            stream_file(&source.join(&spec.path), Some(&mut out), &spec, &cancel)?;
            out.sync_all().map_err(io)?;
        }
        check_names(source, false)?;
        validate_metadata(stage.path())?;
        let manifest = MnnPackageManifestV1::candidate();
        let mut out = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(stage.path().join("manifest.json"))
            .map_err(io)?;
        out.write_all(&serde_json::to_vec(&manifest).map_err(|_| invalid())?)
            .map_err(io)?;
        out.sync_all().map_err(io)?;
        File::open(stage.path())
            .map_err(io)?
            .sync_all()
            .map_err(io)?;
        checkpoint(&cancel)?;
        self.publish(stage, manifest)
    }
    fn publish(
        &mut self,
        stage: tempfile::TempDir,
        manifest: MnnPackageManifestV1,
    ) -> Result<(), RuntimeError> {
        self.healthy()?;
        let suffix = stage
            .path()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .trim_start_matches(".stage-");
        let dest = self.root.path.join(format!("generation-{suffix}"));
        require_absent(&dest)?;
        self.failpoint("publish_rename")?;
        fs::rename(stage.path(), &dest).map_err(io)?;
        // After rename, visibility has committed even if the durability barrier
        // fails. Register it before that barrier, then poison on uncertainty.
        self.entries.insert(
            manifest.id.clone(),
            Arc::new(Entry {
                root: self.root.clone(),
                path: dest,
                manifest,
            }),
        );
        self.sync_root("publish_sync")
    }
    pub fn snapshot(&self) -> Result<MnnRegistrySnapshot, RuntimeError> {
        self.healthy()?;
        Ok(MnnRegistrySnapshot {
            entries: self.entries.clone(),
        })
    }
    pub fn remove_generation(&mut self, id: &ModelId) -> Result<(), RuntimeError> {
        self.healthy()?;
        let e = self
            .entries
            .get(id)
            .ok_or_else(|| err(ErrorCode::ModelNotFound, "MNN candidate not installed"))?;
        if Arc::strong_count(e) != 1 {
            return Err(err(
                ErrorCode::ModelFileInUse,
                "MNN generation has a live snapshot or lease",
            ));
        }
        private_dir(&e.path)?;
        let suffix = e
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_prefix("generation-"))
            .ok_or_else(invalid)?;
        let source = e.path.clone();
        let tombstone = self.root.path.join(format!(".trash-{suffix}"));
        require_absent(&tombstone)?;
        self.failpoint("remove_rename")?;
        fs::rename(source, &tombstone).map_err(io)?;
        // No registered generation is ever partially removed. Tombstones may
        // survive errors/crashes and are recovered while holding the store lock.
        self.entries.remove(id);
        self.sync_root("remove_sync")?;
        let result = (|| {
            #[cfg(test)]
            if self
                .fault
                .as_ref()
                .is_some_and(|f| f.point == "remove_partial")
            {
                fs::remove_file(tombstone.join("config.json")).map_err(io)?;
                self.failpoint("remove_partial")?;
            }
            private_dir(&tombstone)?;
            fs::remove_dir_all(&tombstone).map_err(io)
        })();
        if result.is_err() {
            self.poisoned = true;
        }
        result?;
        self.sync_root("remove_cleanup_sync")
    }
}
impl MnnRegistrySnapshot {
    pub fn resolve_candidate(&self, id: &ModelId) -> Result<ResolvedModel, RuntimeError> {
        let e = self
            .entries
            .get(id)
            .ok_or_else(|| err(ErrorCode::ModelNotFound, "MNN candidate not installed"))?;
        Ok(ResolvedModel {
            id: id.clone(),
            path: e.path.join("manifest.json"),
            context_limit: 2048,
            default_context: 2048,
            validated: false,
        })
    }
    pub fn acquire(
        &self,
        resolved: &ResolvedModel,
        options: LoadOptions,
        cancel: impl Fn() -> bool,
    ) -> Result<LoadLease, RuntimeError> {
        options.validate()?;
        if options.context_size > 2048 || options.threads > 2 || options.batch_size > 128 {
            return Err(RuntimeError::invalid(
                "MNN CPU requires context <=2048, threads <=2, chunk <=128",
            ));
        }
        let e = self.entries.get(&resolved.id).ok_or_else(invalid)?.clone();
        if resolved.path != e.path.join("manifest.json")
            || resolved.context_limit != 2048
            || resolved.default_context != 2048
        {
            return Err(err(
                ErrorCode::ModelConflict,
                "MNN snapshot generation mismatch",
            ));
        }
        check_names(&e.path, true)?;
        let mut b = Vec::new();
        regular(&e.path.join("manifest.json"))?
            .take(MAX_JSON + 1)
            .read_to_end(&mut b)
            .map_err(io)?;
        if MnnPackageManifestV1::parse(&b)? != e.manifest {
            return Err(invalid());
        }
        for f in files() {
            stream_file(&e.path.join(&f.path), None, &f, &cancel)?;
        }
        let metadata = validate_metadata(&e.path)?;
        checkpoint(&cancel)?;
        let work = tempfile::Builder::new()
            .prefix(".work-")
            .rand_bytes(16)
            .tempdir_in(&e.root.path)
            .map_err(io)?;
        fs::set_permissions(work.path(), fs::Permissions::from_mode(0o700)).map_err(io)?;
        // Explicitly rebuild only the fixed metadata keys; never merge source config.
        let mut config = metadata;
        let o = config.as_object_mut().ok_or_else(invalid)?;
        for (k,v) in json!({"base_dir":format!("{}/",e.path.to_str().ok_or_else(invalid)?),"llm_model":"llm.mnn","llm_weight":"llm.mnn.weight","llm_config":"llm_config.json","tokenizer_file":"tokenizer.txt","context_file":"nexa-disabled-context.json","backend_type":"cpu","thread_num":options.threads,"precision":"high","memory":"low","sampler_type":"greedy","chunk":options.batch_size,"max_all_tokens":options.context_size,"max_new_tokens":1,"reuse_kv":false,"prompt_cache":false,"use_mmap":false,"use_cached_mmap":false,"kvcache_mmap":false,"speculative_type":"","async":false}).as_object().unwrap(){o.insert(k.clone(),v.clone());}
        o.get_mut("jinja")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("context".into(), json!({"enable_thinking":false}));
        let config_path = work.path().join("runtime.json");
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&config_path)
            .map_err(io)?;
        f.write_all(&serde_json::to_vec(&config).map_err(|_| invalid())?)
            .map_err(io)?;
        f.sync_all().map_err(io)?;
        File::open(work.path())
            .map_err(io)?
            .sync_all()
            .map_err(io)?;
        Ok(LoadLease {
            entry: e,
            work,
            config: config_path,
        })
    }
}
impl LoadLease {
    pub fn runtime_config(&self) -> &Path {
        &self.config
    }
    pub fn artifact_digest(&self) -> &str {
        &self.entry.manifest.artifact_digest
    }
    pub fn work_directory(&self) -> &Path {
        self.work.path()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn strict_manifest() {
        let m = MnnPackageManifestV1::candidate();
        let b = serde_json::to_vec(&m).unwrap();
        assert!(MnnPackageManifestV1::parse(&b).is_ok());
        for s in [br#"{"x":1,"x":2}"#.as_slice(), br#"{"x":{"a":1,"a":2}}"#] {
            assert!(strict_json::parse(s).is_err());
        }
        let mut v = serde_json::to_value(m).unwrap();
        v["validated"] = json!(true);
        assert!(MnnPackageManifestV1::parse(&serde_json::to_vec(&v).unwrap()).is_err());
        v.as_object_mut().unwrap().remove("validated");
        v["identity"]["context_limit"] = json!(4096);
        assert!(MnnPackageManifestV1::parse(&serde_json::to_vec(&v).unwrap()).is_err());
    }
    #[test]
    fn root_lock_and_private_permissions() {
        let d = tempfile::tempdir().unwrap();
        fs::set_permissions(d.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let s = MnnModelStore::open(d.path()).unwrap();
        assert!(MnnModelStore::open(d.path()).is_err());
        drop(s);
        assert!(MnnModelStore::open(d.path()).is_ok());
        fs::set_permissions(d.path(), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(MnnModelStore::open(d.path()).is_err());
    }
    #[test]
    fn links_and_special_files_rejected() {
        let d = tempfile::tempdir().unwrap();
        fs::set_permissions(d.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let p = d.path().join("plain");
        fs::write(&p, b"x").unwrap();
        fs::hard_link(&p, d.path().join("hard")).unwrap();
        assert!(regular(&p).is_err());
        std::os::unix::fs::symlink(&p, d.path().join("sym")).unwrap();
        assert!(regular(&d.path().join("sym")).is_err());
        assert!(safe_directory(&d.path().join("..")).is_err());
    }
    #[test]
    fn interrupted_copy_does_not_publish() {
        let d = tempfile::tempdir().unwrap();
        fs::set_permissions(d.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let src = tempfile::tempdir().unwrap();
        let mut s = MnnModelStore::open(d.path()).unwrap();
        assert!(s.import_candidate(src.path(), || true).is_err());
        assert!(s.snapshot().unwrap().entries.is_empty());
        assert_eq!(fs::read_dir(d.path()).unwrap().count(), 1);
    }
}

#[cfg(test)]
mod negative_tests {
    use super::*;
    use std::cell::Cell;
    fn dir() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        fs::set_permissions(d.path(), fs::Permissions::from_mode(0o700)).unwrap();
        d
    }
    fn tiny_file(d: &Path) -> PackageFile {
        let data = b"bounded test fixture";
        fs::write(d.join("input"), data).unwrap();
        PackageFile {
            path: "input".into(),
            size_bytes: data.len() as u64,
            sha256: hash(data),
            role: "test".into(),
        }
    }
    #[test]
    fn streaming_rejects_hash_size_and_cancel() {
        let d = dir();
        let mut spec = tiny_file(d.path());
        assert!(stream_file(&d.path().join("input"), None, &spec, &|| false).is_ok());
        assert_eq!(
            stream_file(&d.path().join("input"), None, &spec, &|| true)
                .unwrap_err()
                .code,
            ErrorCode::RequestCancelled
        );
        spec.sha256 = "0".repeat(64);
        assert_eq!(
            stream_file(&d.path().join("input"), None, &spec, &|| false)
                .unwrap_err()
                .code,
            ErrorCode::IntegrityFailure
        );
        spec.size_bytes += 1;
        assert!(stream_file(&d.path().join("input"), None, &spec, &|| false).is_err());
    }
    #[test]
    fn space_failure_is_explicit() {
        struct Full;
        impl Write for Full {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::from_raw_os_error(libc::ENOSPC))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let d = dir();
        let spec = tiny_file(d.path());
        assert_eq!(
            stream_file(&d.path().join("input"), Some(&mut Full), &spec, &|| false)
                .unwrap_err()
                .code,
            ErrorCode::InsufficientSpace
        );
    }
    #[test]
    fn copy_checks_cancel_between_blocks() {
        let d = dir();
        let bytes = vec![42u8; 3 * 65536];
        fs::write(d.path().join("input"), &bytes).unwrap();
        let spec = PackageFile {
            path: "input".into(),
            size_bytes: bytes.len() as u64,
            sha256: hash(&bytes),
            role: "test".into(),
        };
        let checks = Cell::new(0);
        let mut copied = Vec::new();
        assert_eq!(
            stream_file(&d.path().join("input"), Some(&mut copied), &spec, &|| {
                checks.set(checks.get() + 1);
                checks.get() > 1
            })
            .unwrap_err()
            .code,
            ErrorCode::RequestCancelled
        );
        assert_eq!(copied.len(), 65536);
    }
    #[test]
    fn fixed_manifest_rejects_closure_and_provenance_changes() {
        let base = serde_json::to_value(MnnPackageManifestV1::candidate()).unwrap();
        for pointer in [
            "/identity/files/0/path",
            "/identity/files/1/sha256",
            "/identity/template/sha256",
            "/identity/artifact_source/revision",
            "/identity/variants/0/id",
        ] {
            for bad in ["../outside", "/absolute", "CONFIG.JSON", "context.json"] {
                let mut v = base.clone();
                *v.pointer_mut(pointer).unwrap() = json!(bad);
                v["artifact_digest"] = json!(hash(&serde_json::to_vec(&v["identity"]).unwrap()));
                assert!(MnnPackageManifestV1::parse(&serde_json::to_vec(&v).unwrap()).is_err());
            }
        }
        let mut v = base.clone();
        v["identity"]["files"][0]["untrusted_execution"] = json!(true);
        assert!(MnnPackageManifestV1::parse(&serde_json::to_vec(&v).unwrap()).is_err());
        let mut v = base;
        v["identity"]["artifact_source"]["provenance_kind"] = json!("self_exported");
        assert!(MnnPackageManifestV1::parse(&serde_json::to_vec(&v).unwrap()).is_err());
    }
    #[test]
    fn json_duplicates_trailing_and_oversized_rejected() {
        for bad in [
            br#"{"a":{"x":1,"x":2}}"#.as_slice(),
            br#"[ {"x":0,"x":1}]"#,
            br#"{"x":NaN}"#,
            br#"{}{}"#,
        ] {
            assert!(strict_json::parse(bad).is_err());
        }
        let d = dir();
        fs::write(d.path().join("huge"), vec![b' '; MAX_JSON as usize + 1]).unwrap();
        assert!(bounded_json(&d.path().join("huge")).is_err());
    }
    #[test]
    fn source_closure_rejects_missing_extra_directory_device_and_symlink_ancestor() {
        let d = dir();
        for f in files() {
            fs::write(d.path().join(f.path), b"").unwrap();
        }
        assert!(check_names(d.path(), false).is_ok());
        fs::write(d.path().join("context.json"), b"{}").unwrap();
        assert!(check_names(d.path(), false).is_err());
        fs::remove_file(d.path().join("context.json")).unwrap();
        fs::remove_file(d.path().join("config.json")).unwrap();
        assert!(check_names(d.path(), false).is_err());
        fs::create_dir(d.path().join("config.json")).unwrap();
        assert!(check_names(d.path(), false).is_err());
        fs::remove_dir(d.path().join("config.json")).unwrap();
        assert!(regular(Path::new("/dev/null")).is_err());
        let parent = dir();
        std::os::unix::fs::symlink(d.path(), parent.path().join("link")).unwrap();
        assert!(safe_directory(&parent.path().join("link")).is_err());
    }
    #[test]
    fn stale_staging_recovers_but_published_corruption_fails_closed() {
        let d = dir();
        let stage = d.path().join(".stage-0123456789abcdef");
        fs::create_dir(&stage).unwrap();
        fs::set_permissions(&stage, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(stage.join("partial"), b"partial").unwrap();
        let store = MnnModelStore::open(d.path()).unwrap();
        assert!(!stage.exists());
        assert!(store.entries.is_empty());
        drop(store);
        let generation = d.path().join("generation-broken");
        fs::create_dir(&generation).unwrap();
        fs::set_permissions(&generation, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(MnnModelStore::open(d.path()).is_err());
    }
    #[test]
    #[ignore = "explicit real candidate copy and on-disk tampering of its private copy only"]
    fn real_store_lease_reopen_tamper_and_interruption() {
        let source = std::path::PathBuf::from(std::env::var_os("NEXA_MNN_TEST_MODEL").unwrap());
        let d = dir();
        let mut store = MnnModelStore::open(d.path()).unwrap();
        let calls = Cell::new(0);
        assert_eq!(
            store
                .import_candidate(&source, || {
                    calls.set(calls.get() + 1);
                    calls.get() > 10
                })
                .unwrap_err()
                .code,
            ErrorCode::RequestCancelled
        );
        assert_eq!(fs::read_dir(d.path()).unwrap().count(), 1);
        assert!(store.entries.is_empty());
        store.import_candidate(&source, || false).unwrap();
        drop(store);
        let mut store = MnnModelStore::open(d.path()).unwrap();
        let snapshot = store.snapshot().unwrap();
        let id = ModelId::new(MODEL_ID).unwrap();
        let resolved = snapshot.resolve_candidate(&id).unwrap();
        let options = LoadOptions {
            context_size: 2048,
            threads: 2,
            batch_size: 32,
        };
        let lease = snapshot.acquire(&resolved, options, || false).unwrap();
        let runtime = bounded_json(lease.runtime_config()).unwrap();
        assert_eq!(runtime["thread_num"], 2);
        assert_eq!(runtime["precision"], "high");
        assert_eq!(runtime["jinja"]["context"]["enable_thinking"], false);
        assert_ne!(lease.runtime_config().parent(), resolved.path.parent());
        drop(snapshot);
        assert_eq!(
            store.remove_generation(&id).unwrap_err().code,
            ErrorCode::ModelFileInUse
        );
        drop(lease);
        let snapshot = store.snapshot().unwrap();
        let mut forged = resolved.clone();
        forged.path = source.join("manifest.json");
        assert_eq!(
            snapshot
                .acquire(&forged, options, || false)
                .err()
                .unwrap()
                .code,
            ErrorCode::ModelConflict
        );
        let root = resolved.path.parent().unwrap();
        fs::write(root.join("context.json"), b"{}").unwrap();
        assert!(snapshot.acquire(&resolved, options, || false).is_err());
        fs::remove_file(root.join("context.json")).unwrap();
        let original = fs::read(root.join("config.json")).unwrap();
        fs::write(root.join("config.json"), b"{\"llm_model\":\"../outside\"}").unwrap();
        assert!(snapshot.acquire(&resolved, options, || false).is_err());
        fs::write(root.join("config.json"), &original).unwrap();
        fs::hard_link(root.join("config.json"), d.path().join("hard-copy")).unwrap();
        assert!(snapshot.acquire(&resolved, options, || false).is_err());
        fs::remove_file(d.path().join("hard-copy")).unwrap();
        fs::rename(root.join("config.json"), d.path().join("saved-config")).unwrap();
        std::os::unix::fs::symlink(d.path().join("saved-config"), root.join("config.json"))
            .unwrap();
        assert!(snapshot.acquire(&resolved, options, || false).is_err());
        fs::remove_file(root.join("config.json")).unwrap();
        fs::rename(d.path().join("saved-config"), root.join("config.json")).unwrap();
        let work = snapshot
            .acquire(&resolved, options, || false)
            .unwrap()
            .work_directory()
            .to_path_buf();
        assert!(!work.exists());
        drop(snapshot);
        store.remove_generation(&id).unwrap();
        assert_eq!(fs::read_dir(d.path()).unwrap().count(), 1);
    }
}

#[cfg(test)]
mod transaction_tests {
    use super::*;
    fn root() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        fs::set_permissions(d.path(), fs::Permissions::from_mode(0o700)).unwrap();
        d
    }
    // No inference uses these fixtures: publication state-machine tests need only
    // the exact directory/manifest shape, not 450 MB of real model bytes.
    fn stage(store: &MnnModelStore) -> tempfile::TempDir {
        let d = tempfile::Builder::new()
            .prefix(".stage-")
            .rand_bytes(16)
            .tempdir_in(&store.root.path)
            .unwrap();
        fs::set_permissions(d.path(), fs::Permissions::from_mode(0o700)).unwrap();
        for f in files() {
            fs::write(d.path().join(f.path), b"fixture").unwrap();
        }
        fs::write(
            d.path().join("manifest.json"),
            serde_json::to_vec(&MnnPackageManifestV1::candidate()).unwrap(),
        )
        .unwrap();
        d
    }
    fn inject(store: &mut MnnModelStore, point: &'static str, errno: i32) {
        store.fault = Some(Fault { point, errno });
    }
    #[test]
    fn publish_fsync_failure_registers_and_poison_blocks_duplicate() {
        for errno in [libc::EIO, libc::ENOSPC] {
            let d = root();
            let mut store = MnnModelStore::open(d.path()).unwrap();
            let staged = stage(&store);
            inject(&mut store, "publish_sync", errno);
            assert!(
                store
                    .publish(staged, MnnPackageManifestV1::candidate())
                    .is_err()
            );
            assert_eq!(store.entries.len(), 1);
            assert!(store.snapshot().is_err());
            assert_eq!(
                store.import_candidate(d.path(), || false).unwrap_err().code,
                ErrorCode::ModelConflict
            );
            assert!(
                store
                    .remove_generation(&ModelId::new(MODEL_ID).unwrap())
                    .is_err()
            );
            assert_eq!(fs::read_dir(d.path()).unwrap().count(), 2);
            drop(store);
            let reopened = MnnModelStore::open(d.path()).unwrap();
            assert_eq!(reopened.snapshot().unwrap().entries.len(), 1);
        }
    }
    #[test]
    fn publish_rename_failure_keeps_registry_empty_and_allows_retry() {
        let d = root();
        let mut store = MnnModelStore::open(d.path()).unwrap();
        let staged = stage(&store);
        inject(&mut store, "publish_rename", libc::EIO);
        assert!(
            store
                .publish(staged, MnnPackageManifestV1::candidate())
                .is_err()
        );
        assert!(store.snapshot().unwrap().entries.is_empty());
        assert_eq!(fs::read_dir(d.path()).unwrap().count(), 1);
        store
            .publish(stage(&store), MnnPackageManifestV1::candidate())
            .unwrap();
    }
    #[test]
    fn delete_rename_failure_preserves_generation() {
        let d = root();
        let mut store = MnnModelStore::open(d.path()).unwrap();
        store
            .publish(stage(&store), MnnPackageManifestV1::candidate())
            .unwrap();
        inject(&mut store, "remove_rename", libc::EIO);
        let id = ModelId::new(MODEL_ID).unwrap();
        assert!(store.remove_generation(&id).is_err());
        assert_eq!(store.snapshot().unwrap().entries.len(), 1);
        drop(store);
        assert_eq!(
            MnnModelStore::open(d.path())
                .unwrap()
                .snapshot()
                .unwrap()
                .entries
                .len(),
            1
        );
    }
    #[test]
    fn delete_fsync_and_partial_cleanup_recover_only_tombstone() {
        for point in ["remove_sync", "remove_partial", "remove_cleanup_sync"] {
            let d = root();
            let mut store = MnnModelStore::open(d.path()).unwrap();
            store
                .publish(stage(&store), MnnPackageManifestV1::candidate())
                .unwrap();
            inject(&mut store, point, libc::EIO);
            assert!(
                store
                    .remove_generation(&ModelId::new(MODEL_ID).unwrap())
                    .is_err()
            );
            assert!(store.entries.is_empty());
            assert!(store.snapshot().is_err());
            assert!(!fs::read_dir(d.path()).unwrap().any(|e| {
                e.unwrap()
                    .file_name()
                    .to_str()
                    .unwrap()
                    .starts_with("generation-")
            }));
            drop(store);
            let reopened = MnnModelStore::open(d.path()).unwrap();
            assert!(reopened.snapshot().unwrap().entries.is_empty());
            assert_eq!(fs::read_dir(d.path()).unwrap().count(), 1);
        }
    }
    #[test]
    fn recovery_rejects_unknown_or_symlink_tombstone_without_deleting() {
        let d = root();
        let unknown = d.path().join(".trash-user-files");
        fs::create_dir(&unknown).unwrap();
        fs::write(unknown.join("keep"), b"keep").unwrap();
        assert!(MnnModelStore::open(d.path()).is_err());
        assert!(unknown.join("keep").exists());
        fs::remove_dir_all(unknown).unwrap();
        let outside = root();
        fs::write(outside.path().join("keep"), b"keep").unwrap();
        std::os::unix::fs::symlink(outside.path(), d.path().join(".trash-0123456789abcdef"))
            .unwrap();
        assert!(MnnModelStore::open(d.path()).is_err());
        assert!(outside.path().join("keep").exists());
    }
}
