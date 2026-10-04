use crate::{BridgeError, Result, dto::DesktopPreferences};
use runtime_api::{
    Config,
    token::{load_private_token, write_private_new},
};
use std::{
    fs,
    io::{self, Read},
    path::Path,
};
use uuid::Uuid;
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
    let bytes = read_bounded(&root.join("config.toml"), 65536)?;
    Config::from_toml(
        std::str::from_utf8(&bytes).map_err(|_| BridgeError::new("configuration_invalid"))?,
    )
    .map_err(|_| BridgeError::new("configuration_invalid"))
}
pub(crate) fn require_initialized(root: &Path) -> Result<Config> {
    load_private_token(root).map_err(|_| BridgeError::new("credentials_unavailable"))?;
    config(root)
}
pub(crate) fn preferences(root: &Path) -> Result<DesktopPreferences> {
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
    let bytes =
        serde_json::to_vec(preferences).map_err(|_| BridgeError::new("settings_invalid"))?;
    atomic_replace(&root.join(PREFERENCES), &bytes)
}
/// Publish one fully-synced private file. No remove+rename gap and no rewriting
/// runtime configuration to store unrelated UI preferences.
pub(crate) fn atomic_replace(target: &Path, bytes: &[u8]) -> Result<()> {
    let parent = target
        .parent()
        .ok_or_else(|| BridgeError::new("unsafe_file"))?;
    runtime_api::token::create_private_dir(parent).map_err(|_| BridgeError::new("unsafe_file"))?;
    match fs::symlink_metadata(target) {
        Ok(m) if !m.file_type().is_file() => return Err(BridgeError::new("unsafe_file")),
        Ok(_) => (),
        Err(e) if e.kind() == io::ErrorKind::NotFound => (),
        Err(_) => return Err(BridgeError::new("settings_write_failed")),
    }
    let temp = parent.join(format!(".desktop-{}.tmp", Uuid::new_v4()));
    write_private_new(&temp, bytes).map_err(|_| BridgeError::new("settings_write_failed"))?;
    let result = replace(&temp, target);
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result.map_err(|_| BridgeError::new("settings_write_failed"))?;
    #[cfg(unix)]
    fs::File::open(parent)
        .and_then(|file| file.sync_all())
        .map_err(|_| BridgeError::new("settings_durability_unconfirmed"))?;
    Ok(())
}
#[cfg(not(windows))]
fn replace(source: &Path, target: &Path) -> io::Result<()> {
    fs::rename(source, target)
}
#[cfg(windows)]
fn replace(source: &Path, target: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    let s: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let t: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: valid terminated wide strings remain alive throughout the call.
    if unsafe {
        MoveFileExW(
            s.as_ptr(),
            t.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
