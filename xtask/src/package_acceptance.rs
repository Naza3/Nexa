//! T05 portable-package acceptance. The product is never repaired by this tool.
//! All subprocesses use the validated package, an owned temporary data directory,
//! a scrubbed search path, and the existing same-connection HMAC client.
use crate::api_smoke;
use runtime_cli::instance::{Discovery, InstanceLock, wait_stopped};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs::{self, File},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::{ExitCode, Stdio},
    time::{Duration, Instant},
};
use tokio::process::Child;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
const MODEL_HASH: &str = "9465e63a22add5354d9bb4b99e90117043c7124007664907259bd16d043bb031";
const LLAMA_COMMIT: &str = "2149c00f4442dc59302e134a02e4c99d5f7ed9fc";
const MODEL_ID: &str = "acceptance-qwen3-06b-q8";
const HELP: &str = "Nexa standalone package acceptance\n\
Usage: nexa-acceptance --model MODEL.gguf --out REPORT.json [--package DIRECTORY] [--machine-role target|ci|unknown] [--disconnect-cycles 1..50]\n\
Default package: ../windows-x64-cpu beside the acceptance-tools directory.\n\
Requires the separate, unmodified Windows x64 CPU product package and fixed Qwen3-0.6B Q8_0 model.\n\
Uses only an owned temporary directory and loopback port; no installation, admin access, Python, Cargo, or persistent service.\n\
Exit: 0 short package checks passed (inspect A20 evidence limits); 1 failed; 2 invalid arguments.\n\
SHA256 detects changed bytes, not publisher authenticity. Obtain tool/package hashes through a trusted channel.";

