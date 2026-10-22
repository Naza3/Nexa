//! Validate declared desktop payload files before deriving the executable.
//! Extra installation files and user directories are not inventoried or opened.
//! Hashes establish package consistency, not a publisher signature or trust grant.
use crate::selection::regular_file;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
};

const MAX_FILES: usize = 8192;
const MAX_MANIFEST: u64 = 8 * 1024 * 1024;

#[derive(Deserialize)]
struct Manifest {
    product: String,
    project_commit: String,
    project_dirty: bool,
    files: Vec<FileIdentity>,
}
#[derive(Deserialize)]
struct FileIdentity {
    path: String,
    sha256: String,
    size_bytes: u64,
}

pub struct ProductLayout {
    pub package_root: PathBuf,
    pub runtime_executable: PathBuf,
    pub project_commit: String,
    pub project_dirty: bool,
}

fn valid_relative(value: &str) -> bool {
    !value.is_empty()
        && !value.contains(['\\', ':', '\0'])
        && value.split('/').all(|part| {
            !part.is_empty() && part != "." && part != ".." && !part.ends_with([' ', '.'])
        })
}
fn valid_hash(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn hash(path: &Path) -> Result<String, &'static str> {
    let path = regular_file(path)?;
    let mut file = fs::File::open(path).map_err(|_| "package_file_unavailable")?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let length = file
            .read(&mut buffer)
            .map_err(|_| "package_file_unavailable")?;
        if length == 0 {
            break;
        }
        hasher.update(&buffer[..length]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}
fn bounded_read(path: &Path) -> Result<Vec<u8>, &'static str> {
    let mut file =
        fs::File::open(regular_file(path)?).map_err(|_| "package_manifest_unavailable")?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_MANIFEST + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "package_manifest_unavailable")?;
    if bytes.len() as u64 > MAX_MANIFEST {
        return Err("package_manifest_too_large");
    }
    Ok(bytes)
}
fn verify(
    root: &Path,
    expected_product: &str,
    required_files: &[&str],
) -> Result<Manifest, &'static str> {
    let manifest: Manifest = serde_json::from_slice(&bounded_read(&root.join("manifest.json"))?)
        .map_err(|_| "package_manifest_invalid")?;
    if manifest.product != expected_product
        || !valid_hash(&manifest.project_commit, 40)
        || manifest.files.is_empty()
        || manifest.files.len() > MAX_FILES
    {
        return Err("package_identity_invalid");
    }
    if required_files
        .iter()
        .any(|required| !manifest.files.iter().any(|item| item.path == *required))
    {
        return Err("package_inventory_invalid");
    }
    let mut declared = BTreeMap::new();
    let mut folded = BTreeSet::new();
    for item in &manifest.files {
        if !valid_relative(&item.path)
            || !valid_hash(&item.sha256, 64)
            || !folded.insert(item.path.to_lowercase())
            || matches!(item.path.as_str(), "manifest.json" | "SHA256SUMS")
        {
            return Err("package_inventory_invalid");
        }
        if item.path.split('/').count() > 17 {
            return Err("package_path_invalid");
        }
        let path = root.join(&item.path);
        if fs::metadata(regular_file(&path)?)
            .map_err(|_| "package_file_unavailable")?
            .len()
            != item.size_bytes
            || hash(&path)? != item.sha256
        {
            return Err("package_file_hash_mismatch");
        }
        declared.insert(item.path.clone(), item.sha256.clone());
    }
    declared.insert("manifest.json".into(), hash(&root.join("manifest.json"))?);
    let sums = String::from_utf8(bounded_read(&root.join("SHA256SUMS"))?)
        .map_err(|_| "package_checksum_invalid")?;
    let mut checksums = BTreeMap::new();
    for line in sums.lines() {
        let (sha, relative) = line.split_once("  ").ok_or("package_checksum_invalid")?;
        if !valid_hash(sha, 64)
            || !valid_relative(relative)
            || checksums
                .insert(relative.to_owned(), sha.to_owned())
                .is_some()
        {
            return Err("package_checksum_invalid");
        }
    }
    if checksums != declared {
        return Err("package_checksum_mismatch");
    }
    // Only declared paths are opened; regular_file checks every ancestor.
    // Unlisted files are neither trusted nor executed by this verification.
    Ok(manifest)
}

