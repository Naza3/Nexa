#!/usr/bin/env python3
"""Verify a freshly extracted desktop package and its actual bridge, not UI claims."""
from __future__ import annotations
import argparse
from contextlib import contextmanager
import json
import os
from pathlib import Path
import re
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
    "external_apply", "external_rescan", "external_start", "external_list", "external_direct_chat",
    "external_cancel_prepare", "external_unload", "external_stop", "external_preexisting_writer",
    "external_source_changed", "external_final_stop",
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
    "model_directory_required", "model_directory_unavailable", "model_directory_unsupported",
    "model_library_unsupported", "model_library_limit", "model_library_changed", "model_list_changed",
    "model_scan_timeout", "model_scan_cancelled", "model_file_changed", "model_file_unavailable",
    "model_file_in_use", "model_library_write_failed",
    "model_load_interrupted",
    "invalid_manifest", "invalid_argument",
})
CLEANUP_STATUSES = frozenset({"not_needed", "confirmed", "unconfirmed"})
LOCK_STATES = frozenset({"free", "held", "unavailable", "not_checked"})
DISCOVERY_STATES = frozenset({"absent", "present", "unavailable", "not_checked"})
PROBE_CODES = frozenset({"observed_pending", "observed_signal", "signal_registration_failed", "spawn_failed", "child_failed",
                         "timeout", "probe_io_failed", "invalid_report", "unsupported_platform"})
SIGNAL_STATES = frozenset({"pending", "received", "error", "not_observed"})
LAUNCH_STRATEGIES = frozenset({"breakaway", "inherit_job"})
PROBE_KEYS = frozenset({"schema_version", "kind", "success", "code", "os_error", "spawn_os_error", "child_exit_code",
                        "signal_state", "signal_os_error", "cleanup_confirmed", "strategy", "parent_in_job", "child_in_job",
                        "parent_job_os_error", "child_job_os_error"})
PROBE_REPORTS = {"breakaway": "launch-probe-breakaway.json", "inherit_job": "launch-probe-inherit-job.json"}
DIAGNOSTIC_REPORTS = ("diagnostics.json", "diagnostics-root-model.json", "diagnostics-model-directory.json",
                      "diagnostics-models-directory.json", "diagnostics-unlisted-dll.json", "diagnostics-tampered-manifest.json")
DIAGNOSTIC_KEYS = frozenset({"schema_version", "package_verified", "package_error_code", "project_commit", "project_dirty", "webview2_version", "native_window_tested"})
PACKAGE_ERROR_CODES = frozenset({
    "current_executable_unavailable", "desktop_executable_name_invalid", "package_root_unavailable",
    "selected_path_invalid", "selected_path_indirect", "selected_file_invalid", "selected_file_unavailable",
    "package_directory_unavailable", "package_file_unavailable", "package_manifest_unavailable",
    "package_manifest_too_large", "package_manifest_invalid", "package_identity_invalid", "package_inventory_invalid",
    "package_path_invalid", "package_indirect_path", "package_file_hash_mismatch", "package_checksum_invalid",
    "package_checksum_mismatch", "package_unlisted_file", "package_external_model_header_invalid",
    "runtime_source_identity_mismatch", "package_validation_failed",
})
EXTERNAL_LIBRARY_KEYS = frozenset({
    "supported", "catalog_registered_without_copy", "source_unchanged", "automatic_display_name",
    "stable_id_rescan", "legacy_managed_preserved", "effective_directory_verified", "direct_chat_prepared",
    "owned_preparation_cancelled", "write_access_blocked_while_loaded", "delete_access_blocked_while_loaded",
    "guard_retained_after_unload", "guard_released_after_stop", "preexisting_writer_rejected",
    "failed_preparation_updates_list", "changed_identity_rejected",
})


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