struct Options {
    package: PathBuf,
    model: PathBuf,
    out: PathBuf,
    machine_role: String,
    disconnect_cycles: u32,
}
#[derive(Deserialize)]
struct FileEntry {
    path: String,
    size_bytes: u64,
    sha256: String,
}
#[derive(Deserialize)]
struct Dependency {
    name: String,
    kind: String,
}
#[derive(Deserialize)]
struct Dependencies {
    imports: Vec<Dependency>,
}
#[derive(Deserialize)]
struct Manifest {
    schema_version: u32,
    product: String,
    package_version: String,
    project_commit: String,
    llama_commit: String,
    protocol_version: u32,
    worker_protocol_version: u32,
    shim_version: u32,
    platform: String,
    target: String,
    architecture: String,
    backend: String,
    configuration: String,
    files: Vec<FileEntry>,
    dependencies: BTreeMap<String, Dependencies>,
}
struct Package {
    root: PathBuf,
    manifest: Manifest,
    manifest_hash: String,
}
#[derive(Serialize)]
struct Check {
    id: &'static str,
    status: &'static str,
    detail: &'static str,
}
#[derive(Serialize)]
struct Report {
    schema_version: u32,
    kind: &'static str,
    last_stage: &'static str,
    verifier: Value,
    product: Value,
    environment: Value,
    path_coverage: Value,
    model_sha256: Option<String>,
    disconnect_cycles_requested: u32,
    inference: Value,
    checks: Vec<Check>,
    http_cli: Option<api_smoke::Report>,
    short_package_checks_passed: bool,
    a20_target_evidence: &'static str,
}
impl Report {
    fn check(&mut self, id: &'static str, pass: bool, detail: &'static str) {
        self.checks.push(Check {
            id,
            status: if pass { "pass" } else { "fail" },
            detail,
        });
    }
    fn skip(&mut self, id: &'static str, detail: &'static str) {
        self.checks.push(Check {
            id,
            status: "skipped",
            detail,
        });
    }
}
fn parse(args: &[OsString]) -> Result<Options> {
    let mut values = BTreeMap::new();
    for pair in args.chunks(2) {
        if pair.len() != 2 {
            return Err("each option requires a value".into());
        }
        let key = pair[0].to_str().ok_or("invalid option")?;
        if ![
            "--model",
            "--out",
            "--package",
            "--machine-role",
            "--disconnect-cycles",
        ]
        .contains(&key)
            || values.insert(key, pair[1].clone()).is_some()
        {
            return Err("unknown or duplicate option".into());
        }
    }
    let model = values.remove("--model").ok_or("--model required")?.into();
    let out = values.remove("--out").ok_or("--out required")?.into();
    let package = match values.remove("--package") {
        Some(path) => path.into(),
        None => std::env::current_exe()?
            .parent()
            .and_then(Path::parent)
            .ok_or("cannot locate adjacent product package")?
            .join("windows-x64-cpu"),
    };
    let machine_role = values
        .remove("--machine-role")
        .unwrap_or_else(|| "unknown".into())
        .into_string()
        .map_err(|_| "invalid machine role")?;
    if !["target", "ci", "unknown"].contains(&machine_role.as_str()) {
        return Err("invalid machine role".into());
    }
    let disconnect_cycles = values
        .remove("--disconnect-cycles")
        .map(|v| {
            v.into_string()
                .map_err(|_| "invalid cycles")
                .and_then(|v| v.parse::<u32>().map_err(|_| "invalid cycles"))
        })
        .transpose()?
        .unwrap_or(5);
    if !(1..=50).contains(&disconnect_cycles) {
        return Err("disconnect cycles must be in 1..=50".into());
    }
    Ok(Options {
        package,
        model,
        out,
        machine_role,
        disconnect_cycles,
    })
}
fn valid_hex(value: &str, count: usize) -> bool {
    value.len() == count
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn safe_relative(value: &str) -> Result<PathBuf> {
    if value.is_empty()
        || value.len() > 240
        || value.contains(['\\', ':', '\0', '\r', '\n'])
        || value.starts_with('/')
    {
        return Err("unsafe manifest path".into());
    }
    for part in value.split('/') {
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with([' ', '.'])
            || part.chars().any(char::is_control)
        {
            return Err("unsafe manifest path".into());
        }
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        if [
            "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
            "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
        ]
        .contains(&stem.as_str())
        {
            return Err("reserved Windows manifest path".into());
        }
    }
    let path = PathBuf::from(value);
    if !path.components().all(|p| matches!(p, Component::Normal(_))) {
        return Err("unsafe manifest path".into());
    }
    Ok(path)
}
fn ordinary_metadata(path: &Path) -> Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err("links are forbidden in a release package".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("reparse points are forbidden in a release package".into());
        }
    }
    Ok(metadata)
}
fn file_hash(path: &Path) -> Result<String> {
    if !ordinary_metadata(path)?.is_file() {
        return Err("expected a regular file".into());
    }
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn bounded_read(path: &Path, max: u64) -> Result<Vec<u8>> {
    if !ordinary_metadata(path)?.is_file() {
        return Err("expected a regular file".into());
    }
    let mut bytes = Vec::new();
    File::open(path)?.take(max + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err("metadata exceeds limit".into());
    }
    Ok(bytes)
}
fn inventory(root: &Path, relative: &str, files: &mut BTreeSet<String>) -> Result<()> {
    if files.len() > 4096 {
        return Err("package file limit exceeded".into());
    }
    for entry in fs::read_dir(root.join(relative))? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "package path is not Unicode")?;
        let joined = if relative.is_empty() {
            name
        } else {
            format!("{relative}/{name}")
        };
        safe_relative(&joined)?;
        let meta = ordinary_metadata(&entry.path())?;
        if meta.is_dir() {
            inventory(root, &joined, files)?;
        } else if meta.is_file() {
            files.insert(joined);
            if files.len() > 4096 {
                return Err("package file limit exceeded".into());
            }
        } else {
            return Err("package contains a non-regular entry".into());
        }
    }
    Ok(())
}
fn safe_license_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    ![
        ".exe", ".dll", ".gguf", ".pdb", ".lib", ".a", ".so", ".log", ".bat", ".ps1", ".py",
    ]
    .iter()
    .any(|suffix| lower.ends_with(suffix))
        && !lower.split('/').any(|part| {
            [
                ".git",
                "userdata",
                "secrets",
                "logs",
                "models",
                "node_modules",
                "target",
                "build",
            ]
            .contains(&part)
        })
}
fn verify_package(root: &Path) -> Result<Package> {
    if !ordinary_metadata(root)?.is_dir() {
        return Err("package directory required".into());
    }
    let root = root.canonicalize()?;
    let manifest_bytes = bounded_read(&root.join("manifest.json"), 4 * 1024 * 1024)?;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)?;
    if manifest.schema_version != 1
        || manifest.product != "nexa-runtime"
        || manifest.platform != "windows-x64"
        || manifest.target != "x86_64-pc-windows-msvc"
        || manifest.architecture != "x86_64"
        || manifest.backend != "cpu"
        || manifest.configuration != "Release"
        || manifest.protocol_version != 1
        || manifest.worker_protocol_version != 1
        || manifest.shim_version != 2
        || manifest.llama_commit != LLAMA_COMMIT
        || !valid_hex(&manifest.project_commit, 40)
        || manifest.package_version != env!("CARGO_PKG_VERSION")
    {
        return Err("unsupported release identity or protocol".into());
    }
    let mut listed = BTreeSet::new();
    let mut casefold = BTreeSet::new();
    for entry in &manifest.files {
        let relative = safe_relative(&entry.path)?;
        if entry.path.starts_with("licenses/") && !safe_license_path(&entry.path) {
            return Err("binary, model, or user-state pollution under licenses".into());
        }
        let allowed = [
            "ai-runtime.exe",
            "ai-runtime-worker.exe",
            "config.example.toml",
            "README.md",
            "THIRD_PARTY_NOTICES.md",
        ]
        .contains(&entry.path.as_str())
            || entry.path.starts_with("licenses/")
            || (!entry.path.contains('/') && valid_dll_name(&entry.path.to_ascii_lowercase()));
        if !allowed {
            return Err("unexpected product payload".into());
        }
        if ["manifest.json", "SHA256SUMS"].contains(&entry.path.as_str())
            || !listed.insert(entry.path.clone())
            || !casefold.insert(entry.path.to_lowercase())
            || !valid_hex(&entry.sha256, 64)
        {
            return Err("invalid or duplicate inventory entry".into());
        }
        let path = root.join(relative);
        if !path.canonicalize()?.starts_with(&root)
            || ordinary_metadata(&path)?.len() != entry.size_bytes
            || file_hash(&path)? != entry.sha256
        {
            return Err("package file integrity mismatch".into());
        }
    }
    for required in [
        "ai-runtime.exe",
        "ai-runtime-worker.exe",
        "config.example.toml",
        "README.md",
        "THIRD_PARTY_NOTICES.md",
    ] {
        if !listed.contains(required) {
            return Err("required package file missing".into());
        }
    }
    if !listed.iter().any(|p| p.starts_with("licenses/")) {
        return Err("package licenses missing".into());
    }
    listed.insert("manifest.json".into());
    listed.insert("SHA256SUMS".into());
    let mut actual = BTreeSet::new();
    inventory(&root, "", &mut actual)?;
    if listed != actual {
        return Err("package contains missing or unlisted files".into());
    }
    let sums = String::from_utf8(bounded_read(&root.join("SHA256SUMS"), 1024 * 1024)?)?;
    let mut checked = BTreeSet::new();
    for line in sums.lines() {
        let (hash, path) = line.split_once("  ").ok_or("invalid checksum line")?;
        safe_relative(path)?;
        if path == "SHA256SUMS"
            || !listed.contains(path)
            || !valid_hex(hash, 64)
            || !checked.insert(path.to_string())
            || file_hash(&root.join(path))? != hash
        {
            return Err("checksum inventory mismatch".into());
        }
    }
    listed.remove("SHA256SUMS");
    if checked != listed {
        return Err("checksum inventory incomplete".into());
    }
    verify_pe_closure(&root, &manifest)?;
    Ok(Package {
        root,
        manifest,
        manifest_hash: format!("{:x}", Sha256::digest(manifest_bytes)),
    })
}

