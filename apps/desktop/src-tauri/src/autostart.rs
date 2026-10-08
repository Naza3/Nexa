//! Owns exactly HKCU Run\Nexa. No arbitrary registry keys or startup arguments.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct Snapshot {
    pub registered: bool,
    pub current_executable: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SetRequest {
    pub enabled: bool,
}

fn quoted_command(path: &[u16]) -> Result<Vec<u16>, &'static str> {
    if path.is_empty() || path.iter().any(|c| *c == 0 || *c == b'"' as u16) {
        return Err("autostart_path_invalid");
    }
    // Run/RunOnce accepts a command line of at most 260 characters. Count
    // both quotes (the registry's terminating NUL is not command-line text).
    if path.len() > 258 {
        return Err("autostart_path_too_long");
    }
    let mut command = Vec::with_capacity(path.len() + 3);
    command.push(b'"' as u16);
    command.extend_from_slice(path);
    command.extend_from_slice(&[b'"' as u16, 0]);
    Ok(command)
}

#[cfg(windows)]
mod registry {
    use super::*;
    use std::{os::windows::ffi::OsStrExt, ptr};
    use windows_sys::{
        Win32::{
            Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
            System::Registry::{
                HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW,
                RegSetKeyValueW,
            },
        },
        core::w,
    };

    const RUN_KEY: windows_sys::core::PCWSTR =
        w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
    const VALUE_NAME: windows_sys::core::PCWSTR = w!("Nexa");

    fn command() -> Result<Vec<u16>, &'static str> {
        let executable = std::env::current_exe().map_err(|_| "autostart_path_invalid")?;
        quoted_command(&executable.as_os_str().encode_wide().collect::<Vec<_>>())
    }

    fn read_command() -> Result<Option<Vec<u16>>, &'static str> {
        let mut value = vec![0u16; 32768];
        let mut bytes = (value.len() * 2) as u32;
        // Fixed current-user key/value, bounded output, REG_SZ only. Windows
        // ensures termination when it fits; malformed or oversized data fails.
        let result = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                VALUE_NAME,
                RRF_RT_REG_SZ,
                ptr::null_mut(),
                value.as_mut_ptr().cast(),
                &mut bytes,
            )
        };
        if result == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if result != ERROR_SUCCESS
            || bytes < 2
            || !bytes.is_multiple_of(2)
            || bytes as usize > value.len() * 2
        {
            return Err("autostart_read_failed");
        }
        value.truncate(bytes as usize / 2);
        if value.last() != Some(&0) || value[..value.len() - 1].contains(&0) {
            return Err("autostart_read_failed");
        }
        Ok(Some(value))
    }

    pub(crate) fn get() -> Result<Snapshot, &'static str> {
        let value = read_command()?;
        if value.is_none() {
            return Ok(Snapshot {
                registered: false,
                current_executable: false,
            });
        }
        let current = command()?;
        Ok(Snapshot {
            registered: value.is_some(),
            current_executable: value.as_ref() == Some(&current),
        })
    }

    pub(crate) fn set(enabled: bool) -> Result<Snapshot, &'static str> {
        let result = if enabled {
            let value = command()?;
            // This Windows helper creates the fixed subkey if it is missing.
            // The only persisted command is our correctly quoted executable.
            unsafe {
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    RUN_KEY,
                    VALUE_NAME,
                    REG_SZ,
                    value.as_ptr().cast(),
                    (value.len() * 2) as u32,
                )
            }
        } else {
            // Remove only our own value; never delete the shared Run key.
            unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE_NAME) }
        };
        if result != ERROR_SUCCESS && !(result == ERROR_FILE_NOT_FOUND && !enabled) {
            return Err("autostart_write_failed");
        }
        let saved = get()?;
        if saved.registered != enabled || enabled && !saved.current_executable {
            return Err("autostart_verify_failed");
        }
        Ok(saved)
    }
}
#[cfg(windows)]
pub(crate) use registry::{get, set};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executable_with_spaces_and_unicode_is_one_quoted_command_without_arguments() {
        let path: Vec<u16> = r"C:\用户 目录\Nexa\nexa-desktop.exe"
            .encode_utf16()
            .collect();
        let result = quoted_command(&path).unwrap();
        assert_eq!(
            String::from_utf16(&result[..result.len() - 1]).unwrap(),
            r#""C:\用户 目录\Nexa\nexa-desktop.exe""#
        );
        assert_eq!(result.last(), Some(&0));
    }

    #[test]
    fn command_rejects_injection_nul_empty_and_oversized_paths() {
        for path in [vec![], vec![0], vec![b'"' as u16]] {
            assert_eq!(quoted_command(&path).unwrap_err(), "autostart_path_invalid");
        }
        assert_eq!(
            quoted_command(&[b'x' as u16; 32761]).unwrap_err(),
            "autostart_path_too_long"
        );
    }

    #[test]
    fn run_command_limit_includes_both_quotes_and_counts_utf16_units() {
        let path: Vec<u16> = "字".repeat(258).encode_utf16().collect();
        let command = quoted_command(&path).unwrap();
        assert_eq!(command.len(), 261); // 260 command characters plus NUL.
        let longer: Vec<u16> = "字".repeat(259).encode_utf16().collect();
        assert_eq!(
            quoted_command(&longer).unwrap_err(),
            "autostart_path_too_long"
        );
        let supplementary: Vec<u16> = "🙂".repeat(130).encode_utf16().collect();
        assert_eq!(
            quoted_command(&supplementary).unwrap_err(),
            "autostart_path_too_long"
        );
    }

    #[test]
    fn request_accepts_only_boolean_and_no_registry_paths() {
        assert!(
            serde_json::from_str::<SetRequest>(r#"{"enabled":true}"#)
                .unwrap()
                .enabled
        );
        assert!(
            serde_json::from_str::<SetRequest>(r#"{"enabled":false,"path":"arbitrary"}"#).is_err()
        );
        assert!(serde_json::from_str::<SetRequest>(r#"{"enabled":"true"}"#).is_err());
        assert_eq!(
            serde_json::to_value(Snapshot {
                registered: false,
                current_executable: false
            })
            .unwrap(),
            serde_json::json!({"registered":false,"current_executable":false})
        );
    }
}
