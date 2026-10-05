//! One bounded configuration schema, resolver and cross-process CAS publisher.
//! Read-only calls never initialize credentials or migrate existing data.
use crate::{Config, LanApiConfig, token};
use fs2::FileExt;
use runtime_types::{LoadOptions, ModelId, RuntimeError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read},
    net::SocketAddr,
    path::Path,
};

pub const MAX_CONFIG_BYTES: usize = 65_536;
pub const MAX_MODEL_PROFILES: usize = 128;
const PREFERENCES: &str = "desktop-settings.json";
pub type Result<T> = std::result::Result<T, ConfigurationError>;
#[derive(Debug, Clone)]
pub struct ConfigurationError {
    pub code: &'static str,
    pub param: Option<&'static str>,
}
impl std::fmt::Display for ConfigurationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code)
    }
}
impl std::error::Error for ConfigurationError {}
fn error(code: &'static str) -> ConfigurationError {
    ConfigurationError {
        code,
        param: if code == "configuration_conflict" {
            Some("expected_revision")
        } else {
            None
        },
    }
}
fn error_at(code: &'static str, param: &'static str) -> ConfigurationError {
    ConfigurationError {
        code,
        param: Some(param),
    }
}
impl From<ConfigurationError> for crate::ApiError {
    fn from(e: ConfigurationError) -> Self {
        use axum::http::StatusCode;
        let status = match e.code {
            "configuration_conflict"
            | "configuration_migration_required"
            | "configuration_restart_required"
            | "configuration_busy"
            | "runtime_running"
            | "configuration_revision_required" => StatusCode::CONFLICT,
            "configuration_invalid" | "model_profile_invalid" => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self::new(
            status,
            e.code,
            if e.code == "model_profile_invalid"
                && e.param.is_some_and(|p| p.ends_with(".context_size"))
            {
                "Context size exceeds the selected model's declared limit."
            } else {
                "Configuration could not be safely read or changed. Check the indicated field or group, and refresh before retrying."
            },
            e.param,
        )
    }
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct LoadOverrides {
    pub context_size: Option<u32>,
    pub threads: Option<u32>,
    pub batch_size: Option<u32>,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LoadDefaults {
    pub context_size: u32,
    pub threads: Option<u32>,
    pub batch_size: u32,
}
impl<'de> Deserialize<'de> for LoadDefaults {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Full {
            context_size: u32,
            threads: serde_json::Value,
            batch_size: u32,
        }
        let f = Full::deserialize(d)?;
        Ok(Self {
            context_size: f.context_size,
            threads: serde_json::from_value(f.threads).map_err(serde::de::Error::custom)?,
            batch_size: f.batch_size,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RequestDefaults {
    pub max_output_tokens: u32,
    pub temperature: f32,
    pub top_p: f32,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimePolicies {
    pub idle_unload_enabled: bool,
    pub idle_unload_seconds: u64,
    pub model_verification_timeout_seconds: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LocalApi {
    pub listen: SocketAddr,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileEntry {
    pub model_id: ModelId,
    pub load_overrides: LoadOverrides,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ConfigurationValues {
    pub global_defaults: LoadDefaults,
    pub request_defaults: RequestDefaults,
    pub runtime: RuntimePolicies,
    pub local_api: LocalApi,
    pub lan_api: LanApiConfig,
    pub model_profiles: Vec<ProfileEntry>,
}
impl From<&Config> for ConfigurationValues {
    fn from(c: &Config) -> Self {
        Self {
            global_defaults: LoadDefaults {
                context_size: c.inference.context_size,
                threads: c.inference.threads,
                batch_size: c.inference.batch_size,
            },
            request_defaults: RequestDefaults {
                max_output_tokens: c.inference.max_output_tokens,
                temperature: c.inference.temperature,
                top_p: c.inference.top_p,
            },
            runtime: RuntimePolicies {
                idle_unload_enabled: c.runtime.idle_unload_enabled,
                idle_unload_seconds: c.runtime.idle_unload_seconds,
                model_verification_timeout_seconds: c.runtime.model_verification_timeout_seconds,
            },
            local_api: LocalApi {
                listen: c.api.listen,
            },
            lan_api: c.lan_api.clone(),
            model_profiles: c
                .model_profiles
                .iter()
                .map(|(id, p)| ProfileEntry {
                    model_id: id.clone(),
                    load_overrides: *p,
                })
                .collect(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MigrationChoice {
    Api,
    Desktop,
    Custom,
    Equal,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MigrationReceipt {
    pub legacy_config_revision: String,
    pub legacy_preferences_revision: Option<String>,
    pub choice: MigrationChoice,
}
impl MigrationReceipt {
    pub(crate) fn validate(&self) -> std::result::Result<(), RuntimeError> {
        if !valid_revision(&self.legacy_config_revision)
            || self
                .legacy_preferences_revision
                .as_ref()
                .is_some_and(|r| !valid_revision(r))
        {
            return Err(RuntimeError::invalid("invalid migration revision"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MigrationDifference {
    pub field: String,
    pub api: Option<u32>,
    pub desktop: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MigrationStatus {
    pub state: String,
    pub preferences_revision: Option<String>,
    pub differences: Vec<MigrationDifference>,
    pub backup_available: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EffectiveConfiguration {
    pub revision: String,
    pub values: ConfigurationValues,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConfigurationSnapshot {
    pub schema_version: u32,
    pub revision: String,
    pub saved: ConfigurationValues,
    pub runtime_effective: Option<EffectiveConfiguration>,
    pub pending_restart: bool,
    pub migration: MigrationStatus,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LoadSources {
    pub context_size: String,
    pub threads: String,
    pub batch_size: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelConfiguration {
    pub configuration_revision: String,
    pub model_id: ModelId,
    pub load_overrides: LoadOverrides,
    pub saved_effective: LoadOptions,
    pub saved_sources: LoadSources,
    pub current_load_options: Option<LoadOptions>,
    pub restore_load_options: Option<LoadOptions>,
    pub pending_apply: bool,
    pub context_limit: Option<u32>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConfigurationUpdate {
    ModelProfile {
        model_id: ModelId,
        #[serde(deserialize_with = "full_overrides")]
        load_overrides: LoadOverrides,
    },
    GlobalDefaults {
        global_defaults: LoadDefaults,
    },
    RequestDefaults {
        request_defaults: RequestDefaults,
    },
    Runtime {
        runtime: RuntimePolicies,
    },
    LocalApi {
        local_api: LocalApi,
    },
    LanApi {
        #[serde(deserialize_with = "full_lan")]
        lan_api: LanApiConfig,
    },
}
impl ConfigurationUpdate {
    fn param(&self) -> &'static str {
        match self {
            Self::ModelProfile { .. } => "update.load_overrides",
            Self::GlobalDefaults { .. } => "update.global_defaults",
            Self::RequestDefaults { .. } => "update.request_defaults",
            Self::Runtime { .. } => "update.runtime",
            Self::LocalApi { .. } => "update.local_api",
            Self::LanApi { .. } => "update.lan_api",
        }
    }
}
/// Wire failures identify only static known groups; never echo a decoder's raw
/// message, an unknown key, a filesystem path, or configuration contents.
pub fn parse_save_request(bytes: &[u8]) -> Result<ConfigurationSaveRequest> {
    serde_json::from_slice(bytes).map_err(|_| {
        let value = serde_json::from_slice::<serde_json::Value>(bytes).ok();
        let param = match value.as_ref() {
            None => "body",
            Some(v)
                if v.get("expected_revision")
                    .and_then(|v| v.as_str())
                    .is_none() =>
            {
                "expected_revision"
            }
            Some(v) => match v
                .get("update")
                .and_then(|v| v.get("kind"))
                .and_then(|v| v.as_str())
            {
                Some("model_profile") => "update.load_overrides",
                Some("global_defaults") => "update.global_defaults",
                Some("request_defaults") => "update.request_defaults",
                Some("runtime") => "update.runtime",
                Some("local_api") => "update.local_api",
                Some("lan_api") => "update.lan_api",
                _ => "update",
            },
        };
        error_at("configuration_invalid", param)
    })
}
fn full_lan<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<LanApiConfig, D::Error> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Full {
        enabled: bool,
        listen: serde_json::Value,
        allowed_cidrs: Vec<String>,
    }
    let f = Full::deserialize(d)?;
    Ok(LanApiConfig {
        enabled: f.enabled,
        listen: serde_json::from_value(f.listen).map_err(serde::de::Error::custom)?,
        allowed_cidrs: f.allowed_cidrs,
    })
}
fn full_overrides<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<LoadOverrides, D::Error> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Full {
        context_size: serde_json::Value,
        threads: serde_json::Value,
        batch_size: serde_json::Value,
    }
    let f = Full::deserialize(d)?;
    Ok(LoadOverrides {
        context_size: serde_json::from_value(f.context_size).map_err(serde::de::Error::custom)?,
        threads: serde_json::from_value(f.threads).map_err(serde::de::Error::custom)?,
        batch_size: serde_json::from_value(f.batch_size).map_err(serde::de::Error::custom)?,
    })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigurationSaveRequest {
    pub expected_revision: String,
    pub update: ConfigurationUpdate,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomDefaults {
    pub global_defaults: LoadDefaults,
    pub request_defaults: RequestDefaults,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigurationMigrateRequest {
    pub expected_revision: String,
    pub expected_preferences_revision: Option<String>,
    pub choice: MigrationChoice,
    pub custom: Option<CustomDefaults>,
}
#[derive(Clone, Debug)]
pub struct Document {
    pub config: Config,
    pub revision: String,
    pub bytes: Vec<u8>,
}
pub fn revision(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}
fn valid_revision(s: &str) -> bool {
    s.len() == 71
        && s.starts_with("sha256:")
        && s.as_bytes()[7..]
            .iter()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c))
}
pub fn read_bytes(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let file = token::open_regular_file(path).map_err(|_| error("configuration_unavailable"))?;
    let mut bytes = Vec::new();
    file.take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| error("configuration_unavailable"))?;
    if bytes.len() > limit {
        return Err(error("configuration_invalid"));
    }
    Ok(bytes)
}
pub fn read(root: &Path) -> Result<Document> {
    let bytes = read_bytes(&root.join("config.toml"), MAX_CONFIG_BYTES)?;
    let config =
        Config::from_toml(std::str::from_utf8(&bytes).map_err(|_| error("configuration_invalid"))?)
            .map_err(|_| error("configuration_invalid"))?;
    Ok(Document {
        config,
        revision: revision(&bytes),
        bytes,
    })
}
pub struct ConfigurationLock {
    _file: fs::File,
}
impl ConfigurationLock {
    pub fn acquire(root: &Path) -> Result<Self> {
        token::create_private_dir(root).map_err(|_| error("configuration_unavailable"))?;
        let dir = root.join("runtime");
        token::create_private_dir(&dir).map_err(|_| error("configuration_unavailable"))?;
        let path = dir.join("configuration.lock");
        match token::write_private_new(&path, b"") {
            Ok(()) => (),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
            Err(_) => return Err(error("configuration_unavailable")),
        }
        let file =
            token::open_private_file(&path).map_err(|_| error("configuration_unavailable"))?;
        file.try_lock_exclusive()
            .map_err(|_| error("configuration_busy"))?;
        Ok(Self { _file: file })
    }
}
pub fn resolve_load_options(
    config: &Config,
    id: &ModelId,
    explicit: LoadOverrides,
) -> std::result::Result<LoadOptions, RuntimeError> {
    let profile = config.model_profiles.get(id).copied().unwrap_or_default();
    let global = config.load_options();
    let options = LoadOptions {
        context_size: explicit
            .context_size
            .or(profile.context_size)
            .unwrap_or(global.context_size),
        threads: explicit
            .threads
            .or(profile.threads)
            .unwrap_or(global.threads),
        batch_size: explicit
            .batch_size
            .or(profile.batch_size)
            .unwrap_or(global.batch_size),
    };
    options.validate()?;
    Ok(options)
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct UiPreferences {
    pub close_runtime_on_exit: bool,
    pub download_source: String,
}
impl Default for UiPreferences {
    fn default() -> Self {
        Self {
            close_runtime_on_exit: false,
            download_source: "modelscope".into(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UiPreferencesSnapshot {
    pub revision: String,
    pub preferences: UiPreferences,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiPreferencesSaveRequest {
    pub expected_revision: String,
    pub preferences: UiPreferences,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UiFile {
    schema_version: u32,
    close_runtime_on_exit: bool,
    download_source: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyPreferences {
    context_size: u32,
    threads: u32,
    batch_size: u32,
    max_output_tokens: u32,
    close_runtime_on_exit: bool,
    #[serde(default = "default_source")]
    download_source: String,
}
fn default_source() -> String {
    "modelscope".into()
}
struct Preferences {
    bytes: Option<Vec<u8>>,
    legacy: Option<LegacyPreferences>,
    ui: UiPreferences,
}
fn preferences(root: &Path) -> Result<Preferences> {
    preferences_inner(root, true)
}
fn preferences_inner(root: &Path, validate_inference: bool) -> Result<Preferences> {
    let path = root.join(PREFERENCES);
    if fs::symlink_metadata(&path).is_err_and(|e| e.kind() == io::ErrorKind::NotFound) {
        return Ok(Preferences {
            bytes: None,
            legacy: None,
            ui: Default::default(),
        });
    }
    let bytes = read_bytes(&path, 4096)?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| error("configuration_invalid"))?;
    let (legacy, ui) = if value.get("schema_version").is_some() {
        let p: UiFile =
            serde_json::from_slice(&bytes).map_err(|_| error("configuration_invalid"))?;
        if p.schema_version != 2 {
            return Err(error("configuration_invalid"));
        }
        (
            None,
            UiPreferences {
                close_runtime_on_exit: p.close_runtime_on_exit,
                download_source: p.download_source,
            },
        )
    } else {
        let p: LegacyPreferences =
            serde_json::from_slice(&bytes).map_err(|_| error("configuration_invalid"))?;
        if validate_inference {
            LoadOptions {
                context_size: p.context_size,
                threads: p.threads,
                batch_size: p.batch_size,
            }
            .validate()
            .map_err(|_| error("configuration_invalid"))?;
            if !(1..=runtime_types::MAX_OUTPUT_TOKENS).contains(&p.max_output_tokens) {
                return Err(error("configuration_invalid"));
            }
        }
        let ui = UiPreferences {
            close_runtime_on_exit: p.close_runtime_on_exit,
            download_source: p.download_source.clone(),
        };
        (Some(p), ui)
    };
    if !["modelscope", "huggingface"].contains(&ui.download_source.as_str()) {
        return Err(error("configuration_invalid"));
    }
    Ok(Preferences {
        bytes: Some(bytes),
        legacy,
        ui,
    })
}
pub fn ui_preferences_get(root: &Path) -> Result<UiPreferencesSnapshot> {
    let p = preferences_inner(root, false)?;
    Ok(UiPreferencesSnapshot {
        revision: p
            .bytes
            .as_ref()
            .map_or_else(|| "absent".into(), |b| revision(b)),
        preferences: p.ui,
    })
}
pub fn ui_preferences_save(
    root: &Path,
    request: UiPreferencesSaveRequest,
) -> Result<UiPreferencesSnapshot> {
    let _guard = ConfigurationLock::acquire(root)?;
    let previous = ui_preferences_get(root)?;
    if previous.revision != request.expected_revision {
        return Err(error("configuration_conflict"));
    }
    // Legacy inference fields remain migration inputs until schema2 is explicit.
    if read(root)?.config.schema_version != 2 {
        return Err(error("configuration_migration_required"));
    }
    if !["modelscope", "huggingface"].contains(&request.preferences.download_source.as_str()) {
        return Err(error("configuration_invalid"));
    }
    let bytes = serde_json::to_vec(&UiFile {
        schema_version: 2,
        close_runtime_on_exit: request.preferences.close_runtime_on_exit,
        download_source: request.preferences.download_source.clone(),
    })
    .map_err(|_| error("configuration_invalid"))?;
    publish_bytes(&root.join(PREFERENCES), &bytes)?;
    Ok(UiPreferencesSnapshot {
        revision: revision(&bytes),
        preferences: request.preferences,
    })
}
fn backups_available(root: &Path, receipt: &MigrationReceipt) -> bool {
    let check = |name: &str, expected: &str| -> bool {
        let path = root
            .join("backups/configuration")
            .join(format!("{name}-{}", &expected[7..]));
        let Ok(file) = token::open_private_file(&path) else {
            return false;
        };
        let mut bytes = Vec::new();
        file.take((MAX_CONFIG_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .is_ok()
            && bytes.len() <= MAX_CONFIG_BYTES
            && revision(&bytes) == expected
    };
    check("config.toml", &receipt.legacy_config_revision)
        && receipt
            .legacy_preferences_revision
            .as_ref()
            .is_none_or(|r| check(PREFERENCES, r))
}
fn migration(root: &Path, document: &Document) -> Result<MigrationStatus> {
    if document.config.schema_version == 2 {
        return Ok(MigrationStatus {
            state: if document.config.migration.is_some() {
                "complete"
            } else {
                "not_needed"
            }
            .into(),
            preferences_revision: None,
            differences: vec![],
            backup_available: document
                .config
                .migration
                .as_ref()
                .is_some_and(|receipt| backups_available(root, receipt)),
        });
    }
    let p = preferences(root)?;
    let mut differences = vec![];
    if let Some(old) = p.legacy {
        for (field, api, desktop) in [
            (
                "context_size",
                Some(document.config.inference.context_size),
                old.context_size,
            ),
            ("threads", document.config.inference.threads, old.threads),
            (
                "batch_size",
                Some(document.config.inference.batch_size),
                old.batch_size,
            ),
            (
                "max_output_tokens",
                Some(document.config.inference.max_output_tokens),
                old.max_output_tokens,
            ),
        ] {
            if api != Some(desktop) {
                differences.push(MigrationDifference {
                    field: field.into(),
                    api,
                    desktop,
                });
            }
        }
    }
    Ok(MigrationStatus {
        state: if differences.is_empty() {
            "legacy_compatible"
        } else {
            "required"
        }
        .into(),
        preferences_revision: p.bytes.as_ref().map(|b| revision(b)),
        differences,
        backup_available: false,
    })
}
pub fn snapshot(
    root: &Path,
    document: &Document,
    active: Option<&Document>,
) -> Result<ConfigurationSnapshot> {
    Ok(ConfigurationSnapshot {
        schema_version: document.config.schema_version,
        revision: document.revision.clone(),
        saved: (&document.config).into(),
        runtime_effective: active.map(|a| EffectiveConfiguration {
            revision: a.revision.clone(),
            values: (&a.config).into(),
        }),
        pending_restart: active.is_some_and(|a| a.revision != document.revision),
        migration: migration(root, document)?,
    })
}
pub fn preview() -> ConfigurationSnapshot {
    let c = Config {
        schema_version: 2,
        ..Default::default()
    };
    ConfigurationSnapshot {
        schema_version: 2,
        revision: "absent".into(),
        saved: (&c).into(),
        runtime_effective: None,
        pending_restart: false,
        migration: MigrationStatus {
            state: "not_needed".into(),
            preferences_revision: None,
            differences: vec![],
            backup_available: false,
        },
    }
}
pub fn model_configuration(
    document: &Document,
    id: ModelId,
    status: Option<&runtime_types::RuntimeStatus>,
    context_limit: Option<u32>,
) -> Result<ModelConfiguration> {
    let config = &document.config;
    let p = config.model_profiles.get(&id).copied().unwrap_or_default();
    let effective = resolve_load_options(config, &id, LoadOverrides::default())
        .map_err(|_| error("model_profile_invalid"))?;
    let selected = status.filter(|s| s.selected_model.as_ref() == Some(&id));
    let current = selected
        .filter(|s| {
            matches!(
                s.state,
                runtime_types::ModelState::Ready | runtime_types::ModelState::Generating
            )
        })
        .and_then(|s| s.load_options);
    let restore = selected
        .filter(|s| {
            !matches!(
                s.state,
                runtime_types::ModelState::Ready | runtime_types::ModelState::Generating
            )
        })
        .and_then(|s| s.load_options);
    Ok(ModelConfiguration {
        configuration_revision: document.revision.clone(),
        model_id: id,
        load_overrides: p,
        saved_effective: effective,
        saved_sources: LoadSources {
            context_size: if p.context_size.is_some() {
                "profile"
            } else {
                "global"
            }
            .into(),
            threads: if p.threads.is_some() {
                "profile"
            } else if config.inference.threads.is_none() {
                "automatic"
            } else {
                "global"
            }
            .into(),
            batch_size: if p.batch_size.is_some() {
                "profile"
            } else {
                "global"
            }
            .into(),
        },
        current_load_options: current,
        restore_load_options: restore,
        pending_apply: current.is_some_and(|v| v != effective),
        context_limit,
    })
}
pub fn same_non_hot(a: &Config, b: &Config) -> bool {
    let mut a = a.clone();
    let mut b = b.clone();
    a.model_profiles.clear();
    b.model_profiles.clear();
    a.inference.max_output_tokens = b.inference.max_output_tokens;
    a.inference.temperature = b.inference.temperature;
    a.inference.top_p = b.inference.top_p;
    a == b
}
fn apply_defaults(c: &mut Config, g: LoadDefaults, r: RequestDefaults) {
    c.inference.context_size = g.context_size;
    c.inference.threads = g.threads;
    c.inference.batch_size = g.batch_size;
    c.inference.max_output_tokens = r.max_output_tokens;
    c.inference.temperature = r.temperature;
    c.inference.top_p = r.top_p;
}
fn check_revision(d: &Document, expected: &str) -> Result<()> {
    if !valid_revision(expected) || d.revision != expected {
        Err(error("configuration_conflict"))
    } else {
        Ok(())
    }
}
fn encode(c: &Config) -> Result<Vec<u8>> {
    c.validate().map_err(|_| error("configuration_invalid"))?;
    let bytes = c
        .to_toml()
        .map_err(|_| error("configuration_invalid"))?
        .into_bytes();
    if bytes.len() > MAX_CONFIG_BYTES {
        return Err(error("configuration_invalid"));
    }
    Ok(bytes)
}
fn publish_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    token::atomic_replace_private(path, bytes).map_err(|e| {
        error(if e.to_string() == "configuration_durability_unconfirmed" {
            "configuration_durability_unconfirmed"
        } else {
            "configuration_write_failed"
        })
    })
}
fn publish(root: &Path, config: Config) -> Result<Document> {
    let bytes = encode(&config)?;
    publish_bytes(&root.join("config.toml"), &bytes)?;
    Ok(Document {
        config,
        revision: revision(&bytes),
        bytes,
    })
}
fn backup(root: &Path, name: &str, bytes: &[u8]) -> Result<()> {
    let dir = root.join("backups").join("configuration");
    token::create_private_dir(&dir).map_err(|_| error("configuration_write_failed"))?;
    let path = dir.join(format!("{name}-{}", &revision(bytes)[7..]));
    match token::write_private_new(&path, bytes) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            if {
                let mut old = Vec::new();
                token::open_private_file(&path)
                    .map_err(|_| error("configuration_write_failed"))?
                    .take((MAX_CONFIG_BYTES + 1) as u64)
                    .read_to_end(&mut old)
                    .map_err(|_| error("configuration_write_failed"))?;
                old
            } == bytes
            {
                Ok(())
            } else {
                Err(error("configuration_write_failed"))
            }
        }
        Err(_) => Err(error("configuration_write_failed")),
    }
}
fn migrate_locked(
    root: &Path,
    old: &Document,
    choice: MigrationChoice,
    custom: Option<CustomDefaults>,
) -> Result<Config> {
    let p = preferences(root)?;
    let mut c = old.config.clone();
    if !matches!(choice, MigrationChoice::Custom) && custom.is_some() {
        return Err(error("configuration_invalid"));
    }
    match choice {
        MigrationChoice::Desktop => {
            let p = p
                .legacy
                .as_ref()
                .ok_or_else(|| error("configuration_invalid"))?;
            c.inference.context_size = p.context_size;
            c.inference.threads = Some(p.threads);
            c.inference.batch_size = p.batch_size;
            c.inference.max_output_tokens = p.max_output_tokens;
        }
        MigrationChoice::Custom => {
            let custom = custom.ok_or_else(|| error("configuration_invalid"))?;
            apply_defaults(&mut c, custom.global_defaults, custom.request_defaults);
        }
        MigrationChoice::Api | MigrationChoice::Equal => {
            if custom.is_some() {
                return Err(error("configuration_invalid"));
            }
        }
    }
    c.schema_version = 2;
    c.migration = Some(MigrationReceipt {
        legacy_config_revision: old.revision.clone(),
        legacy_preferences_revision: p.bytes.as_ref().map(|b| revision(b)),
        choice,
    });
    encode(&c)?;
    Ok(c)
}
fn backup_sources(root: &Path, old: &Document) -> Result<()> {
    backup(root, "config.toml", &old.bytes)?;
    if let Some(bytes) = preferences(root)?.bytes {
        backup(root, PREFERENCES, &bytes)?;
    }
    Ok(())
}
pub fn migrate(root: &Path, request: ConfigurationMigrateRequest) -> Result<Document> {
    let _guard = ConfigurationLock::acquire(root)?;
    let old = read(root)?;
    check_revision(&old, &request.expected_revision)?;
    if old.config.schema_version != 1 || matches!(request.choice, MigrationChoice::Equal) {
        return Err(error("configuration_invalid"));
    }
    let p = preferences(root)?;
    if p.bytes.as_ref().map(|b| revision(b)) != request.expected_preferences_revision {
        return Err(error_at(
            "configuration_conflict",
            "expected_preferences_revision",
        ));
    }
    let config = migrate_locked(root, &old, request.choice, request.custom)?;
    backup_sources(root, &old)?;
    publish(root, config)
}
pub fn save(
    root: &Path,
    request: ConfigurationSaveRequest,
    active: Option<&Document>,
    limits: &BTreeMap<ModelId, u32>,
) -> Result<Document> {
    let group = request.update.param();
    let _guard = ConfigurationLock::acquire(root)?;
    let old = read(root)?;
    check_revision(&old, &request.expected_revision)?;
    if let Some(active) = active {
        if !matches!(
            request.update,
            ConfigurationUpdate::ModelProfile { .. } | ConfigurationUpdate::RequestDefaults { .. }
        ) {
            return Err(error("runtime_running"));
        }
        if old.revision != active.revision || !same_non_hot(&old.config, &active.config) {
            return Err(error("configuration_restart_required"));
        }
    }
    let mut c = old.config.clone();
    if c.schema_version == 1 {
        if active.is_some() || migration(root, &old)?.state == "required" {
            return Err(error("configuration_migration_required"));
        }
        c = migrate_locked(root, &old, MigrationChoice::Equal, None)?;
    }
    match request.update {
        ConfigurationUpdate::ModelProfile {
            model_id,
            load_overrides,
        } => {
            if load_overrides == LoadOverrides::default() {
                c.model_profiles.remove(&model_id);
            } else {
                c.model_profiles.insert(model_id, load_overrides);
            }
        }
        ConfigurationUpdate::GlobalDefaults { global_defaults } => {
            c.inference.context_size = global_defaults.context_size;
            c.inference.threads = global_defaults.threads;
            c.inference.batch_size = global_defaults.batch_size;
        }
        ConfigurationUpdate::RequestDefaults { request_defaults } => {
            c.inference.max_output_tokens = request_defaults.max_output_tokens;
            c.inference.temperature = request_defaults.temperature;
            c.inference.top_p = request_defaults.top_p;
        }
        ConfigurationUpdate::Runtime { runtime } => {
            if !(1..=86400).contains(&runtime.idle_unload_seconds) {
                return Err(error_at(
                    "configuration_invalid",
                    "update.runtime.idle_unload_seconds",
                ));
            }
            c.runtime.idle_unload_enabled = runtime.idle_unload_enabled;
            c.runtime.idle_unload_seconds = runtime.idle_unload_seconds;
            c.runtime.model_verification_timeout_seconds =
                runtime.model_verification_timeout_seconds;
        }
        ConfigurationUpdate::LocalApi { local_api } => {
            c.api.listen = local_api.listen;
        }
        ConfigurationUpdate::LanApi { lan_api } => {
            c.lan_api = lan_api;
        }
    }
    for id in c.model_profiles.keys() {
        let options = resolve_load_options(&c, id, LoadOverrides::default())
            .map_err(|_| error_at("model_profile_invalid", group))?;
        if limits
            .get(id)
            .is_some_and(|limit| options.context_size > *limit)
        {
            return Err(error_at(
                "model_profile_invalid",
                if group == "update.load_overrides" {
                    "update.load_overrides.context_size"
                } else if group == "update.global_defaults" {
                    "update.global_defaults.context_size"
                } else {
                    group
                },
            ));
        }
    }
    encode(&c).map_err(|mut e| {
        if e.code == "configuration_invalid" && e.param.is_none() {
            e.param = Some(group);
        }
        e
    })?;
    if c == old.config {
        return Ok(old);
    }
    if old.config.schema_version == 1 {
        backup_sources(root, &old)?;
    }
    publish(root, c)
}
/// Caller holds the stopped-service InstanceLock. Existing partial/corrupt
/// installations fail closed and cannot create or rotate a replacement token.
pub fn initialize(root: &Path) -> Result<()> {
    let _guard = ConfigurationLock::acquire(root)?;
    let config = root.join("config.toml");
    let secret = root.join("secrets/api-token");
    let has_config = fs::symlink_metadata(&config).is_ok();
    let has_token = fs::symlink_metadata(&secret).is_ok();
    if has_config || has_token {
        read(root)?;
        token::load_private_token(root).map_err(|_| error("configuration_unavailable"))?;
        return Ok(());
    }
    let c = Config {
        schema_version: if preferences(root)?.bytes.is_some() {
            1
        } else {
            2
        },
        ..Default::default()
    };
    let bytes = encode(&c)?;
    token::init_private_token(root).map_err(|_| error("configuration_write_failed"))?;
    token::write_private_new(&config, &bytes).map_err(|_| error("configuration_write_failed"))
}
/// Compatibility writes must still serialize against migration and profile CAS.
/// They are deliberately refused after upgrade because their DTO has no revision.
pub fn legacy_update(root: &Path, update: impl FnOnce(&mut Config)) -> Result<()> {
    let _guard = ConfigurationLock::acquire(root)?;
    let mut c = read(root)?.config;
    if c.schema_version != 1 {
        return Err(error("configuration_revision_required"));
    }
    update(&mut c);
    publish(root, c).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Root {
        _temp: tempfile::TempDir,
        path: std::path::PathBuf,
    }
    impl Root {
        fn path(&self) -> &Path {
            &self.path
        }
    }
    fn root() -> Root {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("private");
        token::create_private_dir(&path).unwrap();
        Root { _temp: temp, path }
    }
    fn write(root: &Path, c: &Config) {
        token::create_private_dir(root).unwrap();
        token::write_private_new(&root.join("config.toml"), c.to_toml().unwrap().as_bytes())
            .unwrap();
    }
    fn profile(revision: &str, id: &str, context: u32) -> ConfigurationSaveRequest {
        ConfigurationSaveRequest {
            expected_revision: revision.into(),
            update: ConfigurationUpdate::ModelProfile {
                model_id: ModelId::new(id).unwrap(),
                load_overrides: LoadOverrides {
                    context_size: Some(context),
                    threads: Some(2),
                    batch_size: Some(128),
                },
            },
        }
    }
    fn legacy(root: &Path) {
        token::write_private_new(&root.join(PREFERENCES),br#"{"context_size":2048,"threads":2,"batch_size":128,"max_output_tokens":128,"close_runtime_on_exit":true,"download_source":"huggingface"}"#).unwrap();
    }
    #[test]
    fn read_and_preview_never_write_or_migrate() {
        let t = root();
        write(t.path(), &Config::default());
        let before = fs::read_dir(t.path()).unwrap().count();
        let d = read(t.path()).unwrap();
        assert_eq!(
            snapshot(t.path(), &d, None).unwrap().migration.state,
            "legacy_compatible"
        );
        assert_eq!(fs::read_dir(t.path()).unwrap().count(), before);
        assert!(!t.path().join("runtime").exists());
    }
    #[test]
    fn schema_and_bounds_are_fail_closed() {
        let mut c = Config::default();
        c.model_profiles
            .insert(ModelId::new("a").unwrap(), Default::default());
        assert!(c.validate().is_err());
        c.schema_version = 2;
        assert!(c.validate().is_ok());
        for i in 0..129 {
            c.model_profiles
                .insert(ModelId::new(format!("m-{i}")).unwrap(), Default::default());
        }
        assert!(c.validate().is_err());
        c.model_profiles.clear();
        c.schema_version = 3;
        assert!(c.validate().is_err());
    }
    #[test]
    fn resolver_uses_explicit_then_profile_then_global() {
        let mut c = Config {
            schema_version: 2,
            ..Default::default()
        };
        c.model_profiles.insert(
            ModelId::new("a").unwrap(),
            LoadOverrides {
                context_size: Some(2048),
                threads: Some(2),
                batch_size: None,
            },
        );
        let id = ModelId::new("a").unwrap();
        let value = resolve_load_options(
            &c,
            &id,
            LoadOverrides {
                threads: Some(3),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            value,
            LoadOptions {
                context_size: 2048,
                threads: 3,
                batch_size: 512
            }
        );
        let b = resolve_load_options(&c, &ModelId::new("b").unwrap(), Default::default()).unwrap();
        assert_eq!(b, c.load_options());
    }
    #[test]
    fn profile_null_requires_all_fields_and_temporary_is_separate() {
        let missing = r#"{"expected_revision":"x","update":{"kind":"model_profile","model_id":"a","load_overrides":{"threads":2}}}"#;
        assert!(serde_json::from_str::<ConfigurationSaveRequest>(missing).is_err());
        let full = r#"{"expected_revision":"x","update":{"kind":"model_profile","model_id":"a","load_overrides":{"threads":null,"context_size":null,"batch_size":null}}}"#;
        assert!(serde_json::from_str::<ConfigurationSaveRequest>(full).is_ok());
        for partial in [
            r#"{"expected_revision":"x","update":{"kind":"global_defaults","global_defaults":{"context_size":2048,"batch_size":128}}}"#,
            r#"{"expected_revision":"x","update":{"kind":"lan_api","lan_api":{"enabled":false}}}"#,
        ] {
            assert!(serde_json::from_str::<ConfigurationSaveRequest>(partial).is_err());
        }
    }
    #[test]
    fn cas_preserves_other_groups_and_noop_revision() {
        let t = root();
        initialize(t.path()).unwrap();
        let initial = read(t.path()).unwrap();
        let first = save(
            t.path(),
            profile(&initial.revision, "a", 2048),
            None,
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(first.config.runtime, initial.config.runtime);
        assert_eq!(first.config.lan_api, initial.config.lan_api);
        assert_eq!(
            save(
                t.path(),
                profile(&initial.revision, "b", 1024),
                None,
                &BTreeMap::new()
            )
            .unwrap_err()
            .code,
            "configuration_conflict"
        );
        assert_eq!(read(t.path()).unwrap().bytes, first.bytes);
        assert_eq!(
            save(
                t.path(),
                profile(&first.revision, "a", 2048),
                None,
                &BTreeMap::new()
            )
            .unwrap()
            .revision,
            first.revision
        );
        let b = save(
            t.path(),
            profile(&first.revision, "b", 1024),
            None,
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(b.config.model_profiles.len(), 2);
        let cleared = save(
            t.path(),
            ConfigurationSaveRequest {
                expected_revision: b.revision,
                update: ConfigurationUpdate::ModelProfile {
                    model_id: ModelId::new("a").unwrap(),
                    load_overrides: Default::default(),
                },
            },
            None,
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(cleared.config.model_profiles.len(), 1);
    }
    #[test]
    fn invalid_save_creates_no_backups_or_config_change() {
        let t = root();
        write(t.path(), &Config::default());
        let d = read(t.path()).unwrap();
        assert!(
            save(
                t.path(),
                profile(&d.revision, "a", 64),
                None,
                &BTreeMap::new()
            )
            .is_err()
        );
        assert!(!t.path().join("backups").exists());
        assert_eq!(read(t.path()).unwrap().bytes, d.bytes);
    }
    #[test]
    fn online_only_profiles_and_request_defaults_without_importing_external_edits() {
        let t = root();
        initialize(t.path()).unwrap();
        let d = read(t.path()).unwrap();
        let update = ConfigurationSaveRequest {
            expected_revision: d.revision.clone(),
            update: ConfigurationUpdate::RequestDefaults {
                request_defaults: RequestDefaults {
                    max_output_tokens: 123,
                    temperature: 0.1,
                    top_p: 0.5,
                },
            },
        };
        let changed = save(t.path(), update, Some(&d), &BTreeMap::new()).unwrap();
        assert_eq!(changed.config.inference.max_output_tokens, 123);
        let req = ConfigurationSaveRequest {
            expected_revision: changed.revision.clone(),
            update: ConfigurationUpdate::GlobalDefaults {
                global_defaults: LoadDefaults {
                    context_size: 2048,
                    threads: None,
                    batch_size: 128,
                },
            },
        };
        assert_eq!(
            save(t.path(), req, Some(&changed), &BTreeMap::new())
                .unwrap_err()
                .code,
            "runtime_running"
        );
        assert_eq!(
            save(
                t.path(),
                profile(&changed.revision, "a", 2048),
                Some(&d),
                &BTreeMap::new()
            )
            .unwrap_err()
            .code,
            "configuration_restart_required"
        );
    }
    #[test]
    fn migration_conflict_preview_and_explicit_desktop_preserve_token() {
        let t = root();
        token::init_private_token(t.path()).unwrap();
        write(t.path(), &Config::default());
        legacy(t.path());
        let d = read(t.path()).unwrap();
        let token = fs::read(t.path().join("secrets/api-token")).unwrap();
        let preview = snapshot(t.path(), &d, None).unwrap();
        assert_eq!(preview.migration.state, "required");
        assert_eq!(preview.migration.differences.len(), 4);
        assert_eq!(
            save(
                t.path(),
                profile(&d.revision, "a", 2048),
                None,
                &BTreeMap::new()
            )
            .unwrap_err()
            .code,
            "configuration_migration_required"
        );
        let mut request = ConfigurationMigrateRequest {
            expected_revision: d.revision.clone(),
            expected_preferences_revision: None,
            choice: MigrationChoice::Desktop,
            custom: None,
        };
        assert_eq!(
            migrate(t.path(), request.clone()).unwrap_err().code,
            "configuration_conflict"
        );
        request.expected_preferences_revision = preview.migration.preferences_revision;
        let next = migrate(t.path(), request).unwrap();
        assert_eq!(next.config.schema_version, 2);
        assert_eq!(next.config.inference.context_size, 2048);
        assert_eq!(
            next.config.inference.temperature,
            d.config.inference.temperature
        );
        assert_eq!(fs::read(t.path().join("secrets/api-token")).unwrap(), token);
        assert_eq!(
            fs::read_dir(t.path().join("backups/configuration"))
                .unwrap()
                .count(),
            2
        );
        assert_eq!(
            snapshot(t.path(), &next, None).unwrap().migration.state,
            "complete"
        );
    }
    #[test]
    fn initialize_only_explicit_and_partial_never_rotates_credentials() {
        let t = root();
        assert!(!t.path().join("secrets").exists());
        let _ = preview();
        assert!(!t.path().join("secrets").exists());
        initialize(t.path()).unwrap();
        let token = fs::read(t.path().join("secrets/api-token")).unwrap();
        assert_eq!(read(t.path()).unwrap().config.schema_version, 2);
        assert!(!t.path().join("secrets/lan-api-token").exists());
        assert!(!t.path().join("models").exists());
        fs::write(t.path().join("config.toml"), b"invalid").unwrap();
        assert!(initialize(t.path()).is_err());
        assert_eq!(fs::read(t.path().join("secrets/api-token")).unwrap(), token);
    }
    #[test]
    fn legacy_preferences_survive_init_and_require_explicit_choice() {
        let t = root();
        legacy(t.path());
        initialize(t.path()).unwrap();
        let d = read(t.path()).unwrap();
        assert_eq!(d.config.schema_version, 1);
        assert_eq!(
            snapshot(t.path(), &d, None).unwrap().migration.state,
            "required"
        );
    }
    #[test]
    fn ui_preferences_cas_does_not_touch_configuration() {
        let t = root();
        initialize(t.path()).unwrap();
        let config = read(t.path()).unwrap().bytes;
        let first = ui_preferences_get(t.path()).unwrap();
        let next = ui_preferences_save(
            t.path(),
            UiPreferencesSaveRequest {
                expected_revision: first.revision.clone(),
                preferences: UiPreferences {
                    close_runtime_on_exit: true,
                    download_source: "huggingface".into(),
                },
            },
        )
        .unwrap();
        assert_ne!(first.revision, next.revision);
        assert_eq!(
            ui_preferences_save(
                t.path(),
                UiPreferencesSaveRequest {
                    expected_revision: first.revision,
                    preferences: UiPreferences::default()
                }
            )
            .unwrap_err()
            .code,
            "configuration_conflict"
        );
        assert_eq!(read(t.path()).unwrap().bytes, config);
    }
    #[test]
    fn saved_context_limit_rejects_without_writes() {
        let t = root();
        initialize(t.path()).unwrap();
        let d = read(t.path()).unwrap();
        let limits = BTreeMap::from([(ModelId::new("a").unwrap(), 1024)]);
        let failure = save(t.path(), profile(&d.revision, "a", 2048), None, &limits).unwrap_err();
        assert_eq!(failure.code, "model_profile_invalid");
        assert_eq!(failure.param, Some("update.load_overrides.context_size"));
        assert_eq!(parse_save_request(br#"{"expected_revision":"x","update":{"kind":"global_defaults","global_defaults":{}}}"#).unwrap_err().param,Some("update.global_defaults"));
        assert_eq!(read(t.path()).unwrap().bytes, d.bytes);
    }
    #[test]
    fn oversized_config_and_lock_contention_fail_closed() {
        let t = root();
        initialize(t.path()).unwrap();
        let guard = ConfigurationLock::acquire(t.path()).unwrap();
        assert_eq!(
            ConfigurationLock::acquire(t.path()).err().unwrap().code,
            "configuration_busy"
        );
        drop(guard);
        fs::write(
            t.path().join("config.toml"),
            vec![b' '; MAX_CONFIG_BYTES + 1],
        )
        .unwrap();
        assert_eq!(read(t.path()).unwrap_err().code, "configuration_invalid");
    }
    #[cfg(unix)]
    #[test]
    fn private_publication_and_linked_config_lock_backup_are_rejected() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let t = root();
        initialize(t.path()).unwrap();
        assert_eq!(
            fs::metadata(t.path().join("config.toml"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let alias = t.path().join("alias");
        fs::hard_link(t.path().join("config.toml"), &alias).unwrap();
        assert!(read(t.path()).is_err());
        fs::remove_file(&alias).unwrap();
        let lock = t.path().join("runtime/configuration.lock");
        fs::remove_file(&lock).unwrap();
        symlink(&alias, &lock).unwrap();
        assert!(ConfigurationLock::acquire(t.path()).is_err());
        assert!(!alias.exists());
    }
}