const OS_DLLS: &[&str] = &[
    "advapi32.dll",
    "bcrypt.dll",
    "bcryptprimitives.dll",
    "combase.dll",
    "crypt32.dll",
    "dbghelp.dll",
    "gdi32.dll",
    "iphlpapi.dll",
    "kernel32.dll",
    "kernelbase.dll",
    "msvcrt.dll",
    "netapi32.dll",
    "normaliz.dll",
    "ntdll.dll",
    "ole32.dll",
    "oleaut32.dll",
    "powrprof.dll",
    "psapi.dll",
    "rpcrt4.dll",
    "secur32.dll",
    "setupapi.dll",
    "shell32.dll",
    "shlwapi.dll",
    "ucrtbase.dll",
    "user32.dll",
    "userenv.dll",
    "version.dll",
    "winhttp.dll",
    "winmm.dll",
    "ws2_32.dll",
];
fn os_dll(name: &str) -> bool {
    OS_DLLS.contains(&name)
        || ["api-ms-win-", "ext-ms-win-"].iter().any(|prefix| {
            name.strip_prefix(prefix)
                .and_then(|s| s.strip_suffix(".dll"))
                .is_some_and(|s| {
                    !s.is_empty()
                        && s.bytes()
                            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
                })
        })
}
fn debug_crt(name: &str) -> bool {
    name == "ucrtbased.dll"
        || ["vcruntime", "msvcp", "msvcr", "concrt", "vcomp"]
            .iter()
            .any(|prefix| {
                name.strip_prefix(prefix)
                    .and_then(|s| s.strip_suffix("d.dll"))
                    .is_some_and(|s| {
                        !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit() || c == b'_')
                    })
            })
}
fn verify_pe_closure(root: &Path, manifest: &Manifest) -> Result<()> {
    let entries: BTreeMap<_, _> = manifest
        .files
        .iter()
        .map(|e| (e.path.to_ascii_lowercase(), e))
        .collect();
    let mut pe_files = BTreeSet::new();
    for entry in &manifest.files {
        let lower = entry.path.to_ascii_lowercase();
        if !lower.ends_with(".exe") && !lower.ends_with(".dll") {
            continue;
        }
        // Loadable dependencies belong beside the CLI and worker, never in the verifier.
        if entry.path.contains('/') || debug_crt(&lower) || os_dll(&lower) {
            return Err("invalid app-local PE layout or Debug CRT".into());
        }
        pe_files.insert(entry.path.clone());
        let bytes = bounded_read(&root.join(&entry.path), 256 * 1024 * 1024)?;
        let actual = pe_imports(&bytes)?;
        let declared = manifest
            .dependencies
            .get(&entry.path)
            .ok_or("PE dependency record missing")?;
        let mut expected = BTreeSet::new();
        for dependency in &declared.imports {
            let name = dependency.name.to_ascii_lowercase();
            if !valid_dll_name(&name) || debug_crt(&name) || !expected.insert(name.clone()) {
                return Err("invalid or duplicate PE dependency".into());
            }
            if os_dll(&name) {
                if dependency.kind != "os" {
                    return Err("OS dependency classification mismatch".into());
                }
            } else if dependency.kind != "app-local" || !entries.contains_key(&name) {
                return Err(
                    "missing app-local dependency; installed runtimes cannot repair the package"
                        .into(),
                );
            }
        }
        if actual != expected {
            return Err("PE imports differ from dependency manifest".into());
        }
    }
    if pe_files != manifest.dependencies.keys().cloned().collect() {
        return Err("PE dependency manifest contains unlisted entries".into());
    }
    Ok(())
}
fn valid_dll_name(name: &str) -> bool {
    name.len() <= 256
        && name.ends_with(".dll")
        && name.len() > 4
        && name.bytes().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b'-' | b'.')
        })
}
fn u16_at(bytes: &[u8], offset: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(
        bytes
            .get(offset..offset.checked_add(2).ok_or("PE overflow")?)
            .ok_or("truncated PE")?
            .try_into()?,
    ))
}
fn u32_at(bytes: &[u8], offset: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(offset..offset.checked_add(4).ok_or("PE overflow")?)
            .ok_or("truncated PE")?
            .try_into()?,
    ))
}
fn u64_at(bytes: &[u8], offset: usize) -> Result<u64> {
    Ok(u64::from_le_bytes(
        bytes
            .get(offset..offset.checked_add(8).ok_or("PE overflow")?)
            .ok_or("truncated PE")?
            .try_into()?,
    ))
}
/// Independent bounded PE32+ reader for ordinary and delay-load imports.
/// Build-time dumpbin and runtime acceptance compare the same closure contract.
fn pe_imports(bytes: &[u8]) -> Result<BTreeSet<String>> {
    if bytes.get(..2) != Some(b"MZ") {
        return Err("PE DOS signature missing".into());
    }
    let pe = u32_at(bytes, 0x3c)? as usize;
    if bytes.get(pe..pe.checked_add(4).ok_or("PE overflow")?) != Some(b"PE\0\0")
        || u16_at(bytes, pe + 4)? != 0x8664
    {
        return Err("product PE must be AMD64".into());
    }
    let sections = u16_at(bytes, pe + 6)? as usize;
    let optional_size = u16_at(bytes, pe + 20)? as usize;
    let optional = pe.checked_add(24).ok_or("PE overflow")?;
    if sections == 0 || sections > 96 || optional_size < 112 || u16_at(bytes, optional)? != 0x20b {
        return Err("invalid PE32+ header".into());
    }
    let section_table = optional.checked_add(optional_size).ok_or("PE overflow")?;
    let headers = u32_at(bytes, optional + 60)?;
    let image_base = u64_at(bytes, optional + 24)?;
    let directories = u32_at(bytes, optional + 108)? as usize;
    if directories > 16 || optional_size < 112 + directories * 8 {
        return Err("invalid PE directories".into());
    }
    let rva_offset = |rva: u32, length: usize| -> Result<usize> {
        let end = (rva as u64)
            .checked_add(length as u64)
            .ok_or("PE overflow")?;
        if end <= headers as u64 && end <= bytes.len() as u64 {
            return Ok(rva as usize);
        }
        for index in 0..sections {
            let section = section_table + index * 40;
            let start = u32_at(bytes, section + 12)?;
            let size = u32_at(bytes, section + 16)?;
            let raw = u32_at(bytes, section + 20)?;
            if rva >= start && end <= start as u64 + size as u64 {
                let offset = raw as u64 + (rva - start) as u64;
                if offset + length as u64 <= bytes.len() as u64 {
                    return Ok(offset as usize);
                }
            }
        }
        Err("PE RVA outside file-backed sections".into())
    };
    let mut result = BTreeSet::new();
    for (directory, descriptor_size) in [(1usize, 20usize), (13, 32)] {
        if directories <= directory {
            continue;
        }
        let address = u32_at(bytes, optional + 112 + directory * 8)?;
        let size = u32_at(bytes, optional + 116 + directory * 8)? as usize;
        if address == 0 && size == 0 {
            continue;
        }
        if address == 0 || size < descriptor_size || size > 1024 * 1024 {
            return Err("invalid import directory".into());
        }
        let mut terminated = false;
        for index in 0..size / descriptor_size {
            let rva = address
                .checked_add((index * descriptor_size) as u32)
                .ok_or("PE overflow")?;
            let offset = rva_offset(rva, descriptor_size)?;
            if bytes[offset..offset + descriptor_size]
                .iter()
                .all(|b| *b == 0)
            {
                terminated = true;
                break;
            }
            let name_rva = if directory == 1 {
                u32_at(bytes, offset + 12)?
            } else {
                let attributes = u32_at(bytes, offset)?;
                let name = u32_at(bytes, offset + 4)?;
                match attributes {
                    1 => name,
                    0 => u32::try_from(
                        (name as u64)
                            .checked_sub(image_base)
                            .ok_or("invalid delay import VA")?,
                    )?,
                    _ => return Err("invalid delay import attributes".into()),
                }
            };
            let mut name = Vec::new();
            for i in 0..=256u32 {
                let character =
                    bytes[rva_offset(name_rva.checked_add(i).ok_or("PE overflow")?, 1)?];
                if character == 0 {
                    break;
                }
                name.push(character);
            }
            let name = std::str::from_utf8(&name)?.to_ascii_lowercase();
            if !valid_dll_name(&name) || debug_crt(&name) {
                return Err("unsafe DLL name or Debug CRT".into());
            }
            result.insert(name);
        }
        if !terminated {
            return Err("unterminated PE imports".into());
        }
    }
    Ok(result)
}

