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

MAX_JSON_BYTES = 1024 * 1024
MAX_FAILURE_BYTES = 4096
FAILURE_KEYS = frozenset({"schema_version", "kind", "success", "stage", "code", "bridge_code", "os_error", "runtime_exit_code", "child_stage", "child_exit_code", "cleanup"})
CLEANUP_KEYS = frozenset({"status", "code", "bridge_code", "os_error", "instance_lock", "discovery", "temporary_data_retained"})
# Frozen with crates/desktop-bridge/src/bin/harness/report.rs. Tests require
# exact agreement; unknown stages/codes are not arbitrary diagnostic strings.
FAILURE_STAGES = frozenset({
    "arguments", "create_private_directory", "initialize_token", "write_config",
    "construct_bridge", "launch_initial_child", "start_after_child_exit", "read_initial_instance",
    "attach_existing", "close_attached_window", "import_model", "list_models",
    "reject_running_idle_change", "load_model", "first_chat_start", "first_chat_consume",
    "cancel_chat_start", "cancel_chat_consume", "wait_ready", "repeat_chat_start",
    "repeat_chat_consume", "close_window", "verify_default_close", "launch_keep_child",
    "verify_process_exit", "unload_model", "reload_model", "launch_stop_child",
    "save_close_preference", "repeated_close", "verify_instance_released", "save_stopped_idle",
    "restart_runtime", "stop_runtime", "cleanup_stop", "remove_temporary_data",
    "child_arguments", "child_construct_bridge", "child_start", "child_save_preferences",
    "child_close",
})
FAILURE_CODES = frozenset({
    "bridge_error", "io_error", "invalid_arguments", "configuration_error",
    "assertion_failed", "timeout", "child_spawn_failed", "child_failed",
    "child_report_invalid", "unexpected_failure", "cleanup_unconfirmed",
})
BRIDGE_CODES = frozenset({
    "configuration_invalid", "configuration_unavailable", "connection_failed", "consumer_busy",
    "credentials_unavailable", "data_directory_unavailable", "desktop_busy", "desktop_closing",
    "execution_timeout", "history_limit", "import_cleanup_unconfirmed", "import_interrupted",
    "instance_unavailable", "invalid_model_source", "invalid_request", "not_initialized",
    "packaged_runtime_missing", "request_cancelled", "request_cleanup_unconfirmed", "request_not_owned",
    "response_invalid", "response_limit", "runtime_running", "runtime_start_failed",
    "runtime_stop_unconfirmed", "settings_durability_unconfirmed", "settings_invalid", "settings_write_failed",
    "slow_consumer", "stream_invalid", "unsafe_file", "unsupported_parameter",
    "unsupported_model", "unsupported_chat_template", "context_length_exceeded", "model_not_found",
    "request_not_found", "model_conflict", "runtime_busy", "already_exists",
    "duplicate_request_id", "consumer_stopped", "queue_full", "queue_timeout",
    "load_timeout", "runtime_faulted", "worker_lost", "runtime_shutdown",
    "model_load_failed", "insufficient_storage", "internal_error", "response_too_large",
    "import_committed_durability_unconfirmed", "api_error", "unrecognized",
})
CLEANUP_STATUSES = frozenset({"not_needed", "confirmed", "unconfirmed"})
LOCK_STATES = frozenset({"free", "held", "unavailable", "not_checked"})
DISCOVERY_STATES = frozenset({"absent", "present", "unavailable", "not_checked"})
PROBE_CODES = frozenset({"observed_pending", "observed_signal", "signal_registration_failed", "spawn_failed", "child_failed",
                         "timeout", "probe_io_failed", "invalid_report", "unsupported_platform"})
SIGNAL_STATES = frozenset({"pending", "received", "error", "not_observed"})


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate report field")
        result[key] = value
    return result


def json_report(raw, limit):
    if not isinstance(raw, str) or len(raw.encode("utf-8")) > limit:
        raise ValueError("report exceeds bounded size")
    try:
        report = json.loads(raw, object_pairs_hook=unique_object,
                            parse_constant=lambda _: (_ for _ in ()).throw(ValueError("non-finite report value")))
    except (ValueError, UnicodeError, RecursionError):
        raise ValueError("report is not strict JSON") from None
    if not isinstance(report, dict):
        raise ValueError("report is not an object")
    return report


