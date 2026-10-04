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
fn gguf_header(reader: &mut impl Read) -> bool {
    let mut magic = [0; 4];
    reader.read_exact(&mut magic).is_ok() && magic == *b"GGUF"
}

fn owned_download_partial(root: &Path, relative: &str) -> Result<bool, &'static str> {
    let parts: Vec<_> = relative.split('/').collect();
    let allowed = parts.len() == 1
        || (parts.len() == 2
            && (parts[0].eq_ignore_ascii_case("model") || parts[0].eq_ignore_ascii_case("models")));
    if !allowed {
        return Ok(false);
    }
    let name = parts.last().unwrap();
    let Some(id) = name
        .strip_prefix(".nexa-download-")
        .and_then(|s| s.strip_suffix(".part"))
    else {
        return Ok(false);
    };
    if !uuid::Uuid::parse_str(id).is_ok_and(|uuid| !uuid.is_nil() && uuid.to_string() == id) {
        return Ok(false);
    }
    // Inert leftovers are tolerated, never opened as models, executed or deleted.
    let file = regular_file(&root.join(relative))?;
    Ok(fs::metadata(file)
        .map_err(|_| "package_file_unavailable")?
        .len()
        <= 16 * 1024 * 1024 * 1024)
}

fn owned_sidecar_task(root: &Path, relative: &str) -> Result<bool, &'static str> {
    let parts: Vec<_> = relative.split('/').collect();
    if !(parts.len() == 1
        || (parts.len() == 2
            && (parts[0].eq_ignore_ascii_case("model") || parts[0].eq_ignore_ascii_case("models"))))
    {
        return Ok(false);
    }
    let Some(id) = parts.last().unwrap().strip_prefix(".nexa-download-") else {
        return Ok(false);
    };
    if !uuid::Uuid::parse_str(id).is_ok_and(|uuid| !uuid.is_nil() && uuid.to_string() == id) {
        return Ok(false);
    }
    // Inert crash leftovers are never resumed, adopted, executed or deleted.
    // inventory() already rejects indirect directories, including ancestors.
    let mut count = 0;
    for entry in fs::read_dir(root.join(relative)).map_err(|_| "package_directory_unavailable")? {
        let entry = entry.map_err(|_| "package_directory_unavailable")?;
        let max = match entry.file_name().to_str() {
            Some("payload.part") => 16 * 1024 * 1024 * 1024u64,
            Some("payload.part.aria2" | "payload.part.aria2__temp") => 1024 * 1024,
            _ => return Err("package_unlisted_file"),
        };
        let file = regular_file(&entry.path())?;
        count += 1;
        if count > 3
            || fs::metadata(file)
                .map_err(|_| "package_file_unavailable")?
                .len()
                > max
        {
            return Err("package_unlisted_file");
        }
    }
    Ok(true)
}

fn external_root_model(root: &Path, relative: &str) -> Result<bool, &'static str> {
    // Only undeclared direct children of root, model/ or models/ are candidates.
    // This is not model validation or import, and never hashes the model body.
    let parts: Vec<_> = relative.split('/').collect();
    let allowed_location = parts.len() == 1
        || (parts.len() == 2
            && (parts[0].eq_ignore_ascii_case("model") || parts[0].eq_ignore_ascii_case("models")));
    if !allowed_location
        || !Path::new(relative)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("gguf"))
    {
        return Ok(false);
    }
    let path = regular_file(&root.join(relative))?;
    let mut file = fs::File::open(path).map_err(|_| "package_file_unavailable")?;
    if !gguf_header(&mut file) {
        return Err("package_external_model_header_invalid");
    }
    Ok(true)
}

