#!/usr/bin/env python3
"""Transfer only same-run, same-source Windows build inputs between CI jobs."""
from __future__ import annotations

import argparse
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import tempfile
import zipfile

import package_desktop_windows as desktop
import package_windows as base

ROOT = base.ROOT
MANIFEST = "handoff.json"
FILES = {
    "desktop": ("build/desktop/cargo/x86_64-pc-windows-msvc/release/nexa-desktop.exe",),
    "runtime": (
        "dist/windows-x64-cpu.zip", "dist/windows-x64-cpu.zip.sha256",
        "dist/acceptance-tools.zip", "dist/acceptance-tools.zip.sha256",
        "build/ci-tools/nexa-desktop-harness.exe",
    ),
}
PACKAGES = {"windows-x64-cpu": "nexa-runtime", "acceptance-tools": "nexa-acceptance-tools"}
MAX_FILE_BYTES = 1024**3
MAX_JSON_BYTES = 16 * 1024**2
MAX_ZIP_BYTES = 2 * 1024**3


def identity():
    commit = os.environ.get("GITHUB_SHA", "")
    run = os.environ.get("GITHUB_RUN_ID", "")
    tools = os.environ.get("NEXA_WINDOWS_HANDOFF_KEY", "")
    if (not re.fullmatch(r"[a-f0-9]{40}", commit) or not re.fullmatch(r"[1-9][0-9]*", run)
            or not re.fullmatch(r"[a-f0-9]{64}", tools)):
        base.fail("handoff requires exact CI commit, run and Windows toolchain identity")
    source = desktop.source_identity(os.environ)
    if (source.get("commit") != commit or source.get("dirty") is not False
            or not re.fullmatch(r"[a-f0-9]{64}", source.get("worktree_files_sha256", ""))):
        base.fail("handoff requires the clean exact CI source")
    # Attempts deliberately are not bound: rerun-failed-jobs may consume the
    # successful sibling's artifact from an earlier attempt of this same run.
    return {"source": source, "run_id": run, "windows_toolchain_key": tools}


def read_json(path):
    with base.regular(path).open("rb") as source:
        raw = source.read(MAX_JSON_BYTES + 1)
    if len(raw) > MAX_JSON_BYTES:
        base.fail("handoff JSON exceeds its size limit")
    value = base.strict_json(raw)
    if not isinstance(value, dict):
        base.fail("handoff JSON must be an object")
    return value


def file_record(root, name):
    path = base.regular(root / name)
    size = path.stat().st_size
    if not 0 < size <= MAX_FILE_BYTES:
        base.fail("handoff file is empty or exceeds its size limit")
    return {"path": name, "size_bytes": size, "sha256": base.digest(path)}


def checked_directory(path):
    base.resolve_checked_path(path)
    if not path.is_dir():
        base.fail("handoff directory is missing")


def unused(path):
    base.resolve_checked_path(path)
    if path.exists() or path.is_symlink():
        base.fail("handoff refuses to overwrite an existing destination")


def copy_checked(source, destination, record):
    destination.parent.mkdir(parents=True, exist_ok=True)
    with base.regular(source).open("rb") as incoming, destination.open("xb") as outgoing:
        shutil.copyfileobj(incoming, outgoing)
    if (destination.stat().st_size != record["size_bytes"]
            or base.digest(destination) != record["sha256"]):
        base.fail("handoff file changed while copying")


def stage(kind, directory):
    expected = identity()
    directory = Path(directory).absolute()
    unused(directory)
    directory.parent.mkdir(parents=True, exist_ok=True)
    records = [file_record(ROOT, name) for name in sorted(FILES[kind])]
    with tempfile.TemporaryDirectory(prefix=".handoff-stage-", dir=directory.parent) as temporary:
        staged = Path(temporary) / "payload"
        staged.mkdir()
        for record in records:
            copy_checked(ROOT / record["path"], staged / record["path"], record)
        if identity() != expected:
            base.fail("source or toolchain changed while staging handoff")
        base.write_json(staged / MANIFEST, {"schema_version": 1, "kind": kind, **expected, "files": records})
        unused(directory)
        staged.rename(directory)


