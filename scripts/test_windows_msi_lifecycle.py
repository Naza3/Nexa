#!/usr/bin/env python3
"""Destructive installer lifecycle acceptance, ONLY on a disposable GitHub runner.

Never run this against a user's installed Nexa. Refuses any existing installation
or data root. Test-only future versions are private fixtures, never release assets.
"""
from __future__ import annotations
import argparse
import ctypes
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

import package_windows_msi as pack
from windows_msi_api import Msi
from windows_installer_diagnostics import Trace

_TRACE = None


def trace(stage, state, **details):
    if _TRACE is not None:
        _TRACE.event(stage, state, **details)

CHECKS = ("install", "repair", "upgrade", "rollback", "downgrade_rejected", "uninstall", "user_data_preserved",
          "running_process_blocked", "setup_install", "setup_repair", "setup_uninstall", "setup_exit_codes", "setup_wizard")


def require(condition, message):
    if not condition:
        raise ValueError(message)


def next_version(version):
    values = list(map(int, pack.validate_version(version).split(".")))
    for index, maximum in ((2, 65535), (1, 255), (0, 255)):
        if values[index] < maximum:
            values[index] += 1
            for lower in range(index + 1, 3):
                values[lower] = 0
            return ".".join(map(str, values))
    raise ValueError("maximum MSI version has no upgrade fixture; release needs a new explicit test strategy")


def invoke(command, accepted=(0,), timeout=240, *, stage, log=None):
    # No shell and no untrusted property/argument forwarding.
    trace(stage, "start", expected=accepted)
    process = subprocess.Popen([str(value) for value in command], stdin=subprocess.DEVNULL)
    try:
        code = process.wait(timeout=timeout)
    except subprocess.TimeoutExpired as error:
        trace(stage, "timeout", expected=accepted, log=log, pid=process.pid)
        # Do not force-kill the installer midway through its transaction.
        raise ValueError("installer stage " + stage + " did not finish within the acceptance deadline") from error
    trace(stage, "complete" if code in accepted else "error", exit_code=code, expected=accepted, log=log, pid=process.pid)
    require(code in accepted, f"installer stage {stage} returned {code}; expected {accepted}")
    return code


def msi_command(msi, operation, log, *properties, accepted=(0,)):
    executable = Path(os.environ["SYSTEMROOT"]) / "System32/msiexec.exe"
    return invoke([executable, operation, msi, "/qn", "/norestart", "/l*v", log, "REBOOT=ReallySuppress", *properties], accepted,
                  stage="msi_" + log.stem.replace("-", "_"), log=log)


def installed_payload(root, files):
    for item in files:
        path = pack.base.regular(root / item["path"])
        require(path.stat().st_size == item["size_bytes"] and pack.base.digest(path) == item["sha256"], "installed payload differs: " + item["path"])


def same_sentinels(sentinels):
    for path, content in sentinels.items():
        require(path.is_file() and path.read_bytes() == content, "user-data sentinel was changed or removed")


def state(api, version):
    return api.MsiQueryProductStateW(pack.product_code(version))


def short_path(path):
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.GetShortPathNameW.argtypes = [ctypes.c_wchar_p, ctypes.c_wchar_p, ctypes.c_uint]
    kernel.GetShortPathNameW.restype = ctypes.c_uint
    buffer = ctypes.create_unicode_buffer(32768)
    size = kernel.GetShortPathNameW(str(path), buffer, len(buffer))
    require(0 < size < len(buffer), "cannot query installed runtime path alias")
    return Path(buffer.value)


