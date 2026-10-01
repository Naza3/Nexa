#!/usr/bin/env python3
"""Verify a freshly extracted desktop package and its actual bridge, not UI claims."""
from __future__ import annotations
import argparse
from contextlib import contextmanager
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import zipfile
import package_desktop_windows as desktop


@contextmanager
def extraction_root():
    root = Path(tempfile.mkdtemp(prefix="Nexa T06 中文 空格 "))
    try:
        yield root
    except BaseException:
        # An unconfirmed shutdown must not remove the live instance's private
        # data. Retain only this run's unique directory; never print its path.
        raise
    else:
        shutil.rmtree(root)


def checked_json(command, cwd, env, timeout):
    result = subprocess.run([str(arg) for arg in command], cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE, encoding="utf-8", errors="strict", timeout=timeout, check=False)
    if result.returncode:
        # Do not echo process output: failures may contain paths or native logs.
        raise ValueError(f"desktop diagnostic or bridge harness failed: exit {result.returncode}")
    if len(result.stdout.encode("utf-8")) > 1024 * 1024:
        raise ValueError("desktop diagnostic exceeded result size limit")
    return json.loads(result.stdout)


def run(archive, model, harness, evidence):
    if os.name != "nt":
        raise ValueError("desktop execution smoke requires Windows")
    expected = archive.with_suffix(".zip.sha256").read_text(encoding="utf-8").strip()
    if expected != f"{desktop.base.digest(archive)}  desktop-windows.zip":
        raise ValueError("desktop archive checksum mismatch")
    evidence.mkdir(parents=True, exist_ok=True)
    report = {"schema_version": 1, "result": "failed", "native_window_tested": False, "failure_directory_policy": "retain after failure for private local diagnosis",
              "ui_acceptance": "Windows 10 real window interactions, clipboard and close-button cases remain separate manual acceptance"}
    try:
        with extraction_root() as root:
            with zipfile.ZipFile(archive) as zipped:
                names = set()
                for member in zipped.infolist():
                    desktop.base.relative(member.filename)
                    if not member.filename.startswith("desktop-windows/") or member.filename.casefold() in names or member.is_dir() or member.file_size > 1024 * 1024 * 1024:
                        raise ValueError("unsafe desktop archive entry")
                    names.add(member.filename.casefold())
                if len(names) > 8192 or sum(item.file_size for item in zipped.infolist()) > 2 * 1024 * 1024 * 1024:
                    raise ValueError("desktop archive exceeds bounded package size")
                zipped.extractall(root)
            package = root / "desktop-windows"
            manifest = json.loads((package / "manifest.json").read_text(encoding="utf-8"))
            desktop.verify(package, manifest)
            if os.environ.get("GITHUB_SHA") and (manifest["project_commit"] != os.environ["GITHUB_SHA"] or manifest["project_dirty"] is not False):
                raise ValueError("desktop archive is not the clean CI source")
            cwd = root / "独立 空目录"
            cwd.mkdir()
            local_model = root / "模型 输入/候选 模型.gguf"
            local_model.parent.mkdir()
            shutil.copyfile(desktop.base.regular(model), local_model)
            if desktop.base.digest(model) != desktop.base.digest(local_model):
                raise ValueError("desktop model copy identity mismatch")
            env = desktop.base.windows_environment(os.environ, "")
            env["PATH"] = env["SYSTEMROOT"] + r"\System32;" + env["SYSTEMROOT"]
            env["LOCALAPPDATA"] = str(root / "隔离 本机数据")
            env["TEMP"] = str(cwd)
            env["TMP"] = str(cwd)
            observed = checked_json([package / "nexa-desktop.exe", "--diagnose"], cwd, env, 60)
            # Persist this useful read-only observation even if real bridge fails.
            desktop.base.write_json(evidence / "diagnostics.json", observed)
            if observed.get("package_verified") is not True or not observed.get("webview2_version") or observed.get("project_commit") != manifest["project_commit"] or observed.get("native_window_tested") is not False:
                raise ValueError("desktop executable observations differ from package")
            bridge = checked_json([harness, "--runtime", package / "runtime/ai-runtime.exe", "--model", local_model], cwd, env, 600)
            # The harness defines its own completed cases. A successful exit with
            # missing pass declaration is never silently promoted to a pass here.
            if bridge.get("success") is not True:
                raise ValueError("desktop real bridge did not report pass")
            desktop.base.write_json(evidence / "bridge-real.json", bridge)
            desktop.verify(package, manifest)
            shapes = {key: {"contains_non_ascii": any(ord(c) > 127 for c in str(path)), "contains_space": " " in str(path)} for key, path in
                      {"desktop_executable": package / "nexa-desktop.exe", "runtime_executable": package / "runtime/ai-runtime.exe", "source_model": local_model, "working_directory": cwd}.items()}
            if "data_dir_path_shape" not in bridge:
                raise ValueError("real bridge omitted its actual data directory shape")
            shapes["actual_data_directory"] = bridge["data_dir_path_shape"]
            if not all(value["contains_non_ascii"] and value["contains_space"] for value in shapes.values()):
                raise ValueError("desktop Unicode/space path coverage was not observed")
            report.update(result="pass", project_commit=manifest["project_commit"], desktop_package_sha256=desktop.base.digest(archive), package_unchanged=True, path_coverage=shapes,
                          webview2_version=observed["webview2_version"], bridge_result="pass")
    finally:
        desktop.base.write_json(evidence / "acceptance.json", report)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, default=desktop.ROOT / "dist/desktop-windows.zip")
    parser.add_argument("--model", type=Path, required=True)
    parser.add_argument("--harness", type=Path, required=True)
    parser.add_argument("--out", type=Path, default=desktop.ROOT / "artifacts/verification/windows-desktop")
    args = parser.parse_args()
    run(args.archive.absolute(), args.model.absolute(), args.harness.absolute(), args.out.absolute())

if __name__ == "__main__":
    main()
