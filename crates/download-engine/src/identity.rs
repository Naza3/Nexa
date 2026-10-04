//! Fixed packaged component identity. Hashes prove consistency, not publisher trust.
//! Windows handles deny write/delete sharing for the entire transfer/reaper lifetime.
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

const ERROR: &str = "model_download_engine_unavailable";
const MAX_MANIFEST: u64 = 2 * 1024 * 1024;
const SOURCE: &str = "aria2-1.37.0-nexa-corresponding-source.tar.gz";
const LICENSES: &[&str] = &[
    "COPYING",
    "COPYING.MinGW-w64-runtime.txt",
    "COPYING.MinGW-w64.txt",
    "COPYING.winpthreads.txt",
    "COPYING.winstorecompat.txt",
    "aria2-COPYING",
    "compiler-rt-LICENSE.TXT",
    "libcxx-LICENSE.TXT",
    "libcxxabi-LICENSE.TXT",
    "libunwind-LICENSE.TXT",
    "llvm-mingw-LICENSE.TXT",
];
const FORBIDDEN_FEATURES: &[&str] = &[
    "ENABLE_BITTORRENT",
    "ENABLE_METALINK",
    "HAVE_LIBCARES",
    "ENABLE_ASYNC_DNS",
    "HAVE_LIBSSH2",
    "HAVE_OPENSSL",
    "HAVE_GNUTLS",
    "HAVE_LIBGNUTLS",
    "HAVE_LIBXML2",
    "HAVE_LIBEXPAT",
    "HAVE_LIBZ",
    "HAVE_SQLITE3",
    "HAVE_LIBNETTLE",
    "HAVE_LIBGMP",
    "HAVE_LIBGCRYPT",
    "ENABLE_WEBSOCKET",
    "ENABLE_NLS",
];
const LOCK: &str = include_str!("../../../third_party/aria2/source-lock.json");
pub struct ComponentGuard {
    _handles: Vec<File>,
}
#[derive(Deserialize)]
struct Manifest {
    product: String,
    project_commit: String,
    files: Vec<Entry>,
}
#[derive(Deserialize)]
struct Entry {
    path: String,
    sha256: String,
    size_bytes: u64,
}
fn hex(s: &str, n: usize) -> bool {
    s.len() == n
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn relative(s: &str) -> bool {
    !s.is_empty()
        && !s.contains(['\\', ':', '\0'])
        && s.split('/')
            .all(|p| !p.is_empty() && p != "." && p != ".." && !p.ends_with([' ', '.']))
}
fn ordinary(path: &Path, directory: bool) -> Result<(), &'static str> {
    let m = fs::symlink_metadata(path).map_err(|_| ERROR)?;
    if m.file_type().is_symlink() || (directory && !m.is_dir()) || (!directory && !m.is_file()) {
        return Err(ERROR);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return Err(ERROR);
        }
    }
    Ok(())
}
fn pin(path: &Path, directory: bool) -> Result<File, &'static str> {
    ordinary(path, directory)?;
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options
            .share_mode(1)
            .custom_flags(0x00200000 | if directory { 0x02000000 } else { 0 });
        if directory {
            options.access_mode(0x80);
        }
    }
    let file = options.open(path).map_err(|_| ERROR)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if file.metadata().map_err(|_| ERROR)?.file_attributes() & 0x400 != 0 {
            return Err(ERROR);
        }
    }
    ordinary(path, directory)?;
    Ok(file)
}
fn bytes(path: &Path, limit: u64) -> Result<Vec<u8>, &'static str> {
    let mut result = Vec::new();
    File::open(path)
        .map_err(|_| ERROR)?
        .take(limit + 1)
        .read_to_end(&mut result)
        .map_err(|_| ERROR)?;
    if result.len() as u64 > limit {
        return Err(ERROR);
    }
    Ok(result)
}
fn digest(path: &Path) -> Result<String, &'static str> {
    let mut file = File::open(path).map_err(|_| ERROR)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = file.read(&mut buffer).map_err(|_| ERROR)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn inventory(
    root: &Path,
    dir: &Path,
    files: &mut BTreeSet<String>,
    handles: &mut Vec<File>,
) -> Result<(), &'static str> {
    for entry in fs::read_dir(dir).map_err(|_| ERROR)? {
        let path = entry.map_err(|_| ERROR)?.path();
        let relative = path
            .strip_prefix(root)
            .map_err(|_| ERROR)?
            .to_str()
            .ok_or(ERROR)?
            .replace('\\', "/");
        if path.is_dir() {
            if relative != "licenses" {
                return Err(ERROR);
            }
            handles.push(pin(&path, true)?);
            inventory(root, &path, files, handles)?;
        } else {
            if !self::relative(&relative) || files.len() >= 64 || !files.insert(relative) {
                return Err(ERROR);
            }
            handles.push(pin(&path, false)?);
        }
    }
    Ok(())
}
/// Native callers alone choose the fixed component directory. WebView never supplies it.
pub fn verify_component(
    root: &Path,
    expected_commit: Option<&str>,
) -> Result<(PathBuf, ComponentGuard), &'static str> {
    if !root.is_absolute() {
        return Err(ERROR);
    }
    let mut handles = Vec::new();
    let mut ancestors: Vec<_> = root.ancestors().collect();
    ancestors.reverse();
    for path in ancestors {
        handles.push(pin(path, true)?);
    }
    let root = fs::canonicalize(root).map_err(|_| ERROR)?;
    let mut actual = BTreeSet::new();
    inventory(&root, &root, &mut actual, &mut handles)?;
    let manifest: Manifest =
        serde_json::from_slice(&bytes(&root.join("manifest.json"), MAX_MANIFEST)?)
            .map_err(|_| ERROR)?;
    if manifest.product != "nexa-download"
        || !hex(&manifest.project_commit, 40)
        || expected_commit.is_some_and(|v| v != manifest.project_commit)
    {
        return Err(ERROR);
    }
    let build: serde_json::Value =
        serde_json::from_slice(&bytes(&root.join("build-manifest.json"), MAX_MANIFEST)?)
            .map_err(|_| ERROR)?;
    if build["schema_version"] != 1
        || build["source_commit"] != manifest.project_commit
        || build["binary_name"] != "nexa-aria2.exe"
        || build["product_relative_path"] != "download/nexa-aria2.exe"
        || build["target"] != "x86_64-w64-mingw32"
        || build["tls_backend"] != "Schannel"
        || build["source_lock"]
            != serde_json::from_str::<serde_json::Value>(LOCK).map_err(|_| ERROR)?
    {
        return Err(ERROR);
    }
    if build["features"]["SECURITY_WIN32"] != true
        || build["features"]["ENABLE_SSL"] != true
        || FORBIDDEN_FEATURES
            .iter()
            .any(|name| build["features"][*name] != false)
    {
        return Err(ERROR);
    }
    let mut declared = BTreeMap::new();
    let mut folded = BTreeSet::new();
    for entry in manifest.files {
        let role = matches!(
            entry.path.as_str(),
            "nexa-aria2.exe" | SOURCE | "build-manifest.json"
        ) || entry
            .path
            .strip_prefix("licenses/")
            .is_some_and(|n| LICENSES.contains(&n));
        if !relative(&entry.path)
            || !role
            || !hex(&entry.sha256, 64)
            || !folded.insert(entry.path.to_lowercase())
        {
            return Err(ERROR);
        }
        let path = root.join(&entry.path);
        if fs::metadata(&path).map_err(|_| ERROR)?.len() != entry.size_bytes
            || digest(&path)? != entry.sha256
        {
            return Err(ERROR);
        }
        if entry.path != "build-manifest.json"
            && (build["files"][&entry.path]["sha256"] != entry.sha256
                || build["files"][&entry.path]["bytes"] != entry.size_bytes)
        {
            return Err(ERROR);
        }
        declared.insert(entry.path, entry.sha256);
    }
    for required in [
        "nexa-aria2.exe",
        SOURCE,
        "build-manifest.json",
        "licenses/aria2-COPYING",
        "licenses/COPYING.MinGW-w64-runtime.txt",
        "licenses/llvm-mingw-LICENSE.TXT",
        "licenses/compiler-rt-LICENSE.TXT",
        "licenses/libcxx-LICENSE.TXT",
        "licenses/libcxxabi-LICENSE.TXT",
        "licenses/libunwind-LICENSE.TXT",
    ] {
        if !declared.contains_key(required) {
            return Err(ERROR);
        }
    }
    for license in LICENSES {
        if !declared.contains_key(&format!("licenses/{license}")) {
            return Err(ERROR);
        }
    }
    declared.insert("manifest.json".into(), digest(&root.join("manifest.json"))?);
    let sums =
        String::from_utf8(bytes(&root.join("SHA256SUMS"), MAX_MANIFEST)?).map_err(|_| ERROR)?;
    let mut checksums = BTreeMap::new();
    for line in sums.lines() {
        let (sha, path) = line.split_once("  ").ok_or(ERROR)?;
        if checksums.insert(path.to_owned(), sha.to_owned()).is_some() {
            return Err(ERROR);
        }
    }
    if declared != checksums {
        return Err(ERROR);
    }
    actual.remove("SHA256SUMS");
    if actual != declared.keys().cloned().collect() {
        return Err(ERROR);
    }
    let executable = root.join("nexa-aria2.exe");
    let imports = pe_imports(&bytes(&executable, 64 * 1024 * 1024)?)?;
    let expected: BTreeSet<_> = build["pe"]["imports"]
        .as_array()
        .ok_or(ERROR)?
        .iter()
        .map(|v| v.as_str().map(str::to_ascii_lowercase).ok_or(ERROR))
        .collect::<Result<_, _>>()?;
    if imports != expected || !imports.contains("secur32.dll") {
        return Err(ERROR);
    }
    Ok((executable, ComponentGuard { _handles: handles }))
}
fn system_dll(s: &str) -> bool {
    [
        "advapi32.dll",
        "bcrypt.dll",
        "crypt32.dll",
        "iphlpapi.dll",
        "kernel32.dll",
        "msvcrt.dll",
        "ntdll.dll",
        "ole32.dll",
        "secur32.dll",
        "shell32.dll",
        "user32.dll",
        "ws2_32.dll",
        "ucrtbase.dll",
        "gdi32.dll",
        "winmm.dll",
        "psapi.dll",
        "wsock32.dll",
    ]
    .contains(&s)
        || s.strip_prefix("api-ms-win-crt-")
            .and_then(|v| v.strip_suffix("-l1-1-0.dll"))
            .is_some_and(|v| {
                !v.is_empty()
                    && v.bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            })
}
/// Bounded AMD64 PE32+ imports parser; delay imports are deliberately forbidden.
fn pe_imports(b: &[u8]) -> Result<BTreeSet<String>, &'static str> {
    fn u16at(b: &[u8], p: usize) -> Result<u16, &'static str> {
        Ok(u16::from_le_bytes(
            b.get(p..p + 2)
                .ok_or(ERROR)?
                .try_into()
                .map_err(|_| ERROR)?,
        ))
    }
    fn u32at(b: &[u8], p: usize) -> Result<u32, &'static str> {
        Ok(u32::from_le_bytes(
            b.get(p..p + 4)
                .ok_or(ERROR)?
                .try_into()
                .map_err(|_| ERROR)?,
        ))
    }
    if b.get(..2) != Some(b"MZ") {
        return Err(ERROR);
    }
    let pe = u32at(b, 0x3c)? as usize;
    if b.get(pe..pe + 4) != Some(b"PE\0\0") || u16at(b, pe + 4)? != 0x8664 {
        return Err(ERROR);
    }
    let count = u16at(b, pe + 6)? as usize;
    let size = u16at(b, pe + 20)? as usize;
    let optional = pe + 24;
    if count == 0
        || count > 96
        || size < 240
        || u16at(b, optional)? != 0x20b
        || u32at(b, optional + 108)? < 14
        || u32at(b, optional + 112 + 13 * 8)? != 0
        || u32at(b, optional + 116 + 13 * 8)? != 0
    {
        return Err(ERROR);
    }
    let rva = |r: u32| -> Result<usize, &'static str> {
        for i in 0..count {
            let s = optional + size + i * 40;
            let start = u32at(b, s + 12)?;
            let length = u32at(b, s + 16)?;
            if r >= start && r - start < length {
                let p = u32at(b, s + 20)? as usize + (r - start) as usize;
                if p < b.len() {
                    return Ok(p);
                }
            }
        }
        Err(ERROR)
    };
    let table = u32at(b, optional + 120)?;
    let table_size = u32at(b, optional + 124)?;
    if table == 0 || !(20..=20 * 256).contains(&table_size) {
        return Err(ERROR);
    }
    let mut imports = BTreeSet::new();
    for index in 0..table_size / 20 {
        let p = rva(table.checked_add(index * 20).ok_or(ERROR)?)?;
        if b.get(p..p + 20).ok_or(ERROR)?.iter().all(|v| *v == 0) {
            return if imports.is_empty() {
                Err(ERROR)
            } else {
                Ok(imports)
            };
        }
        let name = rva(u32at(b, p + 12)?)?;
        let end = b
            .get(name..)
            .ok_or(ERROR)?
            .iter()
            .take(256)
            .position(|v| *v == 0)
            .ok_or(ERROR)?;
        let dll = std::str::from_utf8(&b[name..name + end])
            .map_err(|_| ERROR)?
            .to_ascii_lowercase();
        if !system_dll(&dll) {
            return Err(ERROR);
        }
        imports.insert(dll);
    }
    Err(ERROR)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(root: &Path) {
        fs::create_dir(root.join("licenses")).unwrap();
        let mut pe = vec![0u8; 1024];
        pe[..2].copy_from_slice(b"MZ");
        pe[0x3c..0x40].copy_from_slice(&64u32.to_le_bytes());
        pe[64..68].copy_from_slice(b"PE\0\0");
        pe[68..70].copy_from_slice(&0x8664u16.to_le_bytes());
        pe[70..72].copy_from_slice(&1u16.to_le_bytes());
        pe[84..86].copy_from_slice(&240u16.to_le_bytes());
        pe[88..90].copy_from_slice(&0x20bu16.to_le_bytes());
        pe[196..200].copy_from_slice(&16u32.to_le_bytes());
        pe[208..212].copy_from_slice(&0x1000u32.to_le_bytes());
        pe[212..216].copy_from_slice(&40u32.to_le_bytes());
        pe[340..344].copy_from_slice(&0x1000u32.to_le_bytes());
        pe[344..348].copy_from_slice(&512u32.to_le_bytes());
        pe[348..352].copy_from_slice(&512u32.to_le_bytes());
        pe[524..528].copy_from_slice(&0x1080u32.to_le_bytes());
        pe[640..652].copy_from_slice(b"secur32.dll\0");
        fs::write(root.join("nexa-aria2.exe"), pe).unwrap();
        fs::write(root.join(SOURCE), b"synthetic corresponding source").unwrap();
        for license in LICENSES {
            fs::write(root.join("licenses").join(license), b"synthetic license").unwrap();
        }
        let mut build_files = serde_json::Map::new();
        let mut names = vec!["nexa-aria2.exe".to_owned(), SOURCE.to_owned()];
        names.extend(LICENSES.iter().map(|s| format!("licenses/{s}")));
        for name in &names {
            build_files.insert(name.clone(), serde_json::json!({"sha256": digest(&root.join(name)).unwrap(), "bytes": fs::metadata(root.join(name)).unwrap().len()}));
        }
        let mut features = serde_json::Map::new();
        for name in FORBIDDEN_FEATURES {
            features.insert((*name).into(), false.into());
        }
        features.insert("SECURITY_WIN32".into(), true.into());
        features.insert("ENABLE_SSL".into(), true.into());
        fs::write(root.join("build-manifest.json"), serde_json::to_vec(&serde_json::json!({"schema_version":1,"source_commit":"a".repeat(40),"binary_name":"nexa-aria2.exe","product_relative_path":"download/nexa-aria2.exe","target":"x86_64-w64-mingw32","tls_backend":"Schannel","features":features,"source_lock":serde_json::from_str::<serde_json::Value>(LOCK).unwrap(),"pe":{"imports":["secur32.dll"]},"files":build_files})).unwrap()).unwrap();
        names.push("build-manifest.json".into());
        let files: Vec<_> = names.iter().map(|name| serde_json::json!({"path":name,"sha256":digest(&root.join(name)).unwrap(),"size_bytes":fs::metadata(root.join(name)).unwrap().len()})).collect();
        fs::write(root.join("manifest.json"), serde_json::to_vec(&serde_json::json!({"product":"nexa-download","project_commit":"a".repeat(40),"project_dirty":false,"files":files})).unwrap()).unwrap();
        names.push("manifest.json".into());
        names.sort();
        fs::write(
            root.join("SHA256SUMS"),
            names
                .iter()
                .map(|name| format!("{}  {name}\n", digest(&root.join(name)).unwrap()))
                .collect::<String>(),
        )
        .unwrap();
    }
    #[test]
    fn exact_component_hash_source_and_roles_are_required() {
        let temp = tempfile::tempdir().unwrap();
        fixture(temp.path());
        assert!(verify_component(temp.path(), Some(&"a".repeat(40))).is_ok());
        assert!(verify_component(temp.path(), Some(&"b".repeat(40))).is_err());
        fs::write(temp.path().join("unexpected.exe"), b"extra").unwrap();
        assert!(verify_component(temp.path(), None).is_err());
        fs::remove_file(temp.path().join("unexpected.exe")).unwrap();
        fs::create_dir(temp.path().join("unknown")).unwrap();
        assert!(verify_component(temp.path(), None).is_err());
        fs::remove_dir(temp.path().join("unknown")).unwrap();
        fs::write(temp.path().join(SOURCE), b"changed").unwrap();
        assert!(verify_component(temp.path(), None).is_err());
    }
    #[test]
    fn actual_pe_imports_and_delay_loads_are_checked() {
        let temp = tempfile::tempdir().unwrap();
        fixture(temp.path());
        let mut pe = fs::read(temp.path().join("nexa-aria2.exe")).unwrap();
        assert_eq!(
            pe_imports(&pe).unwrap(),
            BTreeSet::from(["secur32.dll".into()])
        );
        pe[640..652].copy_from_slice(b"unknown.dll\0");
        assert!(pe_imports(&pe).is_err());
        pe[640..652].copy_from_slice(b"secur32.dll\0");
        pe[304..308].copy_from_slice(&0x1000u32.to_le_bytes());
        assert!(pe_imports(&pe).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn symlink_component_and_ancestor_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("component");
        fs::create_dir(&root).unwrap();
        fixture(&root);
        let alias = temp.path().join("alias");
        std::os::unix::fs::symlink(&root, &alias).unwrap();
        assert!(verify_component(&alias, None).is_err());
        fs::remove_file(root.join(SOURCE)).unwrap();
        std::os::unix::fs::symlink(root.join("nexa-aria2.exe"), root.join(SOURCE)).unwrap();
        assert!(verify_component(&root, None).is_err());
    }
    #[cfg(windows)]
    #[test]
    fn guard_denies_write_delete_and_rename_until_released() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("component");
        fs::create_dir(&root).unwrap();
        fixture(&root);
        let (exe, guard) = verify_component(&root, None).unwrap();
        assert!(fs::OpenOptions::new().write(true).open(&exe).is_err());
        assert!(fs::remove_file(&exe).is_err());
        assert!(fs::rename(&root, temp.path().join("moved")).is_err());
        drop(guard);
        assert!(fs::OpenOptions::new().write(true).open(&exe).is_ok());
        fs::rename(root, temp.path().join("moved")).unwrap();
    }
    #[test]
    fn unsafe_names_and_imports_fail_closed() {
        for name in ["", "../evil", "a\\b", "a:b", "a/..", "a."] {
            assert!(!relative(name));
        }
        for name in [
            "unknown.dll",
            "libstdc++-6.dll",
            "api-ms-win-crt-../../evil-l1-1-0.dll",
        ] {
            assert!(!system_dll(name));
        }
        assert!(system_dll("secur32.dll"));
        assert!(pe_imports(b"MZmalformed").is_err());
    }
}
