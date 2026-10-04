use std::{env, fs, path::PathBuf, process::Command};

fn run(command: &mut Command) {
    let result = command
        .status()
        .expect("could not start CMake; install CMake >= 3.24");
    assert!(result.success(), "native build failed: {command:?}");
}
mod native_identity;
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
    assert!(
        !env::var("CARGO_CFG_TARGET_FEATURE")
            .unwrap_or_default()
            .split(',')
            .any(|f| f == "crt-static"),
        "crt-static is unsupported; Nexa Windows builds require the default dynamic MSVC CRT (/MD)"
    );
    let native = if let Some(path) = env::var_os("AIR_NATIVE_DIR") {
        let native =
            std::path::absolute(PathBuf::from(path)).expect("AIR_NATIVE_DIR must be a usable path");
        assert!(native.is_dir(), "AIR_NATIVE_DIR must exist");
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
    let identity_path = native.join("air-native-Release.txt");
    println!("cargo:rerun-if-changed={}", identity_path.display());
    let identity_text = fs::read_to_string(&identity_path).expect("native build identity missing: re-run CMake configure and build Release for this target; AIR_NATIVE_DIR is not trusted by library name alone");
    let identity = native_identity::parse(&identity_text).expect("invalid native build identity");
    native_identity::validate(
        &identity,
        &target_os,
        &env::var("CARGO_CFG_TARGET_ARCH").unwrap(),
        &env::var("CARGO_CFG_TARGET_FEATURE").unwrap_or_default(),
    )
    .expect("native build identity mismatch");
    let libraries = native_identity::libraries(&native, &identity, target_env == "msvc")
        .expect("invalid native target paths");
    for (name, library) in native_identity::LIBRARIES.into_iter().zip(libraries) {
        println!("cargo:rerun-if-changed={}", library.display());
        println!(
            "cargo:rustc-link-search=native={}",
            library.parent().unwrap().display()
        );
        println!("cargo:rustc-link-lib=static={name}");
    }
    if target_os == "windows" {
        // Cargo links the archives directly, so CMake's Windows system-library
        // defaults do not propagate. ggml-cpu's CPU-name lookup calls Reg* APIs
        // (Advapi32); llama-common/arg.cpp uses CommandLineToArgvW (Shell32);
        // cpp-httplib uses Winsock. TLS and LLGuidance are disabled in this build.
        for name in ["advapi32", "shell32", "ws2_32"] {
            println!("cargo:rustc-link-lib={name}");
        }
    } else {
        for name in ["stdc++", "pthread", "dl", "m"] {
            println!("cargo:rustc-link-lib={name}");
        }
    }
}
