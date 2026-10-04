#!/usr/bin/env python3
"""Explicit T07-A CI setup/verification; no network is performed by the build."""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import importlib.metadata
import stat
import subprocess
import urllib.request
import zipfile

import run_probe as probe

NDK = {
    "url": "https://dl.google.com/android/repository/android-ndk-r30-linux.zip",
    "size": 738633529,
    "sha1": "5107f898313790e449e87eee2183d9a20602dee9",
    "sha256": "753611f410d002cfcd3f3dc2ef49aad532089d3180b436c060a90bf0fcb64df2",
    "revision": "30.0.16248370",
}
CASES = ("short-en", "short-zh", "multiturn", "repeat", "boundary-equal", "boundary-over")
STEPS = ("tools", "inputs", "linux_build", "linux_suite", "android_build", "elf")


def write(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def download(url, destination, size, sha256, sha1=None):
    """Only invoked in the explicit setup step; file is committed after identity checks."""
    if destination.exists():
        raise ValueError("download_destination_exists")
    partial = destination.with_suffix(destination.suffix + ".partial")
    h256, h1, total = hashlib.sha256(), hashlib.sha1(), 0
    with urllib.request.urlopen(url, timeout=60) as response, partial.open("xb") as out:
        while block := response.read(1024 * 1024):
            total += len(block)
            if total > size:
                raise ValueError("download_size_exceeded")
            h256.update(block)
            h1.update(block)
            out.write(block)
    if total != size or h256.hexdigest() != sha256 or (sha1 and h1.hexdigest() != sha1):
        raise ValueError("download_identity_mismatch")
    partial.rename(destination)


def safe_member(name):
    path = PurePosixPath(name)
    if path.is_absolute() or not path.parts or path.parts[0] != "android-ndk-r30" or ".." in path.parts or "\\" in name:
        raise ValueError("unsafe_ndk_archive_member")
    return path


def extract_ndk(archive, parent):
    root = parent / "android-ndk-r30"
    if root.exists():
        raise ValueError("ndk_destination_exists")
    with zipfile.ZipFile(archive) as bundle:
        entries = bundle.infolist()
        paths = [safe_member(entry.filename) for entry in entries]
        if len(set(paths)) != len(paths):
            raise ValueError("duplicate_ndk_archive_member")
        links = {safe_member(entry.filename) for entry in entries if stat.S_ISLNK(entry.external_attr >> 16)}
        for relative in paths:
            if any(ancestor in links for ancestor in relative.parents):
                raise ValueError("ndk_archive_symlink_ancestor")
        # No writes may traverse an archive symlink. Ordinary files come first;
        # complete link-graph containment is checked before this function returns.
        for entry in entries:
            relative = safe_member(entry.filename)
            if relative in links:
                target = bundle.read(entry).decode("utf-8")
                resolved = (parent / str(relative)).parent.joinpath(target).resolve()
                if not resolved.is_relative_to(root.resolve()):
                    raise ValueError("unsafe_ndk_archive_symlink")
        for entry in sorted(entries, key=lambda entry: stat.S_ISLNK(entry.external_attr >> 16)):
            relative = safe_member(entry.filename)
            destination = parent / str(relative)
            mode = entry.external_attr >> 16
            if entry.is_dir():
                destination.mkdir(parents=True, exist_ok=True)
            else:
                destination.parent.mkdir(parents=True, exist_ok=True)
                if stat.S_ISLNK(mode):
                    destination.symlink_to(bundle.read(entry).decode("utf-8"))
                else:
                    with bundle.open(entry) as src, destination.open("xb") as dst:
                        shutil.copyfileobj(src, dst, 1024 * 1024)
                    destination.chmod(mode & 0o777)
        for relative in links:
            try:
                resolved = (parent / str(relative)).resolve(strict=True)
            except (OSError, RuntimeError) as error:
                raise ValueError("invalid_ndk_archive_symlink_graph") from error
            if not resolved.is_relative_to(root.resolve()):
                raise ValueError("unsafe_ndk_archive_symlink_graph")
    properties = (root / "source.properties").read_text(encoding="utf-8")
    if not re.search(r"^Pkg\.Revision\s*=\s*" + re.escape(NDK["revision"]) + r"\s*$", properties, re.M):
        raise ValueError("ndk_revision_mismatch")
    return root


def tool_versions(report):
    packages = {name: importlib.metadata.version(name) for name in ("cmake", "ninja")}
    if packages != {"cmake": "4.4.3", "ninja": "1.13.2"}:
        raise ValueError("build_tool_package_version_mismatch")
    result = {"status": "ok", "cmake_package": packages["cmake"], "ninja_package": packages["ninja"]}
    for name in packages:
        version = subprocess.run([name, "--version"], check=True, capture_output=True, text=True, timeout=15).stdout.splitlines()[0]
        result[name + "_binary"] = version
    write(report, result)


def source_identity():
    root = Path(__file__).resolve().parents[2]
    def git(*args):
        return subprocess.run(["git", "-C", str(root), *args], check=True, capture_output=True, text=True, timeout=15).stdout.strip()
    commit, tree = git("rev-parse", "HEAD"), git("rev-parse", "HEAD^{tree}")
    if not re.fullmatch(r"[0-9a-f]{40}", commit) or not re.fullmatch(r"[0-9a-f]{40}", tree):
        raise ValueError("invalid_source_identity")
    if "GITHUB_SHA" in os.environ and os.environ["GITHUB_SHA"] != commit:
        raise ValueError("ci_source_commit_mismatch")
    return {"source_commit": commit, "source_tree": tree,
            "source_clean": not bool(git("status", "--porcelain", "--untracked-files=all"))}


def setup(destination, report):
    destination.mkdir(parents=True, exist_ok=False)
    result = {"status": "failed", "ndk": NDK, "network_step": "explicit-public-input-setup"}
    try:
        archive = destination / "android-ndk-r30-linux.zip"
        download(NDK["url"], archive, NDK["size"], NDK["sha256"], NDK["sha1"])
        extract_ndk(archive, destination)
        lock = probe.load_json(probe.LOCK)
        model = destination / "model"
        model.mkdir()
        for name, item in lock["files"].items():
            url = "https://huggingface.co/" + lock["repository"] + "/resolve/" + lock["revision"] + "/" + name
            download(url, model / name, item["size"], item["sha256"])
        probe.validate_model(model, lock)
        result.update({"status": "ok", "model_revision": lock["revision"], "model_files": lock["files"]})
    finally:
        write(report, result)


def suite(binary, model, out):
    out.mkdir(parents=True, exist_ok=False)
    summary = {"status": "failed", "android_run": False, "cases": {}}
    try:
        for case in CASES:
            fixture = case if case in ("short-en", "short-zh", "multiturn") else "short-en"
            context = 36 if case == "boundary-equal" else 35 if case == "boundary-over" else 512
            tokens = 8 if case.startswith("boundary") else 16
            code = probe.main(["--probe", str(binary), "--model-dir", str(model),
                "--out-dir", str(out / case), "--case", fixture, "--threads", "2",
                "--logical-context", str(context), "--max-new-tokens", str(tokens), "--timeout-seconds", "180"])
            if case == "boundary-over":
                log = (out / case / "private-native.log").read_text(encoding="utf-8")
                passed = code == 1 and "nexa_mnn_probe_error: logical_context_exceeded" in log and not (out / case / "synthetic-result.json").exists()
            else:
                passed = code == 0
            summary["cases"][case] = "pass" if passed else "fail"
            if not passed:
                raise ValueError("synthetic_case_failed")
        first = probe.load_json(out / "short-en" / "synthetic-result.json")
        again = probe.load_json(out / "repeat" / "synthetic-result.json")
        identical = all(first[key] == again[key] for key in ("prompt_tokens", "output_tokens", "rendered_prompt", "output_text"))
        summary["repeat_identical"] = identical
        if not identical:
            raise ValueError("greedy_repeat_mismatch")
        summary["status"] = "ok"
    finally:
        write(out / "suite.json", summary)


def parse_elf(text):
    if not re.search(r"Class:\s+ELF64", text) or not re.search(r"Machine:\s+AArch64", text) or not re.search(r"Type:\s+DYN", text):
        raise ValueError("android_elf_identity_mismatch")
    if "[Requesting program interpreter: /system/bin/linker64]" not in text:
        raise ValueError("android_interpreter_mismatch")
    needed = sorted(re.findall(r"\(NEEDED\).*?\[([^]]+)\]", text))
    expected = ["libandroid.so", "libc.so", "libdl.so", "liblog.so", "libm.so"]
    if needed != expected:
        raise ValueError("android_dependency_mismatch")
    loads = []
    for line in text.splitlines():
        values = line.split()
        if values and values[0] == "LOAD":
            offset, address, alignment = int(values[1], 16), int(values[2], 16), int(values[-1], 16)
            if alignment < 16384 or alignment & (alignment - 1) or offset % 16384 != address % 16384:
                raise ValueError("android_load_alignment_mismatch")
            if "E" in "".join(values[6:-1]) and "W" in "".join(values[6:-1]):
                raise ValueError("android_writable_executable_segment")
            loads.append({"offset": offset, "virtual_address": address, "alignment": alignment})
    relro_lines = [line.split() for line in text.splitlines() if line.lstrip().startswith("GNU_RELRO ")]
    if not loads or len(relro_lines) != 1:
        raise ValueError("android_relro_or_load_missing")
    relro_start, relro_size = int(relro_lines[0][2], 16), int(relro_lines[0][5], 16)
    relro_end = relro_start + relro_size
    if relro_size <= 0 or relro_end % 16384:
        raise ValueError("android_relro_end_alignment_mismatch")
    stack = next((line for line in text.splitlines() if line.lstrip().startswith("GNU_STACK")), "")
    if not stack or "E" in "".join(stack.split()[6:-1]):
        raise ValueError("android_executable_stack")
    return {"status": "ok", "class": "ELF64", "machine": "AArch64", "type": "PIE",
            "interpreter": "/system/bin/linker64", "needed": needed, "load_segments": loads,
            "gnu_relro": True, "relro_start": relro_start, "relro_size": relro_size,
            "relro_end": relro_end, "android_run": False}


def inspect_elf(binary, readelf, report):
    result = {"status": "failed", "android_run": False}
    try:
        completed = subprocess.run([str(readelf), "-h", "-l", "-d", str(binary)], check=True,
                                   capture_output=True, text=True, timeout=30)
        result = parse_elf(completed.stdout)
        result["binary_sha256"] = probe.digest(binary)
        result["binary_size"] = binary.stat().st_size
    finally:
        write(report, result)


# Exact per-report field allowlists. No raw log, source path, text/token body or binary copies.
RUN_FIELDS = {"prototype", "status", "case", "android_validated", "exporter_commit_verified",
    "thread_safe_cancel", "per_request_sampling", "model_repository", "model_revision", "model_files",
    "template_sha256", "probe_sha256", "threads", "precision", "sampler", "process_exit_code",
    "mnn_commit", "probe_report_version", "target_system", "target_processor", "compiler", "runtime_backend",
    "finish_reason", "load_wall_us", "prepare_wall_us", "generate_wall_us", "native_prefill_us",
    "native_decode_us", "prompt_token_count", "output_token_count_including_native_stop",
    "rendered_prompt_sha256", "output_text_sha256", "prompt_tokens_sha256"}
BUILD_KEYS = {"prototype", "mnn_commit", "patches", "system", "processor", "compiler", "cmake", "build_type",
              "android_abi", "android_api", "backend", "http", "omni", "sampler", "cancel"}


def sanitize(value, allowed):
    lock = probe.load_json(probe.LOCK)
    result = {}
    for key in allowed:
        if key not in value:
            continue
        item = value[key]
        if key == "model_files":
            if item != lock["files"]:
                raise ValueError("report_model_inventory_mismatch")
        elif key == "ndk":
            if item != NDK:
                raise ValueError("report_ndk_inventory_mismatch")
        elif key == "cases":
            if not isinstance(item, dict) or set(item) - set(CASES) or any(v not in {"pass", "fail"} for v in item.values()):
                raise ValueError("report_case_mismatch")
        elif key == "needed":
            if item != ["libandroid.so", "libc.so", "libdl.so", "liblog.so", "libm.so"]:
                raise ValueError("report_dependency_mismatch")
        elif key == "load_segments":
            if not isinstance(item, list) or len(item) > 16 or any(not isinstance(x, dict) or set(x) != {"offset", "virtual_address", "alignment"} or any(type(v) is not int or v < 0 for v in x.values()) for x in item):
                raise ValueError("report_load_segment_mismatch")
        elif key == "model_repository":
            if item != lock["repository"]:
                raise ValueError("report_model_repository_mismatch")
        elif key == "interpreter":
            if item != "/system/bin/linker64":
                raise ValueError("report_interpreter_mismatch")
        elif isinstance(item, str):
            if not re.fullmatch(r"[A-Za-z0-9_. :=+-]*", item):
                raise ValueError("unsafe_report_string")
        elif item is not None and type(item) not in {bool, int, float}:
            raise ValueError("unsafe_report_value")
        result[key] = item
    return result


def verify_staged_reports(destination):
    expected = {"tools.json", "setup.json", "suite.json", "android-elf.json", "linux-build.json", "android-build.json"} | {case + ".json" for case in CASES}
    if any(not (destination / name).is_file() for name in expected):
        raise ValueError("required_evidence_missing")
    reports = {name: probe.load_json(destination / name) for name in expected}
    lock = probe.load_json(probe.LOCK)
    for name in ("tools.json", "setup.json", "suite.json", "android-elf.json"):
        if reports[name].get("status") != "ok":
            raise ValueError("required_evidence_status_failed")
    tools, setup = reports["tools.json"], reports["setup.json"]
    if tools.get("cmake_package") != "4.4.3" or tools.get("ninja_package") != "1.13.2":
        raise ValueError("required_tool_identity_missing")
    if setup.get("ndk") != NDK or setup.get("model_files") != lock["files"] or setup.get("model_revision") != lock["revision"]:
        raise ValueError("required_input_identity_missing")
    suite_result = reports["suite.json"]
    if suite_result.get("cases") != {case: "pass" for case in CASES} or suite_result.get("repeat_identical") is not True or suite_result.get("android_run") is not False:
        raise ValueError("required_suite_evidence_failed")
    elf = reports["android-elf.json"]
    if elf.get("android_run") is not False or elf.get("gnu_relro") is not True or type(elf.get("relro_end")) is not int or elf["relro_end"] % 16384:
        raise ValueError("required_elf_evidence_failed")
    for case in CASES:
        report = reports[case + ".json"]
        if case == "boundary-over":
            if report.get("status") != "failed" or report.get("process_exit_code") != 1:
                raise ValueError("required_negative_evidence_failed")
        elif report.get("status") != "ok" or report.get("process_exit_code") != 0 or report.get("mnn_commit") != probe.MNN_COMMIT:
            raise ValueError("required_positive_evidence_failed")
    common = {"prototype": "T07-A", "mnn_commit": probe.MNN_COMMIT, "patches": "none", "build_type": "Release", "backend": "cpu", "http": "off", "omni": "off", "sampler": "load-time-greedy", "cancel": "unsupported", "cmake": "4.4.3"}
    for name, extra in (("linux-build.json", {"system": "Linux", "processor": "x86_64"}), ("android-build.json", {"system": "Android", "processor": "aarch64", "android_abi": "arm64-v8a", "android_api": "android-28"})):
        build = reports[name]
        if any(build.get(key) != value for key, value in dict(common, **extra).items()) or not build.get("compiler"):
            raise ValueError("required_build_identity_failed")


def stage(evidence, linux_build, android_build, destination, outcomes):
    destination.mkdir(parents=True, exist_ok=False)
    pairs = dict(item.split(":", 1) for item in outcomes.split(","))
    if set(pairs) != set(STEPS) or any(x not in {"success", "failure", "cancelled", "skipped"} for x in pairs.values()):
        raise ValueError("invalid_ci_outcomes")
    identity = source_identity()
    steps_succeeded = all(x == "success" for x in pairs.values())
    status = {"steps": pairs, "android_run": False, "steps_succeeded": steps_succeeded,
        "all_required_steps_succeeded": False, "evidence_verified": False,
        "mnn_commit": probe.MNN_COMMIT, **identity}
    write(destination / "ci-status.json", status)
    files = {
        "tools.json": (evidence / "tools.json", {"status", "cmake_package", "ninja_package", "cmake_binary", "ninja_binary"}),
        "setup.json": (evidence / "setup.json", {"status", "ndk", "network_step", "model_revision", "model_files"}),
        "suite.json": (evidence / "suite" / "suite.json", {"status", "android_run", "cases", "repeat_identical"}),
        "android-elf.json": (evidence / "android-elf.json", {"status", "android_run", "class", "machine", "type", "interpreter", "needed", "load_segments", "gnu_relro", "relro_start", "relro_size", "relro_end", "binary_sha256", "binary_size"}),
    }
    for case in CASES:
        files[case + ".json"] = (evidence / "suite" / case / "report.json", RUN_FIELDS)
    for name, (source, allowed) in files.items():
        if source.is_file() and not source.is_symlink():
            value = probe.load_json(source)
            write(destination / name, sanitize(value, allowed))
    for platform_name, build in (("linux", linux_build), ("android", android_build)):
        source = build / "nexa-mnn-build-Release.txt"
        if source.is_file() and not source.is_symlink():
            value = dict(line.split("=", 1) for line in source.read_text(encoding="utf-8").splitlines() if "=" in line)
            write(destination / (platform_name + "-build.json"), sanitize(value, BUILD_KEYS))
    evidence_error = None
    try:
        verify_staged_reports(destination)
        status["evidence_verified"] = True
    except ValueError as error:
        evidence_error = str(error)
        status["evidence_error"] = evidence_error
    dirty_ci = "GITHUB_SHA" in os.environ and not identity["source_clean"]
    status["all_required_steps_succeeded"] = steps_succeeded and status["evidence_verified"] and not dirty_ci
    write(destination / "ci-status.json", status)
    if dirty_ci:
        raise ValueError("ci_source_not_clean")
    if steps_succeeded and evidence_error:
        raise ValueError(evidence_error)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    tools_parser = sub.add_parser("tools")
    tools_parser.add_argument("--report", type=Path, required=True)
    setup_parser = sub.add_parser("setup")
    setup_parser.add_argument("--destination", type=Path, required=True)
    setup_parser.add_argument("--report", type=Path, required=True)
    suite_parser = sub.add_parser("suite")
    suite_parser.add_argument("--binary", type=Path, required=True)
    suite_parser.add_argument("--model", type=Path, required=True)
    suite_parser.add_argument("--out", type=Path, required=True)
    elf_parser = sub.add_parser("elf")
    elf_parser.add_argument("--binary", type=Path, required=True)
    elf_parser.add_argument("--readelf", type=Path, required=True)
    elf_parser.add_argument("--report", type=Path, required=True)
    stage_parser = sub.add_parser("stage")
    for name in ("evidence", "linux-build", "android-build", "destination"):
        stage_parser.add_argument("--" + name, type=Path, required=True)
    stage_parser.add_argument("--outcomes", required=True)
    args = vars(parser.parse_args())
    command = args.pop("command")
    {"tools": tool_versions, "setup": setup, "suite": suite, "elf": inspect_elf, "stage": stage}[command](**args)


if __name__ == "__main__":
    main()