def inventory(kind, directory, expected):
    checked_directory(directory)
    manifest = read_json(directory / MANIFEST)
    fields = {"schema_version", "kind", "source", "run_id", "windows_toolchain_key", "files"}
    if (set(manifest) != fields or type(manifest["schema_version"]) is not int
            or manifest["schema_version"] != 1 or manifest["kind"] != kind
            or not base.exact_json_equal({key: manifest[key] for key in expected}, expected)):
        base.fail("handoff source, run, kind or toolchain identity mismatch")
    records = manifest["files"]
    if not isinstance(records, list) or len(records) != len(FILES[kind]):
        base.fail("handoff file inventory mismatch")
    for record in records:
        if (not isinstance(record, dict) or set(record) != {"path", "size_bytes", "sha256"}
                or record["path"] not in FILES[kind] or type(record["size_bytes"]) is not int
                or not 0 < record["size_bytes"] <= MAX_FILE_BYTES
                or not isinstance(record["sha256"], str)
                or not re.fullmatch(r"[a-f0-9]{64}", record["sha256"])):
            base.fail("handoff file record is invalid")
    if [record["path"] for record in records] != sorted(FILES[kind]):
        base.fail("handoff file inventory is not the exact closed set")
    allowed = set(FILES[kind]) | {MANIFEST}
    directories = {str(parent) for name in allowed for parent in PurePosixPath(name).parents if str(parent) != "."}
    observed = base.entries(directory)
    if ({item["path"] for item in observed} != allowed
            or {path.relative_to(directory).as_posix() for path in directory.rglob("*") if path.is_dir()} != directories
            or not base.exact_json_equal([item for item in observed if item["path"] != MANIFEST], records)):
        base.fail("handoff file set/hash/size mismatch")
    return records


def zip_path(name):
    base.relative(name)
    for part in name.split("/"):
        if (any(ord(c) < 32 or c in '<>"|?*' for c in part)
                or re.fullmatch(r"(?i)(?:CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(?:\..*)?", part)):
            base.fail("unsafe Windows ZIP path")
    return name


def unpack_package(root, name, expected):
    archive = root / "dist" / (name + ".zip")
    checksum = base.regular(archive.with_suffix(".zip.sha256"))
    if checksum.stat().st_size > 256 or checksum.read_text(encoding="utf-8").strip() != f"{base.digest(archive)}  {name}.zip":
        base.fail("handoff package ZIP checksum mismatch")
    with zipfile.ZipFile(archive) as zipped:
        members = zipped.infolist()
        if not members or len(members) > 8192 or sum(item.file_size for item in members) > MAX_ZIP_BYTES:
            base.fail("handoff package ZIP exceeds its bounds")
        seen = set()
        for item in members:
            path = zip_path(item.filename)
            mode = stat.S_IFMT(item.external_attr >> 16)
            if (item.orig_filename != item.filename or not path.startswith(name + "/")
                    or item.is_dir() or mode not in (0, stat.S_IFREG) or item.flag_bits & 1
                    or path.casefold() in seen or item.file_size > MAX_FILE_BYTES):
                base.fail("handoff package ZIP has an unsafe or duplicate member")
            seen.add(path.casefold())
        for item in members:
            destination = root / "dist" / item.filename
            destination.parent.mkdir(parents=True, exist_ok=True)
            with zipped.open(item) as incoming, destination.open("xb") as outgoing:
                shutil.copyfileobj(incoming, outgoing)
    package = root / "dist" / name
    manifest = read_json(package / "manifest.json")
    source = manifest.get("source")
    if (not isinstance(source, dict)
            or not base.exact_json_equal({key: source.get(key) for key in expected["source"]}, expected["source"])
            or manifest.get("project_commit") != expected["source"]["commit"]
            or manifest.get("project_dirty") is not False or manifest.get("product") != PACKAGES[name]
            or manifest.get("target") != base.TARGET or manifest.get("configuration") != "Release"
            or manifest.get("cargo_lock_sha256") != base.digest(base.regular(ROOT / "Cargo.lock"))):
        base.fail("handoff package source/lock/Release identity mismatch")
    base.verify_package(package, manifest)


def restore(kind, directory):
    expected = identity()
    directory = Path(directory).absolute()
    records = inventory(kind, directory, expected)
    destinations = list(sorted(FILES[kind]))
    if kind == "runtime":
        destinations.extend("dist/" + name for name in PACKAGES)
    for name in destinations:
        unused(ROOT / name)
    work = ROOT / "build"
    base.resolve_checked_path(work)
    work.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".handoff-restore-", dir=work) as temporary:
        staged = Path(temporary)
        for record in records:
            copy_checked(directory / record["path"], staged / record["path"], record)
        if kind == "runtime":
            for name in PACKAGES:
                unpack_package(staged, name, expected)
        if identity() != expected:
            base.fail("source or toolchain changed while restoring handoff")
        promoted = []
        try:
            for name in destinations:
                destination = ROOT / name
                unused(destination)
                destination.parent.mkdir(parents=True, exist_ok=True)
                (staged / name).rename(destination)
                promoted.append(destination)
        except BaseException:
            for destination in reversed(promoted):
                shutil.rmtree(destination) if destination.is_dir() else destination.unlink()
            raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("stage", "restore"))
    parser.add_argument("--kind", choices=tuple(FILES), required=True)
    parser.add_argument("--directory", type=Path, required=True)
    args = parser.parse_args()
    {"stage": stage, "restore": restore}[args.action](args.kind, args.directory)
    print(f"Verified {args.kind} CI build handoff: {args.action}")


if __name__ == "__main__":
    main()
