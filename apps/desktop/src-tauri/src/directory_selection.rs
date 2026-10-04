//! Native-only, one-admission folder selection. Never creates or writes files.
use serde::Serialize;
use std::{
    fs,
    path::{Component, Path, PathBuf},
};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize)]
pub struct PickedDirectory {
    pub selection_id: Uuid,
    /// Display only; invoke never accepts this value back as a path.
    pub display_path: String,
}

pub struct DirectorySelection {
    id: Uuid,
    path: PathBuf,
}

#[derive(Debug, PartialEq, Eq)]
pub enum AdmissionError<E> {
    Selection(&'static str),
    Rejected(E),
}

fn ordinary_directory(path: &Path) -> Result<PathBuf, &'static str> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return Err("model_directory_unsupported");
    }
    #[cfg(windows)]
    crate::local_path::require_local_disk(path).map_err(|_| "model_directory_unsupported")?;
    for part in path.ancestors() {
        let metadata = fs::symlink_metadata(part).map_err(|_| "model_directory_unavailable")?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err("model_directory_unsupported");
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err("model_directory_unsupported");
            }
        }
    }
    let canonical = fs::canonicalize(path).map_err(|_| "model_directory_unavailable")?;
    #[cfg(windows)]
    crate::local_path::require_local_disk(&canonical).map_err(|_| "model_directory_unsupported")?;
    Ok(canonical)
}

fn same_component(left: Component<'_>, right: Component<'_>) -> bool {
    #[cfg(windows)]
    {
        // Both operands are canonical OS paths, not unchecked invoke strings.
        left.as_os_str().eq_ignore_ascii_case(right.as_os_str())
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

pub fn supported_directory(path: &Path, package_root: &Path) -> Result<PathBuf, &'static str> {
    let selected = ordinary_directory(path)?;
    let root = ordinary_directory(package_root)?;
    let candidate: Vec<_> = selected.components().collect();
    let package: Vec<_> = root.components().collect();
    let inside = candidate.len() >= package.len()
        && candidate
            .iter()
            .zip(&package)
            .all(|(left, right)| same_component(*left, *right));
    if inside {
        let remaining = &candidate[package.len()..];
        if !(remaining.is_empty()
            || (remaining.len() == 1
                && matches!(remaining[0], Component::Normal(name) if name.eq_ignore_ascii_case("model") || name.eq_ignore_ascii_case("models"))))
        {
            return Err("model_directory_inside_package_unsupported");
        }
    }
    Ok(selected)
}

fn same_path(left: &Path, right: &Path) -> bool {
    left.components().count() == right.components().count()
        && left
            .components()
            .zip(right.components())
            .all(|(left, right)| same_component(left, right))
}

pub fn preflight_directory(path: &Path, package_root: &Path) -> Result<(), &'static str> {
    let selected = supported_directory(path, package_root)?;
    let root = ordinary_directory(package_root)?;
    if same_path(&selected, &root)
        || selected
            .parent()
            .is_some_and(|parent| same_path(parent, &root))
    {
        // Reuse package verification, not catalog scanning: GGUF inputs receive
        // only the existing four-byte header check, never a whole-model hash.
        crate::layout::validate(&root.join("nexa-desktop.exe"))?;
    }
    Ok(())
}

