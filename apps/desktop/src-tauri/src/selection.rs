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

pub struct PairFile {
    /// Preserve the native picker's DOS path for external leases and import.
    pub source: PathBuf,
    /// Canonical paths are only for comparison, never an external source.
    pub canonical: PathBuf,
}

pub fn pair_file(path: &Path, previous_canonical: Option<&Path>) -> Result<PairFile, &'static str> {
    #[cfg(windows)]
    if !matches!(path.components().next(), Some(std::path::Component::Prefix(p)) if matches!(p.kind(), std::path::Prefix::Disk(_)))
    {
        return Err("selected_path_invalid");
    }
    let canonical = regular_file(path)?;
    if !canonical
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("gguf"))
    {
        return Err("selected_file_not_gguf");
    }
    if previous_canonical.is_some_and(|previous| {
        #[cfg(windows)]
        {
            previous
                .as_os_str()
                .eq_ignore_ascii_case(canonical.as_os_str())
        }
        #[cfg(not(windows))]
        {
            previous == canonical
        }
    }) {
        return Err("selected_pair_same_file");
    }
    Ok(PairFile {
        source: path.to_path_buf(),
        canonical,
    })
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
    fn pair_preserves_both_picker_paths_and_checks_canonical_duplicates() {
        let temp = tempfile::tempdir().unwrap();
        let model = temp.path().join("主模型.GGUF");
        let projector = temp.path().join("mmproj.gguf");
        fs::write(&model, b"GGUF").unwrap();
        fs::write(&projector, b"GGUF").unwrap();
        let picked_model = temp.path().join(".").join("主模型.GGUF");
        let first = pair_file(&picked_model, None).unwrap();
        assert_eq!(first.source.as_os_str(), picked_model.as_os_str());
        assert_eq!(first.canonical, fs::canonicalize(&model).unwrap());
        assert!(matches!(
            pair_file(&model, Some(&first.canonical)),
            Err("selected_pair_same_file")
        ));
        let second = pair_file(&projector, Some(&first.canonical)).unwrap();
        assert_eq!(second.source.as_os_str(), projector.as_os_str());
        assert_ne!(first.canonical, second.canonical);
    }

    #[test]
    fn pair_rejects_non_gguf_missing_relative_and_directory_sources() {
        let temp = tempfile::tempdir().unwrap();
        let wrong = temp.path().join("model.txt");
        fs::write(&wrong, b"GGUF").unwrap();
        assert!(matches!(
            pair_file(&wrong, None),
            Err("selected_file_not_gguf")
        ));
        assert!(pair_file(&temp.path().join("missing.gguf"), None).is_err());
        assert!(pair_file(Path::new("model.gguf"), None).is_err());
        assert!(pair_file(temp.path(), None).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_pair_original_paths_pass_leases_and_both_import_source_checks() {
        use std::path::{Component, Prefix};

        let temp = tempfile::tempdir().unwrap();
        let model = temp.path().join("主模型.gguf");
        let projector = temp.path().join("mmproj.gguf");
        fs::write(&model, b"GGUF").unwrap();
        fs::write(&projector, b"GGUF").unwrap();
        let first = pair_file(&model, None).unwrap();
        let second = pair_file(&projector, Some(&first.canonical)).unwrap();
        let mut leases = Vec::new();
        for file in [&first, &second] {
            assert!(matches!(
                file.canonical.components().next(),
                Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::VerbatimDisk(_))
            ));
            // The old picker passed this canonical path and failed before the
            // second dialog. Preserve the external-source policy, not that path.
            let old = desktop_bridge::SelectedFile::open(&file.canonical);
            assert!(
                matches!(old, Err(error) if error.code.as_str() == "model_directory_unsupported")
            );
            assert!(matches!(
                pair_file(&file.canonical, None),
                Err("selected_path_invalid")
            ));
            leases.push(desktop_bridge::SelectedFile::open(&file.source).unwrap());
        }
        let data_dir = temp.path().join("not-running");
        let bridge = desktop_bridge::DesktopBridge::new(
            data_dir.clone(),
            temp.path().join("ai-runtime.exe"),
        )
        .unwrap();
        tauri::async_runtime::block_on(async {
            // No service is started: connection_failed proves both original
            // sources passed local_source. This is admission, not import success.
            let result = bridge
                .import_model_pair(first.source.clone(), second.source.clone(), "pair".into())
                .await;
            assert_eq!(result.unwrap_err().code, "connection_failed");
            for (model, projector) in [
                (first.canonical.clone(), second.source.clone()),
                (first.source.clone(), second.canonical.clone()),
            ] {
                let result = bridge
                    .import_model_pair(model, projector, "pair".into())
                    .await;
                assert_eq!(result.unwrap_err().code, "invalid_model_source");
            }
        });
        assert!(!data_dir.exists());
        drop(leases);
    }

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
        assert!(pair_file(&link, None).is_err());
        let dirlink = temp.path().join("linked");
        std::os::unix::fs::symlink(temp.path(), &dirlink).unwrap();
        assert!(Selection::new(&dirlink.join("model.gguf")).is_err());
        assert!(pair_file(&dirlink.join("model.gguf"), None).is_err());
    }
}
