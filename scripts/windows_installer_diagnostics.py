#!/usr/bin/env python3
"""Bounded allowlist-only installer diagnostics; never publish raw MSI logs/UI text."""
from __future__ import annotations
import argparse
import ctypes
import json
import os
from pathlib import Path
import re
import time
import tempfile

import package_windows as base
from release_version import validate_version

STAGES = frozenset("installer_guard_preflight preflight_wrong_scope preflight_wrong_directory maintenance_scope_verified maintenance_directory_verified preflight wizard_cancel_install wizard_cancel_repair wizard_cancel_uninstall setup_invalid_option setup_conflicting_actions setup_uninstall_absent msi_fresh_wrong_scope msi_fresh_wrong_directory msi_default_ui_install msi_repair msi_alias_directory setup_repair runtime_init runtime_ready msi_busy_fa msi_busy_x setup_busy_repair runtime_stop msi_wrong_scope msi_wrong_directory msi_junction_rejected fixture_build msi_rollback msi_upgrade msi_downgrade msi_uninstall setup_gui_install setup_uninstall setup_silent_install setup_silent_uninstall reboot_fixture_build setup_reboot msi_reboot_fixture_remove complete".split())
ACTIONS = frozenset("INSTALL FindRelatedProducts LaunchConditions CostInitialize FileCost CostFinalize NexaOsGuard NexaLegacyOsProbe NexaGuard InstallValidate InstallInitialize RemoveExistingProducts ProcessComponents UnpublishFeatures RemoveRegistryValues RemoveShortcuts RemoveFiles RemoveFolders CreateFolders InstallFiles CreateShortcuts WriteRegistryValues RegisterUser RegisterProduct PublishFeatures PublishProduct InstallFinalize ExecuteAction ValidateProductID ResolveSource InstallExecute InstallExecuteAgain ScheduleReboot NexaTestRollback".split())
REASON_TEXT = {
    "guard_scope": "only supports a current-user", "guard_os": "requires windows 10",
    "guard_install_root": "must be installed in the current user's", "guard_known_folder": "cannot resolve the current user's",
    "guard_install_path": "installation path is inaccessible", "guard_inventory": "installation inventory",
    "guard_payload_path": "managed files are inaccessible", "guard_menu_path": "start menu path is inaccessible",
    "guard_busy": "process state could not be verified", "installer_open_error": "installation package could not be opened",
    "installer_prepare": "preparing to install", "installer_complete": "installation completed", "installer_error": "error",
    "wizard_active": "windows installer is still running", "wizard_apply": "applying your selection",
    "wizard_success": "completed successfully", "wizard_failure": "did not complete", "wizard_cancel": "setup was canceled",
}
REASONS = frozenset(REASON_TEXT) | {"unclassified"}
CLASSES = frozenset({"#32770", "MsiDialogCloseClass", "NexaSetupWizard", "other"})
MAX_EVENTS = 160


def reason_codes(text):
    lowered = text.lower()
    matches = sorted({key for key, phrase in REASON_TEXT.items() if phrase in lowered})
    return matches or ["unclassified"]


def empty_log():
    return {"present": False, "actions": [], "error_codes": [], "reason_codes": [], "legacy_os": []}


