//! The distributable is built on Windows; a Linux host cannot claim this gate.
use std::{
    ffi::OsString,
    process::{Command, ExitCode},
};

pub fn main(args: &[OsString]) -> ExitCode {
    if let Err(error) = validate(args) {
        eprintln!("{error}");
        return ExitCode::from(2);
    }
    if !cfg!(all(
        target_os = "windows",
        target_arch = "x86_64",
        target_env = "msvc"
    )) {
        eprintln!(
            "Windows x64 CPU packaging requires a native Windows x64 MSVC build host; no package was built"
        );
        return ExitCode::from(2);
    }
    match Command::new("python")
        .arg(crate::root().join("scripts/package_windows.py"))
        .current_dir(crate::root())
        .status()
    {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(_) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("could not run Windows packaging helper: {error}");
            ExitCode::FAILURE
        }
    }
}
fn validate(args: &[OsString]) -> Result<(), &'static str> {
    if args.len() != 4 {
        return Err("build requires exactly --platform windows-x64 --backend cpu");
    }
    let mut platform = false;
    let mut backend = false;
    for pair in args.as_chunks::<2>().0 {
        if pair[0] == "--platform" && pair[1] == "windows-x64" && !platform {
            platform = true;
        } else if pair[0] == "--backend" && pair[1] == "cpu" && !backend {
            backend = true;
        } else {
            return Err(
                "unsupported or duplicate build option; only Windows x64 CPU is implemented",
            );
        }
    }
    if platform && backend {
        Ok(())
    } else {
        Err("--platform and --backend are required")
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }
    #[test]
    fn only_explicit_unique_cpu_platform_is_supported() {
        assert!(validate(&args(&["--platform", "windows-x64", "--backend", "cpu"])).is_ok());
        assert!(validate(&args(&["--backend", "cpu", "--platform", "windows-x64"])).is_ok());
        for invalid in [
            vec![],
            vec!["--platform", "windows-x64"],
            vec!["--platform", "windows-x64", "--backend", "cuda"],
            vec!["--platform", "windows-x64", "--platform", "windows-x64"],
        ] {
            assert!(validate(&args(&invalid)).is_err());
        }
    }
}