pub fn validate(executable: &Path) -> Result<ProductLayout, &'static str> {
    let executable = regular_file(executable)?;
    if executable
        .file_name()
        .is_none_or(|name| !name.eq_ignore_ascii_case("nexa-desktop.exe"))
    {
        return Err("desktop_executable_name_invalid");
    }
    let root = executable.parent().ok_or("package_root_unavailable")?;
    let desktop = verify(root, "nexa-desktop", &["nexa-desktop.exe"])?;
    let runtime_root = root.join("runtime");
    let runtime = verify(
        &runtime_root,
        "nexa-runtime",
        &["ai-runtime.exe", "ai-runtime-worker.exe"],
    )?;
    if runtime.project_commit != desktop.project_commit
        || runtime.project_dirty != desktop.project_dirty
    {
        return Err("runtime_source_identity_mismatch");
    }
    let download = verify(&root.join("download"), "nexa-download", &["nexa-aria2.exe"])?;
    if download.project_commit != desktop.project_commit
        || download.project_dirty != desktop.project_dirty
    {
        return Err("runtime_source_identity_mismatch");
    }
    regular_file(&root.join("download/nexa-aria2.exe"))?;
    let runtime_executable = regular_file(&runtime_root.join("ai-runtime.exe"))?;
    regular_file(&runtime_root.join("ai-runtime-worker.exe"))?;
    Ok(ProductLayout {
        package_root: root.to_path_buf(),
        runtime_executable,
        project_commit: desktop.project_commit,
        project_dirty: desktop.project_dirty,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    fn inventory(
        root: &Path,
        directory: &Path,
        out: &mut BTreeSet<String>,
        directories: &mut BTreeSet<String>,
    ) -> Result<(), &'static str> {
        for entry in fs::read_dir(directory).map_err(|_| "package_directory_unavailable")? {
            let entry = entry.map_err(|_| "package_directory_unavailable")?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(|_| "package_file_unavailable")?;
            if metadata.file_type().is_symlink() {
                return Err("package_indirect_path");
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if metadata.file_attributes() & 0x400 != 0 {
                    return Err("package_indirect_path");
                }
            }
            if metadata.is_dir() {
                // Relative depth is bounded, independently of the OS recursion limit.
                if path
                    .strip_prefix(root)
                    .map_err(|_| "package_path_invalid")?
                    .components()
                    .count()
                    > 16
                {
                    return Err("package_path_invalid");
                }
                let relative = path
                    .strip_prefix(root)
                    .map_err(|_| "package_path_invalid")?
                    .to_str()
                    .ok_or("package_path_invalid")?
                    .replace('\\', "/");
                if !valid_relative(&relative)
                    || !directories.insert(relative)
                    || directories.len() > MAX_FILES
                {
                    return Err("package_inventory_invalid");
                }
                inventory(root, &path, out, directories)?;
            } else {
                regular_file(&path)?;
                let relative = path
                    .strip_prefix(root)
                    .map_err(|_| "package_path_invalid")?
                    .to_str()
                    .ok_or("package_path_invalid")?
                    .replace('\\', "/");
                if !valid_relative(&relative) || !out.insert(relative) || out.len() > MAX_FILES {
                    return Err("package_inventory_invalid");
                }
            }
        }
        Ok(())
    }
    fn fixture(root: &Path, product: &str) {
        let mut items = BTreeSet::new();
        inventory(root, root, &mut items, &mut BTreeSet::new()).unwrap();
        let files: Vec<_> = items.into_iter().map(|path| serde_json::json!({"sha256": hash(&root.join(&path)).unwrap(), "size_bytes": fs::metadata(root.join(&path)).unwrap().len(), "path": path})).collect();
        fs::write(root.join("manifest.json"), serde_json::to_vec(&serde_json::json!({"product":product,"project_commit":"a".repeat(40),"project_dirty":false,"files":files})).unwrap()).unwrap();
        let mut items = BTreeSet::new();
        inventory(root, root, &mut items, &mut BTreeSet::new()).unwrap();
        fs::write(
            root.join("SHA256SUMS"),
            items
                .iter()
                .map(|path| format!("{}  {path}\n", hash(&root.join(path)).unwrap()))
                .collect::<String>(),
        )
        .unwrap();
    }
    pub(crate) fn complete_fixture(root: &Path) {
        fs::create_dir(root.join("runtime")).unwrap();
        for name in ["ai-runtime.exe", "ai-runtime-worker.exe"] {
            fs::write(root.join("runtime").join(name), name).unwrap();
        }
        fixture(&root.join("runtime"), "nexa-runtime");
        fs::create_dir(root.join("download")).unwrap();
        fs::write(root.join("download/nexa-aria2.exe"), "synthetic component").unwrap();
        fixture(&root.join("download"), "nexa-download");
        fs::write(root.join("nexa-desktop.exe"), "desktop").unwrap();
        fixture(root, "nexa-desktop");
    }
    #[test]
    fn extra_files_and_directories_are_ignored_without_modifying_payload() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        complete_fixture(root);
        let manifest = fs::read(root.join("manifest.json")).unwrap();
        let sums = fs::read(root.join("SHA256SUMS")).unwrap();
        for name in [
            "uninstall.exe",
            "desktop.ini",
            "unlisted.dll",
            "script.ps1",
            "invalid.gguf",
            "arbitrary.part",
            "runtime/extra.dll",
            "download/extra.txt",
            "models/nested/model.gguf",
            "arbitrary/nested/notes.txt",
            ".nexa-download-invalid/unknown.bin",
        ] {
            let path = root.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, b"inert extra bytes").unwrap();
            let result = validate(&root.join("nexa-desktop.exe")).unwrap();
            assert_eq!(result.package_root, fs::canonicalize(root).unwrap());
            assert!(result.runtime_executable.ends_with("ai-runtime.exe"));
            assert_eq!(result.project_commit, "a".repeat(40));
            assert!(!result.project_dirty);
            assert_eq!(fs::read(path).unwrap(), b"inert extra bytes");
        }
        fs::create_dir_all(root.join("empty/nested")).unwrap();
        assert!(validate(&root.join("nexa-desktop.exe")).is_ok());
        assert_eq!(fs::read(root.join("manifest.json")).unwrap(), manifest);
        assert_eq!(fs::read(root.join("SHA256SUMS")).unwrap(), sums);
    }

    #[test]
    fn declared_files_remain_required_and_hash_checked_with_extras_present() {
        for name in [
            "nexa-desktop.exe",
            "runtime/ai-runtime.exe",
            "runtime/ai-runtime-worker.exe",
            "download/nexa-aria2.exe",
        ] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path();
            complete_fixture(root);
            fs::write(root.join("uninstall.exe"), b"inert installer extra").unwrap();
            let path = root.join(name);
            let original = fs::read(&path).unwrap();
            // Same-size corruption must be caught by hash, not just length.
            fs::write(&path, vec![b'X'; original.len()]).unwrap();
            assert_eq!(
                validate(&root.join("nexa-desktop.exe")).err(),
                Some("package_file_hash_mismatch"),
                "{name}"
            );
            fs::remove_file(&path).unwrap();
            assert_eq!(
                validate(&root.join("nexa-desktop.exe")).err(),
                Some("selected_file_unavailable"),
                "{name}"
            );
        }
    }

    #[test]
    fn required_executables_cannot_be_omitted_from_self_consistent_manifests() {
        for (folder, product, required, omitted) in [
            (
                "",
                "nexa-desktop",
                &["nexa-desktop.exe"][..],
                "nexa-desktop.exe",
            ),
            (
                "runtime",
                "nexa-runtime",
                &["ai-runtime.exe", "ai-runtime-worker.exe"][..],
                "ai-runtime.exe",
            ),
            (
                "runtime",
                "nexa-runtime",
                &["ai-runtime.exe", "ai-runtime-worker.exe"][..],
                "ai-runtime-worker.exe",
            ),
            (
                "download",
                "nexa-download",
                &["nexa-aria2.exe"][..],
                "nexa-aria2.exe",
            ),
        ] {
            let temp = tempfile::tempdir().unwrap();
            complete_fixture(temp.path());
            let root = temp.path().join(folder);
            let manifest_path = root.join("manifest.json");
            let mut manifest: serde_json::Value =
                serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
            let files = manifest["files"].as_array_mut().unwrap();
            files.retain(|item| item["path"] != omitted);
            // Keep the manifest nonempty so the missing-required check is tested.
            fs::write(root.join("extra.txt"), "declared inert file").unwrap();
            files.push(serde_json::json!({"path": "extra.txt", "size_bytes": 19,
                "sha256": hash(&root.join("extra.txt")).unwrap()}));
            fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
            let mut sums = manifest["files"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| {
                    format!(
                        "{}  {}\n",
                        item["sha256"].as_str().unwrap(),
                        item["path"].as_str().unwrap()
                    )
                })
                .collect::<String>();
            sums.push_str(&format!(
                "{}  manifest.json\n",
                hash(&manifest_path).unwrap()
            ));
            fs::write(root.join("SHA256SUMS"), sums).unwrap();
            assert!(root.join(omitted).is_file());
            assert_eq!(
                verify(&root, product, required).err().unwrap(),
                "package_inventory_invalid",
                "{folder}/{omitted}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn unlisted_links_are_not_traversed_but_declared_links_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("package");
        fs::create_dir(&root).unwrap();
        complete_fixture(&root);
        // A dangling path and directory cycle prove no extra-tree traversal.
        std::os::unix::fs::symlink("missing", root.join("unused-link")).unwrap();
        std::os::unix::fs::symlink(&root, root.join("cycle")).unwrap();
        assert!(validate(&root.join("nexa-desktop.exe")).is_ok());
        let runtime = root.join("runtime");
        let outside = temp.path().join("outside");
        fs::rename(&runtime, &outside).unwrap();
        std::os::unix::fs::symlink(&outside, &runtime).unwrap();
        assert_eq!(
            validate(&root.join("nexa-desktop.exe")).err(),
            Some("selected_path_indirect")
        );
        fs::remove_file(runtime).unwrap();
        fs::rename(outside, root.join("runtime")).unwrap();
        let worker = root.join("runtime/ai-runtime-worker.exe");
        let target = temp.path().join("worker.exe");
        fs::rename(&worker, &target).unwrap();
        std::os::unix::fs::symlink(target, worker).unwrap();
        assert_eq!(
            validate(&root.join("nexa-desktop.exe")).err(),
            Some("selected_path_indirect")
        );
    }

    #[cfg(windows)]
    #[test]
    fn declared_directory_junction_is_rejected_but_unlisted_junction_is_not_scanned() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("package");
        fs::create_dir(&root).unwrap();
        complete_fixture(&root);
        let outside = temp.path().join("outside");
        fs::rename(root.join("runtime"), &outside).unwrap();
        for name in ["runtime", "extra"] {
            let link = root.join(name);
            let output = std::process::Command::new("cmd.exe")
                .args(["/D", "/C", "mklink", "/J"])
                .arg(&link)
                .arg(&outside)
                .output()
                .unwrap();
            assert!(output.status.success(), "junction fixture creation failed");
            if name == "runtime" {
                assert_eq!(
                    validate(&root.join("nexa-desktop.exe")).err(),
                    Some("selected_path_indirect")
                );
                fs::remove_dir(&link).unwrap();
                fs::create_dir(root.join("runtime")).unwrap();
                for entry in fs::read_dir(&outside).unwrap() {
                    let entry = entry.unwrap();
                    fs::copy(entry.path(), root.join("runtime").join(entry.file_name())).unwrap();
                }
            } else {
                assert!(validate(&root.join("nexa-desktop.exe")).is_ok());
                fs::remove_dir(&link).unwrap();
            }
        }
    }

    #[test]
    fn missing_file_invalid_manifest_and_checksum_have_distinct_codes() {
        let temp = tempfile::tempdir().unwrap();
        complete_fixture(temp.path());
        let executable = temp.path().join("nexa-desktop.exe");
        let sums = temp.path().join("SHA256SUMS");
        fs::write(&sums, "invalid").unwrap();
        assert!(matches!(
            validate(&executable),
            Err("package_checksum_invalid")
        ));
        fs::remove_file(temp.path().join("runtime/ai-runtime-worker.exe")).unwrap();
        assert!(matches!(
            validate(&executable),
            Err("selected_file_unavailable")
        ));
        fs::write(temp.path().join("manifest.json"), "invalid").unwrap();
        assert!(matches!(
            validate(&executable),
            Err("package_manifest_invalid")
        ));
    }
    #[test]
    fn parent_directory_name_does_not_enter_package_identity() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("中文 空格 原目录");
        fs::create_dir(&root).unwrap();
        complete_fixture(&root);
        let renamed = temp.path().join("中文 新目录");
        fs::rename(root, &renamed).unwrap();
        assert!(validate(&renamed.join("nexa-desktop.exe")).is_ok());
    }
    #[cfg(windows)]
    #[test]
    fn canonical_verbatim_disk_path_and_original_path_agree() {
        let temp = tempfile::tempdir().unwrap();
        complete_fixture(temp.path());
        let executable = temp.path().join("nexa-desktop.exe");
        let canonical = fs::canonicalize(&executable).unwrap();
        assert!(
            matches!(canonical.components().next(), Some(std::path::Component::Prefix(p)) if matches!(p.kind(), std::path::Prefix::VerbatimDisk(_)))
        );
        assert_eq!(
            validate(&executable).unwrap().project_commit,
            validate(&canonical).unwrap().project_commit
        );
    }
    #[test]
    fn unsafe_relative_paths_rejected() {
        for name in [
            "",
            "/root",
            "../escape",
            "a/../b",
            "a//b",
            "a\\b",
            "C:x",
            "a/.",
            "a ",
            "nul\0",
        ] {
            assert!(!valid_relative(name), "{name:?}");
        }
        assert!(valid_relative("runtime/模型 许可.txt"));
    }
}
