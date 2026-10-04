#!/usr/bin/env python3
"""Run the actual source layout/download identity consumers on Linux package bytes.

The temporary Rust harness imports the product source, uses exact direct dependency
versions from its manifest, runs offline, and never executes a packaged Windows PE.
This does not test Windows filesystem semantics, the native window or inference.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
import tomllib
import package_windows as base

ROOT = base.ROOT


def verify(executable, output):
    if output.exists():
        base.fail("consumer report output already exists")
    shell = ROOT / "apps/desktop/src-tauri"
    dependencies = tomllib.loads((shell / "Cargo.toml").read_text(encoding="utf-8"))["dependencies"]
    for name in ("serde", "serde_json", "sha2", "uuid"):
        item = dependencies[name]
        version = item["version"] if isinstance(item, dict) else item
        if not version.startswith("="):
            base.fail("consumer direct dependencies must use exact versions")
    with tempfile.TemporaryDirectory(prefix="nexa-cross-consumers-") as temporary:
        directory = Path(temporary)
        (directory / "src").mkdir()
        lines = ['[package]', 'name = "nexa-cross-package-consumers"', 'version = "0.0.0"', 'edition = "2024"', '[workspace]', '[dependencies]']
        for name in ("serde", "serde_json", "sha2", "uuid"):
            item = dependencies[name]
            if isinstance(item, dict):
                lines.append(name + " = { version = " + json.dumps(item["version"]) + ", features = " + json.dumps(item.get("features", [])) + " }")
            else:
                lines.append(name + " = " + json.dumps(item))
        lines.append("download-engine = { path = " + json.dumps(str(ROOT / "crates/download-engine")) + " }")
        (directory / "Cargo.toml").write_text("\n".join(lines) + "\n", encoding="utf-8")
        code = '''#![allow(dead_code)]
#[path = SELECTION] mod selection;
#[path = LAYOUT] mod layout;
fn main() {
    let path = std::path::PathBuf::from(std::env::args_os().nth(1).expect("desktop executable required"));
    let layout = layout::validate(&path).expect("actual desktop layout consumer rejected package");
    let (_engine, _guard) = download_engine::identity::verify_component(&layout.package_root.join("download"), Some(&layout.project_commit)).expect("actual download identity consumer rejected package");
    println!("{}", serde_json::json!({"schema_version": 1, "status": "pass", "validation": "actual Rust layout and download identity consumers on Linux", "project_commit": layout.project_commit, "project_dirty": layout.project_dirty, "windows_execution": "not_run"}));
}
'''.replace("SELECTION", json.dumps(str(shell / "src/selection.rs"))).replace("LAYOUT", json.dumps(str(shell / "src/layout.rs")))
        (directory / "src/main.rs").write_text(code, encoding="utf-8")
        env = os.environ.copy()
        env["CARGO_TARGET_DIR"] = str(directory / "target")
        # Offline is deliberate: this validator cannot add network dependencies
        # or perform tool setup after the build source snapshot was captured.
        result = subprocess.run(["cargo", "run", "--offline", "--quiet", "--manifest-path", directory / "Cargo.toml", "--", executable],
                                env=env, cwd=ROOT, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False)
        if result.returncode:
            base.fail("actual package consumers failed: " + result.stderr[-6000:])
        report = json.loads(result.stdout)
        if report.get("status") != "pass":
            base.fail("actual package consumers did not report pass")
        report["source_files"] = [{"path": path, "sha256": base.digest(ROOT / path)} for path in (
            "apps/desktop/src-tauri/src/selection.rs", "apps/desktop/src-tauri/src/layout.rs", "crates/download-engine/src/identity.rs")]
        report["harness_cargo_lock_sha256"] = base.digest(directory / "Cargo.lock")
        base.write_json(output, report)
        return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--desktop-exe", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    try:
        verify(args.desktop_exe.absolute(), args.output)
    except (ValueError, OSError, KeyError, json.JSONDecodeError) as error:
        parser.exit(1, f"Package consumer validation failed: {error}\n")