def wizard_check(setup):
    user = ctypes.WinDLL("user32", use_last_error=True)
    P, U = ctypes.c_void_p, ctypes.c_uint
    user.FindWindowW.argtypes = [ctypes.c_wchar_p, ctypes.c_wchar_p]; user.FindWindowW.restype = P
    user.GetDlgItem.argtypes = [P, ctypes.c_int]; user.GetDlgItem.restype = P
    user.SendMessageW.argtypes = [P, U, ctypes.c_size_t, ctypes.c_ssize_t]; user.SendMessageW.restype = ctypes.c_ssize_t
    user.GetWindowTextW.argtypes = [P, ctypes.c_wchar_p, ctypes.c_int]
    user.GetWindowThreadProcessId.argtypes = [P, ctypes.POINTER(U)]
    user.PostMessageW.argtypes = [P, U, ctypes.c_size_t, ctypes.c_ssize_t]
    observed = []
    for mode, selected in (([], 1001), (["/repair"], 1002), (["/uninstall"], 1003)):
        stage = "wizard_cancel_" + {1001: "install", 1002: "repair", 1003: "uninstall"}[selected]
        trace(stage, "start", expected=(1602,))
        process = subprocess.Popen([str(setup), *mode], stdin=subprocess.DEVNULL)
        hwnd = None
        try:
            deadline = time.monotonic() + 30
            while time.monotonic() < deadline and process.poll() is None:
                candidate = user.FindWindowW("NexaSetupWizard", "Nexa Setup")
                pid = U()
                if candidate:
                    user.GetWindowThreadProcessId(candidate, ctypes.byref(pid))
                    if pid.value == process.pid:
                        hwnd = candidate
                        break
                time.sleep(0.1)
            require(hwnd, "native Setup wizard window did not appear")
            require(user.SendMessageW(user.GetDlgItem(hwnd, selected), 0xF0, 0, 0) == 1, "wizard selected action differs from CLI")
            user.SendMessageW(user.GetDlgItem(hwnd, 1004), 0xF5, 0, 0)  # BM_CLICK Next
            text = ctypes.create_unicode_buffer(2048)
            user.GetWindowTextW(user.GetDlgItem(hwnd, 1100), text, len(text))
            require(("Ready to remove" if selected == 1003 else "Ready to install or repair") in text.value, "wizard confirmation content missing")
            user.SendMessageW(user.GetDlgItem(hwnd, 1006), 0xF5, 0, 0)  # Back
            require(user.SendMessageW(user.GetDlgItem(hwnd, selected), 0xF0, 0, 0) == 1, "wizard lost action after Back")
            user.SendMessageW(user.GetDlgItem(hwnd, 1005), 0xF5, 0, 0)  # Cancel before changes
            require(process.wait(timeout=30) == 1602, "wizard cancel did not return 1602")
            trace(stage, "complete", exit_code=1602, expected=(1602,))
            observed.append(selected)
        finally:
            if process.poll() is None and hwnd:
                user.PostMessageW(hwnd, 0x10, 0, 0)  # WM_CLOSE before Apply only
                process.wait(timeout=30)
    return observed


def wizard_apply(setup):
    """Drive the shipped GUI through Apply, protected progress and Finish."""
    user = ctypes.WinDLL("user32", use_last_error=True)
    P, U = ctypes.c_void_p, ctypes.c_uint
    user.FindWindowW.argtypes = [ctypes.c_wchar_p, ctypes.c_wchar_p]; user.FindWindowW.restype = P
    user.GetDlgItem.argtypes = [P, ctypes.c_int]; user.GetDlgItem.restype = P
    user.SendMessageW.argtypes = [P, U, ctypes.c_size_t, ctypes.c_ssize_t]; user.SendMessageW.restype = ctypes.c_ssize_t
    user.PostMessageW.argtypes = [P, U, ctypes.c_size_t, ctypes.c_ssize_t]
    user.IsWindowEnabled.argtypes = [P]; user.IsWindowEnabled.restype = ctypes.c_int
    user.GetWindowTextW.argtypes = [P, ctypes.c_wchar_p, ctypes.c_int]
    user.GetWindowThreadProcessId.argtypes = [P, ctypes.POINTER(U)]
    trace("setup_gui_install", "start", expected=(0,))
    process = subprocess.Popen([str(setup)], stdin=subprocess.DEVNULL)
    deadline, hwnd = time.monotonic() + 30, None
    while time.monotonic() < deadline and process.poll() is None:
        candidate = user.FindWindowW("NexaSetupWizard", "Nexa Setup")
        pid = U()
        if candidate:
            user.GetWindowThreadProcessId(candidate, ctypes.byref(pid))
            if pid.value == process.pid:
                hwnd = candidate; break
        time.sleep(0.02)
    require(hwnd, "Setup Apply test did not find its wizard")
    user.SendMessageW(user.GetDlgItem(hwnd, 1004), 0xF5, 0, 0)  # Next
    user.SendMessageW(user.GetDlgItem(hwnd, 1004), 0xF5, 0, 0)  # Apply
    require(not user.IsWindowEnabled(user.GetDlgItem(hwnd, 1005)), "wizard Cancel not protected during install")
    user.SendMessageW(hwnd, 0x111, 1005, 0)  # WM_COMMAND Cancel cannot abandon the transaction
    require(process.poll() is None, "wizard abandoned the active installer")
    user.PostMessageW(hwnd, 0x10, 0, 0)  # Close must explain waiting, not detach the installer
    deadline, protected = time.monotonic() + 10, False
    while time.monotonic() < deadline and process.poll() is None:
        dialog = user.FindWindowW("#32770", "Nexa Setup")
        pid = U()
        if dialog:
            user.GetWindowThreadProcessId(dialog, ctypes.byref(pid))
            if pid.value == process.pid:
                user.PostMessageW(dialog, 0x111, 1, 0)  # IDOK in the informational close guard
                protected = True; break
        time.sleep(0.01)
    require(protected, "wizard did not protect Close while Windows Installer was active")
    deadline = time.monotonic() + 240
    while time.monotonic() < deadline and process.poll() is None:
        text = ctypes.create_unicode_buffer(2048)
        user.GetWindowTextW(user.GetDlgItem(hwnd, 1004), text, len(text))
        if text.value == "Finish" and user.IsWindowEnabled(user.GetDlgItem(hwnd, 1004)):
            user.GetWindowTextW(user.GetDlgItem(hwnd, 1100), text, len(text))
            require("completed successfully" in text.value, "wizard did not display installation success")
            user.SendMessageW(user.GetDlgItem(hwnd, 1004), 0xF5, 0, 0)
            require(process.wait(timeout=30) == 0, "wizard Finish returned a failure")
            trace("setup_gui_install", "complete", exit_code=0, expected=(0,))
            return
        time.sleep(0.05)
    trace("setup_gui_install", "timeout", expected=(0,), pid=process.pid)
    raise ValueError("wizard Apply did not reach its final result")