def bridge_failure(raw):
    report = json_report(raw, MAX_FAILURE_BYTES)

    def choice(value, values, nullable=False):
        return (nullable and value is None) or (type(value) is str and value in values)

    def integer(value):
        return value is None or (type(value) is int and -(2 ** 31) <= value < 2 ** 31)

    cleanup = report.get("cleanup")
    if (set(report) != FAILURE_KEYS or type(report["schema_version"]) is not int or report["schema_version"] != 1
            or report["kind"] != "nexa-desktop-bridge-acceptance" or report["success"] is not False
            or not choice(report["stage"], FAILURE_STAGES) or not choice(report["code"], FAILURE_CODES)
            or not choice(report["bridge_code"], BRIDGE_CODES, True) or not integer(report["os_error"])
            or not integer(report["runtime_exit_code"])
            or not choice(report["child_stage"], FAILURE_STAGES, True) or not integer(report["child_exit_code"])
            or not isinstance(cleanup, dict) or set(cleanup) != CLEANUP_KEYS):
        raise ValueError("bridge failure report schema rejected")
    if (not choice(cleanup["status"], CLEANUP_STATUSES)
            or not choice(cleanup["code"], FAILURE_CODES, True)
            or not choice(cleanup["bridge_code"], BRIDGE_CODES, True) or not integer(cleanup["os_error"])
            or not choice(cleanup["instance_lock"], LOCK_STATES)
            or not choice(cleanup["discovery"], DISCOVERY_STATES)
            or type(cleanup["temporary_data_retained"]) is not bool):
        raise ValueError("bridge failure cleanup schema rejected")
    return report


def launch_probe_report(raw):
    report = json_report(raw, MAX_FAILURE_BYTES)
    fields = {"schema_version", "kind", "success", "code", "os_error", "spawn_os_error", "child_exit_code",
              "signal_state", "signal_os_error", "cleanup_confirmed"}
    if (set(report) != fields or type(report["schema_version"]) is not int or report["schema_version"] != 1
            or report["kind"] != "nexa-desktop-launch-probe" or type(report["success"]) is not bool
            or type(report["cleanup_confirmed"]) is not bool
            or type(report["code"]) is not str or report["code"] not in PROBE_CODES
            or type(report["signal_state"]) is not str or report["signal_state"] not in SIGNAL_STATES):
        raise ValueError("launch probe report schema rejected")
    for name in ("os_error", "spawn_os_error", "child_exit_code", "signal_os_error"):
        value = report[name]
        if value is not None and (type(value) is not int or not -(2 ** 31) <= value < 2 ** 31):
            raise ValueError("launch probe numeric field rejected")
    if report["success"] and (report["code"] != "observed_pending" or report["signal_state"] != "pending"
                              or report["child_exit_code"] != 0 or not report["cleanup_confirmed"]
                              or any(report[name] is not None for name in ("os_error", "spawn_os_error", "signal_os_error"))):
        raise ValueError("launch probe success declaration rejected")
    return report


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


def checked_json(command, cwd, env, timeout, *, phase, failure_file=None):
    if phase not in {"desktop_diagnose", "bridge_harness", "launch_probe"}:
        raise ValueError("unknown desktop acceptance phase")
    try:
        result = subprocess.run([str(arg) for arg in command], cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                                stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, encoding="utf-8", errors="strict", timeout=timeout, check=False)
    except subprocess.TimeoutExpired:
        raise ValueError(f"{phase} timed out; cleanup not confirmed") from None
    except (OSError, UnicodeError):
        raise ValueError(f"{phase} launch or output decoding failed") from None
    if result.returncode:
        # Never echo stderr, raw JSON or arbitrary messages. A nonzero harness
        # result remains a failure even when its structured evidence is valid.
        if phase == "bridge_harness":
            try:
                report = bridge_failure(result.stdout)
            except (ValueError, UnicodeError):
                raise ValueError("bridge_harness failed; structured failure report rejected") from None
            if failure_file is not None:
                desktop.base.write_json(failure_file, report)
        raise ValueError(f"{phase} process failed: exit {result.returncode}")
    try:
        if phase == "launch_probe":
            return launch_probe_report(result.stdout)
        return json_report(result.stdout, MAX_JSON_BYTES)
    except (ValueError, UnicodeError):
        raise ValueError(f"{phase} success report rejected") from None