def summarize_log(path):
    result = empty_log()
    if path is None or not Path(path).is_file():
        return result
    with Path(path).open("rb") as source:
        header = source.read(2)
        source.seek(0, os.SEEK_END)
        size = source.tell()
        start = max(0, size - 512 * 1024)
        if header in (b"\xff\xfe", b"\xfe\xff"):
            start -= start % 2
        source.seek(start)
        raw = source.read(512 * 1024 + 2)
    text = raw.decode("utf-16-le" if header == b"\xff\xfe" else "utf-16-be" if header == b"\xfe\xff" else "utf-8", errors="replace")
    result["present"] = True
    for phase, action, returned in re.findall(r"Action (start|ended) [0-9:.]+: ([A-Za-z][A-Za-z0-9_]+)\.(?: Return value ([0-9]+)\.)?", text):
        if action in ACTIONS:
            result["actions"].append({"action": action, "phase": "start" if phase == "start" else "end", "return_code": int(returned) if returned else None})
    result["actions"] = result["actions"][-32:]
    result["error_codes"] = sorted({int(value) for value in re.findall(r"(?:Error\s+|Note: 1: )([0-9]{3,5})\b", text)})[:32]
    result["reason_codes"] = [value for value in reason_codes(text) if value != "unclassified"]
    for values in re.findall(r"NexaLegacyOsProbe stage=([0-7]) success=([01]) major=([0-9]{1,5}) minor=([0-9]{1,5}) build=([0-9]{1,5}) error=([0-9]{1,10})", text):
        stage, success, major, minor, build, error = map(int, values)
        if max(major, minor, build) <= 65535 and error <= 2**32 - 1:
            result["legacy_os"].append({"stage": stage, "success": bool(success), "major": major, "minor": minor, "build": build, "win32_error": error})
    result["legacy_os"] = result["legacy_os"][-4:]
    return result


def own_process_tree(root_pid):
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    U, P = ctypes.c_uint32, ctypes.c_void_p
    class Entry(ctypes.Structure):
        _fields_ = [("size", U), ("usage", U), ("pid", U), ("heap", ctypes.c_size_t), ("module", U), ("threads", U),
                    ("parent", U), ("priority", ctypes.c_long), ("flags", U), ("name", ctypes.c_wchar * 260)]
    kernel.CreateToolhelp32Snapshot.argtypes = [U, U]; kernel.CreateToolhelp32Snapshot.restype = P
    kernel.Process32FirstW.argtypes = [P, ctypes.POINTER(Entry)]
    kernel.Process32NextW.argtypes = [P, ctypes.POINTER(Entry)]
    kernel.CloseHandle.argtypes = [P]
    snapshot = kernel.CreateToolhelp32Snapshot(2, 0)
    if snapshot == ctypes.c_void_p(-1).value:
        return {root_pid}
    parents = {}
    try:
        entry = Entry(); entry.size = ctypes.sizeof(entry)
        more = kernel.Process32FirstW(snapshot, ctypes.byref(entry))
        while more:
            parents[entry.pid] = entry.parent
            more = kernel.Process32NextW(snapshot, ctypes.byref(entry))
    finally:
        kernel.CloseHandle(snapshot)
    owned = {root_pid}
    for _ in range(8):
        updated = owned | {pid for pid, parent in parents.items() if parent in owned}
        if updated == owned: break
        owned = updated
    return owned


def summarize_windows(root_pid):
    if os.name != "nt":
        return []
    owned = own_process_tree(root_pid)
    user = ctypes.WinDLL("user32", use_last_error=True)
    P, U = ctypes.c_void_p, ctypes.c_uint32
    callback_type = ctypes.WINFUNCTYPE(ctypes.c_int, P, ctypes.c_ssize_t)
    user.EnumWindows.argtypes = [callback_type, ctypes.c_ssize_t]
    user.EnumChildWindows.argtypes = [P, callback_type, ctypes.c_ssize_t]
    user.GetWindowThreadProcessId.argtypes = [P, ctypes.POINTER(U)]
    user.GetWindowTextW.argtypes = [P, ctypes.c_wchar_p, ctypes.c_int]
    user.GetClassNameW.argtypes = [P, ctypes.c_wchar_p, ctypes.c_int]
    user.GetDlgCtrlID.argtypes = [P]; user.GetDlgCtrlID.restype = ctypes.c_int
    user.IsWindowVisible.argtypes = [P]; user.IsWindowEnabled.argtypes = [P]
    windows = []
    @callback_type
    def visit(hwnd, unused):
        pid = U(); user.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
        if pid.value not in owned or len(windows) >= 16:
            return 1
        name = ctypes.create_unicode_buffer(128); user.GetClassNameW(hwnd, name, len(name))
        texts, ids = [], []
        @callback_type
        def child(control, ignored):
            if len(ids) < 64:
                value = user.GetDlgCtrlID(control)
                if -1 <= value <= 65535: ids.append(value)
                text = ctypes.create_unicode_buffer(2048)
                user.GetWindowTextW(control, text, len(text)); texts.append(text.value)
            return 1
        user.EnumChildWindows(hwnd, child, 0)
        windows.append({"class": name.value if name.value in CLASSES else "other",
                        "visible": bool(user.IsWindowVisible(hwnd)), "enabled": bool(user.IsWindowEnabled(hwnd)),
                        "control_ids": sorted(set(ids)), "reason_codes": reason_codes("\n".join(texts))})
        return 1
    user.EnumWindows(visit, 0)
    return windows


