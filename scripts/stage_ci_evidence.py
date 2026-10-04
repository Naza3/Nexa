#!/usr/bin/env python3
"""Stage a closed, reviewed CI evidence inventory; never upload raw run trees.

This is not a general secret scrubber. New reports must be reviewed and explicitly
added here. Model inputs, generated text, credentials, runtime data, arbitrary
logs and binary/PDB files are excluded even when adjacent to allowed reports.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import tempfile
from run_desktop_smoke import DIAGNOSTIC_REPORTS, PROBE_REPORTS, bridge_failure, external_library_report, launch_probe_report, startup_diagnostic

JSON_REPORTS = (
    "windows-baseline.json", "windows-catalog-download.json", "windows-aria2-policy.json", "windows-native-smoke.json", "upstream-bench.json",
    "upstream-processes.json", "native-build-manifest.json",
    "windows-api-cli/api-smoke.json", "windows-api-cli/lifecycle.json",
    "windows-api-cli/cli-init.json", "windows-api-cli/cli-import.json",
    "windows-package/build-result.json", "windows-package/pe-inspection.json",
    "windows-package/package-acceptance.json",
    "windows-desktop/build-result.json", "windows-desktop/pe-inspection.json",
    *("windows-desktop/" + name for name in DIAGNOSTIC_REPORTS), "windows-desktop/bridge-real.json",
    "windows-desktop/bridge-failure.json",
    "windows-desktop/launch-probe-breakaway.json", "windows-desktop/launch-probe-inherit-job.json",
    "windows-desktop/acceptance.json",
)
LOG_REPORTS = tuple(f"windows-{name}.log" for name in (
    "rust-setup", "cmake-setup", "toolchain-identity", "upstream-runner-tests", "process-host-dependencies",
    "management-build", "api-cli-dependencies", "api-cli-independent-build", "configure",
    "native-build", "native-identity-tests", "ctest", "rustfmt", "rust-tests", "clippy", "real-model",
    "real-runtime", "worker-real-credit", "real-process-runtime",
    "desktop-dependencies", "desktop-rust-tests", "desktop-clippy", "desktop-build", "desktop-source-status", "desktop-transport-tests",
    "desktop-external-store-tests", "desktop-external-guard-tests", "desktop-mapping-observation", "desktop-discovery-download-tests",
))
# These files are never copied, but their byte identities keep synthetic baseline
# validation auditable when generated text and raw upstream stderr are omitted.
HASH_ONLY = (
    "upstream-zh.txt", "upstream-zh.log", "upstream-bench.log",
    "diagnostic-threads-1.txt", "diagnostic-threads-1.log",
    "diagnostic-threads-4.txt", "diagnostic-threads-4.log",
)
FORBIDDEN_FIELDS = frozenset((
    "token", "api_token", "api-token", "password", "secret", "credentials",
    "authorization", "bearer", "messages", "content", "prompt_text",
    "generated_text", "response_body", "completion_text", "text",
    "failed_file_name",
))
SECRET_VALUE = re.compile(
    r"(?i)(?:\bbearer\s+[A-Za-z0-9_+/=.-]{8,}|"
    r"(?:api[-_]token|password|authorization|secret)\s*[=:]\s*[\"']?[A-Za-z0-9_+/=.-]{16,}|"
    r"-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----)"
)
DRIVE_PATH = re.compile(r"(?i)\b[A-Z]:[\\/][^\s\"<>|]*")
MAX_REPORT_BYTES = 16 * 1024 * 1024


def catalog_download_report(value):
    """The live download proof never carries URLs, paths or raw error details."""
    common = {"schema_version", "success", "source", "catalog_id"}
    if not isinstance(value, dict) or type(value.get("schema_version")) is not int or value["schema_version"] != 1:
        raise ValueError("invalid catalog download report")
    if value.get("source") != "modelscope" or value.get("catalog_id") != "qwen3-0.6b-q8-0":
        raise ValueError("invalid catalog download identity")
    if value.get("success") is False:
        keys = common | {"error"}
        if set(value) not in (keys, keys | {"stage", "code"}) or value["error"] != "catalog_download_verification_failed":
            raise ValueError("invalid catalog download failure")
        if "stage" in value:
            stages = {"platform", "prepare", "admit_directory", "scan", "download_start", "download_poll", "verify_file", "close", "cleanup"}
            codes = {"unsupported_platform", "io", "already_exists", "desktop_busy", "desktop_closing", "runtime_running", "runtime_stop_unconfirmed", "verification_failed",
                     "model_directory_required", "model_directory_unavailable", "model_directory_unsupported", "model_file_changed", "model_file_unavailable", "model_file_in_use",
                     *("model_download_" + code for code in ("engine_unavailable", "network_failed", "http_failed", "redirect_rejected", "identity_mismatch", "size_mismatch", "write_failed", "incomplete", "cancelled", "timeout", "cleanup_unconfirmed"))}
            if not isinstance(value["stage"], str) or value["stage"] not in stages or not isinstance(value["code"], str) or value["code"] not in codes:
                raise ValueError("invalid catalog download failure code")
        return
    expected = common | {"source_revision", "size_bytes", "sha256", "downloaded_bytes", "published", "registered", "elapsed_ms"}
    if value.get("success") is not True or set(value) != expected:
        raise ValueError("invalid catalog download fields")
    if (not isinstance(value["source_revision"], str) or not re.fullmatch(r"[0-9a-f]{40}", value["source_revision"])
            or not isinstance(value["sha256"], str) or not re.fullmatch(r"[0-9a-f]{64}", value["sha256"])):
        raise ValueError("invalid catalog download hashes")
    if (type(value["size_bytes"]) is not int or not 4 <= value["size_bytes"] <= 16 * 1024**3
            or type(value["downloaded_bytes"]) is not int or value["downloaded_bytes"] != value["size_bytes"]
            or type(value["elapsed_ms"]) is not int or not 0 <= value["elapsed_ms"] <= 2 * 60 * 60 * 1000
            or value["published"] is not True or value["registered"] is not True):
        raise ValueError("invalid catalog download completion")


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def read_regular(path):
    for component in (path, *path.parents):
        info = component.lstat()
        if component.is_symlink() or getattr(info, "st_file_attributes", 0) & 0x400:
            raise ValueError("evidence symlink/reparse point rejected")
    if not path.is_file():
        raise ValueError("evidence source is not an ordinary file")
    if path.stat().st_size > MAX_REPORT_BYTES:
        raise ValueError("evidence report exceeds bounded size")
    # The file can grow after stat. Bound the actual read as well, reserving one
    # extra byte to distinguish an exact-limit report from a growing one.
    with path.open("rb") as source:
        data = source.read(MAX_REPORT_BYTES + 1)
    if len(data) > MAX_REPORT_BYTES:
        raise ValueError("evidence report exceeds bounded size")
    if data.startswith((b"MZ", b"GGUF", b"Microsoft C/C++ MSF")):
        raise ValueError("binary/model/symbol content rejected")
    return data


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate report JSON key")
        result[key] = value
    return result


def scrub(text, roots):
    if SECRET_VALUE.search(text):
        raise ValueError("possible credential content in reviewed report; manual review required")
    for root in sorted(roots, key=len, reverse=True):
        for variant in {root, root.replace("\\", "/"), root.replace("/", "\\")}:
            text = re.sub(re.escape(variant), "<local-path>", text, flags=re.IGNORECASE)
    # Only path sanitation is inferred. Unknown content is not claimed safe by
    # this regex; safety depends on the reviewed inventory and JSON field gate.
    return DRIVE_PATH.sub("<local-path>", text)


def clean(value, roots):
    if isinstance(value, str):
        return scrub(value, roots)
    if isinstance(value, list):
        return [clean(item, roots) for item in value]
    if isinstance(value, dict):
        if any(key.lower() in FORBIDDEN_FIELDS for key in value):
            raise ValueError("body/credential field rejected from evidence report")
        return {key: clean(item, roots) for key, item in value.items()}
    return value


def stage(source, destination, repo, environment=None):
    source, destination, repo = Path(source).absolute(), Path(destination).absolute(), Path(repo).absolute()
    env = dict(os.environ if environment is None else environment)
    roots = {str(source), str(repo)} | {
        env[key] for key in ("GITHUB_WORKSPACE", "RUNNER_TEMP", "USERPROFILE", "CARGO_HOME", "RUSTUP_HOME") if env.get(key)
    }
    if destination.exists():
        raise ValueError("evidence destination already exists; never merge stale reports")
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".evidence-stage-", dir=destination.parent) as temporary:
        temp = Path(temporary)
        records, missing, omitted, rejected = [], [], [], []
        for name in JSON_REPORTS + LOG_REPORTS:
            path = source / name
            if not path.exists() and not path.is_symlink():
                missing.append(name)
                continue
            try:
                raw = read_regular(path)
                text = raw.decode("utf-8-sig")
                if name.endswith(".json"):
                    if name == "windows-desktop/bridge-failure.json":
                        bridge_failure(text)
                    if name in {"windows-desktop/" + file for file in DIAGNOSTIC_REPORTS}:
                        startup_diagnostic(text)
                    for strategy, filename in PROBE_REPORTS.items():
                        if name == "windows-desktop/" + filename:
                            launch_probe_report(text, expected_strategy=strategy)
                    value = json.loads(text, object_pairs_hook=unique_object, parse_constant=lambda _: (_ for _ in ()).throw(ValueError("non-finite JSON value")))
                    if name == "windows-catalog-download.json":
                        catalog_download_report(value)
                    if name == "windows-desktop/bridge-real.json":
                        external_library_report(value.get("external_library") if isinstance(value, dict) else None)
                    output = (json.dumps(clean(value, roots), ensure_ascii=False, indent=2, allow_nan=False) + "\n").encode("utf-8")
                else:
                    output = scrub(text, roots).encode("utf-8")
            except (ValueError, OSError, UnicodeError):
                # Never echo an offending value, raw exception or local path.
                # Other reviewed reports and the CI step outcomes remain visible.
                rejected.append({"path": name, "reason": "report_policy_or_format_rejected", "body_uploaded": False})
                continue
            target = temp / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(output)
            records.append({"path": name, "source_sha256": sha256(raw), "sha256": sha256(output), "source_size_bytes": len(raw), "size_bytes": len(output)})
        for name in HASH_ONLY:
            path = source / name
            if path.exists() or path.is_symlink():
                try:
                    raw = read_regular(path)
                    omitted.append({"path": name, "sha256": sha256(raw), "size_bytes": len(raw), "body_uploaded": False})
                except (ValueError, OSError):
                    rejected.append({"path": name, "reason": "hash_only_source_rejected", "body_uploaded": False})
        fixtures = []
        for name in ("tests/fixtures/baseline.json", "tests/fixtures/upstream-prompt-zh.txt", ".github/workflows/native-windows.yml", "crates/desktop-bridge/src/model-catalog.json"):
            path = repo / name
            if path.exists():
                raw = read_regular(path)
                fixtures.append({"path": name, "sha256": sha256(raw), "size_bytes": len(raw)})
        outcomes = {key.removeprefix("NEXA_CI_").lower(): value for key, value in env.items()
                    if key in ("NEXA_CI_NATIVE_BUILD", "NEXA_CI_MODEL_IDENTITY", "NEXA_CI_PORTABLE_PACKAGE", "NEXA_CI_PACKAGE_ACCEPTANCE", "NEXA_CI_DESKTOP_BUILD", "NEXA_CI_DESKTOP_PACKAGE", "NEXA_CI_DESKTOP_ACCEPTANCE")
                    and value in ("success", "failure", "cancelled", "skipped")}
        index = {"schema_version": 1, "project_commit": env.get("GITHUB_SHA"), "sanitized": True,
                 "result": "failed" if rejected else "pass", "ci_step_outcomes": outcomes, "rejected_reports": rejected,
                 "scope": "explicit reviewed report inventory; not a general secret scrubber",
                 "omitted_categories": ["generated text and raw upstream stderr bodies", "raw service logs", "models", "runtime data and credentials", "private native build identities", "unlisted files and binaries"],
                 "files": records, "missing_reports": missing, "hash_only_synthetic_outputs": omitted, "source_fixtures": fixtures}
        (temp / "evidence-index.json").write_text(json.dumps(index, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        if rejected:
            (temp / "staging-failure.json").write_text(json.dumps({"schema_version": 1, "status": "failed", "error": "evidence_policy_or_format_check_failed", "rejected_reports": rejected, "ci_step_outcomes": outcomes}, indent=2) + "\n", encoding="utf-8")
        # Rejected files never reach the upload tree. Publish reviewed reports
        # plus a safe staging failure report rather than losing prior outcomes.
        shutil.move(str(temp), destination)
    return index


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, default=Path("artifacts/verification"))
    parser.add_argument("--out", type=Path, default=Path("artifacts/upload-evidence"))
    parser.add_argument("--repo", type=Path, default=Path.cwd())
    args = parser.parse_args()
    result = stage(args.source, args.out, args.repo)
    if result["result"] != "pass":
        raise SystemExit(1)


if __name__ == "__main__":
    main()