def add_reboot_fixture(msi):
    api = Msi()
    with api.database(msi, 1) as database:
        api.execute(database, "INSERT INTO `InstallExecuteSequence` (`Action`,`Condition`,`Sequence`) VALUES (?,?,?)",
                    ["ScheduleReboot", "NOT Installed", 6550])
        api.check(api.MsiDatabaseCommit(database), "commit test-only reboot fixture")


def lifecycle(msi, setup, payload, report):
    global _TRACE
    require(sys.platform == "win32" and os.environ.get("GITHUB_ACTIONS") == "true" and os.environ.get("RUNNER_OS") == "Windows",
            "lifecycle acceptance is restricted to disposable GitHub Windows runners")
    api = Msi()
    msi, setup = pack.base.regular(msi).resolve(), pack.base.regular(setup).resolve()
    version = pack.repository_version(pack.ROOT)
    payload, manifest, files = pack.validate_payload(payload, version)
    root = Path(os.environ["LOCALAPPDATA"]) / pack.INSTALL_RELATIVE
    data = Path(os.environ["LOCALAPPDATA"]) / "Nexa"
    require(not root.exists() and not data.exists(), "refusing to touch existing Nexa program/user data")
    require(state(api, version) == -1, "refusing an existing registered Nexa installation")
    require(pack.setup_embedded_msi(setup) == msi.read_bytes(), "Setup resource differs from MSI")
    report = Path(report).resolve()
    require(not report.exists() and not report.is_relative_to(payload), "lifecycle report must be a new file outside payload")
    report.parent.mkdir(parents=True, exist_ok=True)
    logs = report.parent / "windows-msi-logs"
    logs.mkdir(exist_ok=True)
    checks = {key: False for key in CHECKS}
    result = {"schema_version": 1, "status": "failed", "version": version, "project_commit": manifest["project_commit"],
              "payload_manifest_sha256": pack.base.digest(payload / "manifest.json"), "msi_sha256": pack.base.digest(msi),
              "setup_sha256": pack.base.digest(setup), "product_code": pack.product_code(version), "upgrade_code": pack.UPGRADE_CODE,
              "unsigned": True, "checks": checks, "windows10_target_hardware": "not_run", "ice_validation": "not_run; table schema/readback and lifecycle are separate checks"}
    future = next_version(version)
    _TRACE = Trace(report.with_name("windows-msi-diagnostics.json"), manifest["project_commit"], version)
    trace("preflight", "complete")
    try:
        with tempfile.TemporaryDirectory(prefix="nexa-msi-acceptance-") as temporary:
            work = Path(temporary)
            result["wizard_actions"] = wizard_check(setup)
            require(not root.exists() and not data.exists(), "canceling Setup modified the product or data")
            invoke([setup, "/S", "/unknown"], (87,), stage="setup_invalid_option")
            invoke([setup, "/S", "/repair", "/install"], (87,), stage="setup_conflicting_actions")
            invoke([setup, "/S", "/uninstall"], (1605,), stage="setup_uninstall_absent")
            # Default /i is the double-click entry; LIMITUI selects native Basic UI.
            invoke([Path(os.environ["SYSTEMROOT"]) / "System32/msiexec.exe", "/i", msi,
                    "/norestart", "/l*v", logs / "default-ui-install.log", "REBOOT=ReallySuppress"],
                   stage="msi_default_ui_install", log=logs / "default-ui-install.log")
            result["msi_default_basic_ui_installed"] = True
            require(state(api, version) == 5, "per-user product not registered")
            installed_payload(root, files)
            checks["install"] = True
            sentinels = {data / "config.toml": b"# CI preservation sentinel; not a usable configuration\n",
                         data / "auth.token": b"CI-NOT-A-CREDENTIAL\n",
                         data / "model-library.json": b'{"ci-preserve":true}\n',
                         data / "models/keep.gguf": b"CI non-model preservation sentinel",
                         root / "models/keep.gguf": b"CI installed-folder model sentinel",
                         root / "model/keep.gguf": b"CI legacy installed-folder model sentinel",
                         work / "external/keep.gguf": b"CI external model sentinel"}
            for path, content in sentinels.items():
                path.parent.mkdir(parents=True, exist_ok=True); path.write_bytes(content)
            # Repair from both public formats restores a missing owned file.
            (root / "README.md").unlink()
            msi_command(msi, "/fa", logs / "repair.log")
            installed_payload(root, files); same_sentinels(sentinels)
            checks["repair"] = True
            alias_directory = short_path(root)
            result["install_short_path_alias_available"] = str(alias_directory).casefold() != str(root).casefold()
            msi_command(msi, "/fa", logs / "alias-directory.log", "INSTALLFOLDER=" + str(alias_directory))
            installed_payload(root, files); same_sentinels(sentinels)
            (root / "README.md").unlink()
            invoke([setup, "/S", "/repair"], stage="setup_repair")
            installed_payload(root, files); same_sentinels(sentinels)
            checks["setup_repair"] = True
            # A real installed runtime is stopped through its own management API.
            runtime = root / "runtime/ai-runtime.exe"
            runtime_data = work / "runtime-data"
            invoke([runtime, "--data-dir", runtime_data, "init"], stage="runtime_init")
            alias = short_path(runtime)
            result["runtime_short_path_alias_available"] = str(alias).casefold() != str(runtime).casefold()
            trace("runtime_ready", "start")
            service = subprocess.Popen([str(alias), "--data-dir", str(runtime_data), "serve"], stdin=subprocess.DEVNULL,
                                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            try:
                deadline = time.monotonic() + 30
                while time.monotonic() < deadline:
                    require(service.poll() is None, "installed runtime exited before busy test")
                    status = subprocess.run([str(runtime), "--data-dir", str(runtime_data), "status"], capture_output=True, timeout=10)
                    if status.returncode == 0:
                        break
                    time.sleep(0.2)
                else:
                    raise ValueError("installed runtime did not become ready")
                trace("runtime_ready", "complete")
                for operation in ("/fa", "/x"):
                    msi_command(msi, operation, logs / ("busy-" + operation[1:] + ".log"), accepted=(1603,))
                invoke([setup, "/S", "/repair"], (1603,), stage="setup_busy_repair")
                require(service.poll() is None, "installer force-stopped a live runtime")
                installed_payload(root, files); same_sentinels(sentinels)
                checks["running_process_blocked"] = True
            finally:
                if service.poll() is None:
                    trace("runtime_stop", "start")
                    stopped = subprocess.run([str(runtime), "--data-dir", str(runtime_data), "stop"], capture_output=True, timeout=30)
                    require(stopped.returncode == 0, "test runtime graceful stop failed")
                    service.wait(timeout=30)
                    trace("runtime_stop", "complete", exit_code=stopped.returncode)
            # All install-owned locations must reject redirection/extra scope.
            msi_command(msi, "/fa", logs / "wrong-scope.log", "ALLUSERS=1", accepted=(1603,))
            msi_command(msi, "/fa", logs / "wrong-directory.log", "INSTALLFOLDER=" + str(work / "wrong-directory"), accepted=(1603,))
            require(not (work / "wrong-directory").exists(), "installer accepted directory override")
            original = root / "runtime"
            saved = root / "runtime-original"
            original.rename(saved)
            external = work / "redirected"; external.mkdir()
            try:
                cmd = Path(os.environ["SYSTEMROOT"]) / "System32/cmd.exe"
                created = subprocess.run([str(cmd), "/d", "/c", "mklink", "/J", str(original), str(external)], capture_output=True)
                require(created.returncode == 0, "cannot establish disposable junction rejection fixture")
                msi_command(msi, "/fa", logs / "junction-rejected.log", accepted=(1603,))
                require(not list(external.iterdir()), "installer followed a directory junction")
            finally:
                if original.exists(): os.rmdir(original)
                saved.rename(original)
            result["path_redirection_rejected"] = True
            trace("fixture_build", "start")
            guard, _ = pack.compile_native(work)
            pack.compile_native(work, os_check=True)
            cab = pack.make_cabinet(payload, files, work)
            broken = work / "rollback.msi"
            upgraded = work / "upgrade.msi"
            pack.write_msi(files, future, cab, guard, broken, rollback_fixture=True)
            pack.write_msi(files, future, cab, guard, upgraded)
            trace("fixture_build", "complete")
            msi_command(broken, "/i", logs / "rollback.log", accepted=(1603,))
            require(state(api, version) == 5 and state(api, future) == -1, "failed upgrade did not restore previous product")
            installed_payload(root, files); same_sentinels(sentinels)
            checks["rollback"] = True
            msi_command(upgraded, "/i", logs / "upgrade.log")
            require(state(api, version) == -1 and state(api, future) == 5, "upgrade left duplicate/wrong product registration")
            installed_payload(root, files); same_sentinels(sentinels)
            checks["upgrade"] = True
            msi_command(msi, "/i", logs / "downgrade.log", accepted=(1603,))
            require(state(api, future) == 5, "downgrade damaged newer installation")
            installed_payload(root, files); same_sentinels(sentinels)
            checks["downgrade_rejected"] = True
            msi_command(upgraded, "/x", logs / "uninstall.log")
            require(state(api, future) == -1, "uninstall kept registration")
            require(all(not (root / item["path"]).exists() for item in files), "uninstall left an owned payload file")
            same_sentinels(sentinels)
            checks["uninstall"] = True
            wizard_apply(setup)
            checks["setup_wizard"] = True
            result["setup_gui_apply_progress_finish"] = True
            require(state(api, version) == 5, "Setup did not install the same MSI product")
            installed_payload(root, files); same_sentinels(sentinels)
            checks["setup_install"] = True
            invoke([setup, "/S", "/uninstall"], stage="setup_uninstall")
            require(state(api, version) == -1, "Setup uninstall kept MSI registration")
            same_sentinels(sentinels)
            checks["setup_uninstall"] = True
            # Also install silently through the unmodified shipped Setup.
            invoke([setup, "/S"], stage="setup_silent_install")
            installed_payload(root, files); same_sentinels(sentinels)
            invoke([setup, "/S", "/uninstall"], stage="setup_silent_uninstall")
            require(state(api, version) == -1, "silent Setup cycle left product registered")
            # Test real msiexec 3010 propagation without rebooting the runner.
            trace("reboot_fixture_build", "start")
            reboot = work / "reboot.msi"
            pack.write_msi(files, future, cab, guard, reboot)
            add_reboot_fixture(reboot)
            reboot_setup, _ = pack.compile_native(work, reboot)
            trace("reboot_fixture_build", "complete")
            invoke([reboot_setup, "/S"], (3010,), stage="setup_reboot")
            require(state(api, future) == 5, "3010 fixture did not install")
            msi_command(reboot, "/x", logs / "reboot-fixture-remove.log", accepted=(0, 3010))
            same_sentinels(sentinels)
            result["setup_observed_exit_codes"] = [0, 87, 1602, 1603, 1605, 3010]
            checks["setup_exit_codes"] = True
            checks["user_data_preserved"] = True
            result["status"] = "pass"
    finally:
        # A failed check is evidence, never a successful report. Do not recursively
        # remove install/user trees: preserved sentinels are intentional evidence.
        if result["status"] != "pass" and _TRACE.document["status"] != "failed":
            trace(_TRACE.document["last_stage"], "error")
        _TRACE.finish(result["status"] == "pass")
        pack.base.write_json(report, result)
        _TRACE = None
    require(all(checks.values()), "incomplete installer lifecycle acceptance")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--msi", type=Path, required=True)
    parser.add_argument("--setup", type=Path, required=True)
    parser.add_argument("--payload", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    try:
        lifecycle(args.msi, args.setup, args.payload, args.report)
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        print(str(error), file=sys.stderr); return 1
    print("MSI and Setup lifecycle acceptance passed on the disposable native Windows runner")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
