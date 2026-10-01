//! Standalone, precompiled verifier. No Cargo, Python, or native inference linkage.
#[path = "../api_smoke.rs"]
#[allow(dead_code)]
mod api_smoke;
#[path = "../package_acceptance.rs"]
mod package_acceptance;

fn main() -> std::process::ExitCode {
    package_acceptance::main(&std::env::args_os().skip(1).collect::<Vec<_>>())
}