def run_launch_probe(harness, evidence):
    # A valid observation is useful even if it records a platform failure. It
    # does not start the product, use credentials, or replace Release acceptance.
    env = desktop.base.windows_environment(os.environ, "")
    if os.name == "nt":
        env["PATH"] = env["SYSTEMROOT"] + r"\System32;" + env["SYSTEMROOT"]
    report = checked_json([harness, "--probe-launch"], desktop.ROOT, env, 20, phase="launch_probe")
    evidence.mkdir(parents=True, exist_ok=True)
    desktop.base.write_json(evidence / "launch-probe.json", report)


def run(archive, model, harness, evidence):
    if os.name != "nt":
        raise ValueError("desktop execution smoke requires Windows")
    expected = archive.with_suffix(".zip.sha256").read_text(encoding="utf-8").strip()
    if expected != f"{desktop.base.digest(archive)}  desktop-windows.zip":
        raise ValueError("desktop archive checksum mismatch")
    evidence.mkdir(parents=True, exist_ok=True)
    report = {"schema_version": 1, "result": "failed", "last_stage": "package_verification", "native_window_tested": False, "failure_directory_policy": "retain after failure for private local diagnosis",
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
            report["last_stage"] = "desktop_diagnose"
            observed = checked_json([package / "nexa-desktop.exe", "--diagnose"], cwd, env, 60, phase="desktop_diagnose")
            # Persist this useful read-only observation even if real bridge fails.
            desktop.base.write_json(evidence / "diagnostics.json", observed)
            if observed.get("package_verified") is not True or not observed.get("webview2_version") or observed.get("project_commit") != manifest["project_commit"] or observed.get("native_window_tested") is not False:
                raise ValueError("desktop executable observations differ from package")
            report["last_stage"] = "bridge_harness"
            bridge = checked_json([harness, "--runtime", package / "runtime/ai-runtime.exe", "--model", local_model], cwd, env, 600,
                                  phase="bridge_harness", failure_file=evidence / "bridge-failure.json")
            # The harness defines its own completed cases. A successful exit with
            # missing pass declaration is never silently promoted to a pass here.
            if bridge.get("success") is not True:
                raise ValueError("desktop real bridge did not report pass")
            desktop.base.write_json(evidence / "bridge-real.json", bridge)
            report["last_stage"] = "package_unchanged"
            desktop.verify(package, manifest)
            shapes = {key: {"contains_non_ascii": any(ord(c) > 127 for c in str(path)), "contains_space": " " in str(path)} for key, path in
                      {"desktop_executable": package / "nexa-desktop.exe", "runtime_executable": package / "runtime/ai-runtime.exe", "source_model": local_model, "working_directory": cwd}.items()}
            if "data_dir_path_shape" not in bridge:
                raise ValueError("real bridge omitted its actual data directory shape")
            shapes["actual_data_directory"] = bridge["data_dir_path_shape"]
            if not all(value["contains_non_ascii"] and value["contains_space"] for value in shapes.values()):
                raise ValueError("desktop Unicode/space path coverage was not observed")
            report.update(result="pass", last_stage="complete", project_commit=manifest["project_commit"], desktop_package_sha256=desktop.base.digest(archive), package_unchanged=True, path_coverage=shapes,
                          webview2_version=observed["webview2_version"], bridge_result="pass")
    finally:
        desktop.base.write_json(evidence / "acceptance.json", report)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, default=desktop.ROOT / "dist/desktop-windows.zip")
    parser.add_argument("--model", type=Path)
    parser.add_argument("--probe-launch", action="store_true", help="record bounded launch observations only; no product runtime or model")
    parser.add_argument("--harness", type=Path, required=True)
    parser.add_argument("--out", type=Path, default=desktop.ROOT / "artifacts/verification/windows-desktop")
    args = parser.parse_args()
    if args.probe_launch:
        if args.model is not None:
            parser.error("launch probe does not accept a model")
        run_launch_probe(args.harness.absolute(), args.out.absolute())
        return
    if args.model is None:
        parser.error("desktop package acceptance requires --model")
    run(args.archive.absolute(), args.model.absolute(), args.harness.absolute(), args.out.absolute())

if __name__ == "__main__":
    main()
