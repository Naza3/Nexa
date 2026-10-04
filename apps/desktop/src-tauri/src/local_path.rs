//! A DOS drive prefix alone does not prove locality: mapped drives use it too.
#[cfg(any(windows, test))]
fn local_drive_type(value: u32) -> bool {
    // GetDriveTypeW: REMOVABLE=2, FIXED=3, CDROM=5, RAMDISK=6.
    // UNKNOWN=0, NO_ROOT_DIR=1, REMOTE=4 and future values fail closed.
    matches!(value, 2 | 3 | 5 | 6)
}

#[cfg(windows)]
pub fn require_local_disk(path: &std::path::Path) -> Result<(), &'static str> {
    use std::path::{Component, Prefix};
    use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
    let drive = match path.components().next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => drive,
            _ => return Err("selected_path_invalid"),
        },
        _ => return Err("selected_path_invalid"),
    };
    let root = [u16::from(drive), u16::from(b':'), u16::from(b'\\'), 0];
    // SAFETY: root is a live, NUL-terminated drive root with trailing slash.
    // This reads the drive type; it does not map drives or alter any settings.
    if local_drive_type(unsafe { GetDriveTypeW(root.as_ptr()) }) {
        Ok(())
    } else {
        Err("selected_path_invalid")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapped_remote_unknown_and_unmounted_drives_are_not_local() {
        for value in [0, 1, 4, 7, u32::MAX] {
            assert!(!local_drive_type(value));
        }
        for value in [2, 3, 5, 6] {
            assert!(local_drive_type(value));
        }
    }
}
