#!/usr/bin/env python3
"""Fixed inputs, fail-closed feature checks and corresponding-source artifact.

The shell driver performs compilation; this helper never invokes a target binary.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
MATERIALS = ROOT / "third_party" / "aria2"
LOCK = MATERIALS / "source-lock.json"
FORBIDDEN = ("ENABLE_BITTORRENT", "ENABLE_METALINK", "HAVE_LIBCARES", "ENABLE_ASYNC_DNS", "HAVE_LIBSSH2",
             "HAVE_OPENSSL", "HAVE_GNUTLS", "HAVE_LIBGNUTLS", "HAVE_LIBXML2", "HAVE_LIBEXPAT", "HAVE_LIBZ",
             "HAVE_SQLITE3", "HAVE_LIBNETTLE", "HAVE_LIBGMP", "HAVE_LIBGCRYPT", "ENABLE_WEBSOCKET", "ENABLE_NLS")
SYSTEM_IMPORTS = {"advapi32.dll", "bcrypt.dll", "crypt32.dll", "iphlpapi.dll", "kernel32.dll", "msvcrt.dll",
                  "ntdll.dll", "ole32.dll", "secur32.dll", "shell32.dll", "user32.dll", "ws2_32.dll", "ucrtbase.dll", "gdi32.dll", "winmm.dll", "psapi.dll", "wsock32.dll"}


def sha(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def verified(path: Path, expected: str) -> None:
    if sha(path) != expected:
        raise ValueError(f"SHA256 mismatch: {path.name}")


def fetch(item: dict, work: Path) -> Path:
    path = work / item["filename"]
    if not path.exists():
        partial = path.with_suffix(path.suffix + ".partial")
        with urllib.request.urlopen(item["url"], timeout=120) as response, partial.open("wb") as out:
            shutil.copyfileobj(response, out)
        verified(partial, item["sha256"])
        partial.replace(path)
    verified(path, item["sha256"])
    return path


def source_identity(source: Path) -> dict:
    return {p.relative_to(source).as_posix(): sha(p) for p in sorted(source.rglob("*")) if p.is_file()}


def prepare(work: Path) -> None:
    lock = json.loads(LOCK.read_text(encoding="utf-8"))
    patch = MATERIALS / "patches" / lock["patch"]["filename"]
    verified(patch, lock["patch"]["sha256"])
    archive = fetch(lock["aria2"], work)
    source = work / lock["aria2"]["root"]
    identity = work / "prepared-source.json"
    if source.exists():
        if not identity.exists() or source_identity(source) != json.loads(identity.read_text(encoding="utf-8")):
            raise ValueError("Existing source differs or was not prepared here; use a fresh work directory")
    else:
        with tarfile.open(archive) as tar:
            tar.extractall(work, filter="data")
        subprocess.run(["patch", "--batch", "--fuzz=0", "-p1", "-i", str(patch)], cwd=source, check=True)
        identity.write_text(json.dumps(source_identity(source), indent=2) + "\n", encoding="utf-8")
    archive = fetch(lock["toolchain"], work)
    toolchain = work / lock["toolchain"]["root"]
    marker = work / "prepared-toolchain.sha256"
    if toolchain.exists():
        if not marker.exists() or marker.read_text(encoding="utf-8").strip() != lock["toolchain"]["sha256"]:
            raise ValueError("Unverified existing toolchain; use a fresh work directory")
    else:
        with tarfile.open(archive) as tar:
            members = [m for m in tar if not any(p in m.name.split("/") for p in
                       ("aarch64-w64-mingw32", "armv7-w64-mingw32", "i686-w64-mingw32"))]
            tar.extractall(work, members=members, filter="data")
        marker.write_text(lock["toolchain"]["sha256"] + "\n", encoding="utf-8")


def check_config_text(config: str) -> dict:
    defines = dict(re.findall(r"^#define\s+(\w+)\s+(.+)$", config, re.M))
    forbidden = [key for key in FORBIDDEN if defines.get(key) == "1"]
    if forbidden or defines.get("SECURITY_WIN32") != "1" or defines.get("ENABLE_SSL") != "1":
        raise ValueError(f"Unexpected TLS/features: forbidden={forbidden}, WinTLS={defines.get('SECURITY_WIN32')}")
    return {key: defines.get(key) == "1" for key in ("SECURITY_WIN32", "ENABLE_SSL", *FORBIDDEN)}


def pe_metadata(text: str) -> dict:
    if not re.search(r"Machine:\s+IMAGE_FILE_MACHINE_AMD64", text):
        raise ValueError("Expected Windows AMD64 PE")
    imports = sorted(set(re.findall(r"^\s+Name:\s+(\S+\.dll)\s*$", text, re.I | re.M)))
    if not imports or "secur32.dll" not in {s.lower() for s in imports}:
        raise ValueError("Missing Schannel system import")
    unknown = [s for s in imports if s.lower() not in SYSTEM_IMPORTS and
               not re.fullmatch(r"api-ms-win-crt-[a-z0-9-]+-l1-1-0\.dll", s.lower())]
    if unknown:
        raise ValueError(f"Non-system imports require explicit packaging review: {unknown}")
    return {"machine": "AMD64", "machine_hex": "0x8664", "imports": imports,
            "dependency_closure": "system DLL/API-set imports only; runtime Windows load test is required"}


def bundle(work: Path) -> None:
    lock = json.loads(LOCK.read_text(encoding="utf-8"))
    source = work / lock["aria2"]["root"]
    if source_identity(source) != json.loads((work / "prepared-source.json").read_text(encoding="utf-8")):
        raise ValueError("Source changed after patch verification")
    artifacts = work / "artifacts"
    config = (work / "build/config.h").read_text(encoding="utf-8")
    features = check_config_text(config)
    pe = pe_metadata((artifacts / "pe.txt").read_text(encoding="utf-8"))
    shutil.copy2(work / "build/config.h", artifacts / "config.h")
    licenses = artifacts / "licenses"
    licenses.mkdir(exist_ok=True)
    shutil.copy2(source / "COPYING", licenses / "aria2-COPYING")
    toolchain = work / lock["toolchain"]["root"]
    shutil.copy2(toolchain / "LICENSE.TXT", licenses / "llvm-mingw-LICENSE.TXT")
    for p in sorted((toolchain / "x86_64-w64-mingw32/share/mingw32").glob("COPYING*")):
        shutil.copy2(p, licenses / p.name)
    if not (licenses / "COPYING.MinGW-w64-runtime.txt").is_file():
        raise ValueError("Missing MinGW runtime license")
    runtime_license_dir = work / "runtime-licenses"
    runtime_license_dir.mkdir(exist_ok=True)
    for item in lock["runtime_licenses"]:
        shutil.copy2(fetch(item, runtime_license_dir), licenses / item["filename"])
    # Preserve the entire *patched* pristine tree, including the added policy
    # header (upstream make dist does not know about it), original archive,
    # patch, exact build/test scripts, generated configuration, and notices.
    source_bundle = artifacts / "aria2-1.37.0-nexa-corresponding-source.tar.gz"
    with tarfile.open(source_bundle, "w:gz") as tar:
        tar.add(source, arcname="source/aria2-1.37.0")
        tar.add(work / lock["aria2"]["filename"], arcname="upstream/" + lock["aria2"]["filename"])
        tar.add(MATERIALS, arcname="build-materials/third_party/aria2", filter=lambda i: None if "__pycache__" in i.name else i)
        for p in sorted((ROOT / "scripts").glob("*aria2*build*.py")):
            tar.add(p, arcname="build-materials/scripts/" + p.name)
        for name in ("build_aria2_windows.py", "build_aria2_windows.sh"):
            tar.add(ROOT / "scripts" / name, arcname="build-materials/scripts/" + name)
        tar.add(ROOT / ".github/workflows/aria2-build-probe.yml", arcname="build-materials/.github/workflows/aria2-build-probe.yml")
        tar.add(work / "build/config.h", arcname="configuration/config.h")
        tar.add(work / "build/config.status", arcname="configuration/config.status")
        tar.add(licenses, arcname="licenses")
    manifest = {"schema_version": 1, "source_commit": os.environ.get("GITHUB_SHA", "local-uncommitted"),
                "binary_name": "nexa-aria2.exe", "product_relative_path": "download/nexa-aria2.exe",
                "aria2_version": lock["aria2"]["version"], "source_lock": lock,
                "target": "x86_64-w64-mingw32", "win32_winnt": "0x0A00", "tls_backend": "Schannel",
                "features": features, "pe": pe, "compiler": (artifacts / "compiler.txt").read_text(encoding="utf-8"),
                "windows_runtime_tested": False, "target_win10_device_tested": False,
                "platform_network_boundary": "OS DNS and Schannel AIA/CRL/OCSP are outside download-socket policy",
                "files": {p.relative_to(artifacts).as_posix(): {"sha256": sha(p), "bytes": p.stat().st_size}
                          for p in sorted(artifacts.rglob("*")) if p.is_file() and p.name != "build-manifest.json"}}
    (artifacts / "build-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("action", choices=("prepare", "check-config", "bundle"))
    parser.add_argument("--work", type=Path, required=True)
    args = parser.parse_args()
    work = args.work.resolve()
    work.mkdir(parents=True, exist_ok=True)
    if args.action == "prepare":
        prepare(work)
    elif args.action == "check-config":
        check_config_text((work / "build/config.h").read_text(encoding="utf-8"))
    else:
        bundle(work)


if __name__ == "__main__":
    main()