def launch_probe_report(raw, expected_strategy=None):
    report = json_report(raw, MAX_FAILURE_BYTES)
    if (set(report) != PROBE_KEYS or type(report["schema_version"]) is not int or report["schema_version"] != 2
            or report["kind"] != "nexa-desktop-launch-probe" or type(report["success"]) is not bool
            or type(report["cleanup_confirmed"]) is not bool
            or type(report["code"]) is not str or report["code"] not in PROBE_CODES
            or type(report["signal_state"]) is not str or report["signal_state"] not in SIGNAL_STATES
            or type(report["strategy"]) is not str or report["strategy"] not in LAUNCH_STRATEGIES
            or (expected_strategy is not None and report["strategy"] != expected_strategy)):
        raise ValueError("launch probe report schema rejected")
    for name in ("os_error", "spawn_os_error", "child_exit_code", "signal_os_error", "parent_job_os_error", "child_job_os_error"):
        value = report[name]
        if value is not None and (type(value) is not int or not -(2 ** 31) <= value < 2 ** 31):
            raise ValueError("launch probe numeric field rejected")
    for role in ("parent", "child"):
        value = report[role + "_in_job"]
        if value is not None and (type(value) is not bool or report[role + "_job_os_error"] is not None):
            raise ValueError("launch probe job observation rejected")
    if report["success"] and (report["code"] != "observed_pending" or report["signal_state"] != "pending"
                              or report["child_exit_code"] != 0 or not report["cleanup_confirmed"]
                              or any(report[name] is not None for name in ("os_error", "spawn_os_error", "signal_os_error", "parent_job_os_error", "child_job_os_error"))
                              or report["parent_in_job"] is None or report["child_in_job"] is None):
        raise ValueError("launch probe success declaration rejected")
    return report


