use crate::{BridgeError, Result, dto::DesktopPreferences};
use runtime_api::{Config, token::load_private_token};
use std::{
    fs,
    io::{self, Read},
    path::Path,
};
const PREFERENCES: &str = "desktop-settings.json";

pub(crate) fn read_bounded(path: &Path, maximum: usize) -> Result<Vec<u8>> {
    if !fs::symlink_metadata(path)
        .map_err(|_| BridgeError::new("configuration_unavailable"))?
        .file_type()
        .is_file()
    {
        return Err(BridgeError::new("unsafe_file"));
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .and_then(|f| f.take((maximum + 1) as u64).read_to_end(&mut bytes))
        .map_err(|_| BridgeError::new("configuration_unavailable"))?;
    if bytes.len() > maximum {
        return Err(BridgeError::new("configuration_invalid"));
    }
    Ok(bytes)
}
pub(crate) fn config(root: &Path) -> Result<Config> {
    runtime_api::configuration::read(root)
        .map(|document| document.config)
        .map_err(BridgeError::from)
}
/// Uninitialized model-library operations retain the historical default, while
/// malformed or unreadable existing configuration must never silently downgrade.
pub(crate) fn verification_timeout(root: &Path) -> Result<std::time::Duration> {
    match fs::symlink_metadata(root.join("config.toml")) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Ok(model_store::library::SCAN_TIMEOUT)
        }
        _ => Ok(config(root)?.model_verification_timeout()),
    }
}
/// Bounded metadata lookup only; never opens/hashes any GGUF. Even a currently
/// loaded external model may idle-unload before API admission, so the local
/// transport allows its prospective verification without extending native work.
pub(crate) fn external_verification_budget(
    root: &Path,
    model: &str,
) -> Result<Option<std::time::Duration>> {
    let id = runtime_types::ModelId::new(model).map_err(|_| BridgeError::new("invalid_request"))?;
    let library = model_store::library::ModelLibrary::read(root)
        .map_err(|e| BridgeError::new(e.code.as_str()))?;
    if library
        .as_ref()
        .is_some_and(|library| library.entry(&id).is_some())
    {
        Ok(Some(config(root)?.model_verification_timeout()))
    } else {
        Ok(None)
    }
}
pub(crate) fn require_initialized(root: &Path) -> Result<Config> {
    load_private_token(root).map_err(|_| BridgeError::new("credentials_unavailable"))?;
    config(root)
}
pub(crate) fn preferences(root: &Path) -> Result<DesktopPreferences> {
    if root.join("config.toml").exists() && config(root)?.schema_version == 2 {
        let config = config(root)?;
        let ui = runtime_api::configuration::ui_preferences_get(root)
            .map_err(BridgeError::from)?
            .preferences;
        return Ok(DesktopPreferences {
            context_size: config.inference.context_size,
            threads: config.load_options().threads,
            batch_size: config.inference.batch_size,
            max_output_tokens: config.inference.max_output_tokens,
            close_runtime_on_exit: ui.close_runtime_on_exit,
            download_source: serde_json::from_value(serde_json::Value::String(ui.download_source))
                .map_err(|_| BridgeError::new("settings_invalid"))?,
        });
    }
    let path = root.join(PREFERENCES);
    if fs::symlink_metadata(&path).is_err_and(|e| e.kind() == io::ErrorKind::NotFound) {
        return Ok(DesktopPreferences::default());
    }
    let result: DesktopPreferences = serde_json::from_slice(&read_bounded(&path, 4096)?)
        .map_err(|_| BridgeError::new("settings_invalid"))?;
    validate(&result)?;
    Ok(result)
}
pub(crate) fn validate(settings: &DesktopPreferences) -> Result<()> {
    runtime_types::LoadOptions {
        context_size: settings.context_size,
        threads: settings.threads,
        batch_size: settings.batch_size,
    }
    .validate()
    .map_err(|_| BridgeError::new("settings_invalid"))?;
    if !(1..=runtime_types::MAX_OUTPUT_TOKENS).contains(&settings.max_output_tokens) {
        return Err(BridgeError::new("settings_invalid"));
    }
    Ok(())
}
pub(crate) fn save_preferences(root: &Path, preferences: &DesktopPreferences) -> Result<()> {
    validate(preferences)?;
    let _lock =
        runtime_api::configuration::ConfigurationLock::acquire(root).map_err(BridgeError::from)?;
    if root.join("config.toml").exists() && config(root)?.schema_version == 2 {
        return Err(BridgeError::new("configuration_revision_required"));
    }
    let bytes =
        serde_json::to_vec(preferences).map_err(|_| BridgeError::new("settings_invalid"))?;
    atomic_replace(&root.join(PREFERENCES), &bytes)
}
/// Publish one fully-synced private file. No remove+rename gap and no rewriting
/// runtime configuration to store unrelated UI preferences.
pub(crate) fn atomic_replace(target: &Path, bytes: &[u8]) -> Result<()> {
    runtime_api::token::create_private_dir(
        target
            .parent()
            .ok_or_else(|| BridgeError::new("unsafe_file"))?,
    )
    .map_err(|_| BridgeError::new("unsafe_file"))?;
    runtime_api::token::atomic_replace_private(target, bytes).map_err(|e| {
        BridgeError::new(if e.to_string() == "configuration_durability_unconfirmed" {
            "settings_durability_unconfirmed"
        } else {
            "settings_write_failed"
        })
    })
}