fn verify(
    root: &Path,
    expected_product: &str,
    allow_external_root_models: bool,
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
    let mut directories = BTreeSet::new();
    inventory(root, root, &mut actual, &mut directories)?;
    actual.remove("SHA256SUMS");
    let declared_names = declared.keys().cloned().collect::<BTreeSet<_>>();
    let mut task_directories = BTreeSet::new();
    if allow_external_root_models {
        for directory in &directories {
            if owned_sidecar_task(root, directory)? {
                task_directories.insert(directory.clone());
                if task_directories.len() > 64 {
                    return Err("package_unlisted_file");
                }
            }
        }
    }
    let mut partial_count = task_directories.len();
    for name in actual.difference(&declared_names) {
        if name
            .rsplit_once('/')
            .is_some_and(|(parent, _)| task_directories.contains(parent))
        {
            continue;
        }
        if allow_external_root_models && owned_download_partial(root, name)? {
            partial_count += 1;
            if partial_count > 64 {
                return Err("package_unlisted_file");
            }
            continue;
        }
        if !allow_external_root_models || !external_root_model(root, name)? {
            return Err("package_unlisted_file");
        }
    }
    if !declared_names.is_subset(&actual) {
        return Err("package_unlisted_file");
    }
    let mut declared_directories = BTreeSet::new();
    for name in &declared_names {
        for parent in Path::new(name)
            .ancestors()
            .skip(1)
            .filter(|path| !path.as_os_str().is_empty())
        {
            declared_directories.insert(
                parent
                    .to_str()
                    .ok_or("package_path_invalid")?
                    .replace('\\', "/"),
            );
        }
    }
    for directory in directories.difference(&declared_directories) {
        if task_directories.contains(directory) {
            continue;
        }
        if !(allow_external_root_models
            && (directory.eq_ignore_ascii_case("model")
                || directory.eq_ignore_ascii_case("models")))
        {
            return Err("package_unlisted_file");
        }
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
    let desktop = verify(root, "nexa-desktop", true)?;
    let runtime_root = root.join("runtime");
    let runtime = verify(&runtime_root, "nexa-runtime", false)?;
    if runtime.project_commit != desktop.project_commit
        || runtime.project_dirty != desktop.project_dirty
    {
        return Err("runtime_source_identity_mismatch");
    }
    let download = verify(&root.join("download"), "nexa-download", false)?;
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
    fn complete_matching_nested_runtime_is_required() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        complete_fixture(root);
        let result = validate(&root.join("nexa-desktop.exe")).unwrap();
        assert_eq!(result.package_root, fs::canonicalize(root).unwrap());
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
        assert!(matches!(
            validate(&root.join("nexa-desktop.exe")),
            Err("package_file_hash_mismatch")
        ));
    }
    #[test]
    fn additional_nonexecutable_file_has_distinct_code_and_is_not_modified() {
        let temp = tempfile::tempdir().unwrap();
        complete_fixture(temp.path());
        let extra = temp.path().join("desktop.ini");
        fs::write(&extra, "synthetic harmless extra file").unwrap();
        assert!(matches!(
            validate(&temp.path().join("nexa-desktop.exe")),
            Err("package_unlisted_file")
        ));
        assert_eq!(
            fs::read_to_string(&extra).unwrap(),
            "synthetic harmless extra file"
        );
    }
    #[test]
    fn external_root_gguf_is_input_not_declared_payload() {
        let temp = tempfile::tempdir().unwrap();
        complete_fixture(temp.path());
        let executable = temp.path().join("nexa-desktop.exe");
        let manifest = fs::read(temp.path().join("manifest.json")).unwrap();
        let sums = fs::read(temp.path().join("SHA256SUMS")).unwrap();
        for name in ["候选 模型.gguf", "UPPER.GGUF"] {
            fs::write(temp.path().join(name), b"GGUFsynthetic model input").unwrap();
        }
        assert!(validate(&executable).is_ok());
        assert_eq!(
            fs::read(temp.path().join("manifest.json")).unwrap(),
            manifest
        );
        assert_eq!(fs::read(temp.path().join("SHA256SUMS")).unwrap(), sums);
        // Payload integrity is still enforced while external inputs exist.
        fs::write(temp.path().join("runtime/ai-runtime.exe"), b"changed").unwrap();
        assert!(matches!(
            validate(&executable),
            Err("package_file_hash_mismatch")
        ));
    }
    #[test]
    fn only_ordinary_root_gguf_inputs_are_exempt() {
        for (name, content, expected) in [
            (
                "unlisted.dll",
                &b"GGUFnot a model"[..],
                "package_unlisted_file",
            ),
            (
                "unlisted.exe",
                &b"GGUFnot a model"[..],
                "package_unlisted_file",
            ),
            (
                "unlisted.ps1",
                &b"GGUFnot a model"[..],
                "package_unlisted_file",
            ),
            (
                "disguised.gguf",
                &b"MZexecutable"[..],
                "package_external_model_header_invalid",
            ),
            (
                "short.gguf",
                &b"GGU"[..],
                "package_external_model_header_invalid",
            ),
            (
                "runtime/model.gguf",
                &b"GGUFinput"[..],
                "package_unlisted_file",
            ),
        ] {
            let temp = tempfile::tempdir().unwrap();
            complete_fixture(temp.path());
            fs::write(temp.path().join(name), content).unwrap();
            assert_eq!(
                validate(&temp.path().join("nexa-desktop.exe")).err(),
                Some(expected),
                "{name}"
            );
        }
        let temp = tempfile::tempdir().unwrap();
        complete_fixture(temp.path());
        fs::create_dir(temp.path().join("arbitrary")).unwrap();
        fs::write(temp.path().join("arbitrary/input.gguf"), b"GGUFinput").unwrap();
        assert!(matches!(
            validate(&temp.path().join("nexa-desktop.exe")),
            Err("package_unlisted_file")
        ));
    }
    #[test]
    fn conventional_input_directories_allow_only_direct_regular_gguf() {
        let temp = tempfile::tempdir().unwrap();
        complete_fixture(temp.path());
        for name in ["model", "models"] {
            let directory = temp.path().join(name);
            fs::create_dir(&directory).unwrap();
            assert!(validate(&temp.path().join("nexa-desktop.exe")).is_ok());
            fs::write(directory.join("中文 模型.GGUF"), b"GGUFinput").unwrap();
            assert!(validate(&temp.path().join("nexa-desktop.exe")).is_ok());
            for extra in ["unknown.dll", "unknown.exe", "index.json", "script.ps1"] {
                let path = directory.join(extra);
                fs::write(&path, b"GGUFnot an approved input").unwrap();
                assert!(matches!(
                    validate(&temp.path().join("nexa-desktop.exe")),
                    Err("package_unlisted_file")
                ));
                fs::remove_file(path).unwrap();
            }
            fs::create_dir(directory.join("nested")).unwrap();
            assert!(matches!(
                validate(&temp.path().join("nexa-desktop.exe")),
                Err("package_unlisted_file")
            ));
            fs::remove_dir(directory.join("nested")).unwrap();
        }
    }

    #[test]
    fn owned_partial_leftovers_do_not_block_restart_or_weaken_package_integrity() {
        for folder in ["", "model", "models"] {
            let temp = tempfile::tempdir().unwrap();
            complete_fixture(temp.path());
            let target = temp.path().join(folder);
            if !folder.is_empty() {
                fs::create_dir(&target).unwrap();
            }
            let manifest = fs::read(temp.path().join("manifest.json")).unwrap();
            let sums = fs::read(temp.path().join("SHA256SUMS")).unwrap();
            let name = format!(".nexa-download-{}.part", uuid::Uuid::new_v4());
            fs::write(target.join(&name), b"incomplete inert bytes").unwrap();
            assert!(
                validate(&temp.path().join("nexa-desktop.exe")).is_ok(),
                "{folder}"
            );
            assert_eq!(
                fs::read(temp.path().join("manifest.json")).unwrap(),
                manifest
            );
            assert_eq!(fs::read(temp.path().join("SHA256SUMS")).unwrap(), sums);
            for bad in [
                "arbitrary.part",
                ".nexa-download-not-a-uuid.part",
                ".nexa-download-00000000-0000-0000-0000-000000000000.part",
                "extra.dll",
            ] {
                fs::write(target.join(bad), b"inert").unwrap();
                assert!(validate(&temp.path().join("nexa-desktop.exe")).is_err());
                fs::remove_file(target.join(bad)).unwrap();
            }
            fs::write(temp.path().join("runtime/ai-runtime.exe"), b"tampered").unwrap();
            assert!(matches!(
                validate(&temp.path().join("nexa-desktop.exe")),
                Err("package_file_hash_mismatch")
            ));
        }
    }

    #[test]
    fn bounded_sidecar_crash_leftovers_are_inert_and_do_not_block_restart() {
        for folder in ["", "model", "models"] {
            let temp = tempfile::tempdir().unwrap();
            complete_fixture(temp.path());
            let task = temp
                .path()
                .join(folder)
                .join(format!(".nexa-download-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&task).unwrap();
            let executable = temp.path().join("nexa-desktop.exe");
            assert!(validate(&executable).is_ok());
            for name in [
                "payload.part",
                "payload.part.aria2",
                "payload.part.aria2__temp",
            ] {
                fs::write(task.join(name), b"inert incomplete bytes").unwrap();
            }
            assert!(validate(&executable).is_ok());
            for unknown in ["unknown.dll", "other.part", "payload.part.exe"] {
                fs::write(task.join(unknown), b"untrusted").unwrap();
                assert!(matches!(
                    validate(&executable),
                    Err("package_unlisted_file")
                ));
                fs::remove_file(task.join(unknown)).unwrap();
            }
            fs::create_dir(task.join("nested")).unwrap();
            assert!(validate(&executable).is_err());
            fs::remove_dir(task.join("nested")).unwrap();
            fs::OpenOptions::new()
                .write(true)
                .open(task.join("payload.part.aria2"))
                .unwrap()
                .set_len(1024 * 1024 + 1)
                .unwrap();
            assert!(validate(&executable).is_err());
            fs::remove_file(task.join("payload.part.aria2")).unwrap();
            fs::write(temp.path().join("runtime/ai-runtime.exe"), b"tampered").unwrap();
            assert!(matches!(
                validate(&executable),
                Err("package_file_hash_mismatch")
            ));
        }
    }
    #[test]
    fn sidecar_leftover_names_and_count_remain_bounded() {
        let temp = tempfile::tempdir().unwrap();
        complete_fixture(temp.path());
        let executable = temp.path().join("nexa-desktop.exe");
        for name in [
            ".nexa-download-invalid",
            ".nexa-download-AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA",
            ".nexa-download-aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa",
            ".nexa-download-00000000-0000-0000-0000-000000000000",
        ] {
            fs::create_dir(temp.path().join(name)).unwrap();
            assert!(validate(&executable).is_err());
            fs::remove_dir(temp.path().join(name)).unwrap();
        }
        for _ in 0..64 {
            fs::create_dir(
                temp.path()
                    .join(format!(".nexa-download-{}", uuid::Uuid::new_v4())),
            )
            .unwrap();
        }
        assert!(validate(&executable).is_ok());
        fs::create_dir(
            temp.path()
                .join(format!(".nexa-download-{}", uuid::Uuid::new_v4())),
        )
        .unwrap();
        assert!(validate(&executable).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn sidecar_leftover_symlinks_are_never_followed() {
        let temp = tempfile::tempdir().unwrap();
        complete_fixture(temp.path());
        let task = temp
            .path()
            .join(format!(".nexa-download-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&task).unwrap();
        std::os::unix::fs::symlink(
            temp.path().join("nexa-desktop.exe"),
            task.join("payload.part"),
        )
        .unwrap();
        assert!(matches!(
            validate(&temp.path().join("nexa-desktop.exe")),
            Err("package_indirect_path")
        ));
    }
    #[cfg(unix)]
    #[test]
    fn sidecar_payload_size_budget_is_enforced_without_reading_body() {
        let temp = tempfile::tempdir().unwrap();
        complete_fixture(temp.path());
        let task = temp
            .path()
            .join(format!(".nexa-download-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&task).unwrap();
        let file = fs::File::create(task.join("payload.part")).unwrap();
        // Unix sparse extension tests the metadata bound without writing 16GiB.
        file.set_len(16 * 1024 * 1024 * 1024).unwrap();
        assert!(validate(&temp.path().join("nexa-desktop.exe")).is_ok());
        file.set_len(16 * 1024 * 1024 * 1024 + 1).unwrap();
        assert!(validate(&temp.path().join("nexa-desktop.exe")).is_err());
    }
    #[cfg(windows)]
    #[test]
    fn sidecar_leftover_junction_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("package");
        fs::create_dir(&root).unwrap();
        complete_fixture(&root);
        let outside = temp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        let task = root.join(format!(".nexa-download-{}", uuid::Uuid::new_v4()));
        let output = std::process::Command::new("cmd.exe")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(&task)
            .arg(&outside)
            .output()
            .unwrap();
        assert!(output.status.success(), "junction fixture creation failed");
        assert!(matches!(
            validate(&root.join("nexa-desktop.exe")),
            Err("package_indirect_path")
        ));
        fs::remove_dir(task).unwrap();
        assert!(outside.is_dir());
    }
    #[test]
    fn unknown_empty_directory_is_not_a_model_directory() {
        let temp = tempfile::tempdir().unwrap();
        complete_fixture(temp.path());
        fs::create_dir(temp.path().join("arbitrary")).unwrap();
        assert!(matches!(
            validate(&temp.path().join("nexa-desktop.exe")),
            Err("package_unlisted_file")
        ));
    }
    #[test]
    fn model_candidate_read_is_exactly_four_bytes() {
        struct FourBytes {
            read: usize,
        }
        impl Read for FourBytes {
            fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
                assert!(output.len() <= 4 - self.read, "model body must not be read");
                output.copy_from_slice(&b"GGUF"[self.read..self.read + output.len()]);
                self.read += output.len();
                Ok(output.len())
            }
        }
        let mut input = FourBytes { read: 0 };
        assert!(gguf_header(&mut input));
        assert_eq!(input.read, 4);
    }
    #[cfg(unix)]
    #[test]
    fn indirect_external_root_gguf_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("package");
        fs::create_dir(&root).unwrap();
        complete_fixture(&root);
        let model = temp.path().join("outside.gguf");
        fs::write(&model, b"GGUFinput").unwrap();
        std::os::unix::fs::symlink(&model, root.join("linked.gguf")).unwrap();
        assert!(matches!(
            validate(&root.join("nexa-desktop.exe")),
            Err("package_indirect_path")
        ));
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