def startup_diagnostic(raw):
    report = json_report(raw, MAX_FAILURE_BYTES)
    if (set(report) != DIAGNOSTIC_KEYS or type(report["schema_version"]) is not int or report["schema_version"] != 2
            or type(report["package_verified"]) is not bool or report["native_window_tested"] is not False):
        raise ValueError("desktop diagnostic schema rejected")
    if report["package_verified"]:
        if (report["package_error_code"] is not None or type(report["project_commit"]) is not str
                or re.fullmatch(r"[0-9a-f]{40}", report["project_commit"]) is None
                or type(report["project_dirty"]) is not bool):
            raise ValueError("desktop diagnostic identity rejected")
    elif (type(report["package_error_code"]) is not str or report["package_error_code"] not in PACKAGE_ERROR_CODES
          or report["project_commit"] is not None or report["project_dirty"] is not None):
        raise ValueError("desktop diagnostic failure rejected")
    version = report["webview2_version"]
    if version is not None and (type(version) is not str or len(version) > 128
                               or re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+(?: (?:beta|dev|canary))?", version) is None):
        raise ValueError("desktop diagnostic WebView version rejected")
    return report


def external_library_report(value, *, require_windows=False):
    if (type(value) is not dict or set(value) != EXTERNAL_LIBRARY_KEYS
            or any(type(item) is not bool for item in value.values())):
        raise ValueError("external library acceptance schema rejected")
    if require_windows and not all(value.values()):
        raise ValueError("Windows external library acceptance incomplete")
    return value


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


class ChildExitUnconfirmed(Exception):
    """Only the verifier-owned direct child was targeted; reap was not proven."""


def bounded_process(command, cwd, env, timeout):
    # A descendant may keep an inherited stdout writer after the direct child
    # exits. A pipe/communicate (including run's Windows timeout cleanup) can
    # therefore wait forever for EOF. Regular-file reads never await pipe EOF.
    with tempfile.TemporaryFile(mode="w+b") as output:
        process = subprocess.Popen(command, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                                   stdout=output, stderr=subprocess.DEVNULL, close_fds=True)
        try:
            process.wait(timeout=timeout)
        except (subprocess.TimeoutExpired, OSError):
            # Terminate only this owned verifier child, never a discovered PID
            # or its runtime. Never use Popen's context-manager unbounded wait.
            try:
                process.kill()
            except OSError:
                pass
            try:
                process.wait(timeout=1)
            except (subprocess.TimeoutExpired, OSError):
                raise ChildExitUnconfirmed() from None
            raise
        output.seek(0)
        # Success reports may be larger than failures. The caller applies the
        # stricter 4 KiB failure/probe policy before parsing or saving them.
        raw = output.read(MAX_JSON_BYTES + 1)
    if len(raw) > MAX_JSON_BYTES:
        raise ValueError("process report exceeds bounded size")
    return subprocess.CompletedProcess(command, process.returncode, raw.decode("utf-8", errors="strict"))


def checked_json(command, cwd, env, timeout, *, phase, failure_file=None, diagnostic_file=None):
    if phase not in {"desktop_diagnose", "bridge_harness", "launch_probe"}:
        raise ValueError("unknown desktop acceptance phase")
    try:
        result = bounded_process([str(arg) for arg in command], cwd, env, timeout)
    except ChildExitUnconfirmed:
        raise ValueError(f"{phase} owned child exit unconfirmed; cleanup not confirmed") from None
    except subprocess.TimeoutExpired:
        raise ValueError(f"{phase} timed out; cleanup not confirmed") from None
    except (OSError, UnicodeError):
        raise ValueError(f"{phase} launch or output decoding failed") from None
    except ValueError:
        raise ValueError(f"{phase} report exceeded bounded size") from None
    if phase == "desktop_diagnose":
        try:
            report = startup_diagnostic(result.stdout)
        except (ValueError, UnicodeError):
            raise ValueError("desktop_diagnose structured diagnostic report rejected") from None
        if diagnostic_file is not None:
            desktop.base.write_json(diagnostic_file, report)
        if result.returncode:
            raise ValueError(f"desktop_diagnose process failed: exit {result.returncode}")
        if not report["package_verified"] or report["webview2_version"] is None:
            raise ValueError("desktop_diagnose success declaration rejected")
        return report
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


def expect_package_rejection(executable, cwd, env, expected_code, diagnostic_file):
    try:
        result = bounded_process([str(executable), "--diagnose"], cwd, env, 60)
        report = startup_diagnostic(result.stdout)
    except (ChildExitUnconfirmed, OSError, subprocess.TimeoutExpired, ValueError, UnicodeError):
        raise ValueError("negative desktop input diagnostic did not return a valid bounded report") from None
    desktop.base.write_json(diagnostic_file, report)
    if result.returncode != 1 or report["package_verified"] or report["package_error_code"] != expected_code:
        raise ValueError("negative desktop input diagnostic did not reject the expected condition")


def package_input_checks(package, model, cwd, env, evidence, manifest, checks):
    """Exercise the actual EXE; mutate only a newly extracted CI-owned package."""
    executable = package / "nexa-desktop.exe"
    root_model = package / "验收 同级外置模型.GGUF"
    owned_model = root_model
    owned_directories = []
    extra_dll = package / "nexa-test-unlisted.dll"
    # Exclusive creation: never overwrite an existing input or payload file.
    model_created = False
    try:
        with root_model.open("xb") as destination:
            model_created = True
            with desktop.base.regular(model).open("rb") as source:
                shutil.copyfileobj(source, destination)
        observed = checked_json([executable, "--diagnose"], cwd, env, 60, phase="desktop_diagnose",
                                diagnostic_file=evidence / "diagnostics-root-model.json")
        if observed["project_commit"] != manifest["project_commit"] or observed["project_dirty"] != manifest["project_dirty"]:
            raise ValueError("same-directory model changed package identity")
        checks["root_gguf_accepted"] = True

        for directory_name in ("model", "models"):
            directory = package / directory_name
            directory.mkdir()  # Exclusive: never adopt a preexisting directory.
            owned_directories.append(directory)
            target = directory / "验收 中文 模型.GGUF"
            owned_model.rename(target)
            owned_model = target
            observed = checked_json([executable, "--diagnose"], cwd, env, 60, phase="desktop_diagnose",
                                    diagnostic_file=evidence / f"diagnostics-{directory_name}-directory.json")
            if observed["project_commit"] != manifest["project_commit"] or observed["project_dirty"] != manifest["project_dirty"]:
                raise ValueError("program-side model directory changed package identity")
            checks[directory_name + "_directory_gguf_accepted"] = True
            owned_model.rename(root_model)
            owned_model = root_model
            directory.rmdir()
            owned_directories.pop()

        dll_created = False
        try:
            with extra_dll.open("xb") as destination:
                dll_created = True
                destination.write(b"MZsynthetic unlisted file; never executed")
            expect_package_rejection(executable, cwd, env, "package_unlisted_file",
                                     evidence / "diagnostics-unlisted-dll.json")
            checks["unlisted_dll_rejected"] = True
        finally:
            if dll_created:
                extra_dll.unlink()

        manifest_file = desktop.base.regular(package / "manifest.json")
        original = manifest_file.read_bytes()
        try:
            # Keep JSON valid, but change the declared manifest byte identity.
            manifest_file.write_bytes(original + b"\n ")
            expect_package_rejection(executable, cwd, env, "package_checksum_mismatch",
                                     evidence / "diagnostics-tampered-manifest.json")
            checks["tampered_manifest_rejected"] = True
        finally:
            manifest_file.write_bytes(original)
    finally:
        if model_created:
            owned_model.unlink()
            for directory in reversed(owned_directories):
                directory.rmdir()
            checks["owned_model_input_removed"] = True
    # Shipping verification remains exact: the external input must not enter
    # manifests, checksums, licenses, size accounting, or the uploaded ZIP.
    desktop.verify(package, manifest)
    checks["package_payload_restored"] = True


def run_launch_probe(harness, evidence, strategy="breakaway"):
    # A valid observation is useful even if it records a platform failure. It
    # does not start the product, use credentials, or replace Release acceptance.
    if strategy not in LAUNCH_STRATEGIES:
        raise ValueError("unknown launch probe strategy")
    env = desktop.base.windows_environment(os.environ, "")
    if os.name == "nt":
        env["PATH"] = env["SYSTEMROOT"] + r"\System32;" + env["SYSTEMROOT"]
    report = checked_json([harness, "--probe-launch", strategy], desktop.ROOT, env, 20, phase="launch_probe")
    launch_probe_report(json.dumps(report), expected_strategy=strategy)
    evidence.mkdir(parents=True, exist_ok=True)
    desktop.base.write_json(evidence / PROBE_REPORTS[strategy], report)


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
            observed = checked_json([package / "nexa-desktop.exe", "--diagnose"], cwd, env, 60, phase="desktop_diagnose",
                                    diagnostic_file=evidence / "diagnostics.json")
            # The checked diagnostic is persisted on both success and failure.
            if observed.get("package_verified") is not True or not observed.get("webview2_version") or observed.get("project_commit") != manifest["project_commit"] or observed.get("native_window_tested") is not False:
                raise ValueError("desktop executable observations differ from package")
            report["last_stage"] = "desktop_input_compatibility"
            report["input_compatibility"] = {name: False for name in (
                "root_gguf_accepted", "model_directory_gguf_accepted", "models_directory_gguf_accepted",
                "unlisted_dll_rejected", "tampered_manifest_rejected",
                "owned_model_input_removed", "package_payload_restored")}
            package_input_checks(package, local_model, cwd, env, evidence, manifest, report["input_compatibility"])
            report["last_stage"] = "bridge_harness"
            bridge = checked_json([harness, "--runtime", package / "runtime/ai-runtime.exe", "--model", local_model], cwd, env, 600,
                                  phase="bridge_harness", failure_file=evidence / "bridge-failure.json")
            # The harness defines its own completed cases. A successful exit with
            # missing pass declaration is never silently promoted to a pass here.
            if bridge.get("success") is not True:
                raise ValueError("desktop real bridge did not report pass")
            # An old managed-only harness or unsupported-platform observation
            # cannot stand in for the real Windows external-source assertions.
            external_library_report(bridge.get("external_library"))
            desktop.base.write_json(evidence / "bridge-real.json", bridge)
            external_library_report(bridge["external_library"], require_windows=True)
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
    parser.add_argument("--probe-launch", nargs="?", const="breakaway", choices=sorted(LAUNCH_STRATEGIES), help="record bounded launch observations only; no product runtime or model")
    parser.add_argument("--harness", type=Path, required=True)
    parser.add_argument("--out", type=Path, default=desktop.ROOT / "artifacts/verification/windows-desktop")
    args = parser.parse_args()
    if args.probe_launch:
        if args.model is not None:
            parser.error("launch probe does not accept a model")
        run_launch_probe(args.harness.absolute(), args.out.absolute(), args.probe_launch)
        return
    if args.model is None:
        parser.error("desktop package acceptance requires --model")
    run(args.archive.absolute(), args.model.absolute(), args.harness.absolute(), args.out.absolute())

if __name__ == "__main__":
    main()
