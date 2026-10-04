//! Deliberately dependency-free so this contract can be tested without native code.
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
pub const LIBRARIES: [&str; 8] = [
    "air_llama",
    "llama-common",
    "llama-common-base",
    "cpp-httplib",
    "llama",
    "ggml",
    "ggml-cpu",
    "ggml-base",
];
const LLAMA_COMMIT: &str = "2149c00f4442dc59302e134a02e4c99d5f7ed9fc";
pub fn parse(text: &str) -> Result<BTreeMap<String, String>, String> {
    let mut fields = BTreeMap::new();
    for line in text.lines() {
        let (key, value) = line.split_once('=').ok_or("malformed native identity")?;
        if key.is_empty() || fields.insert(key.into(), value.into()).is_some() {
            return Err("duplicate/empty native identity key".into());
        }
    }
    Ok(fields)
}
pub fn validate(
    fields: &BTreeMap<String, String>,
    os: &str,
    arch: &str,
    features: &str,
) -> Result<(), String> {
    let get = |key: &str| fields.get(key).map(String::as_str).unwrap_or("");
    if features.split(',').any(|f| f == "crt-static") {
        return Err(
            "crt-static is unsupported; use Rust's default dynamic MSVC CRT and /MD".into(),
        );
    }
    if get("schema") != "1"
        || get("configuration") != "Release"
        || get("llama_commit") != LLAMA_COMMIT
    {
        return Err("native identity schema/configuration/llama commit mismatch; reconfigure the native Release build".into());
    }
    let system = match os {
        "windows" => "Windows",
        "linux" => "Linux",
        _ => return Err("unsupported native target OS".into()),
    };
    if get("system") != system {
        return Err("native target OS mismatch".into());
    }
    if arch != "x86_64"
        || get("pointer_bytes") != "8"
        || !["amd64", "x86_64"].contains(&get("processor").to_ascii_lowercase().as_str())
    {
        return Err("native architecture mismatch; only x86_64 is currently supported".into());
    }
    if os == "windows" && (get("crt") != "MD" || get("compiler_id") != "MSVC") {
        return Err("Windows native build must be Release MSVC /MD".into());
    }
    for option in [
        "GGML_NATIVE",
        "GGML_BACKEND_DL",
        "GGML_OPENMP",
        "GGML_CUDA",
        "GGML_VULKAN",
        "GGML_METAL",
        "LLAMA_OPENSSL",
        "BUILD_SHARED_LIBS",
    ] {
        if get(option) != "OFF" {
            return Err(format!("unsupported native option {option}"));
        }
    }
    Ok(())
}
fn reject_reparse_ancestors(path: &Path) -> Result<(), String> {
    for part in path.ancestors() {
        let metadata = fs::symlink_metadata(part).map_err(|e| e.to_string())?;
        if metadata.file_type().is_symlink() {
            return Err("native path contains a symlink/reparse point".into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err("native path contains a Windows reparse point".into());
            }
        }
    }
    Ok(())
}
pub fn libraries(
    root: &Path,
    fields: &BTreeMap<String, String>,
    windows: bool,
) -> Result<Vec<PathBuf>, String> {
    reject_reparse_ancestors(root)?;
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let mut result = Vec::new();
    for name in LIBRARIES {
        let path = PathBuf::from(
            fields
                .get(&format!("library.{name}"))
                .ok_or_else(|| format!("missing {name} target path"))?,
        );
        if !path.is_absolute() {
            return Err("native library target path must be absolute".into());
        }
        reject_reparse_ancestors(&path)?;
        let path = path
            .canonicalize()
            .map_err(|e| format!("native library not built: {e}"))?;
        if !path.starts_with(&root) || !path.is_file() {
            return Err("native target path escapes build directory".into());
        }
        let expected = if windows {
            format!("{name}.lib")
        } else {
            format!("lib{name}.a")
        };
        if path.file_name().and_then(|p| p.to_str()) != Some(&expected) {
            return Err("native target filename mismatch".into());
        }
        result.push(path);
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn valid() -> BTreeMap<String, String> {
        parse(&format!("schema=1\nsystem=Windows\nprocessor=AMD64\npointer_bytes=8\nconfiguration=Release\ncrt=MD\nllama_commit={LLAMA_COMMIT}\ncompiler_id=MSVC\nGGML_NATIVE=OFF\nGGML_BACKEND_DL=OFF\nGGML_OPENMP=OFF\nGGML_CUDA=OFF\nGGML_VULKAN=OFF\nGGML_METAL=OFF\nLLAMA_OPENSSL=OFF\nBUILD_SHARED_LIBS=OFF\n")).unwrap()
    }
    #[test]
    fn rejects_platform_configuration_crt_and_cpu_mismatches() {
        assert!(validate(&valid(), "windows", "x86_64", "sse2").is_ok());
        for (key, value) in [
            ("system", "Linux"),
            ("configuration", "Debug"),
            ("crt", "MDd"),
            ("processor", "ARM64"),
            ("pointer_bytes", "4"),
            ("GGML_NATIVE", "ON"),
            ("llama_commit", "bad"),
        ] {
            let mut fields = valid();
            fields.insert(key.into(), value.into());
            assert!(validate(&fields, "windows", "x86_64", "").is_err());
        }
        assert!(validate(&valid(), "windows", "x86_64", "sse2,crt-static").is_err());
    }
    #[test]
    fn rejects_duplicate_fields() {
        assert!(parse("schema=1\nschema=1").is_err());
    }
}

#[cfg(test)]
mod path_tests {
    use super::*;
    #[test]
    fn exact_paths_reject_escape_and_symlinked_root() {
        let base = std::env::temp_dir().join(format!(
            "nexa-native-identity-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let root = base.join("native");
        fs::create_dir_all(&root).unwrap();
        let mut fields = BTreeMap::new();
        for name in LIBRARIES {
            let path = root.join(format!("lib{name}.a"));
            fs::write(&path, b"test archive identity only").unwrap();
            fields.insert(
                format!("library.{name}"),
                path.to_str().unwrap().to_string(),
            );
        }
        assert_eq!(libraries(&root, &fields, false).unwrap().len(), 8);
        let outside = base.join("libair_llama.a");
        fs::write(&outside, b"not in native root").unwrap();
        fields.insert("library.air_llama".into(), outside.to_str().unwrap().into());
        assert!(
            libraries(&root, &fields, false)
                .unwrap_err()
                .contains("escapes")
        );
        #[cfg(unix)]
        {
            let alias = base.join("alias");
            std::os::unix::fs::symlink(&root, &alias).unwrap();
            assert!(
                libraries(&alias, &fields, false)
                    .unwrap_err()
                    .contains("symlink")
            );
        }
        #[cfg(windows)]
        {
            let alias = base.join("junction");
            let status = std::process::Command::new("cmd.exe")
                .args(["/d", "/c", "mklink", "/J"])
                .arg(&alias)
                .arg(&root)
                .status()
                .unwrap();
            assert!(status.success());
            assert!(
                libraries(&alias, &fields, false)
                    .unwrap_err()
                    .contains("reparse")
            );
            fs::remove_dir(&alias).unwrap();
        }
        fs::remove_dir_all(base).unwrap();
    }
}
