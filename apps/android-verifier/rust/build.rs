fn main() {
    println!("cargo:rerun-if-changed=../pubspec.yaml");
    let pubspec = std::fs::read_to_string("../pubspec.yaml").expect("App pubspec is required");
    let version = pubspec
        .lines()
        .find_map(|line| line.strip_prefix("version: "))
        .expect("App version is required");
    assert!(
        version
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b'.' || b == b'+')
    );
    println!("cargo:rustc-env=NEXA_APP_VERSION={version}");
    println!("cargo:rerun-if-env-changed=NEXA_LINK_MAP");
    if let Ok(map) = std::env::var("NEXA_LINK_MAP") {
        println!("cargo:rustc-link-arg-cdylib=-Wl,-Map,{map}");
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        println!("cargo:rustc-link-arg-cdylib=-Wl,-z,max-page-size=16384");
        println!("cargo:rustc-link-arg-cdylib=-Wl,-z,common-page-size=16384");
    }
    for key in ["NEXA_SOURCE_COMMIT", "NEXA_SOURCE_DIRTY", "NEXA_BUILD_MODE"] {
        println!("cargo:rerun-if-env-changed={key}");
        println!(
            "cargo:rustc-env={key}={}",
            std::env::var(key).unwrap_or_else(|_| "unknown".into())
        );
    }
}
