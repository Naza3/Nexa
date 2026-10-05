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
const MAX_LICENSE_BUNDLE: u64 = 32 * 1024 * 1024;
const SOURCE: &str = "aria2-1.37.0-nexa-corresponding-source.tar.gz";
const LICENSE_INDEX: &str = "licenses/index.json";
const LICENSE_BUNDLE: &str = "licenses/THIRD_PARTY_LICENSES.txt";
const LICENSE_PREFIX: &[u8] = b"Nexa third-party license bundle (format 1)\nOriginal license and notice bytes are preserved without modification.\nComponent, version and source mappings are in index.json.\n";
const LICENSE_FOOTER: &[u8] = b"\nEND ORIGINAL\n";
const DOWNLOAD_FILES: &[&str] = &[
    "nexa-aria2.exe",
    SOURCE,
    "build-manifest.json",
    LICENSE_INDEX,
    LICENSE_BUNDLE,
];
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
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LicenseIndex {
    schema_version: u64,
    format: String,
    documents: Vec<LicenseDocument>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LicenseDocument {
    original_path: String,
    sha256: String,
    size_bytes: u64,
    stored_path: String,
    offset_bytes: u64,
    attributions: Vec<LicenseAttribution>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LicenseAttribution {
    component: String,
    version: String,
    source: String,
}
fn license_header(path: &str, sha256: &str, size: u64) -> String {
    format!(
        "\n===== ORIGINAL FILE: {path} =====\nSHA256: {sha256}\nBytes: {size}\nBEGIN ORIGINAL\n"
    )
}
fn license_attribution(
    name: &str,
    lock: &serde_json::Value,
) -> Result<LicenseAttribution, &'static str> {
    let runtime = lock["runtime_licenses"]
        .as_array()
        .ok_or(ERROR)?
        .iter()
        .find(|item| item["filename"] == name);
    let (component, origin) = if name == "aria2-COPYING" {
        ("aria2", &lock["aria2"])
    } else if let Some(item) = runtime {
        (name.strip_suffix("-LICENSE.TXT").ok_or(ERROR)?, item)
    } else {
        ("LLVM/MinGW runtime", &lock["toolchain"])
    };
    let version = if name == "aria2-COPYING" {
        &lock["aria2"]["version"]
    } else {
        &lock["toolchain"]["version"]
    };
    Ok(LicenseAttribution {
        component: component.into(),
        version: version.as_str().ok_or(ERROR)?.into(),
        source: origin["url"].as_str().ok_or(ERROR)?.into(),
    })
}
/// The final bundle preserves every original build record, including duplicates.
/// Exact framing binds offsets to the complete file, not just isolated slices.
fn verify_license_bundle(
    index: &[u8],
    bundle: &[u8],
    build: &serde_json::Value,
) -> Result<(), &'static str> {
    if index.len() as u64 > MAX_MANIFEST || bundle.len() as u64 > MAX_LICENSE_BUNDLE {
        return Err(ERROR);
    }
    let index: LicenseIndex = serde_json::from_slice(index).map_err(|_| ERROR)?;
    if index.schema_version != 1
        || index.format != "nexa-license-bundle-v1"
        || index.documents.len() != LICENSES.len()
    {
        return Err(ERROR);
    }
    fn consume(bundle: &[u8], cursor: &mut usize, expected: &[u8]) -> Result<(), &'static str> {
        let end = cursor.checked_add(expected.len()).ok_or(ERROR)?;
        if bundle.get(*cursor..end) != Some(expected) {
            return Err(ERROR);
        }
        *cursor = end;
        Ok(())
    }
    let mut cursor = 0;
    consume(bundle, &mut cursor, LICENSE_PREFIX)?;
    // LICENSES is lexicographically ordered. Zipping also requires every path
    // exactly once, with no aliases or alternate case accepted.
    for (document, name) in index.documents.iter().zip(LICENSES) {
        let expected = license_attribution(name, &build["source_lock"])?;
        if document.original_path != format!("licenses/{name}")
            || document.stored_path != LICENSE_BUNDLE
            || !hex(&document.sha256, 64)
            || build["files"][&document.original_path]["sha256"] != document.sha256
            || build["files"][&document.original_path]["bytes"] != document.size_bytes
            || document.attributions.len() != 1
        {
            return Err(ERROR);
        }
        let attribution = &document.attributions[0];
        if attribution.component != expected.component
            || attribution.version != expected.version
            || attribution.source != expected.source
        {
            return Err(ERROR);
        }
        consume(
            bundle,
            &mut cursor,
            license_header(
                &document.original_path,
                &document.sha256,
                document.size_bytes,
            )
            .as_bytes(),
        )?;
        let offset = usize::try_from(document.offset_bytes).map_err(|_| ERROR)?;
        let size = usize::try_from(document.size_bytes).map_err(|_| ERROR)?;
        let end = offset.checked_add(size).ok_or(ERROR)?;
        if offset != cursor {
            return Err(ERROR);
        }
        let original = bundle.get(offset..end).ok_or(ERROR)?;
        if std::str::from_utf8(original).is_err()
            || original.contains(&0)
            || format!("{:x}", Sha256::digest(original)) != document.sha256
        {
            return Err(ERROR);
        }
        cursor = end;
        consume(bundle, &mut cursor, LICENSE_FOOTER)?;
    }
    if cursor != bundle.len() {
        return Err(ERROR);
    }
    Ok(())
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
        if !relative(&entry.path)
            || !DOWNLOAD_FILES.contains(&entry.path.as_str())
            || !hex(&entry.sha256, 64)
            || !folded.insert(entry.path.to_lowercase())
            || (entry.path == LICENSE_INDEX && entry.size_bytes > MAX_MANIFEST)
            || (entry.path == LICENSE_BUNDLE && entry.size_bytes > MAX_LICENSE_BUNDLE)
        {
            return Err(ERROR);
        }
        let path = root.join(&entry.path);
        if fs::metadata(&path).map_err(|_| ERROR)?.len() != entry.size_bytes
            || digest(&path)? != entry.sha256
        {
            return Err(ERROR);
        }
        if matches!(entry.path.as_str(), "nexa-aria2.exe" | SOURCE)
            && (build["files"][&entry.path]["sha256"] != entry.sha256
                || build["files"][&entry.path]["bytes"] != entry.size_bytes)
        {
            return Err(ERROR);
        }
        declared.insert(entry.path, entry.sha256);
    }
    for required in DOWNLOAD_FILES {
        if !declared.contains_key(*required) {
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
    verify_license_bundle(
        &bytes(&root.join(LICENSE_INDEX), MAX_MANIFEST)?,
        &bytes(&root.join(LICENSE_BUNDLE), MAX_LICENSE_BUNDLE)?,
        &build,
    )?;
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
            // Identical originals still have distinct framed segments. Include
            // CRLF, UTF-8 and marker-like bytes to forbid text rewriting.
            fs::write(
                root.join("licenses").join(license),
                "synthetic license\r\n版权\nEND ORIGINAL\n".as_bytes(),
            )
            .unwrap();
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
        let build = serde_json::json!({"schema_version":1,"source_commit":"a".repeat(40),"binary_name":"nexa-aria2.exe","product_relative_path":"download/nexa-aria2.exe","target":"x86_64-w64-mingw32","tls_backend":"Schannel","features":features,"source_lock":serde_json::from_str::<serde_json::Value>(LOCK).unwrap(),"pe":{"imports":["secur32.dll"]},"files":build_files});
        fs::write(
            root.join("build-manifest.json"),
            serde_json::to_vec(&build).unwrap(),
        )
        .unwrap();
        let mut bundle = LICENSE_PREFIX.to_vec();
        let mut documents = Vec::new();
        for name in LICENSES {
            let original_path = format!("licenses/{name}");
            let raw = fs::read(root.join(&original_path)).unwrap();
            let sha256 = format!("{:x}", Sha256::digest(&raw));
            let size_bytes = raw.len() as u64;
            bundle
                .extend_from_slice(license_header(&original_path, &sha256, size_bytes).as_bytes());
            let offset_bytes = bundle.len() as u64;
            bundle.extend_from_slice(&raw);
            bundle.extend_from_slice(LICENSE_FOOTER);
            let attribution = license_attribution(name, &build["source_lock"]).unwrap();
            documents.push(serde_json::json!({
                "original_path": original_path,
                "sha256": sha256,
                "size_bytes": size_bytes,
                "stored_path": LICENSE_BUNDLE,
                "offset_bytes": offset_bytes,
                "attributions": [{"component": attribution.component,
                    "version": attribution.version, "source": attribution.source}],
            }));
            fs::remove_file(root.join(&original_path)).unwrap();
        }
        fs::write(root.join(LICENSE_BUNDLE), bundle).unwrap();
        fs::write(
            root.join(LICENSE_INDEX),
            serde_json::to_vec(&serde_json::json!({"schema_version": 1,
                "format": "nexa-license-bundle-v1", "documents": documents}))
            .unwrap(),
        )
        .unwrap();
        refresh_manifest(root);
    }
    fn refresh_manifest(root: &Path) {
        let mut names: Vec<_> = DOWNLOAD_FILES
            .iter()
            .map(|name| (*name).to_owned())
            .collect();
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
    fn bundle_fixture(root: &Path) -> (serde_json::Value, Vec<u8>, serde_json::Value) {
        (
            serde_json::from_slice(&fs::read(root.join(LICENSE_INDEX)).unwrap()).unwrap(),
            fs::read(root.join(LICENSE_BUNDLE)).unwrap(),
            serde_json::from_slice(&fs::read(root.join("build-manifest.json")).unwrap()).unwrap(),
        )
    }
    #[test]
    fn python_generated_bundle_contract_is_accepted() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../scripts/fixtures/license-bundle/download-contract.json"
        ))
        .unwrap();
        let build = serde_json::json!({
            "source_lock": serde_json::from_str::<serde_json::Value>(LOCK).unwrap(),
            "files": fixture["build_files"],
        });
        assert!(
            verify_license_bundle(
                &serde_json::to_vec(&fixture["index"]).unwrap(),
                fixture["bundle"].as_str().unwrap().as_bytes(),
                &build,
            )
            .is_ok()
        );
    }
    #[test]
    fn bundle_has_exact_originals_and_preserves_original_text_bytes() {
        let temp = tempfile::tempdir().unwrap();
        fixture(temp.path());
        let (index, bundle, build) = bundle_fixture(temp.path());
        assert!(
            verify_license_bundle(&serde_json::to_vec(&index).unwrap(), &bundle, &build).is_ok()
        );
        let mut previous_offset = 0;
        for document in index["documents"].as_array().unwrap() {
            let offset = document["offset_bytes"].as_u64().unwrap() as usize;
            let size = document["size_bytes"].as_u64().unwrap() as usize;
            assert!(offset > previous_offset);
            assert_eq!(
                &bundle[offset..offset + size],
                "synthetic license\r\n版权\nEND ORIGINAL\n".as_bytes()
            );
            previous_offset = offset;
        }
        assert!(LICENSES.windows(2).all(|names| names[0] < names[1]));
        let mut files = BTreeSet::new();
        inventory(temp.path(), temp.path(), &mut files, &mut Vec::new()).unwrap();
        assert_eq!(files.len(), 7);
    }
    #[test]
    fn bundle_rejects_binary_originals_even_with_matching_hashes() {
        for invalid in [0, 0xff] {
            let temp = tempfile::tempdir().unwrap();
            fixture(temp.path());
            let (mut index, mut bundle, mut build) = bundle_fixture(temp.path());
            let document = &mut index["documents"][0];
            let offset = document["offset_bytes"].as_u64().unwrap() as usize;
            let size = document["size_bytes"].as_u64().unwrap() as usize;
            bundle[offset] = invalid;
            let sha = format!("{:x}", Sha256::digest(&bundle[offset..offset + size]));
            document["sha256"] = sha.clone().into();
            build["files"]["licenses/COPYING"]["sha256"] = sha.clone().into();
            let header = license_header("licenses/COPYING", &sha, size as u64);
            bundle[LICENSE_PREFIX.len()..offset].copy_from_slice(header.as_bytes());
            assert!(
                verify_license_bundle(&serde_json::to_vec(&index).unwrap(), &bundle, &build)
                    .is_err()
            );
        }
    }
    #[test]
    fn bundle_index_rejects_schema_aliases_and_invalid_attributions() {
        let temp = tempfile::tempdir().unwrap();
        fixture(temp.path());
        let (valid, bundle, build) = bundle_fixture(temp.path());
        for mutation in [
            "schema",
            "boolean-schema",
            "float-schema",
            "format",
            "extra-field",
            "missing-field",
            "missing-document",
            "extra-document",
            "duplicate-document",
            "reordered-documents",
            "path-traversal",
            "path-case",
            "stored-path",
            "sha256",
            "sha256-case",
            "size",
            "negative-size",
            "float-size",
            "offset",
            "negative-offset",
            "float-offset",
            "overflow-offset",
            "overlap",
            "extra-document-field",
            "empty-attributions",
            "extra-attribution",
            "missing-attribution-field",
            "extra-attribution-field",
            "component",
            "version",
            "source",
            "wrong-runtime-source",
        ] {
            let mut index = valid.clone();
            match mutation {
                "schema" => index["schema_version"] = 2.into(),
                "boolean-schema" => index["schema_version"] = true.into(),
                "float-schema" => index["schema_version"] = 1.0.into(),
                "format" => index["format"] = "nexa-license-bundle-v2".into(),
                "extra-field" => index["extra"] = true.into(),
                "missing-field" => {
                    index.as_object_mut().unwrap().remove("format");
                }
                "missing-document" => {
                    index["documents"].as_array_mut().unwrap().pop();
                }
                "extra-document" => {
                    let duplicate = index["documents"][0].clone();
                    index["documents"].as_array_mut().unwrap().push(duplicate);
                }
                "duplicate-document" => index["documents"][1] = index["documents"][0].clone(),
                "reordered-documents" => index["documents"].as_array_mut().unwrap().swap(0, 1),
                "path-traversal" => {
                    index["documents"][0]["original_path"] = "licenses/../COPYING".into()
                }
                "path-case" => index["documents"][0]["original_path"] = "licenses/copying".into(),
                "stored-path" => {
                    index["documents"][0]["stored_path"] = "licenses/elsewhere.txt".into()
                }
                "sha256" => index["documents"][0]["sha256"] = "0".repeat(64).into(),
                "sha256-case" => {
                    index["documents"][0]["sha256"] = index["documents"][0]["sha256"]
                        .as_str()
                        .unwrap()
                        .to_ascii_uppercase()
                        .into()
                }
                "size" => index["documents"][0]["size_bytes"] = 1.into(),
                "negative-size" => index["documents"][0]["size_bytes"] = (-1).into(),
                "float-size" => index["documents"][0]["size_bytes"] = 1.0.into(),
                "offset" => index["documents"][0]["offset_bytes"] = 1.into(),
                "negative-offset" => index["documents"][0]["offset_bytes"] = (-1).into(),
                "float-offset" => index["documents"][0]["offset_bytes"] = 1.0.into(),
                "overflow-offset" => index["documents"][0]["offset_bytes"] = u64::MAX.into(),
                "overlap" => {
                    index["documents"][1]["offset_bytes"] =
                        index["documents"][0]["offset_bytes"].clone()
                }
                "extra-document-field" => index["documents"][0]["extra"] = true.into(),
                "empty-attributions" => {
                    index["documents"][0]["attributions"] = serde_json::json!([])
                }
                "extra-attribution" => {
                    let duplicate = index["documents"][0]["attributions"][0].clone();
                    index["documents"][0]["attributions"]
                        .as_array_mut()
                        .unwrap()
                        .push(duplicate);
                }
                "missing-attribution-field" => {
                    index["documents"][0]["attributions"][0]
                        .as_object_mut()
                        .unwrap()
                        .remove("source");
                }
                "extra-attribution-field" => {
                    index["documents"][0]["attributions"][0]["extra"] = true.into()
                }
                "component" => index["documents"][0]["attributions"][0]["component"] = "".into(),
                "version" => {
                    index["documents"][0]["attributions"][0]["version"] = "unlocked".into()
                }
                "source" => {
                    index["documents"][0]["attributions"][0]["source"] =
                        "https://example.invalid/LICENSE".into()
                }
                "wrong-runtime-source" => {
                    index["documents"][6]["attributions"][0]["source"] =
                        build["source_lock"]["toolchain"]["url"].clone()
                }
                _ => unreachable!(),
            }
            assert!(
                verify_license_bundle(&serde_json::to_vec(&index).unwrap(), &bundle, &build)
                    .is_err(),
                "accepted {mutation}"
            );
        }
        let index = serde_json::to_string(&valid).unwrap();
        // Typed parsing must reject duplicate keys, even with identical values.
        for key in ["schema_version", "original_path", "component"] {
            let token = match key {
                "schema_version" => "\"schema_version\":1",
                "original_path" => "\"original_path\":\"licenses/COPYING\"",
                _ => "\"component\":\"LLVM/MinGW runtime\"",
            };
            let duplicate = index.replacen(token, &format!("{token},{token}"), 1);
            assert_ne!(duplicate, index);
            assert!(verify_license_bundle(duplicate.as_bytes(), &bundle, &build).is_err());
        }
    }
    #[test]
    fn bundle_bytes_and_build_originals_remain_bound_after_rehashing() {
        for mutation in [
            "prefix",
            "header",
            "raw",
            "footer",
            "gap",
            "truncated",
            "extra",
            "build-hash",
            "build-size",
            "build-missing",
        ] {
            let temp = tempfile::tempdir().unwrap();
            fixture(temp.path());
            let (index, mut bundle, mut build) = bundle_fixture(temp.path());
            let first = &index["documents"][0];
            let offset = first["offset_bytes"].as_u64().unwrap() as usize;
            let size = first["size_bytes"].as_u64().unwrap() as usize;
            match mutation {
                "prefix" => bundle[0] ^= 1,
                "header" => bundle[LICENSE_PREFIX.len() + 1] ^= 1,
                "raw" => bundle[offset] ^= 1,
                "footer" => bundle[offset + size + 1] ^= 1,
                "gap" => bundle.insert(offset, b'\n'),
                "truncated" => {
                    bundle.pop();
                }
                "extra" => bundle.push(b'\n'),
                "build-hash" => {
                    build["files"]["licenses/COPYING"]["sha256"] = "0".repeat(64).into()
                }
                "build-size" => build["files"]["licenses/COPYING"]["bytes"] = 0.into(),
                "build-missing" => {
                    build["files"]
                        .as_object_mut()
                        .unwrap()
                        .remove("licenses/COPYING");
                }
                _ => unreachable!(),
            }
            fs::write(temp.path().join(LICENSE_BUNDLE), &bundle).unwrap();
            fs::write(
                temp.path().join("build-manifest.json"),
                serde_json::to_vec(&build).unwrap(),
            )
            .unwrap();
            refresh_manifest(temp.path());
            assert!(
                verify_component(temp.path(), None).is_err(),
                "accepted {mutation}"
            );
        }
    }
    #[test]
    fn bundle_offsets_and_lengths_cannot_overflow_or_exceed_file() {
        let temp = tempfile::tempdir().unwrap();
        fixture(temp.path());
        let (valid, _, original_build) = bundle_fixture(temp.path());
        for size in [u64::MAX, MAX_LICENSE_BUNDLE + 1] {
            let mut index = valid.clone();
            let mut build = original_build.clone();
            index["documents"][0]["size_bytes"] = size.into();
            build["files"]["licenses/COPYING"]["bytes"] = size.into();
            let sha = index["documents"][0]["sha256"].as_str().unwrap();
            let mut bundle = LICENSE_PREFIX.to_vec();
            bundle.extend_from_slice(license_header("licenses/COPYING", sha, size).as_bytes());
            index["documents"][0]["offset_bytes"] = (bundle.len() as u64).into();
            assert!(
                verify_license_bundle(&serde_json::to_vec(&index).unwrap(), &bundle, &build)
                    .is_err()
            );
        }
    }
    #[test]
    fn component_retains_exact_file_closure_and_build_identity() {
        for mutation in [
            "source",
            "executable",
            "legacy-license",
            "missing-index",
            "missing-bundle",
        ] {
            let temp = tempfile::tempdir().unwrap();
            fixture(temp.path());
            match mutation {
                "source" => fs::write(temp.path().join(SOURCE), b"different source").unwrap(),
                "executable" => {
                    let path = temp.path().join("nexa-aria2.exe");
                    let mut pe = fs::read(&path).unwrap();
                    // Leave the valid PE import table intact, changing only bytes
                    // that the original build manifest is responsible for binding.
                    pe[1000] = 1;
                    fs::write(path, pe).unwrap();
                }
                "legacy-license" => fs::write(
                    temp.path().join("licenses/COPYING"),
                    b"unexpected legacy payload",
                )
                .unwrap(),
                "missing-index" => fs::remove_file(temp.path().join(LICENSE_INDEX)).unwrap(),
                "missing-bundle" => fs::remove_file(temp.path().join(LICENSE_BUNDLE)).unwrap(),
                _ => unreachable!(),
            }
            if !mutation.starts_with("missing-") {
                refresh_manifest(temp.path());
            }
            assert!(
                verify_component(temp.path(), None).is_err(),
                "accepted {mutation}"
            );
        }
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
    #[cfg(unix)]
    #[test]
    fn symlink_license_payloads_are_rejected() {
        for name in [LICENSE_INDEX, LICENSE_BUNDLE] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("component");
            fs::create_dir(&root).unwrap();
            fixture(&root);
            let outside = temp.path().join("original");
            fs::rename(root.join(name), &outside).unwrap();
            std::os::unix::fs::symlink(&outside, root.join(name)).unwrap();
            assert!(verify_component(&root, None).is_err());
        }
    }
    #[cfg(windows)]
    #[test]
    fn guard_denies_write_delete_and_rename_until_released() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("component");
        fs::create_dir(&root).unwrap();
        fixture(&root);
        let (exe, guard) = verify_component(&root, None).unwrap();
        for path in [
            exe.clone(),
            root.join(LICENSE_INDEX),
            root.join(LICENSE_BUNDLE),
        ] {
            assert!(fs::OpenOptions::new().write(true).open(&path).is_err());
            assert!(fs::remove_file(&path).is_err());
        }
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