impl DirectorySelection {
    #[cfg(test)]
    pub fn new(path: &Path, package_root: &Path) -> Result<(Self, PickedDirectory), &'static str> {
        #[cfg(windows)]
        if !matches!(path.components().next(), Some(Component::Prefix(p)) if matches!(p.kind(), std::path::Prefix::Disk(_)))
        {
            return Err("model_directory_unsupported");
        }
        preflight_directory(path, package_root)?;
        let display_path = path
            .to_str()
            .ok_or("model_directory_unsupported")?
            .to_owned();
        let id = Uuid::new_v4();
        Ok((
            Self {
                id,
                path: path.to_path_buf(),
            },
            PickedDirectory {
                selection_id: id,
                display_path,
            },
        ))
    }

    /// Download-location selection does no package inventory or GGUF reads.
    pub fn new_location(
        path: &Path,
        package_root: &Path,
    ) -> Result<(Self, PickedDirectory), &'static str> {
        supported_directory(path, package_root)?;
        let id = Uuid::new_v4();
        let display_path = path
            .to_str()
            .ok_or("model_directory_unsupported")?
            .to_owned();
        Ok((
            Self {
                id,
                path: path.to_owned(),
            },
            PickedDirectory {
                selection_id: id,
                display_path,
            },
        ))
    }
    pub fn admit_location<T, E>(
        slot: &mut Option<Self>,
        id: Uuid,
        package_root: &Path,
        accept: impl FnOnce(PathBuf) -> Result<T, E>,
    ) -> Result<T, AdmissionError<E>> {
        let selection = slot
            .as_ref()
            .filter(|s| s.id == id)
            .ok_or(AdmissionError::Selection("selection_expired"))?;
        supported_directory(&selection.path, package_root).map_err(AdmissionError::Selection)?;
        let admitted = accept(selection.path.clone()).map_err(AdmissionError::Rejected)?;
        slot.take();
        Ok(admitted)
    }

    pub fn admit<T, E>(
        slot: &mut Option<Self>,
        id: Uuid,
        package_root: &Path,
        accept: impl FnOnce(PathBuf) -> Result<T, E>,
    ) -> Result<T, AdmissionError<E>> {
        let selection = slot
            .as_ref()
            .filter(|selection| selection.id == id)
            .ok_or(AdmissionError::Selection("selection_expired"))?;
        preflight_directory(&selection.path, package_root).map_err(AdmissionError::Selection)?;
        // No await between admission and consumption. A busy/rejected bridge
        // leaves the pending selection available; success consumes it exactly once.
        let admitted = accept(selection.path.clone()).map_err(AdmissionError::Rejected)?;
        slot.take();
        Ok(admitted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configure_location_never_checks_package_inventory_or_payloads() {
        let package = tempfile::tempdir().unwrap();
        fs::write(package.path().join("model.gguf"), b"invalid GGUF").unwrap();
        // No nexa-desktop.exe or package manifest: inventory validation would fail.
        let (selection, dto) =
            DirectorySelection::new_location(package.path(), package.path()).unwrap();
        let mut slot = Some(selection);
        let admitted = DirectorySelection::admit_location(
            &mut slot,
            dto.selection_id,
            package.path(),
            Ok::<_, ()>,
        )
        .unwrap();
        assert_eq!(admitted, package.path());
        assert!(slot.is_none());
    }

    #[test]
    fn arbitrary_external_and_exact_package_directories_are_supported_without_writes() {
        let temp = tempfile::tempdir().unwrap();
        let package = temp.path().join("program");
        fs::create_dir(&package).unwrap();
        crate::layout::tests::complete_fixture(&package);
        let external = temp.path().join("外部 只读模型");
        fs::create_dir(&external).unwrap();
        for directory in [&package, &external] {
            let before = fs::read_dir(directory).unwrap().count();
            let (_, dto) = DirectorySelection::new(directory, &package).unwrap();
            assert_eq!(dto.display_path, directory.to_str().unwrap());
            assert_eq!(fs::read_dir(directory).unwrap().count(), before);
        }
        for name in ["model", "models"] {
            let path = package.join(name);
            fs::create_dir(&path).unwrap();
            assert!(DirectorySelection::new(&path, &package).is_ok());
            assert_eq!(fs::read_dir(path).unwrap().count(), 0);
        }
        for name in ["runtime", "licenses", "arbitrary", "model/nested"] {
            let path = package.join(name);
            fs::create_dir_all(&path).unwrap();
            assert_eq!(
                DirectorySelection::new(&path, &package).err(),
                Some("model_directory_inside_package_unsupported")
            );
        }
    }

    #[test]
    fn selection_is_consumed_only_after_matching_successful_admission() {
        let temp = tempfile::tempdir().unwrap();
        crate::layout::tests::complete_fixture(temp.path());
        let (selected, dto) = DirectorySelection::new(temp.path(), temp.path()).unwrap();
        let mut slot = Some(selected);
        assert_eq!(
            DirectorySelection::admit(
                &mut slot,
                Uuid::new_v4(),
                temp.path(),
                |_| Ok::<_, &str>(())
            ),
            Err(AdmissionError::Selection("selection_expired"))
        );
        assert_eq!(
            DirectorySelection::admit(&mut slot, dto.selection_id, temp.path(), |_| Err::<(), _>(
                "desktop_busy"
            )),
            Err(AdmissionError::Rejected("desktop_busy"))
        );
        assert!(slot.is_some());
        assert_eq!(
            DirectorySelection::admit(&mut slot, dto.selection_id, temp.path(), Ok::<_, &str>),
            Ok(temp.path().to_path_buf())
        );
        assert!(slot.is_none());
    }

    #[test]
    fn invalid_or_changed_directory_never_reaches_bridge() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("selected");
        fs::create_dir(&path).unwrap();
        let package = temp.path().join("package");
        fs::create_dir(&package).unwrap();
        let (selected, dto) = DirectorySelection::new(&path, &package).unwrap();
        let mut slot = Some(selected);
        fs::remove_dir(&path).unwrap();
        assert!(
            DirectorySelection::admit(
                &mut slot,
                dto.selection_id,
                &package,
                |_| -> Result<(), ()> { panic!("must not reach bridge") }
            )
            .is_err()
        );
        assert!(slot.is_some());
        fs::write(&path, "not a directory").unwrap();
        assert!(DirectorySelection::new(&path, &package).is_err());
        assert!(DirectorySelection::new(Path::new("relative"), &package).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn indirect_directory_or_ancestor_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let package = temp.path().join("package");
        fs::create_dir(&package).unwrap();
        let target = temp.path().join("source");
        fs::create_dir(&target).unwrap();
        fs::create_dir(target.join("child")).unwrap();
        let link = temp.path().join("linked");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        for path in [&link, &link.join("child")] {
            assert_eq!(
                DirectorySelection::new(path, &package).err(),
                Some("model_directory_unsupported")
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn canonical_case_variants_cannot_turn_package_children_into_external_paths() {
        let temp = tempfile::tempdir().unwrap();
        let package = temp.path().join("Nexa Package");
        let runtime = package.join("runtime");
        fs::create_dir_all(&runtime).unwrap();
        let upper = PathBuf::from(runtime.to_str().unwrap().to_ascii_uppercase());
        assert_eq!(
            supported_directory(&upper, &package).err(),
            Some("model_directory_inside_package_unsupported")
        );
    }

    #[test]
    fn in_package_apply_and_rescan_reject_files_added_after_selection() {
        let temp = tempfile::tempdir().unwrap();
        crate::layout::tests::complete_fixture(temp.path());
        for name in ["model", "models"] {
            let directory = temp.path().join(name);
            fs::create_dir(&directory).unwrap();
            let (selected, dto) = DirectorySelection::new(&directory, temp.path()).unwrap();
            let mut slot = Some(selected);
            let dll = directory.join("added-after-window-open.dll");
            fs::write(&dll, "synthetic").unwrap();
            assert_eq!(
                DirectorySelection::admit(
                    &mut slot,
                    dto.selection_id,
                    temp.path(),
                    |_| -> Result<(), ()> { panic!("must not admit") }
                ),
                Err(AdmissionError::Selection("package_unlisted_file"))
            );
            assert_eq!(
                preflight_directory(&directory, temp.path()),
                Err("package_unlisted_file")
            );
            assert!(slot.is_some());
            fs::remove_file(dll).unwrap();
            let nested = directory.join("empty-nested");
            fs::create_dir(&nested).unwrap();
            assert_eq!(
                preflight_directory(&directory, temp.path()),
                Err("package_unlisted_file")
            );
            assert!(DirectorySelection::new(&directory, temp.path()).is_err());
            fs::remove_dir(nested).unwrap();
            assert!(
                DirectorySelection::admit(
                    &mut slot,
                    dto.selection_id,
                    temp.path(),
                    |_| Ok::<_, ()>(())
                )
                .is_ok()
            );
        }
    }

    #[test]
    fn external_directory_does_not_trigger_package_inventory_scan() {
        let temp = tempfile::tempdir().unwrap();
        let package = temp.path().join("program");
        fs::create_dir(&package).unwrap();
        let external = temp.path().join("external");
        fs::create_dir(&external).unwrap();
        // Deliberately no package manifest; external selection must not inspect it.
        assert!(preflight_directory(&external, &package).is_ok());
    }
}