fn initial_report(options: &Options) -> Report {
    let tool_hash = std::env::current_exe()
        .ok()
        .and_then(|p| file_hash(&p).ok());
    Report {
        schema_version: 1,
        kind: "nexa-t05-portable-package-acceptance",
        last_stage: "preflight",
        verifier: json!({"version":env!("CARGO_PKG_VERSION"),"sha256":tool_hash,"native_inference_linkage":false,"integrity_is_publisher_authentication":false}),
        product: Value::Null,
        environment: environment(&options.machine_role),
        path_coverage: Value::Null,
        model_sha256: None,
        disconnect_cycles_requested: options.disconnect_cycles,
        inference: json!({"backend":"cpu","context_size":2048,"threads":2,"batch_size":128,"gpu_layers":0}),
        checks: Vec::new(),
        http_cli: None,
        short_package_checks_passed: false,
        a20_target_evidence: "unverified: short runtime checks do not establish no-development-tools or offline target conditions",
    }
}
fn visible_on_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|p| p.join(name).is_file()))
}
fn environment(role: &str) -> Value {
    json!({
        "os":std::env::consts::OS,"architecture":std::env::consts::ARCH,
        "os_version":os_version(),
        "logical_processors":std::thread::available_parallelism().ok().map(usize::from),
        "machine_role_user_declared":role,
        "ci_environment_variable_present":std::env::var_os("CI").is_some(),
        "observations_scope":"PATH visibility only; absence is not proof of uninstalled development tools",
        "tools_visible_on_path":{
            "cargo":visible_on_path(if cfg!(windows) {"cargo.exe"}else{"cargo"}),
            "rustc":visible_on_path(if cfg!(windows) {"rustc.exe"}else{"rustc"}),
            "python":visible_on_path(if cfg!(windows) {"python.exe"}else{"python3"}),
            "msvc_cl":visible_on_path("cl.exe"),"msbuild":visible_on_path("MSBuild.exe")
        },
        "vs_environment_present":std::env::var_os("VSINSTALLDIR").is_some() || std::env::var_os("VCToolsInstallDir").is_some(),
        "development_tools_installed":"unknown","vc_runtime_preinstalled":"unknown",
        "offline":"unknown; no external networking is used or probed by this verifier",
        "target_cpu_model":"unavailable","target_ram_bytes":"unavailable"
    })
}
#[cfg(windows)]
fn os_version() -> Value {
    // RtlGetVersion is not affected by Win32 application-compatibility version lies.
    #[repr(C)]
    struct Version {
        size: u32,
        major: u32,
        minor: u32,
        build: u32,
        platform: u32,
        service_pack: [u16; 128],
        service_pack_major: u16,
        service_pack_minor: u16,
        suite_mask: u16,
        product_type: u8,
        reserved: u8,
    }
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn RtlGetVersion(version: *mut Version) -> i32;
    }
    let mut version = Version {
        size: std::mem::size_of::<Version>() as u32,
        major: 0,
        minor: 0,
        build: 0,
        platform: 0,
        service_pack: [0; 128],
        service_pack_major: 0,
        service_pack_minor: 0,
        suite_mask: 0,
        product_type: 0,
        reserved: 0,
    };
    if unsafe { RtlGetVersion(&mut version) } == 0 {
        json!({"source":"RtlGetVersion","major":version.major,"minor":version.minor,"build":version.build,"product_type":version.product_type,
        "windows_family":match (version.product_type, version.major, version.build) {
            (1,10,22000..) => "windows-11-workstation",
            (1,10,_) => "windows-10-workstation",
            (2 | 3,_,_) => "windows-server",
            _ => "other-or-unknown",
        }})
    } else {
        json!("unavailable")
    }
}
#[cfg(not(windows))]
fn os_version() -> Value {
    json!("unavailable on this development host")
}

