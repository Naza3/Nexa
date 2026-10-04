//! One bounded, single-use native file selection. No WebView path is accepted.
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize)]
pub struct PickedModel {
    pub selection_id: Uuid,
    pub file_name: String,
    pub size_bytes: u64,
    pub destination: String,
}

pub struct Selection {
    id: Uuid,
    path: PathBuf,
}

pub fn regular_file(path: &Path) -> Result<PathBuf, &'static str> {
    if !path.is_absolute() {
        return Err("selected_path_invalid");
    }
    #[cfg(windows)]
    crate::local_path::require_local_disk(path)?;
    for part in path.ancestors() {
        let metadata = fs::symlink_metadata(part).map_err(|_| "selected_file_unavailable")?;
        if metadata.file_type().is_symlink() {
            return Err("selected_path_indirect");
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err("selected_path_indirect");
            }
        }
    }
    let metadata = fs::metadata(path).map_err(|_| "selected_file_unavailable")?;
    if !metadata.is_file() {
        return Err("selected_file_invalid");
    }
    let canonical = fs::canonicalize(path).map_err(|_| "selected_file_unavailable")?;
    #[cfg(windows)]
    crate::local_path::require_local_disk(&canonical)?;
    Ok(canonical)
}

impl Selection {
    pub fn new(path: &Path) -> Result<(Self, PickedModel), &'static str> {
        let original = path.to_path_buf();
        #[cfg(windows)]
        if !matches!(path.components().next(), Some(std::path::Component::Prefix(p)) if matches!(p.kind(), std::path::Prefix::Disk(_)))
        {
            return Err("selected_path_invalid");
        }
        let path = regular_file(path)?;
        if !path
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("gguf"))
        {
            return Err("selected_file_not_gguf");
        }
        let size_bytes = fs::metadata(&path)
            .map_err(|_| "selected_file_unavailable")?
            .len();
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("selected_file_name_invalid")?
            .to_owned();
        let id = Uuid::new_v4();
        Ok((
            Self { id, path: original },
            PickedModel {
                selection_id: id,
                file_name,
                size_bytes,
                destination: "%LOCALAPPDATA%\\Nexa\\models（受管理副本）".to_owned(),
            },
        ))
    }

    pub fn consume(slot: &mut Option<Self>, id: Uuid) -> Result<PathBuf, &'static str> {
        if slot.as_ref().is_none_or(|selection| selection.id != id) {
            return Err("selection_expired");
        }
        let selection = slot.take().ok_or("selection_expired")?;
        // Recheck after the dialog and consume even on failure. A retry requires a
        // fresh user selection; the backend independently verifies the copy source.
        regular_file(&selection.path)?;
        Ok(selection.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selections_are_one_time_and_wrong_id_does_not_consume() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("候选 模型.gguf");
        fs::write(&path, b"GGUF").unwrap();
        let (selection, dto) = Selection::new(&path).unwrap();
        assert_eq!(dto.file_name, "候选 模型.gguf");
        let mut slot = Some(selection);
        assert!(Selection::consume(&mut slot, Uuid::new_v4()).is_err());
        assert!(Selection::consume(&mut slot, dto.selection_id).is_ok());
        assert!(Selection::consume(&mut slot, dto.selection_id).is_err());
    }
    #[test]
    fn directories_non_gguf_and_changed_selection_fail() {
        let temp = tempfile::tempdir().unwrap();
        assert!(Selection::new(temp.path()).is_err());
        let wrong = temp.path().join("model.txt");
        fs::write(&wrong, b"GGUF").unwrap();
        assert!(Selection::new(&wrong).is_err());
        let path = temp.path().join("model.gguf");
        fs::write(&path, b"GGUF").unwrap();
        let (selection, dto) = Selection::new(&path).unwrap();
        let mut slot = Some(selection);
        fs::remove_file(path).unwrap();
        assert!(Selection::consume(&mut slot, dto.selection_id).is_err());
        assert!(slot.is_none());
    }
    #[cfg(unix)]
    #[test]
    fn symlink_files_and_ancestors_fail() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("model.gguf");
        fs::write(&path, b"GGUF").unwrap();
        let link = temp.path().join("link.gguf");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(Selection::new(&link).is_err());
        let dirlink = temp.path().join("linked");
        std::os::unix::fs::symlink(temp.path(), &dirlink).unwrap();
        assert!(Selection::new(&dirlink.join("model.gguf")).is_err());
    }
}
