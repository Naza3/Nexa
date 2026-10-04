//! Installation-scoped observations. These never modify curated validation or
//! capabilities, and contain no prompt, response, or private path strings.
use crate::{Result, inventory::InventoryEntry, library};
use runtime_types::{ErrorCode, LoadOptions, ModelId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

const FILE: &str = "local-model-validation.json";
const MAX_BYTES: usize = 256 * 1024;
const MAX_RECEIPTS: usize = 128;
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationState {
    #[default]
    Untested,
    Loaded,
    Passed,
    Failed,
    Stale,
    Deferred,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalValidation {
    pub state: ValidationState,
    pub checked_at_unix_ms: Option<u64>,
    pub error_code: Option<String>,
    pub load_success: bool,
    pub generation_pass: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    pub model_id: ModelId,
    pub model_sha256: String,
    pub template_sha256: String,
    pub file_identity: library::FileIdentity,
    pub engine_build: String,
    pub platform: String,
    pub installation: String,
    pub options: LoadOptions,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub scope: Scope,
    pub observation: LocalValidation,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipts {
    pub schema_version: u32,
    pub entries: Vec<Receipt>,
}
impl Default for Receipts {
    fn default() -> Self {
        Self {
            schema_version: 1,
            entries: Vec::new(),
        }
    }
}
impl Receipts {
    pub fn read(root: &Path) -> Result<Self> {
        let path = root.join(FILE);
        if !path.try_exists().map_err(crate::io_error)? {
            return Ok(Self::default());
        }
        library::validate_directory_candidate(root)?;
        let file = library::open_read_file(&path, false)?;
        if file.metadata().map_err(crate::io_error)?.len() > MAX_BYTES as u64 {
            return Err(invalid());
        }
        let mut bytes = Vec::new();
        file.take(MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(crate::io_error)?;
        if bytes.len() > MAX_BYTES {
            return Err(invalid());
        }
        let receipts: Self = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        if receipts.schema_version != 1
            || receipts.entries.len() > MAX_RECEIPTS
            || receipts.entries.iter().any(|r| {
                r.scope.engine_build.len() > 256
                    || r.scope.platform.len() > 128
                    || r.scope.installation.len() != 64
                    || r.scope.model_sha256.len() != 64
                    || r.scope.template_sha256.len() != 64
                    || r.scope.options.validate().is_err()
                    || r.observation.error_code.as_ref().is_some_and(|s| {
                        s.len() > 96 || !s.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
                    })
            })
        {
            return Err(invalid());
        }
        Ok(receipts)
    }
    pub fn observation(&self, scope: &Scope, available: bool) -> LocalValidation {
        let Some(receipt) = self
            .entries
            .iter()
            .rev()
            .find(|r| r.scope.model_id == scope.model_id)
        else {
            return LocalValidation::default();
        };
        let mut observation = receipt.observation.clone();
        if !available || &receipt.scope != scope {
            observation.state = ValidationState::Stale;
        }
        observation
    }
    pub fn record(&mut self, root: &Path, receipt: Receipt) -> Result<()> {
        self.entries
            .retain(|r| r.scope.model_id != receipt.scope.model_id);
        self.entries.push(receipt);
        if self.entries.len() > MAX_RECEIPTS {
            self.entries.remove(0);
        }
        let bytes = serde_json::to_vec(self).map_err(|_| invalid())?;
        if bytes.len() > MAX_BYTES {
            return Err(invalid());
        }
        library::validate_directory_candidate(root)?;
        let target = root.join(FILE);
        if target.try_exists().map_err(crate::io_error)? {
            let _ = library::open_read_file(&target, false)?;
        }
        let temp = root.join(format!(".validation-{}.tmp", uuid::Uuid::new_v4()));
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp).map_err(crate::io_error)?;
        let result = (|| {
            file.write_all(&bytes).map_err(crate::io_error)?;
            file.sync_all().map_err(crate::io_error)?;
            drop(file);
            replace(&temp, &target).map_err(crate::io_error)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result
    }
}
fn invalid() -> runtime_types::RuntimeError {
    library::library_error(ErrorCode::InvalidManifest)
}
pub fn scope(
    root: &Path,
    entry: &InventoryEntry,
    options: LoadOptions,
    engine_build: &str,
) -> Result<Scope> {
    let (volume, file) = library::directory_object_identity(root)?;
    let installation = format!(
        "{:x}",
        Sha256::digest(format!("{volume}:{file}:{}", std::env::consts::OS).as_bytes())
    );
    Ok(Scope {
        model_id: entry.manifest.id.clone(),
        model_sha256: entry.manifest.sha256.clone(),
        template_sha256: entry.manifest.template_sha256.clone(),
        file_identity: entry.file_stamp.clone().ok_or_else(invalid)?,
        engine_build: engine_build.into(),
        platform: format!(
            "{}-{}-cpu-{}",
            std::env::consts::OS,
            std::env::consts::ARCH,
            std::thread::available_parallelism().map_or(1, |p| p.get())
        ),
        installation,
        options,
    })
}
/// The actual paired executables identify engine/shim/native code as well as
/// the Rust build. This reads no model; bounded to 128 MiB per executable.
pub fn engine_file_stamps(runtime_executable: &Path) -> Result<Vec<library::FileIdentity>> {
    let worker = runtime_executable.with_file_name(if cfg!(windows) {
        "ai-runtime-worker.exe"
    } else {
        "ai-runtime-worker"
    });
    [runtime_executable, worker.as_path()]
        .iter()
        .map(|path| library::open_read_file(path, false).and_then(|file| library::identity(&file)))
        .collect()
}
pub fn engine_build(runtime_executable: &Path) -> Result<String> {
    let worker = runtime_executable.with_file_name(if cfg!(windows) {
        "ai-runtime-worker.exe"
    } else {
        "ai-runtime-worker"
    });
    let mut hash = Sha256::new();
    hash.update(b"nexa-cpu-text-probe-v1");
    for path in [runtime_executable, worker.as_path()] {
        let mut file = library::open_read_file(path, false)?;
        if file.metadata().map_err(crate::io_error)?.len() > 128 * 1024 * 1024 {
            return Err(invalid());
        }
        let mut bytes = [0; 64 * 1024];
        let mut total = 0;
        loop {
            let n = file.read(&mut bytes).map_err(crate::io_error)?;
            if n == 0 {
                break;
            }
            total += n;
            if total > 128 * 1024 * 1024 {
                return Err(invalid());
            }
            hash.update(&bytes[..n]);
        }
        hash.update([0]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
#[cfg(not(windows))]
fn replace(source: &Path, target: &Path) -> std::io::Result<()> {
    fs::rename(source, target)
}
#[cfg(windows)]
fn replace(source: &Path, target: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    let s: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let t: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: both terminated strings remain alive throughout the call.
    if unsafe {
        MoveFileExW(
            s.as_ptr(),
            t.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scope() -> Scope {
        Scope {
            model_id: ModelId::new("fixture").unwrap(),
            model_sha256: "0".repeat(64),
            template_sha256: "1".repeat(64),
            file_identity: library::FileIdentity {
                volume: 1,
                file: 2,
                size: 3,
                modified_seconds: 4,
                modified_nanos: 5,
            },
            engine_build: "actual-build".into(),
            platform: "windows-x86_64-cpu-6".into(),
            installation: "2".repeat(64),
            options: LoadOptions::default(),
        }
    }
    fn passed() -> LocalValidation {
        LocalValidation {
            state: ValidationState::Passed,
            checked_at_unix_ms: Some(now_ms()),
            error_code: None,
            load_success: true,
            generation_pass: true,
        }
    }
    #[test]
    fn receipts_roundtrip_without_text_and_invalidate_every_scope_dimension() {
        let root = tempfile::tempdir().unwrap();
        let expected = scope();
        let mut receipts = Receipts::default();
        receipts
            .record(
                root.path(),
                Receipt {
                    scope: expected.clone(),
                    observation: passed(),
                },
            )
            .unwrap();
        let receipts = Receipts::read(root.path()).unwrap();
        assert_eq!(
            receipts.observation(&expected, true).state,
            ValidationState::Passed
        );
        let mut changes = Vec::new();
        let mut s = expected.clone();
        s.model_sha256 = "a".repeat(64);
        changes.push(s);
        let mut s = expected.clone();
        s.template_sha256 = "b".repeat(64);
        changes.push(s);
        let mut s = expected.clone();
        s.engine_build = "different".into();
        changes.push(s);
        let mut s = expected.clone();
        s.platform = "different".into();
        changes.push(s);
        let mut s = expected.clone();
        s.installation = "c".repeat(64);
        changes.push(s);
        let mut s = expected.clone();
        s.file_identity.modified_nanos += 1;
        changes.push(s);
        let mut s = expected.clone();
        s.options.context_size += 1;
        changes.push(s);
        let mut s = expected.clone();
        s.options.threads += 1;
        changes.push(s);
        let mut s = expected.clone();
        s.options.batch_size += 1;
        changes.push(s);
        for changed in changes {
            let observation = receipts.observation(&changed, true);
            assert_eq!(observation.state, ValidationState::Stale);
            assert!(observation.load_success);
        }
        assert_eq!(
            receipts.observation(&expected, false).state,
            ValidationState::Stale
        );
        let text = fs::read_to_string(root.path().join(FILE)).unwrap();
        for private in ["messages", "prompt", "content", "response", "Reply with"] {
            assert!(!text.contains(private));
        }
        let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
        value["schema_version"] = 2.into();
        fs::write(root.path().join(FILE), serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(Receipts::read(root.path()).is_err());
        value.as_object_mut().unwrap().remove("schema_version");
        fs::write(root.path().join(FILE), serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(Receipts::read(root.path()).is_err());
    }
    #[test]
    fn receipt_failure_preserves_load_and_repeated_record_is_bounded() {
        let root = tempfile::tempdir().unwrap();
        let mut receipts = Receipts::default();
        for i in 0..140 {
            let mut s = scope();
            s.model_id = ModelId::new(format!("fixture-{i}")).unwrap();
            receipts
                .record(
                    root.path(),
                    Receipt {
                        scope: s,
                        observation: LocalValidation {
                            state: ValidationState::Failed,
                            error_code: Some("execution_timeout".into()),
                            generation_pass: false,
                            ..passed()
                        },
                    },
                )
                .unwrap();
        }
        assert_eq!(receipts.entries.len(), MAX_RECEIPTS);
        assert!(receipts.entries.last().unwrap().observation.load_success);
        let receipt = receipts.entries.last().unwrap().clone();
        receipts.record(root.path(), receipt).unwrap();
        assert_eq!(
            Receipts::read(root.path()).unwrap().entries.len(),
            MAX_RECEIPTS
        );
    }
}