#[cfg(test)]
mod policy_tests {
    use super::*;
    use std::time::Duration;
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

    #[test]
    fn verification_config_defaults_only_when_absent_and_reads_once_per_operation() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private");
        runtime_api::token::create_private_dir(&root).unwrap();
        assert_eq!(
            verification_timeout(&root).unwrap(),
            Duration::from_secs(300)
        );
        assert!(!&root.join("config.toml").exists());
        let mut config = Config::default();
        config.runtime.model_verification_timeout_seconds = 30;
        fs::write(root.join("config.toml"), config.to_toml().unwrap()).unwrap();
        let frozen_budget = verification_timeout(&root).unwrap();
        config.runtime.model_verification_timeout_seconds = 7200;
        fs::write(root.join("config.toml"), config.to_toml().unwrap()).unwrap();
        assert_eq!(frozen_budget, Duration::from_secs(30));
        assert_eq!(
            verification_timeout(&root).unwrap(),
            Duration::from_secs(7200)
        );
        fs::write(root.join("config.toml"), b"malformed").unwrap();
        assert_eq!(
            verification_timeout(&root).unwrap_err().code,
            "configuration_invalid"
        );
    }
    #[test]
    fn transport_budget_is_only_for_registered_external_models_without_payload_io() {
        use model_store::library::{LIBRARY_FILE, ScanControl, scan_directory};
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private");
        runtime_api::token::create_private_dir(&root).unwrap();
        let source = tempfile::tempdir().unwrap();
        let path = source.path().join("tiny.gguf");
        tiny_model(&path);
        let scanned = scan_directory(&root, source.path(), None, &ScanControl::default()).unwrap();
        let library = scanned.library().unwrap().clone();
        let id = library.models[0].manifest.id.clone();
        fs::write(root.join(LIBRARY_FILE), library.encode().unwrap()).unwrap();
        drop(scanned);
        let mut config = Config::default();
        config.runtime.model_verification_timeout_seconds = 7200;
        fs::write(root.join("config.toml"), config.to_toml().unwrap()).unwrap();
        assert_eq!(
            external_verification_budget(&root, id.as_str()).unwrap(),
            Some(Duration::from_secs(7200))
        );
        assert_eq!(
            external_verification_budget(&root, "managed").unwrap(),
            None
        );
        fs::remove_file(path).unwrap();
        assert_eq!(
            external_verification_budget(&root, id.as_str()).unwrap(),
            Some(Duration::from_secs(7200))
        );
        fs::write(root.join("config.toml"), b"malformed").unwrap();
        assert_eq!(
            external_verification_budget(&root, "managed").unwrap(),
            None
        );
        assert_eq!(
            external_verification_budget(&root, id.as_str())
                .unwrap_err()
                .code,
            "configuration_invalid"
        );
        fs::write(root.join(LIBRARY_FILE), b"malformed").unwrap();
        assert_eq!(
            external_verification_budget(&root, id.as_str())
                .unwrap_err()
                .code,
            "model_library_changed"
        );
    }
}
