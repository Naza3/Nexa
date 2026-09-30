use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

fn run(command: &mut Command) {
    let result = command
        .status()
        .expect("could not start CMake; install CMake >= 3.24");
    assert!(result.success(), "native build failed: {command:?}");
}
fn find_library(root: &Path, name: &str, suffix: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let filename = format!("{}{name}{suffix}", if suffix == ".a" { "lib" } else { "" });
    for entry in fs::read_dir(root).unwrap_or_else(|error| panic!("read native directory: {error}"))
    {
        let entry = entry.expect("read native entry");
        let path = entry.path();
        if entry.file_type().expect("native entry type").is_dir() {
            found.extend(find_library(&path, name, suffix));
        } else if path
            .file_name()
            .is_some_and(|value| value == filename.as_str())
        {
            found.push(path);
        }
    }
    found
}
fn main() {
    for variable in [
        "AIR_NATIVE_DIR",
        "CMAKE",
        "CMAKE_GENERATOR",
        "CMAKE_BUILD_PARALLEL_LEVEL",
    ] {
        println!("cargo:rerun-if-env-changed={variable}");
    }
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let source = root.join("native/llama-shim");
    println!("cargo:rerun-if-changed={}", source.display());
    println!(
        "cargo:rerun-if-changed={}",
        root.join("vendor/llama.cpp").display()
    );
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap();
    assert!(
        target_os == "linux" || (target_os == "windows" && target_env == "msvc"),
        "this T01 build entry supports Linux host verification and Windows MSVC only; Android integration is T07"
    );
    let native = if let Some(path) = env::var_os("AIR_NATIVE_DIR") {
        let native = PathBuf::from(path)
            .canonicalize()
            .expect("AIR_NATIVE_DIR must exist");
        println!("cargo:rerun-if-changed={}", native.display());
        native
    } else {
        assert_eq!(
            env::var("HOST").unwrap(),
            env::var("TARGET").unwrap(),
            "cross compilation requires an independently configured AIR_NATIVE_DIR"
        );
        let native = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("native");
        let cmake = env::var_os("CMAKE").unwrap_or_else(|| "cmake".into());
        let mut configure = Command::new(&cmake);
        configure
            .arg("-S")
            .arg(&source)
            .arg("-B")
            .arg(&native)
            .arg("-DCMAKE_BUILD_TYPE=Release");
        if let Some(generator) = env::var_os("CMAKE_GENERATOR") {
            configure.arg("-G").arg(generator);
        }
        // Rust's default MSVC runtime is /MD; use the same runtime for C++.
        if target_env == "msvc" {
            configure.arg("-DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreadedDLL");
        }
        run(&mut configure);
        let jobs = env::var("CMAKE_BUILD_PARALLEL_LEVEL").unwrap_or_else(|_| "2".into());
        run(Command::new(cmake)
            .arg("--build")
            .arg(&native)
            .arg("--config")
            .arg("Release")
            .arg("--target")
            .arg("air_llama")
            .arg("--parallel")
            .arg(jobs));
        native
    };
    let suffix = if target_env == "msvc" { ".lib" } else { ".a" };
    for name in [
        "air_llama",
        "llama-common",
        "llama-common-base",
        "cpp-httplib",
        "llama",
        "ggml",
        "ggml-cpu",
        "ggml-base",
    ] {
        let libraries = find_library(&native, name, suffix);
        assert_eq!(
            libraries.len(),
            1,
            "expected exactly one {name}{suffix} in AIR_NATIVE_DIR/native build; got {libraries:?}"
        );
        println!(
            "cargo:rustc-link-search=native={}",
            libraries[0].parent().unwrap().display()
        );
        println!("cargo:rustc-link-lib=static={name}");
    }
    if target_os == "windows" {
        println!("cargo:rustc-link-lib=ws2_32");
    } else {
        for name in ["stdc++", "pthread", "dl", "m"] {
            println!("cargo:rustc-link-lib={name}");
        }
    }
}
