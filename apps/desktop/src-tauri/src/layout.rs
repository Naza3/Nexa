//! Validate the complete private desktop layout before deriving the executable.
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
fn inventory(
    root: &Path,
    directory: &Path,
    out: &mut BTreeSet<String>,
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
            inventory(root, &path, out)?;
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
fn verify(root: &Path, expected_product: &str) -> Result<Manifest, &'static str> {
    let manifest: Manifest = serde_json::from_slice(&bounded_read(&root.join("manifest.json"))?)
        .map_err(|_| "package_manifest_invalid")?;
    if manifest.product != expected_product
        || !valid_hash(&manifest.project_commit, 40)
        || manifest.files.is_empty()
        || manifest.files.len() > MAX_FILES
    {
        return Err("package_identity_invalid");
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
    let mut actual = BTreeSet::new();
    inventory(root, root, &mut actual)?;
    actual.remove("SHA256SUMS");
    if actual != declared.keys().cloned().collect() {
        return Err("package_unlisted_file");
    }
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
    let desktop = verify(root, "nexa-desktop")?;
    let runtime_root = root.join("runtime");
    let runtime = verify(&runtime_root, "nexa-runtime")?;
    if runtime.project_commit != desktop.project_commit
        || runtime.project_dirty != desktop.project_dirty
    {
        return Err("runtime_source_identity_mismatch");
    }
    let runtime_executable = regular_file(&runtime_root.join("ai-runtime.exe"))?;
    regular_file(&runtime_root.join("ai-runtime-worker.exe"))?;
    Ok(ProductLayout {
        runtime_executable,
        project_commit: desktop.project_commit,
        project_dirty: desktop.project_dirty,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(root: &Path, product: &str) {
        let mut items = BTreeSet::new();
        inventory(root, root, &mut items).unwrap();
        let files: Vec<_> = items.into_iter().map(|path| serde_json::json!({"sha256": hash(&root.join(&path)).unwrap(), "size_bytes": fs::metadata(root.join(&path)).unwrap().len(), "path": path})).collect();
        fs::write(root.join("manifest.json"), serde_json::to_vec(&serde_json::json!({"product":product,"project_commit":"a".repeat(40),"project_dirty":false,"files":files})).unwrap()).unwrap();
        let mut items = BTreeSet::new();
        inventory(root, root, &mut items).unwrap();
        fs::write(
            root.join("SHA256SUMS"),
            items
                .iter()
                .map(|path| format!("{}  {path}\n", hash(&root.join(path)).unwrap()))
                .collect::<String>(),
        )
        .unwrap();
    }
    #[test]
    fn complete_matching_nested_runtime_is_required() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::create_dir(root.join("runtime")).unwrap();
        for name in ["ai-runtime.exe", "ai-runtime-worker.exe"] {
            fs::write(root.join("runtime").join(name), name).unwrap();
        }
        fixture(&root.join("runtime"), "nexa-runtime");
        fs::write(root.join("nexa-desktop.exe"), "desktop").unwrap();
        fixture(root, "nexa-desktop");
        let result = validate(&root.join("nexa-desktop.exe")).unwrap();
        assert!(result.runtime_executable.ends_with("ai-runtime.exe"));
        assert_eq!(result.project_commit.len(), 40);
        assert!(!result.project_dirty);
        fs::write(root.join("unlisted.dll"), "unexpected").unwrap();
        assert!(matches!(
            validate(&root.join("nexa-desktop.exe")),
            Err("package_unlisted_file")
        ));
        fs::remove_file(root.join("unlisted.dll")).unwrap();
        fs::write(root.join("runtime/ai-runtime.exe"), "tampered").unwrap();
        assert!(validate(&root.join("nexa-desktop.exe")).is_err());
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
