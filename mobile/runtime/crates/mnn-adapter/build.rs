use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    android_ndk_revision: Option<String>,
    android_api: Option<u32>,
    silent_logs: bool,
    schema_version: u32,
    target: String,
    abi_version: u32,
    upstream_commit: String,
    patch_set_sha256: String,
    policy_sha256: String,
    header_sha256: String,
    compiler: String,
    build_type: String,
    static_libraries: Vec<Library>,
    system_libraries: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Library {
    name: String,
    path: PathBuf,
    sha256: String,
}
fn hash(path: &Path) -> String {
    println!("cargo:rerun-if-changed={}", path.display());
    let mut file = fs::File::open(path).expect("native identity input is missing");
    let mut h = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = file.read(&mut buffer).expect("cannot read native input");
        if n == 0 {
            break;
        }
        h.update(&buffer[..n]);
    }
    format!("{:x}", h.finalize())
}
fn archive_architecture(path: &Path, machine: u16) {
    let data = fs::read(path).expect("cannot read archive");
    assert!(
        data.starts_with(b"!<arch>\n"),
        "native library must be a regular static archive"
    );
    let mut cursor = 8;
    let mut objects = 0;
    while cursor < data.len() {
        let header = data
            .get(cursor..cursor + 60)
            .expect("truncated archive header");
        assert_eq!(&header[58..], b"`\n", "invalid archive member");
        let size: usize = std::str::from_utf8(&header[48..58])
            .unwrap()
            .trim()
            .parse()
            .expect("invalid archive size");
        cursor += 60;
        let member = data
            .get(cursor..cursor.checked_add(size).unwrap())
            .expect("truncated archive");
        if member.starts_with(b"\x7fELF") {
            assert!(
                member.len() >= 20 && member[4] == 2 && member[5] == 1,
                "requires ELF64 little-endian native objects"
            );
            assert_eq!(
                u16::from_le_bytes([member[18], member[19]]),
                machine,
                "native archive target architecture mismatch"
            );
            objects += 1;
        } else {
            let name = std::str::from_utf8(&header[..16]).unwrap().trim();
            assert!(
                name == "/" || name == "//" || name == "/SYM64/",
                "non-ELF native archive member (bitcode/thin archive unsupported)"
            );
        }
        cursor += size + size % 2;
    }
    assert!(objects > 0, "native archive has no ELF objects");
}
fn main() {
    println!("cargo:rerun-if-env-changed=NEXA_MNN_ARTIFACT_DIR");
    let rustc = std::process::Command::new(env::var_os("RUSTC").expect("RUSTC missing"))
        .arg("--version")
        .output()
        .expect("cannot inspect Rust toolchain");
    assert!(
        rustc.status.success() && rustc.stdout.starts_with(b"rustc 1.98.1 "),
        "mnn-adapter requires pinned Rust 1.98.1"
    );
    let target = env::var("TARGET").unwrap();
    let (machine, allowed): (u16, &[&str]) = match target.as_str() {
        "x86_64-unknown-linux-gnu" => (62, &["stdc++", "pthread", "m", "dl"]),
        "aarch64-linux-android" => (183, &["log", "android", "m", "dl"]),
        _ => panic!(
            "mnn-adapter supports only Linux x86_64 development and Android aarch64; Windows root workspace is independent"
        ),
    };
    let root = PathBuf::from(env::var("NEXA_MNN_ARTIFACT_DIR").expect("set NEXA_MNN_ARTIFACT_DIR to an explicit verified native artifact directory; no automatic download or fake backend"));
    assert!(root.is_absolute(), "NEXA_MNN_ARTIFACT_DIR must be absolute");
    let root = root
        .canonicalize()
        .expect("native artifact directory missing");
    let manifest_path = root.join("artifact.json");
    println!("cargo:rerun-if-changed={}", manifest_path.display());
    let manifest_bytes = fs::read(&manifest_path).expect("native artifact.json missing");
    let manifest: Manifest =
        serde_json::from_slice(&manifest_bytes).expect("invalid native artifact manifest");
    // Fingerprint the exact bytes actually parsed and checked. It binds all
    // manifest fields, including shim archive hashes, compiler, target and ABI.
    // Formatting changes conservatively produce a different build identity.
    let manifest_sha256 = format!("{:x}", Sha256::digest(&manifest_bytes));
    assert!(
        manifest.silent_logs,
        "native production logging must be silent"
    );
    assert_eq!(manifest.schema_version, 1);
    assert_eq!(manifest.abi_version, 1);
    assert_eq!(manifest.target, target, "native manifest target mismatch");
    assert_eq!(manifest.build_type, "Release");
    if target == "aarch64-linux-android" {
        assert_eq!(
            manifest.android_ndk_revision.as_deref(),
            Some("30.0.16248370"),
            "requires NDK r30"
        );
        assert_eq!(manifest.android_api, Some(28), "requires Android API28");
        assert!(
            manifest.compiler.contains("clang version 21.0.0"),
            "requires pinned Android Clang21"
        );
    } else {
        assert!(manifest.android_ndk_revision.is_none() && manifest.android_api.is_none());
        // Only the explicitly selected Debian development and Ubuntu 24.04 CI
        // GCC profiles. Keep the complete actual compiler identity in artifact.json;
        // the native exporter checks it against CMake's real compiler --version.
        // A permitted build profile is not evidence that its CI gates have passed.
        let debian_development = manifest.compiler == "c++ (Debian 14.2.0-19) 14.2.0";
        let ubuntu_ci = manifest
            .compiler
            .strip_prefix("c++ (Ubuntu 13.3.0-")
            .or_else(|| manifest.compiler.strip_prefix("g++-13 (Ubuntu 13.3.0-"))
            .and_then(|rest| rest.strip_suffix(") 13.3.0"))
            .is_some_and(|package_revision| {
                !package_revision.is_empty()
                    && package_revision.contains("ubuntu")
                    && package_revision.bytes().all(|b| {
                        b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+' | b'~')
                    })
            });
        assert!(
            debian_development || ubuntu_ci,
            "Linux research compiler identity mismatch"
        );
    }
    assert!(
        !manifest.compiler.trim().is_empty(),
        "compiler identity missing"
    );
    let repo = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../../..");
    let lock_path = repo.join("native/mnn-patches/lock.json");
    println!("cargo:rerun-if-changed={}", lock_path.display());
    let lock: serde_json::Value =
        serde_json::from_slice(&fs::read(lock_path).expect("patch lock missing")).unwrap();
    assert_eq!(lock["abi_version"], 1);
    for (key, value) in [
        ("upstream_commit", &manifest.upstream_commit),
        ("patch_set_sha256", &manifest.patch_set_sha256),
        ("policy_sha256", &manifest.policy_sha256),
    ] {
        assert_eq!(
            lock[key].as_str(),
            Some(value.as_str()),
            "native manifest differs from repository lock: {key}"
        );
    }
    // Handwritten Rust layouts were reviewed against exactly this frozen header.
    // A header revision requires updating/retesting the bindings, not merely
    // regenerating the artifact manifest with the new header hash.
    assert_eq!(
        manifest.header_sha256, "40d1a79df99f5200d92e689baf791d5105d2da71d521fcd885a2807f0b916b8e",
        "native ABI header differs from reviewed Rust bindings"
    );
    assert_eq!(
        hash(&repo.join("native/mnn-shim/include/nexa_mnn.h")),
        manifest.header_sha256,
        "native ABI header mismatch"
    );
    assert!(
        !manifest.static_libraries.is_empty(),
        "native link closure missing"
    );
    assert_eq!(manifest.static_libraries[0].name, "nexa-mnn-shim");
    for lib in &manifest.static_libraries {
        assert!(
            lib.name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'+'),
            "invalid library name"
        );
        assert!(
            lib.path
                .components()
                .all(|c| matches!(c, Component::Normal(_))),
            "library path must be a relative confined path"
        );
        let path = root
            .join(&lib.path)
            .canonicalize()
            .expect("native archive missing");
        assert!(
            path.starts_with(&root),
            "native archive escapes artifact directory"
        );
        assert_eq!(
            path.file_name().unwrap().to_str().unwrap(),
            format!("lib{}.a", lib.name)
        );
        assert_eq!(hash(&path), lib.sha256, "native archive digest mismatch");
        archive_architecture(&path, machine);
        println!(
            "cargo:rustc-link-search=native={}",
            path.parent().unwrap().display()
        );
        println!("cargo:rustc-link-lib=static={}", lib.name);
    }
    for lib in &manifest.system_libraries {
        assert!(
            allowed.contains(&lib.as_str()),
            "unexpected native system library"
        );
        println!("cargo:rustc-link-lib={lib}");
    }
    if target == "aarch64-linux-android" {
        // Set both: max-page-size alone leaves GNU_RELRO rounded to 4 KiB.
        println!("cargo:rustc-link-arg=-Wl,-z,max-page-size=16384");
        println!("cargo:rustc-link-arg=-Wl,-z,common-page-size=16384");
        assert!(
            manifest
                .static_libraries
                .iter()
                .any(|l| l.name == "c++_static"),
            "Android C++ runtime archive must be explicitly pinned"
        );
    }
    for (key, value) in [
        ("COMMIT", manifest.upstream_commit),
        ("PATCH", manifest.patch_set_sha256),
        ("POLICY", manifest.policy_sha256),
        ("ARTIFACT_MANIFEST_SHA256", manifest_sha256),
        ("TARGET", manifest.target),
        ("COMPILER", manifest.compiler),
    ] {
        println!("cargo:rustc-env=NEXA_MNN_{key}={value}");
    }
}