fn valid_online_import(value: &Value) -> bool {
    value["id"] == MODEL_ID
        && value["sha256"] == MODEL_HASH
        && value["model"]["id"] == MODEL_ID
        && value["model"]["sha256"] == MODEL_HASH
        && value["model"]["validated"] == true
        && value["size_bytes"].as_u64().is_some_and(|n| n > 0)
}
async fn cli(binary: &Path, data: &Path, args: &[&str]) -> Result<Value> {
    api_smoke::cli_json(
        binary,
        data,
        &args.iter().map(OsString::from).collect::<Vec<_>>(),
    )
    .await
}
async fn await_started(child: &mut Child, data: &Path) -> Result<Discovery> {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if child.try_wait()?.is_some() {
            return Err("product server exited before discovery".into());
        }
        if let Ok(record) = Discovery::read(data) {
            if Some(record.pid) != child.id() {
                return Err("unexpected process published discovery".into());
            }
            // The client proves this exact new connection before releasing bearer.
            let _ = runtime_cli::client::connect_data_dir(data).await?;
            return Ok(record);
        }
        if Instant::now() >= deadline {
            return Err("server startup deadline expired".into());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
fn path_shape(path: &Path) -> Value {
    let text = path.to_string_lossy();
    json!({"contains_non_ascii":!text.is_ascii(),"contains_space":text.contains(' ')})
}
async fn exercise_product(
    binary: &Path,
    model: &Path,
    report: &mut Report,
    disconnect_cycles: u32,
) -> Result<()> {
    let temp = tempfile::Builder::new()
        .prefix("Nexa 验收 空格 ")
        .tempdir()?;
    let data = temp.path().join("私有 数据");
    report.path_coverage = json!({
        "observations":"booleans only; actual paths are withheld",
        "product_executable":path_shape(binary),"source_model":path_shape(model),
        "owned_working_directory":path_shape(temp.path()),"owned_data_and_imported_model_directory":path_shape(&data),
        "original_model_modified":false,"original_model_hardlinked":false
    });
    let mut server: Option<Child> = None;
    let mut instance = None;
    let work = async {
        report.last_stage = "cli_init";
        let initialized = cli(binary, &data, &["init"]).await?;
        report.check("actual_cli_init", initialized["initialized"] == true, "product init creates a temporary private token and configuration");
        let token = bounded_read(&data.join("secrets/api-token"), 4096)?;
        let original_config = bounded_read(&data.join("config.toml"), 65536)?;
        report.last_stage = "cli_repeated_init";
        let repeated = cli(binary, &data, &["init"]).await?;
        let unchanged = repeated["initialized"] == true && bounded_read(&data.join("secrets/api-token"), 4096)? == token
            && bounded_read(&data.join("config.toml"), 65536)? == original_config;
        report.check("actual_cli_repeated_init", unchanged, "repeated init preserves token and configuration byte-for-byte; neither is reported");
        if !unchanged { return Err("repeat init changed private state".into()); }
        // Fixed probe-only settings; never alter product defaults or user data.
        fs::write(data.join("config.toml"), "schema_version = 1\n[api]\nlisten = '127.0.0.1:0'\n[inference]\nbackend = 'cpu'\ncontext_size = 2048\nthreads = 2\nbatch_size = 128\ngpu_layers = 0\n")?;
        report.last_stage = "cli_version";
        let version = cli(binary, &data, &["version", "--json"]).await?;
        let identity = version["name"] == "Nexa" && version["version"] == env!("CARGO_PKG_VERSION")
            && version["protocol_version"] == 1 && version["management_native_linkage"] == false
            && version["target_os"] == std::env::consts::OS && version["target_arch"] == std::env::consts::ARCH;
        report.check("actual_cli_identity", identity, "executed CLI reports the expected protocol, architecture, OS, version, and no native management linkage");
        if !identity { return Err("CLI identity mismatch".into()); }
        report.last_stage = "cli_serve";
        server = Some(api_smoke::product_command(binary).arg("--data-dir").arg(&data).arg("serve")
            .current_dir(temp.path()).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).kill_on_drop(true).spawn()?);
        let record = await_started(server.as_mut().unwrap(), &data).await?;
        instance = Some(record.instance_id);
        report.check("actual_cli_serve", true, "owned CLI child starts on a random loopback port and passes same-connection HMAC proof");
        report.last_stage = "cli_model_import";
        let imported = api_smoke::cli_json(binary, &data, &[
            "models".into(), "import".into(), "--id".into(), MODEL_ID.into(), "--file".into(), model.as_os_str().to_owned(),
        ]).await?;
        let imported_ok = valid_online_import(&imported);
        report.check("actual_cli_model_import", imported_ok, "separate fixed-hash GGUF is imported through the running product CLI and validated registry");
        if !imported_ok { return Err("imported model identity mismatch".into()); }
        report.last_stage = "cli_model_list";
        let listed = cli(binary, &data, &["models", "list"]).await?;
        let list_ok = listed["data"].as_array().is_some_and(|models| models.iter().any(|m| m["id"] == MODEL_ID && m["sha256"] == MODEL_HASH));
        report.check("actual_cli_model_list", list_ok, "actual CLI lists the separately imported fixed-hash model");
        report.last_stage = "cli_load";
        let loaded = cli(binary, &data, &["load", MODEL_ID, "--backend", "cpu", "--context", "2048", "--threads", "2", "--batch", "128", "--gpu-layers", "0"]).await?;
        report.check("actual_cli_load", loaded["state"] == "ready", "actual CLI loads the same-directory worker with fixed CPU acceptance parameters");
        if loaded["state"] != "ready" { return Err("CLI load did not become ready".into()); }
        let status = cli(binary, &data, &["status"]).await?;
        let configured = status["load_options"]["context_size"] == 2048
            && status["load_options"]["threads"] == 2 && status["load_options"]["batch_size"] == 128
            && status["configured_backend"] == "cpu";
        report.check("actual_fixed_load_options", configured, "product status confirms the exact CPU, context, thread and batch settings without parameter fallback");
        let running = configured && status["worker"]["pid"].as_u64().is_some_and(|pid| pid > 0)
            && status["worker"]["sessions_started"].as_u64().is_some_and(|n| n > 0);
        report.check("actual_worker_session", running, "product status reports an actual worker process and a started session");
        if !running { return Err("no actual worker session".into()); }
        let unloaded = cli(binary, &data, &["unload"]).await?;
        report.check("actual_cli_unload", unloaded["state"] == "unloaded" && unloaded["active_request"].is_null() && unloaded["queued_jobs"] == 0, "actual CLI unload completes before the shared independent HTTP oracle reloads");
        report.last_stage = "shared_http_oracle";
        let http = api_smoke::run(api_smoke::Options {
            address: record.listen, root: data.clone(), model: MODEL_ID.into(), out: PathBuf::new(),
            disconnect_cycles, cli: Some(binary.to_path_buf()), release_acceptance: true,
        }).await;
        let passed = http.passed();
        report.http_cli = Some(http);
        report.check("shared_http_cli_oracle", passed, "same independent T04 oracle validates genuine completion/SSE/usage, queuing, cancellation, disconnect recovery, and actual CLI stop");
        if !passed { return Err("independent HTTP oracle failed".into()); }
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    }.await;
    // Every failure follows the same authenticated cleanup attempt. Never kill by
    // a discovery/API PID: only the child handle created above may be terminated.
    let mut clean = true;
    if let Some(child) = server.as_mut() {
        if !matches!(child.try_wait(), Ok(Some(_))) {
            let stopped = cli(binary, &data, &["stop"]).await;
            clean &= stopped.is_ok();
        }
        match tokio::time::timeout(Duration::from_secs(40), child.wait()).await {
            Ok(Ok(status)) => clean &= status.success(),
            _ => {
                clean = false;
                let _ = child.kill().await;
                let _ = child.wait().await;
            }
        }
        if let Some(id) = instance {
            clean &= wait_stopped(&data, id, Duration::from_secs(5))
                .await
                .is_ok();
        }
        // Strictly require this owned directory's marker absent and lock free.
        clean &= !data.join("runtime/instance.json").exists()
            && InstanceLock::try_acquire(&data).is_ok_and(|lock| lock.is_some());
        report.check("owned_server_reaped", clean, "owned server exited successfully, its marker disappeared and instance lock released; no arbitrary PID was signaled");
    }
    if clean {
        report.check("owned_temporary_data_removed", temp.close().is_ok(), "only verifier-owned temporary state is removed; the source model and package remain unchanged");
    } else {
        let _ = temp.keep();
        report.check("owned_temporary_data_removed", false, "cleanup unconfirmed; temporary private state retained without publishing its path or token");
    }
    work?;
    if !clean {
        return Err("owned service cleanup unconfirmed".into());
    }
    Ok(())
}
async fn run(options: &Options, report: &mut Report) -> Result<()> {
    report.last_stage = "product_inventory_identity_pe_closure";
    let package = verify_package(&options.package)?;
    report.product = json!({"project_commit":package.manifest.project_commit,"package_version":package.manifest.package_version,
        "manifest_sha256":package.manifest_hash,"llama_commit":package.manifest.llama_commit,"configuration":"Release","architecture":"x86_64","backend":"cpu","file_count":package.manifest.files.len()});
    report.check("product_inventory_pe_integrity", true, "manifest, checksums, AMD64 PE imports including delay-load dependencies, Release identity and closed app-local dependency inventory verified");
    let executable = std::env::current_exe()?.canonicalize()?;
    if executable.starts_with(&package.root) {
        return Err("verifier must remain outside the product package".into());
    }
    let out = if options.out.is_absolute() {
        options.out.clone()
    } else {
        std::env::current_dir()?.join(&options.out)
    };
    if out.starts_with(&package.root) {
        return Err("report must be outside the product package".into());
    }
    if !cfg!(all(windows, target_arch = "x86_64")) {
        return Err("Windows x64 is required for target package execution".into());
    }
    report.last_stage = "fixed_model_hash";
    let model = local_model_path(&options.model)?;
    let model_hash = file_hash(&model)?;
    let valid_model = model_hash == MODEL_HASH;
    report.model_sha256 = Some(model_hash);
    report.check(
        "fixed_model_sha256",
        valid_model,
        "independent source GGUF SHA256 must equal the frozen Qwen3-0.6B Q8_0 baseline",
    );
    if !valid_model {
        return Err("wrong model baseline".into());
    }
    if out.canonicalize().is_ok_and(|p| p == model) {
        return Err("report cannot overwrite the source model".into());
    }
    exercise_product(
        &package.root.join("ai-runtime.exe"),
        &model,
        report,
        options.disconnect_cycles,
    )
    .await?;
    report.last_stage = "product_unchanged_after_acceptance";
    report.check("product_unchanged_after_acceptance", verify_package(&package.root).is_ok(), "the original extracted product still verifies after real execution; no DLL or other repair was supplied by the verifier");
    Ok(())
}
fn local_model_path(path: &Path) -> Result<PathBuf> {
    let text = path.to_string_lossy();
    if text.starts_with("\\\\") || text.starts_with("//") || text.contains("://") {
        return Err("model must be an ordinary local path".into());
    }
    let canonical = path.canonicalize()?;
    #[cfg(windows)]
    if let Some(normal) = canonical
        .to_str()
        .and_then(|p| p.strip_prefix(r"\\?\"))
        .filter(|p| p.as_bytes().get(1) == Some(&b':') && p.as_bytes().get(2) == Some(&b'\\'))
    {
        return Ok(PathBuf::from(normal));
    }
    Ok(canonical)
}
/// Resolve the destination's existing ancestor so '..' and directory symlinks
/// cannot put a failure report inside the very package being checked.
fn destination_path(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            value => normalized.push(value.as_os_str()),
        }
    }
    let mut ancestor = normalized.as_path();
    let mut tail = Vec::new();
    while !ancestor.exists() {
        tail.push(
            ancestor
                .file_name()
                .ok_or("invalid report destination")?
                .to_owned(),
        );
        ancestor = ancestor.parent().ok_or("invalid report destination")?;
    }
    let mut resolved = ancestor.canonicalize()?;
    for part in tail.into_iter().rev() {
        resolved.push(part);
    }
    Ok(resolved)
}
fn validate_output(options: &Options) -> Result<()> {
    let out = destination_path(&options.out)?;
    let package = destination_path(&options.package)?;
    let verifier = std::env::current_exe()?.canonicalize()?;
    let verifier_directory = verifier
        .parent()
        .ok_or("cannot resolve verifier directory")?;
    if out.starts_with(package)
        || out.starts_with(verifier_directory)
        || options.model.canonicalize().is_ok_and(|model| model == out)
    {
        return Err("report must not overwrite package, verifier bundle, or source model".into());
    }
    if fs::symlink_metadata(&options.out).is_ok() {
        let prior: Value = serde_json::from_slice(&bounded_read(&options.out, 8 * 1024 * 1024)?)?;
        if prior["schema_version"] != 1 || prior["kind"] != "nexa-t05-portable-package-acceptance" {
            return Err("existing destination is not a recognized acceptance report".into());
        }
    }
    Ok(())
}

fn write_report(out: &Path, report: &Report) -> Result<()> {
    let parent = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(&serde_json::to_vec_pretty(report)?)?;
    temporary.flush()?;
    // A completed report replaces only the exact user-requested report path.
    temporary.persist(out).map_err(|e| e.error)?;
    Ok(())
}
pub fn main(args: &[OsString]) -> ExitCode {
    if args.is_empty() || args == ["--help"] || args == ["-h"] {
        println!("{HELP}");
        return ExitCode::SUCCESS;
    }
    let options = match parse(args) {
        Ok(v) => v,
        Err(_) => {
            eprintln!("nexa-acceptance: invalid arguments; use --help");
            return ExitCode::from(2);
        }
    };
    if validate_output(&options).is_err() {
        eprintln!(
            "nexa-acceptance: report must be outside product/model/tool files and may only replace a recognized prior report"
        );
        return ExitCode::from(2);
    }
    let mut report = initial_report(&options);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build();
    let result = match runtime {
        Ok(rt) => rt.block_on(run(&options, &mut report)),
        Err(e) => Err(e.into()),
    };
    if result.is_err() {
        report.check(
            "package_acceptance_execution",
            false,
            "acceptance failed; raw errors, paths, token, request and generated text are withheld",
        );
    }
    report.skip("A20_no_development_tools_offline_target", "requires actual independent target-machine evidence or explicit user declarations; OS name, CI pass and PATH absence alone do not establish clean-machine/offline acceptance");
    report.skip("hardware_long_term_phase_boundaries", "short package smoke does not establish i5-8400 performance, long-term memory, exact native phase cancellation, or full A01-A26 acceptance; retain separate T04 evidence");
    report.short_package_checks_passed = result.is_ok()
        && report.checks.iter().all(|c| c.status != "fail")
        && report
            .http_cli
            .as_ref()
            .is_some_and(api_smoke::Report::passed);
    if write_report(&options.out, &report).is_err() {
        eprintln!("nexa-acceptance: cannot write sanitized report");
        return ExitCode::FAILURE;
    }
    println!(
        "nexa-acceptance: {}; A20 target environment remains unverified; sanitized report saved locally",
        if report.short_package_checks_passed {
            "short package checks passed"
        } else {
            "failed"
        }
    );
    if report.short_package_checks_passed {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn set16(bytes: &mut [u8], at: usize, value: u16) {
        bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }
    fn set32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn sample_pe(import: Option<&str>, delay: bool) -> Vec<u8> {
        let mut bytes = vec![0u8; 2048];
        bytes[..2].copy_from_slice(b"MZ");
        set32(&mut bytes, 0x3c, 128);
        bytes[128..132].copy_from_slice(b"PE\0\0");
        set16(&mut bytes, 132, 0x8664);
        set16(&mut bytes, 134, 1);
        set16(&mut bytes, 148, 240);
        set16(&mut bytes, 152, 0x20b);
        set32(&mut bytes, 212, 512);
        set32(&mut bytes, 260, 16);
        set32(&mut bytes, 404, 4096);
        set32(&mut bytes, 408, 1536);
        set32(&mut bytes, 412, 512);
        if let Some(name) = import {
            let directory = if delay { 13 } else { 1 };
            set32(&mut bytes, 264 + directory * 8, 4096);
            set32(&mut bytes, 268 + directory * 8, if delay { 64 } else { 40 });
            if delay {
                set32(&mut bytes, 512, 1);
                set32(&mut bytes, 516, 4352);
            } else {
                set32(&mut bytes, 524, 4352);
            }
            bytes[768..768 + name.len()].copy_from_slice(name.as_bytes());
        }
        bytes
    }
    fn fixture() -> tempfile::TempDir {
        let temp = tempfile::Builder::new()
            .prefix("测试 包 空格")
            .tempdir()
            .unwrap();
        fs::create_dir(temp.path().join("licenses")).unwrap();
        for (name, bytes) in [
            ("ai-runtime.exe", sample_pe(Some("KERNEL32.dll"), false)),
            (
                "ai-runtime-worker.exe",
                sample_pe(Some("KERNEL32.dll"), true),
            ),
            ("config.example.toml", b"schema_version=1".to_vec()),
            ("README.md", b"usage".to_vec()),
            ("THIRD_PARTY_NOTICES.md", b"notices".to_vec()),
            ("licenses/LICENSE", b"license".to_vec()),
        ] {
            fs::write(temp.path().join(name), bytes).unwrap();
        }
        let mut files = BTreeSet::new();
        inventory(temp.path(), "", &mut files).unwrap();
        let entries: Vec<_> = files.iter().map(|p|json!({"path":p,"size_bytes":fs::metadata(temp.path().join(p)).unwrap().len(),"sha256":file_hash(&temp.path().join(p)).unwrap()})).collect();
        let manifest = json!({"schema_version":1,"product":"nexa-runtime","package_version":env!("CARGO_PKG_VERSION"),"project_commit":"a".repeat(40),"llama_commit":LLAMA_COMMIT,"protocol_version":1,"worker_protocol_version":1,"shim_version":2,"platform":"windows-x64","target":"x86_64-pc-windows-msvc","architecture":"x86_64","backend":"cpu","configuration":"Release","files":entries,"dependencies":{"ai-runtime.exe":{"imports":[{"name":"KERNEL32.dll","kind":"os"}]},"ai-runtime-worker.exe":{"imports":[{"name":"KERNEL32.dll","kind":"os"}]}}});
        fs::write(
            temp.path().join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        rewrite_sums(temp.path());
        temp
    }
    fn rewrite_sums(root: &Path) {
        let mut files = BTreeSet::new();
        inventory(root, "", &mut files).unwrap();
        files.remove("SHA256SUMS");
        let sums = files
            .iter()
            .map(|p| format!("{}  {p}\n", file_hash(&root.join(p)).unwrap()))
            .collect::<String>();
        fs::write(root.join("SHA256SUMS"), sums).unwrap();
    }
    #[test]
    fn traversal_windows_ads_devices_and_ambiguous_names_are_rejected() {
        for path in [
            "../outside",
            "/abs",
            "C:/file",
            "a\\b",
            "a//b",
            "./x",
            "a/../b",
            "x:stream",
            "a.",
            "a ",
            "CON",
            "dir/NUL.txt",
            "COM1",
            "x\ny",
        ] {
            assert!(safe_relative(path).is_err(), "{path}");
        }
        assert!(safe_relative("licenses/中文 空格.txt").is_ok());
    }
    #[test]
    fn package_disconnect_count_is_explicit_and_bounded() {
        let base = [
            "--package",
            "product",
            "--model",
            "model",
            "--out",
            "report",
        ]
        .map(OsString::from)
        .to_vec();
        assert_eq!(parse(&base).unwrap().disconnect_cycles, 5);
        for (value, valid) in [
            ("1", true),
            ("50", true),
            ("0", false),
            ("51", false),
            ("-1", false),
            ("oops", false),
        ] {
            let mut args = base.clone();
            args.extend([OsString::from("--disconnect-cycles"), OsString::from(value)]);
            assert_eq!(parse(&args).is_ok(), valid);
        }
    }
    #[test]
    fn online_import_requires_the_nested_verified_model_summary() {
        let value = json!({"id":MODEL_ID,"sha256":MODEL_HASH,"size_bytes":1,"model":{"id":MODEL_ID,"sha256":MODEL_HASH,"validated":true}});
        assert!(valid_online_import(&value));
        assert!(!valid_online_import(
            &json!({"id":MODEL_ID,"sha256":MODEL_HASH,"validated":true})
        ));
        let mut wrong = value;
        wrong["model"]["sha256"] = json!("0".repeat(64));
        assert!(!valid_online_import(&wrong));
    }
    #[test]
    fn licenses_cannot_smuggle_models_binaries_or_user_state() {
        for path in [
            "licenses/model.gguf",
            "licenses/fault-worker.EXE",
            "licenses/secrets/api-token",
            "licenses/debug.pdb",
            "licenses/run.ps1",
        ] {
            assert!(!safe_license_path(path));
        }
        assert!(safe_license_path(
            "licenses/rust-crates/tokio-1.53.1/LICENSE"
        ));
    }
    #[test]
    fn pe_machine_headers_imports_and_delay_loads_are_independently_checked() {
        for delay in [false, true] {
            assert_eq!(
                pe_imports(&sample_pe(Some("VCRUNTIME140.dll"), delay)).unwrap(),
                BTreeSet::from(["vcruntime140.dll".into()])
            );
            assert!(pe_imports(&sample_pe(Some("VCRUNTIME140D.dll"), delay)).is_err());
            assert!(pe_imports(&sample_pe(Some("../escape.dll"), delay)).is_err());
        }
        let mut pe = sample_pe(None, false);
        set16(&mut pe, 132, 0xaa64);
        assert!(pe_imports(&pe).is_err());
        for length in [0, 1, 64, 128, 256, 450] {
            assert!(pe_imports(&sample_pe(Some("kernel32.dll"), false)[..length]).is_err());
        }
    }
    #[test]
    fn release_inventory_checksums_and_pe_closure_accept_unicode_root() {
        let package = fixture();
        assert!(verify_package(package.path()).is_ok());
        fs::write(package.path().join("ai-runtime.exe"), b"tampered").unwrap();
        assert!(verify_package(package.path()).is_err());
    }
    #[test]
    fn extra_missing_and_dependency_mismatch_files_are_failures() {
        let package = fixture();
        fs::write(package.path().join("extra.dll"), b"unlisted").unwrap();
        assert!(verify_package(package.path()).is_err());
        let package = fixture();
        fs::remove_file(package.path().join("ai-runtime-worker.exe")).unwrap();
        assert!(verify_package(package.path()).is_err());
        let package = fixture();
        let mut manifest: Value =
            serde_json::from_slice(&fs::read(package.path().join("manifest.json")).unwrap())
                .unwrap();
        manifest["dependencies"]["ai-runtime.exe"]["imports"][0]["name"] =
            json!("vcruntime140.dll");
        manifest["dependencies"]["ai-runtime.exe"]["imports"][0]["kind"] = json!("app-local");
        fs::write(
            package.path().join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        rewrite_sums(package.path());
        assert!(verify_package(package.path()).is_err());
    }
    #[test]
    fn debug_configuration_protocol_mismatch_and_escape_cannot_be_rehashed_into_valid_package() {
        for (field, value) in [
            ("configuration", json!("Debug")),
            ("worker_protocol_version", json!(2)),
            ("architecture", json!("arm64")),
        ] {
            let package = fixture();
            let mut manifest: Value =
                serde_json::from_slice(&fs::read(package.path().join("manifest.json")).unwrap())
                    .unwrap();
            manifest[field] = value;
            fs::write(
                package.path().join("manifest.json"),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
            rewrite_sums(package.path());
            assert!(verify_package(package.path()).is_err());
        }
    }
    #[cfg(unix)]
    #[test]
    fn symlink_inside_package_is_rejected_even_if_target_hash_matches() {
        let package = fixture();
        let outside = tempfile::NamedTempFile::new().unwrap();
        fs::write(outside.path(), b"usage").unwrap();
        fs::remove_file(package.path().join("README.md")).unwrap();
        std::os::unix::fs::symlink(outside.path(), package.path().join("README.md")).unwrap();
        assert!(verify_package(package.path()).is_err());
    }
    #[test]
    fn reports_never_contain_model_paths_or_claim_clean_machine() {
        let options = Options {
            package: "/private/package".into(),
            model: "/private/model.gguf".into(),
            out: "/private/report.json".into(),
            machine_role: "ci".into(),
            disconnect_cycles: 5,
        };
        let report = initial_report(&options);
        let text = serde_json::to_string(&report).unwrap();
        assert!(!text.contains("/private/"));
        assert_eq!(report.environment["development_tools_installed"], "unknown");
        assert!(!report.short_package_checks_passed);
    }
    #[test]
    fn failure_report_destination_cannot_modify_package_or_source_model() {
        let package = fixture();
        let options = Options {
            package: package.path().into(),
            model: package.path().join("README.md"),
            out: package.path().join("sub/../report.json"),
            machine_role: "ci".into(),
            disconnect_cycles: 5,
        };
        assert!(validate_output(&options).is_err());
        let options = Options {
            out: package.path().join("README.md"),
            ..options
        };
        assert!(validate_output(&options).is_err());
    }
    #[test]
    fn report_replacement_requires_a_recognized_prior_report() {
        let temp = tempfile::tempdir().unwrap();
        let options = Options {
            package: temp.path().join("product"),
            model: temp.path().join("model"),
            out: temp.path().join("report.json"),
            machine_role: "unknown".into(),
            disconnect_cycles: 5,
        };
        fs::write(&options.out, b"precious unrelated data").unwrap();
        assert!(validate_output(&options).is_err());
        fs::write(
            &options.out,
            br#"{"schema_version":1,"kind":"nexa-t05-portable-package-acceptance"}"#,
        )
        .unwrap();
        assert!(validate_output(&options).is_ok());
        let options = Options {
            out: std::env::current_exe().unwrap(),
            ..options
        };
        assert!(validate_output(&options).is_err());
    }
    #[tokio::test]
    #[ignore = "requires an explicitly supplied real product CLI and fixed-hash GGUF; development evidence only"]
    async fn real_product_lifecycle_uses_shared_oracle_and_reaps() {
        let binary = PathBuf::from(std::env::var_os("NEXA_ACCEPTANCE_CLI").expect("explicit CLI"));
        let model =
            PathBuf::from(std::env::var_os("NEXA_ACCEPTANCE_MODEL").expect("explicit model"));
        assert_eq!(file_hash(&model).unwrap(), MODEL_HASH);
        let disconnect_cycles = std::env::var("NEXA_ACCEPTANCE_DISCONNECT_CYCLES")
            .unwrap_or_else(|_| "5".into())
            .parse::<u32>()
            .expect("integer cycles");
        assert!((1..=50).contains(&disconnect_cycles));
        let options = Options {
            package: binary.parent().unwrap().into(),
            model: model.clone(),
            out: PathBuf::new(),
            machine_role: "ci".into(),
            disconnect_cycles,
        };
        let mut report = initial_report(&options);
        let result =
            exercise_product(&binary, &model, &mut report, options.disconnect_cycles).await;
        if let Some(out) = std::env::var_os("NEXA_ACCEPTANCE_REPORT") {
            write_report(Path::new(&out), &report).unwrap();
        }
        assert!(result.is_ok(), "sanitized real product lifecycle failure");
        assert!(report.checks.iter().all(|c| c.status != "fail"));
        assert!(report.http_cli.as_ref().unwrap().passed());
    }
}