def exact_keys(value, keys):
    if type(value) is not dict or set(value) != set(keys):
        raise ValueError("installer diagnostics contain unexpected fields")


def bounded_integer(value, low, high):
    if type(value) is not int or not low <= value <= high:
        raise ValueError("installer diagnostics integer out of range")


def validate_document(document, commit):
    exact_keys(document, ("schema_version", "project_commit", "version", "status", "last_stage", "events"))
    if type(document["schema_version"]) is not int or document["schema_version"] != 1 or not re.fullmatch(r"[a-f0-9]{40}", commit) or document["project_commit"] != commit:
        raise ValueError("installer diagnostic source/schema mismatch")
    validate_version(document["version"])
    if document["status"] not in ("running", "pass", "failed") or document["last_stage"] not in STAGES:
        raise ValueError("installer diagnostic status/stage invalid")
    if type(document["events"]) is not list or len(document["events"]) > MAX_EVENTS:
        raise ValueError("installer diagnostic event count invalid")
    for event in document["events"]:
        exact_keys(event, ("stage", "state", "elapsed_ms", "exit_code", "expected_exit_codes", "msi_log", "windows", "contexts", "targets_unchanged"))
        if event["stage"] not in STAGES or event["state"] not in ("start", "complete", "timeout", "error"):
            raise ValueError("installer diagnostic event stage/state invalid")
        bounded_integer(event["elapsed_ms"], 0, 2 * 60 * 60 * 1000)
        if event["exit_code"] is not None: bounded_integer(event["exit_code"], 0, 2**32 - 1)
        if type(event["expected_exit_codes"]) is not list or len(event["expected_exit_codes"]) > 8:
            raise ValueError("installer expected-code list invalid")
        for code in event["expected_exit_codes"]: bounded_integer(code, 0, 65535)
        if event["contexts"] is not None and (type(event["contexts"]) is not list or len(event["contexts"]) > 3 or any(type(value) is not int or value not in (1, 2, 4) for value in event["contexts"])):
            raise ValueError("installer registration context invalid")
        if event["targets_unchanged"] is not None and type(event["targets_unchanged"]) is not bool:
            raise ValueError("installer target observation invalid")
        log = event["msi_log"]
        exact_keys(log, ("present", "actions", "error_codes", "reason_codes", "legacy_os"))
        if type(log["present"]) is not bool or type(log["actions"]) is not list or len(log["actions"]) > 32:
            raise ValueError("installer action log invalid")
        for action in log["actions"]:
            exact_keys(action, ("action", "phase", "return_code"))
            if action["action"] not in ACTIONS or action["phase"] not in ("start", "end"):
                raise ValueError("installer action invalid")
            if action["return_code"] is not None: bounded_integer(action["return_code"], 0, 65535)
        if type(log["error_codes"]) is not list or len(log["error_codes"]) > 32:
            raise ValueError("installer error-code list invalid")
        for code in log["error_codes"]: bounded_integer(code, 0, 65535)
        if type(log["reason_codes"]) is not list or len(log["reason_codes"]) > len(REASONS) or not set(log["reason_codes"]) <= REASONS:
            raise ValueError("installer log reason invalid")
        if type(log["legacy_os"]) is not list or len(log["legacy_os"]) > 4:
            raise ValueError("installer OS observation count invalid")
        for observation in log["legacy_os"]:
            exact_keys(observation, ("stage", "success", "major", "minor", "build", "win32_error"))
            bounded_integer(observation["stage"], 0, 7)
            if type(observation["success"]) is not bool: raise ValueError("installer OS success flag invalid")
            for key in ("major", "minor", "build"): bounded_integer(observation[key], 0, 65535)
            bounded_integer(observation["win32_error"], 0, 2**32 - 1)
        if type(event["windows"]) is not list or len(event["windows"]) > 16:
            raise ValueError("installer window list invalid")
        for window in event["windows"]:
            exact_keys(window, ("class", "visible", "enabled", "control_ids", "reason_codes"))
            if window["class"] not in CLASSES or type(window["visible"]) is not bool or type(window["enabled"]) is not bool:
                raise ValueError("installer window class/state invalid")
            if type(window["control_ids"]) is not list or len(window["control_ids"]) > 64:
                raise ValueError("installer control list invalid")
            for value in window["control_ids"]: bounded_integer(value, -1, 65535)
            if type(window["reason_codes"]) is not list or len(window["reason_codes"]) > len(REASONS) or not set(window["reason_codes"]) <= REASONS:
                raise ValueError("installer window reason invalid")
    return document


class Trace:
    def __init__(self, path, commit, version):
        self.path = Path(path)
        self.document = {"schema_version": 1, "project_commit": commit, "version": version, "status": "running", "last_stage": "preflight", "events": []}
        self.started = time.monotonic()
        self.write()

    def write(self):
        validate_document(self.document, self.document["project_commit"])
        self.path.parent.mkdir(parents=True, exist_ok=True)
        temporary = self.path.with_suffix(".tmp")
        temporary.write_text(json.dumps(self.document, sort_keys=True, indent=2) + "\n", encoding="utf-8")
        temporary.replace(self.path)

    def event(self, stage, state, *, exit_code=None, expected=(), log=None, pid=None, contexts=None, targets_unchanged=None):
        item = {"stage": stage, "state": state, "elapsed_ms": int((time.monotonic() - self.started) * 1000),
                "exit_code": exit_code, "expected_exit_codes": list(expected), "msi_log": summarize_log(log),
                "windows": summarize_windows(pid) if pid and state in ("timeout", "error") else [],
                "contexts": list(contexts) if contexts is not None else None, "targets_unchanged": targets_unchanged}
        self.document["last_stage"] = stage
        if state in ("timeout", "error"): self.document["status"] = "failed"
        self.document["events"].append(item)
        self.write()
        print(json.dumps({"installer_stage": stage, "state": state, "exit_code": exit_code,
                          "expected_exit_codes": list(expected), "elapsed_ms": item["elapsed_ms"],
                          "msi_log": item["msi_log"], "windows": item["windows"],
                          "contexts": item["contexts"], "targets_unchanged": targets_unchanged}, sort_keys=True), flush=True)

    def finish(self, passed):
        self.document["status"] = "pass" if passed else "failed"
        if passed: self.document["last_stage"] = "complete"
        self.write()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--commit", required=True)
    args = parser.parse_args()
    if not re.fullmatch(r"[a-f0-9]{40}", args.commit): parser.error("invalid source commit")
    output = base.resolve_checked_path(args.output)
    source = base.resolve_checked_path(args.input)
    if output.exists(): parser.error("evidence output must not already exist")
    if not source.exists():
        document = {"schema_version": 1, "project_commit": args.commit, "installer_diagnostics": "missing"}
    else:
        if not source.is_file() or source.stat().st_size > 1024 * 1024: parser.error("invalid diagnostic input file")
        document = validate_document(base.strict_json(source.read_bytes()), args.commit)
    # Validate everything before creating the destination. A failure never leaves
    # a partially reviewed directory for the workflow to upload.
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="installer-evidence-", dir=output.parent) as temporary:
        staged = Path(temporary)
        (staged / "windows-msi-diagnostics.json").write_text(json.dumps(document, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        staged.replace(output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
