#!/usr/bin/env python3
"""Build one private Windows x64 CPU package. No downloads, installers or model data.

Python, Cargo, CMake and the selected VS installation are build-time tools only.
Runtime acceptance is a separate executable and a separate gate.
"""
from __future__ import annotations
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import tomllib
import uuid
import zipfile

ROOT = Path(__file__).resolve().parents[1]
TARGET = "x86_64-pc-windows-msvc"
LLAMA_COMMIT = "2149c00f4442dc59302e134a02e4c99d5f7ed9fc"
OS_DLLS = set("advapi32 bcrypt bcryptprimitives combase crypt32 dbghelp gdi32 iphlpapi kernel32 kernelbase msvcrt netapi32 normaliz ntdll ole32 oleaut32 powrprof psapi rpcrt4 secur32 setupapi shell32 shlwapi ucrtbase user32 userenv version winhttp winmm ws2_32".split())
API_SET = re.compile(r"^(?:api|ext)-ms-win-[a-z0-9-]+\.dll$")
DEBUG_CRT = re.compile(r"^(?:ucrtbased|(?:vcruntime|msvcp|msvcr|concrt|vcomp)\d+(?:_\d+)?d)\.dll$")
ROOT_FILES = {"ai-runtime.exe", "ai-runtime-worker.exe", "config.example.toml", "README.md", "manifest.json", "SHA256SUMS", "THIRD_PARTY_NOTICES.md"}
VS_GENERATORS = {17: "Visual Studio 17 2022", 18: "Visual Studio 18 2026"}
CMAKE_MINIMUM = (4, 2, 0)
VS_INSTALL_HELP = ("If Visual Studio is already installed, open Visual Studio Installer > Modify and add "
                   "Desktop development with C++, MSVC x64/x86 tools and a Windows 10/11 SDK. "
                   "Otherwise install Visual Studio 2022 Build Tools from "
                   "https://aka.ms/vs/17/release/vs_buildtools.exe and select that workload. "
                   "Review the installer terms yourself; this packaging script does not download or install software.")


def fail(message):
    raise ValueError(message)


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def regular(path):
    """Reject symlinks and Windows reparse points, including ancestor directories."""
    path = Path(path).absolute()
    for part in (path, *path.parents):
        info = part.lstat()
        if part.is_symlink() or getattr(info, "st_file_attributes", 0) & 0x400:
            fail(f"symlink/reparse path is forbidden: {path.name}")
    if not path.is_file():
        fail(f"not a regular file: {path.name}")
    return path


def resolve_checked_path(path):
    """Normalize filesystem aliases only after checking their original ancestry."""
    path = Path(path).absolute()
    missing = False
    for part in (path, *path.parents):
        try:
            info = part.lstat()
        except FileNotFoundError:
            # Keep missing tails for the caller's layout/version diagnostics.
            # Callers must still require the actual directory/files before use.
            missing = True
            continue
        if part.is_symlink() or getattr(info, "st_file_attributes", 0) & 0x400:
            fail(f"symlink/reparse path is forbidden: {path.name}")
    resolved = path.resolve()
    if missing and resolved.exists():
        # Non-strict resolve can erase a nonexistent "missing/.." component.
        # Such a spelling is not an existing alias of the resulting directory.
        fail("path does not exist before resolution")
    return resolved


def relative(value):
    if not isinstance(value, str) or not value or "\\" in value or ":" in value or "\x00" in value:
        fail("invalid package relative path")
    value.encode("utf-8", errors="strict")
    parts = value.split("/")
    if PurePosixPath(value).is_absolute() or any(p in ("", ".", "..") or p.endswith((" ", ".")) for p in parts):
        fail("package path escapes root or is not normalized")
    return value


def entries(folder):
    result, seen = [], set()
    for path in sorted(folder.rglob("*")):
        if path.is_symlink() or getattr(path.lstat(), "st_file_attributes", 0) & 0x400:
            fail("symlink/reparse point in package")
        if path.is_dir():
            continue
        regular(path)
        name = relative(path.relative_to(folder).as_posix())
        if name.casefold() in seen:
            fail("case-insensitive duplicate package path")
        seen.add(name.casefold())
        result.append({"path": name, "size_bytes": path.stat().st_size, "sha256": digest(path)})
    return result


def sanitize(value, replacements):
    """Published evidence keeps identities and relative paths, never user roots."""
    if isinstance(value, dict):
        return {k: sanitize(v, replacements) for k, v in value.items()}
    if isinstance(value, list):
        return [sanitize(v, replacements) for v in value]
    if isinstance(value, str):
        for old, label in sorted(replacements, key=lambda x: len(x[0]), reverse=True):
            if old:
                value = re.sub(re.escape(old), lambda _: label, value, flags=re.IGNORECASE)
                value = re.sub(re.escape(old.replace("\\", "/")), lambda _: label, value, flags=re.IGNORECASE)
        return value
    return value


def write_json(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def command(args, env=None, cwd=ROOT, allowed=(0,)):
    result = subprocess.run([str(a) for a in args], cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, encoding="utf-8", errors="replace", check=False)
    if result.returncode not in allowed:
        fail(f"command failed ({result.returncode}): {args[0]}\n{result.stdout[-6000:]}")
    return result.stdout.strip()


def powershell(script, env=None):
    child_env = windows_environment(os.environ if env is None else env, "")
    system_root = child_env.get("SYSTEMROOT")
    if not system_root:
        fail("SYSTEMROOT is required to locate Windows PowerShell")
    host = regular(Path(system_root) / "System32/WindowsPowerShell/v1.0/powershell.exe")
    # pwsh -> Python/cmd -> Windows PowerShell otherwise inherits the PS7 module
    # search path. Remove only this child variable; never change the parent or OS.
    child_env.pop("PSMODULEPATH", None)
    prefix = ("$ErrorActionPreference='Stop';"
              "[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false);"
              "$OutputEncoding=[Console]::OutputEncoding;"
              "$PSModuleAutoLoadingPreference='None';")
    for name in ("Microsoft.PowerShell.Security", "Microsoft.PowerShell.Utility", "Microsoft.PowerShell.Management"):
        module = regular(host.parent / "Modules" / name / (name + ".psd1"))
        prefix += ("try {Import-Module -Name " + ps_literal(module) + " -ErrorAction Stop;} catch {"
                   "[Console]::Error.WriteLine('Nexa system module import failed; module=" + name +
                   "; host=' + $PSHOME + '; version=' + $PSVersionTable.PSVersion + '; type=' + "
                   "$_.Exception.GetType().FullName + '; id=' + $_.FullyQualifiedErrorId + "
                   "'; message=' + $_.Exception.Message); exit 1;}")
    return command([host, "-NoLogo", "-NoProfile", "-NonInteractive", "-Command", prefix + script], child_env)


def authenticode_info(source, env):
    info = json.loads(powershell("$p=" + ps_literal(source) + "; $s=Get-AuthenticodeSignature -LiteralPath $p; $f=Get-Item -LiteralPath $p; @{signature_status=[string]$s.Status; signer=[string]$s.SignerCertificate.Subject; file_version=$f.VersionInfo.FileVersion; product_version=$f.VersionInfo.ProductVersion; powershell_version=[string]$PSVersionTable.PSVersion; powershell_host=$PSHOME} | ConvertTo-Json -Compress", env))
    if info["signature_status"] != "Valid" or "Microsoft Corporation" not in info["signer"]:
        fail("CRT DLL does not have a valid Microsoft signature")
    return info


def ps_literal(value):
    return "'" + str(value).replace("'", "''") + "'"


def pe_machine(path):
    with regular(path).open("rb") as file:
        if file.read(2) != b"MZ":
            fail("not a PE binary")
        file.seek(0x3C)
        offset = struct.unpack("<I", file.read(4))[0]
        if offset > path.stat().st_size - 26:
            fail("invalid PE header offset")
        file.seek(offset)
        if file.read(4) != b"PE\0\0" or struct.unpack("<H", file.read(2))[0] != 0x8664:
            fail("package binary is not Windows AMD64")


def parse_dependents(output):
    # dumpbin /DEPENDENTS emits both normal and delay-load imports. DLL names
    # are whole lines; headings and addresses are intentionally not interpreted.
    names = []
    for line in output.splitlines():
        value = line.strip().lower()
        if re.fullmatch(r"[a-z0-9_.-]+\.dll", value):
            if value not in names:
                names.append(value)
    if not names:
        fail("dumpbin returned no import DLLs; refusing an unverified binary")
    return sorted(names)


def dependency_kind(name, redist):
    name = name.lower()
    if DEBUG_CRT.fullmatch(name):
        fail(f"Debug CRT dependency is forbidden: {name}")
    if name.removesuffix(".dll") in OS_DLLS or API_SET.fullmatch(name):
        return "os"
    if name in redist:
        return "app-local"
    fail(f"unknown non-OS dependency: {name}")


def collect_dependencies(stage, redist, inspect, copy_crt, binaries=("ai-runtime.exe", "ai-runtime-worker.exe")):
    queue = list(binaries)
    result = {}
    while queue:
        filename = queue.pop(0)
        if filename in result:
            continue
        imports = []
        for name in inspect(stage / filename):
            kind = dependency_kind(name, redist)
            imports.append({"name": name, "kind": kind})
            if kind == "app-local" and name not in result and name not in queue:
                copy_crt(redist[name], stage / name)
                queue.append(name)
        result[filename] = {"imports": imports}
    return result


# A package keeps directly readable notices and an exact original-file mapping.
# These files are a presentation change, not a selection of alternate licenses.
LICENSE_INDEX = "licenses/index.json"
LICENSE_TEXT = "licenses/THIRD_PARTY_LICENSES.txt"
LICENSE_HTML = "licenses/COPYRIGHT.html"
LICENSE_NOTICES = "THIRD_PARTY_NOTICES.md"
LICENSE_FORMAT = "nexa-license-bundle-v1"
LICENSE_PREFIX = (b"Nexa third-party license bundle (format 1)\n"
                  b"Original license and notice bytes are preserved without modification.\n"
                  b"Component, version and source mappings are in index.json.\n")
LICENSE_FOOTER = b"\nEND ORIGINAL\n"
ORIGINAL_INDEXES = {LICENSE_INDEX, "licenses/npm-index.json", "licenses/microsoft-crt/index.json"}


def strict_json(raw):
    # Match the native consumer: no UTF-16/32 auto-detection or UTF-8 BOM.
    if isinstance(raw, (bytes, bytearray)):
        raw = raw.decode("utf-8", errors="strict")
    def object_pairs(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                fail("duplicate JSON key in license inventory")
            result[key] = value
        return result
    def invalid_constant(value):
        fail("non-JSON numeric constant in license inventory")
    def exact_integer(value):
        number = int(value)
        if value == "-0" or not -(2**63) <= number <= 2**64 - 1:
            fail("license JSON integer is outside the exact native range")
        return number
    def invalid_float(value):
        fail("floating-point numbers are forbidden in license JSON")
    return json.loads(raw, object_pairs_hook=object_pairs, parse_constant=invalid_constant,
                      parse_int=exact_integer, parse_float=invalid_float)


def exact_json_equal(left, right):
    """Compare preserved records without Python's bool/int equality aliases."""
    if type(left) is not type(right):
        return False
    if isinstance(left, dict):
        return left.keys() == right.keys() and all(exact_json_equal(left[key], right[key]) for key in left)
    if isinstance(left, list):
        return len(left) == len(right) and all(exact_json_equal(a, b) for a, b in zip(left, right))
    return left == right


def license_header(document):
    return (f"\n===== ORIGINAL FILE: {document['original_path']} =====\n"
            f"SHA256: {document['sha256']}\nBytes: {document['size_bytes']}\nBEGIN ORIGINAL\n").encode("utf-8")


def license_original_path(name):
    relative(name)
    if name == LICENSE_NOTICES:
        return name
    if (not name.startswith("licenses/") or
            Path(name).suffix.lower() in {".exe", ".dll", ".gguf", ".pdb", ".lib", ".a", ".so", ".log", ".bat", ".ps1", ".py"} or
            any(part.lower() in {".git", "userdata", "secrets", "logs", "models", "node_modules", "target", "build"} for part in name.split("/"))):
        fail("unsupported license original path or non-text original")
    return name


def license_storage(name, raw):
    """Keep native HTML and non-text Microsoft originals directly openable."""
    try:
        raw.decode("utf-8", errors="strict")
        text = b"\0" not in raw
    except UnicodeDecodeError:
        text = False
    suffix = Path(name).suffix.lower()
    if not text or suffix in {".pdf", ".docx"}:
        if (not name.startswith("licenses/microsoft-crt/") or
                suffix not in {".txt", ".rtf", ".html", ".htm", ".md", ".pdf", ".docx"}):
            fail("unsupported binary license original; conversion or omission is forbidden")
        return "licenses/ORIGINAL-" + Path(name).name
    return LICENSE_HTML if name == "licenses/rust-std/COPYRIGHT-library.html" else LICENSE_TEXT


def license_attributions(originals, supplied=None):
    """Check the pre-consolidation closure, retaining all original index bytes."""
    result = {name: list(records) for name, records in (supplied or {}).items()}
    if LICENSE_NOTICES in originals:
        result[LICENSE_NOTICES] = [{"component": "Nexa third-party notices", "source": LICENSE_NOTICES}]
    if not set(result) <= set(originals):
        fail("license attribution names an absent original")
    for name in sorted(ORIGINAL_INDEXES & originals.keys()):
        index = strict_json(originals[name])
        if not isinstance(index, dict) or not isinstance(index.get("files"), list) or not index["files"]:
            fail("original license inventory is empty or invalid")
        seen = set()
        for record in index["files"]:
            if not isinstance(record, dict):
                fail("invalid original license record")
            path = license_original_path(record.get("path"))
            if path.casefold() in seen or path in ORIGINAL_INDEXES or path not in originals:
                fail("duplicate or missing original license record")
            seen.add(path.casefold())
            if hashlib.sha256(originals[path]).hexdigest() != record.get("sha256"):
                fail("original license inventory hash mismatch")
            result.setdefault(path, []).append(record)
        result[name] = [{"component": "Original component/license inventory", "source": name}]
    if set(result) != set(originals) or any(not records for records in result.values()):
        fail("license originals and attribution closure differ")
    return result


def consolidate_licenses(stage, supplied=None):
    """Replace staged license presentation, preserving every original byte.

    The Rust copyright HTML remains an ordinary HTML file; all other UTF-8
    originals, including previous JSON inventories, are concatenated verbatim.
    Non-text Microsoft originals remain separate. In that case the root notices
    join the text bundle to free a file slot without losing any notice bytes.
    No archive, license selection, newline conversion or source pruning occurs.
    """
    original_entries = entries(stage / "licenses")
    originals = {"licenses/" + item["path"]: regular(stage / "licenses" / item["path"]).read_bytes()
                 for item in original_entries}
    if not originals:
        fail("license closure is empty")
    standalone = any(license_storage(name, raw) not in {LICENSE_TEXT, LICENSE_HTML}
                     for name, raw in originals.items())
    if standalone and (stage / LICENSE_NOTICES).exists():
        originals[LICENSE_NOTICES] = regular(stage / LICENSE_NOTICES).read_bytes()
    attributes = license_attributions(originals, supplied)
    bundle, documents, standalone_files = bytearray(LICENSE_PREFIX), [], {}
    for name, raw in sorted(originals.items()):
        license_original_path(name)
        storage = license_storage(name, raw)
        document = {"original_path": name, "sha256": hashlib.sha256(raw).hexdigest(),
                    "size_bytes": len(raw), "stored_path": storage, "offset_bytes": 0,
                    "attributions": attributes[name]}
        if storage != LICENSE_TEXT:
            if storage.casefold() in {path.casefold() for path in standalone_files}:
                fail("duplicate standalone license storage path")
            standalone_files[storage] = raw
        else:
            bundle.extend(license_header(document))
            document["offset_bytes"] = len(bundle)
            bundle.extend(raw)
            bundle.extend(LICENSE_FOOTER)
        documents.append(document)
    index = {"schema_version": 1, "format": LICENSE_FORMAT, "documents": documents}
    # Validate the replacement before touching any original staging file.
    with tempfile.TemporaryDirectory(prefix=".license-stage-", dir=stage) as temporary:
        compact = Path(temporary)
        (compact / "licenses").mkdir()
        (compact / LICENSE_TEXT).write_bytes(bundle)
        for name, raw in standalone_files.items():
            (compact / name).write_bytes(raw)
        write_json(compact / LICENSE_INDEX, index)
        recovered = verify_license_bundle(compact, supplied=supplied)
        if {name: item["raw"] for name, item in recovered.items()} != originals:
            fail("license consolidation failed lossless round-trip")
        shutil.rmtree(stage / "licenses")
        (compact / "licenses").rename(stage / "licenses")
        if LICENSE_NOTICES in originals:
            (stage / LICENSE_NOTICES).unlink()
    return index


def verify_license_bundle(stage, supplied=None):
    """Validate every byte, offset, original attribution and closed output role."""
    index = strict_json(regular(stage / LICENSE_INDEX).read_bytes())
    if (not isinstance(index, dict) or set(index) != {"schema_version", "format", "documents"} or
            type(index["schema_version"]) is not int or index["schema_version"] != 1 or
            index["format"] != LICENSE_FORMAT or not isinstance(index["documents"], list) or not index["documents"]):
        fail("unsupported or empty license bundle index")
    bundle = regular(stage / LICENSE_TEXT).read_bytes()
    if not bundle.startswith(LICENSE_PREFIX):
        fail("license bundle header mismatch")
    cursor, result, folded, storage_files = len(LICENSE_PREFIX), {}, set(), set()
    for document in index["documents"]:
        if not isinstance(document, dict) or set(document) != {"original_path", "sha256", "size_bytes", "stored_path", "offset_bytes", "attributions"}:
            fail("invalid license bundle document")
        name = license_original_path(document["original_path"])
        size, offset = document["size_bytes"], document["offset_bytes"]
        if (name.casefold() in folded or (result and name <= next(reversed(result))) or
                not isinstance(document["sha256"], str) or not re.fullmatch(r"[a-f0-9]{64}", document["sha256"]) or
                type(size) is not int or size < 0 or type(offset) is not int or offset < 0 or
                not isinstance(document["attributions"], list) or not document["attributions"] or
                any(not isinstance(record, dict) or not record for record in document["attributions"])):
            fail("duplicate, unordered or invalid license bundle record")
        folded.add(name.casefold())
        storage = document["stored_path"]
        if storage == LICENSE_TEXT:
            header = license_header(document)
            if bundle[cursor:cursor + len(header)] != header or offset != cursor + len(header) or size > len(bundle) - offset:
                fail("license byte range/framing mismatch")
            raw = bundle[offset:offset + size]
            cursor = offset + size
            if bundle[cursor:cursor + len(LICENSE_FOOTER)] != LICENSE_FOOTER:
                fail("license bundle footer mismatch")
            cursor += len(LICENSE_FOOTER)
        else:
            expected = LICENSE_HTML if name == "licenses/rust-std/COPYRIGHT-library.html" else "licenses/ORIGINAL-" + Path(name).name
            if storage != expected or storage in storage_files or offset != 0:
                fail("license document escapes fixed standalone storage roles")
            raw = regular(stage / storage).read_bytes()
            storage_files.add(storage)
        if license_storage(name, raw) != storage:
            fail("license original must remain directly readable in its original format")
        if len(raw) != size or hashlib.sha256(raw).hexdigest() != document["sha256"]:
            fail("license original hash/size mismatch")
        result[name] = {"document": document, "raw": raw}
    if cursor != len(bundle):
        fail("unmapped trailing bytes in license bundle")
    expected = {LICENSE_INDEX, LICENSE_TEXT} | storage_files
    if {"licenses/" + item["path"] for item in entries(stage / "licenses")} != expected:
        fail("license bundle file closure mismatch")
    if any(path.is_dir() for path in (stage / "licenses").rglob("*")):
        fail("unexpected directory in consolidated licenses")
    if LICENSE_NOTICES in result and (stage / LICENSE_NOTICES).exists():
        fail("bundled notices must not also exist as an unindexed standalone copy")
    originals = {name: item["raw"] for name, item in result.items()}
    attributes = license_attributions(originals, supplied)
    if any(not exact_json_equal(item["document"]["attributions"], attributes[name]) for name, item in result.items()):
        fail("license attribution differs from original inventory")
    return result


def license_related_files(stage):
    return [item["path"] for item in entries(stage)
            if "licenses" in item["path"].lower().split("/") or
            Path(item["path"]).name.lower().startswith(("license", "licence", "copying", "notice", "unlicense", "copyright", "third_party_notice"))]


def verify_package(stage, manifest):
    declared = {}
    for item in manifest["files"]:
        name = relative(item["path"])
        if name.casefold() in declared:
            fail("duplicate manifest path")
        declared[name.casefold()] = item
    actual = entries(stage)
    payload = [x for x in actual if x["path"] not in ("manifest.json", "SHA256SUMS")]
    if {x["path"].casefold(): x for x in payload} != declared:
        fail("manifest file set/hash/size mismatch")
    originals = verify_license_bundle(stage)
    helper = manifest.get("product") == "nexa-acceptance-tools"
    allowed_root = ROOT_FILES | ({"nexa-acceptance.exe"} if helper else set())
    for item in actual:
        name = item["path"]
        if name.startswith("licenses/"):
            if Path(name).suffix.lower() in {".exe", ".dll", ".gguf", ".pdb", ".lib", ".a", ".so", ".log", ".bat", ".ps1", ".py"} or any(part.lower() in {".git", "userdata", "secrets", "logs", "models", "node_modules", "target", "build"} for part in name.split("/")):
                fail("binary, model, user data or tooling pollution under licenses")
            continue
        if name in allowed_root:
            continue
        if "/" in name or name not in manifest["dependencies"] or not name.endswith(".dll"):
            fail(f"unexpected package content: {name}")
    names = {x["path"] for x in payload}
    required = {"nexa-acceptance.exe", LICENSE_INDEX} if helper else {"ai-runtime.exe", "ai-runtime-worker.exe", "config.example.toml", "README.md", LICENSE_INDEX}
    if not required <= names:
        fail("required package content missing")
    if LICENSE_NOTICES not in names and LICENSE_NOTICES not in originals:
        fail("required package notices missing from both standalone and bundled roles")
    for name, node in manifest["dependencies"].items():
        if name not in names:
            fail("dependency node missing its file")
        for imported in node["imports"]:
            dll, kind = imported["name"], imported["kind"]
            if dependency_kind(dll, {n: None for n in manifest["dependencies"] if n.endswith(".dll")}) != kind:
                fail("dependency classification mismatch")
            if kind == "app-local" and dll not in names:
                fail("app-local dependency closure is incomplete")
    if (stage / "SHA256SUMS").exists():
        expected = "".join(f"{x['sha256']}  {x['path']}\n" for x in actual if x["path"] != "SHA256SUMS")
        if (stage / "SHA256SUMS").read_text(encoding="utf-8") != expected:
            fail("SHA256SUMS mismatch")


def windows_environment(base, devcmd_output):
    # CreateProcess receives this actual dictionary. Do not retain PATH/Path or
    # similar duplicate spellings: Windows treats environment names alike.
    env = {key.upper(): value for key, value in base.items()}
    for line in devcmd_output.splitlines():
        if "=" in line and not line.startswith("="):
            key, value = line.split("=", 1)
            env[key.upper()] = value
    return env


def devcmd_command_line(cmd, dev):
    # cmd.exe does not parse the MS C runtime quoting used by list2cmdline.
    # This one shell command is deliberately a raw CreateProcess command line.
    for path in (str(cmd), str(dev)):
        if any(character in path for character in '\0\r\n"%'):
            fail("unsupported character in Windows command interpreter or VS script path")
    return f'"{cmd}" /d /s /u /v:off /c ""{dev}" -no_logo -arch=x64 -host_arch=x64 >nul && set"'


def devcmd_environment(dev, env):
    system_root = env.get("SYSTEMROOT")
    if not system_root:
        fail("SYSTEMROOT is required to locate the Windows command interpreter")
    cmd = regular(Path(system_root) / "System32/cmd.exe")
    # /u makes the built-in `set` output UTF-16LE, including non-ASCII paths.
    result = subprocess.run(devcmd_command_line(cmd, dev), executable=str(cmd),
                            shell=False, cwd=ROOT, env=env, stdin=subprocess.DEVNULL,
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                            encoding="utf-16-le", errors="replace", check=False)
    if result.returncode != 0:
        fail(f"Visual Studio environment initialization failed ({result.returncode}): cmd.exe\n{result.stdout[-6000:]}")
    return windows_environment(env, result.stdout)


def visual_studio_version(selected):
    value = selected.get("installationVersion", "")
    if not isinstance(value, str) or not re.fullmatch(r"\d+\.\d+\.\d+\.\d+", value):
        return ()
    return tuple(int(part) for part in value.split("."))


def msvc_toolset(version):
    # VS2022 also ships 14.4x under v143; VS2026 introduces v145/14.5x.
    # https://cmake.org/cmake/help/latest/generator/Visual%20Studio%2018%202026.html
    if re.fullmatch(r"14\.[34]\d\.\d+", version):
        return "v143"
    if re.fullmatch(r"14\.5\d\.\d+", version):
        return "v145"
    fail("unsupported MSVC toolset version; expected stable 14.3x/14.4x (v143) or 14.5x (v145)")


def visual_studio_environment(base):
    env = base.copy()
    # Re-enter from the pre-developer PATH when available. Stale VS variables
    # otherwise let VsDevCmd skip setup or reuse another instance's toolchain.
    if env.get("__VSCMD_PREINIT_PATH"):
        env["PATH"] = env["__VSCMD_PREINIT_PATH"]
    for key in list(env):
        if key.startswith(("VSCMD_", "__VSCMD_", "VCTOOLS", "WINDOWSSDK", "UNIVERSALCRT")) or key in {
            "VSINSTALLDIR", "VCINSTALLDIR", "VISUALSTUDIOVERSION", "DEVENVDIR", "UCRTVERSION",
            "INCLUDE", "LIB", "LIBPATH", "VSCMD_VER", "VCVARSALL_INIT", "VCVARSALL_INIT_VERSION",
        }:
            env.pop(key)
    return env


def selected_msvc_tool(vs, env, name):
    version = env.get("VCTOOLSVERSION", "").strip()
    msvc_toolset(version)
    expected = regular(vs / "VC/Tools/MSVC" / version / "bin/Hostx64/x64" / name)
    # Avoid decoding localized where.exe output. Resolve from the explicit
    # child PATH, and reject current-directory shadowing before any build.
    for directory in (ROOT, Path.cwd()):
        shadow = directory / name
        if shadow.exists() and shadow.resolve() != expected.resolve():
            fail(f"{name} is shadowed by a current-directory executable")
    found = shutil.which(name, path=env.get("PATH", ""), mode=os.F_OK)
    if not found or regular(Path(found)).resolve() != expected.resolve():
        fail(f"{name} is not from the selected Visual Studio x64 toolset")
    return expected


def visual_studio_paths(selected, base_env):
    installation = selected.get("installationPath", "")
    if not installation or not Path(installation).is_absolute():
        fail("Visual Studio installation path must be absolute")
    # Check the original path before resolve(), which would conceal a symlink.
    vs = Path(installation)
    dev = regular(vs / "Common7/Tools/VsDevCmd.bat")
    vs = vs.resolve()
    env = devcmd_environment(dev, visual_studio_environment(base_env))
    # VsDevCmd can prepend its bundled older CMake. CI explicitly selects the
    # Python Scripts directory where this run installed a compatible version;
    # restore it after *every* VS initialization, including final packaging.
    if base_env.get("NEXA_CMAKE_BIN"):
        cmake_bin = Path(base_env["NEXA_CMAKE_BIN"])
        if not cmake_bin.is_absolute():
            fail("explicit CMake directory must be absolute")
        for name in ("cmake.exe", "ctest.exe"):
            regular(cmake_bin / name)
        env["NEXA_CMAKE_BIN"] = str(cmake_bin)
        env["PATH"] = str(cmake_bin) + os.pathsep + env.get("PATH", "")
    if not env.get("VSINSTALLDIR") or resolve_checked_path(env["VSINSTALLDIR"]) != vs:
        fail("VsDevCmd initialized a different Visual Studio instance")
    version = env.get("VCTOOLSVERSION", "").strip()
    if not version:
        fail("selected VS has no MSVC C++ build tools; add Desktop development with C++ through Visual Studio Installer > Modify")
    toolset = msvc_toolset(version)
    if visual_studio_version(selected)[0] == 17 and toolset != "v143":
        fail("VS2022 requires its supported v143 toolset")
    tools = vs / "VC/Tools/MSVC" / version
    if not env.get("VCTOOLSINSTALLDIR") or resolve_checked_path(env["VCTOOLSINSTALLDIR"]) != resolve_checked_path(tools):
        fail("MSVC tools are outside the selected Visual Studio toolset")
    for name in ("cl.exe", "link.exe", "lib.exe", "dumpbin.exe"):
        selected_msvc_tool(vs, env, name)
    sdk_version = env.get("WINDOWSSDKVERSION", "").rstrip("\\/")
    if not env.get("WINDOWSSDKDIR") or not re.fullmatch(r"10\.0\.\d+\.\d+", sdk_version):
        fail("selected VS has no Windows 10/11 SDK; add it through Visual Studio Installer > Modify")
    sdk = Path(env["WINDOWSSDKDIR"])
    for name in (f"Include/{sdk_version}/um/Windows.h", f"Lib/{sdk_version}/um/x64/kernel32.lib", f"Lib/{sdk_version}/ucrt/x64/ucrt.lib"):
        regular(sdk / name)
    value = env.get("VCTOOLSREDISTDIR")
    if not value:
        fail("selected VS has no VCToolsRedistDir; add its C++ redistributable tools through Visual Studio Installer > Modify")
    # Windows 8.3 names and long names can identify the same directory. Check
    # original ancestors before resolving both sides, so links cannot disguise
    # an external source as part of this VS instance.
    redist_root = resolve_checked_path(value)
    try:
        subpath = redist_root.relative_to(resolve_checked_path(vs / "VC/Redist/MSVC"))
    except ValueError:
        fail("CRT source is outside the selected Visual Studio redistribution directory")
    if len(subpath.parts) != 1:
        fail("selected VS Release x64 CRT redistribution source is missing/invalid")
    redist_toolset = msvc_toolset(subpath.name)
    if tuple(map(int, subpath.name.split(".")[:2])) < tuple(map(int, version.split(".")[:2])):
        fail("selected VS CRT redistribution version is older than its compiler toolset")
    redist_dir = resolve_checked_path(redist_root / "x64" / ("Microsoft.VC" + redist_toolset[1:] + ".CRT"))
    if not redist_dir.is_dir():
        fail("selected VS Release x64 CRT redistribution source is missing/invalid")
    redist = {}
    for path in redist_dir.glob("*.dll"):
        regular(path)
        key = path.name.lower()
        if key in redist:
            fail("duplicate CRT source filename")
        redist[key] = path
    if not redist:
        fail("no existing Release CRT DLLs; no installer will be run")
    return selected, vs, env, env.copy(), redist


def selected_visual_studio():
    env = windows_environment(os.environ, "")
    base = Path(env.get("PROGRAMFILES(X86)", "C:/Program Files (x86)"))
    discovery = base / "Microsoft Visual Studio/Installer/vswhere.exe"
    if not discovery.exists():
        fail("Visual Studio discovery tool vswhere.exe is missing; repair Visual Studio Installer if installed. " + VS_INSTALL_HELP)
    vswhere = regular(discovery)
    query = [vswhere, "-products", "*", "-format", "json", "-utf8"]
    # A version-specific side-by-side toolset need not have the generic
    # "latest C++ tools" component ID. Validate the actual initialized tools.
    choices = json.loads(command(query, env))
    candidates = [item for item in choices if visual_studio_version(item)[:1] in ((17,), (18,))
                  and not item.get("isPrerelease") and item.get("isComplete") is True]
    candidates.sort(key=lambda item: (visual_studio_version(item), item.get("instanceId", "")), reverse=True)
    errors = []
    for selected in candidates:
        try:
            return visual_studio_paths(selected, env)
        except (ValueError, OSError) as error:
            errors.append(f"VS {selected['installationVersion']}: {error}")
    if not errors:
        installed = json.loads(command([*query, "-all", "-prerelease"], env))
        if installed:
            errors.append("Existing Visual Studio installations have no supported complete stable VS2022/VS2026 C++ environment; preview/incomplete/unknown versions are not used")
        else:
            errors.append("No existing Visual Studio C++ installation was found")
    fail("; ".join(errors) + ". " + VS_INSTALL_HELP)


def checked_cmake(env, generator):
    """Require local build capabilities; keep the actual version for the manifest."""
    cmake = command(["cmake", "--version"], env)
    first_line = cmake.splitlines()[0] if cmake else ""
    version = re.fullmatch(r"cmake version ([0-9]+)\.([0-9]+)\.([0-9]+)(?:-[0-9A-Za-z][0-9A-Za-z.+-]*)?", first_line)
    if version is None:
        fail("cannot parse CMake version; expected 'cmake version <major>.<minor>.<patch>'")
    # CMake 4.2 introduces the VS2026 generator. Use one minimum for both
    # supported VS releases, without restricting newer minor or patch versions.
    if tuple(map(int, version.groups())) < CMAKE_MINIMUM:
        fail("CMake 4.2 or newer is required for local Windows builds")
    try:
        capabilities = json.loads(command(["cmake", "-E", "capabilities"], env))
    except json.JSONDecodeError:
        fail("CMake capabilities did not return valid JSON")
    generators = capabilities.get("generators") if isinstance(capabilities, dict) else None
    if not isinstance(generators, list) or any(not isinstance(item, dict) or not isinstance(item.get("name"), str) for item in generators):
        fail("CMake capabilities did not return a valid generator list")
    if not any(item["name"] == generator for item in generators):
        fail(f"installed CMake does not provide the required generator: {generator}")
    return cmake


def native_build_settings(selected, vs, env):
    version_parts = visual_studio_version(selected)
    generator = VS_GENERATORS.get(version_parts[0] if version_parts else None)
    if generator is None:
        fail("unsupported Visual Studio CMake generator")
    version = env["VCTOOLSVERSION"].strip()
    toolset = f"{msvc_toolset(version)},host=x64,version={version}"
    # Never switch generator/instance/toolset inside a prior build tree, or
    # remove the user's build/native-release cache to make configuration pass.
    key = hashlib.sha256((str(vs.resolve()).casefold() + "\n" + generator + "\n" + toolset).encode()).hexdigest()[:16]
    native = ROOT / "build/windows-x64-cpu" / ("native-" + key)
    cache = native / "CMakeCache.txt"
    if cache.exists():
        values = dict(re.findall(r"^([^#/:\n][^:\n]*):[^=\n]*=(.*)$", regular(cache).read_text(encoding="utf-8"), re.M))
        expected = {"CMAKE_GENERATOR": generator, "CMAKE_GENERATOR_PLATFORM": "x64", "CMAKE_GENERATOR_TOOLSET": toolset}
        instance = values.get("CMAKE_GENERATOR_INSTANCE", "")
        if any(values.get(k) != v for k, v in expected.items()) or not instance or Path(instance).resolve() != vs.resolve():
            fail("native CMake cache differs from selected VS generator/instance/toolset/x64; move that build directory aside and retry (no files were deleted)")
    args = ["cmake", "-S", ROOT / "native/llama-shim", "-B", native, "-G", generator, "-A", "x64", "-T", toolset,
            f"-DCMAKE_GENERATOR_INSTANCE={vs}", "-DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreadedDLL"]
    return native, args


def visual_studio_redistribution(selected):
    version = visual_studio_version(selected)
    year = {17: "2022", 18: "2026"}.get(version[0] if version else None)
    if year is None:
        fail("unsupported Visual Studio redistribution reference")
    return {"purpose": "private development acceptance",
            "source_rule": "unmodified Release x64 files from the selected Visual Studio VC/Redist/MSVC tree",
            "redist_list": f"https://learn.microsoft.com/en-us/visualstudio/releases/{year}/redistribution",
            "community_terms_reference": ("https://visualstudio.microsoft.com/wp-content/uploads/2021/11/Visual-Studio-2022-Community-License-EN.docx" if year == "2022" else "https://visualstudio.microsoft.com/license-terms/vs2026-ga-community/"),
            "license_terms_directory": "https://visualstudio.microsoft.com/license-terms/",
            "edition": selected.get("productId"), "license_acceptance_performed": False}


def license_files(package_dir, explicit=None):
    files = []
    if explicit:
        files.append(package_dir / explicit)
    for path in sorted(package_dir.iterdir()):
        if path.name.lower().startswith(("license", "licence", "copying", "notice", "unlicense")):
            files.extend([path] if path.is_file() else [x for x in path.rglob("*") if x.is_file()])
    # aws-lc-sys carries native code and nested third-party notices. Preserve
    # those exact notice files as well as its aggregate crate-root LICENSE.
    native = package_dir / "aws-lc"
    if (native / "LICENSE").is_file():
        files.extend(path for path in native.rglob("*") if path.is_file()
                     and path.name.lower().startswith(("license", "licence", "copying", "notice", "unlicense")))
    return sorted(set(files))


def copy_licenses(stage, metadata, env, roots=("runtime-cli", "runtime-worker")):
    records = []
    def save(source, destination, component, origin):
        regular(source)
        relative(destination)
        out = stage / "licenses" / destination
        out.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, out)
        records.append({"component": component, "source": origin, "path": "licenses/" + destination, "sha256": digest(out)})
    packages = {p["id"]: p for p in metadata["packages"]}
    nodes = {n["id"]: n for n in metadata["resolve"]["nodes"]}
    queue = [p["id"] for p in packages.values() if p["name"] in roots and p["source"] is None]
    visited = set()
    while queue:
        key = queue.pop()
        if key in visited:
            continue
        visited.add(key)
        queue.extend(d["pkg"] for d in nodes[key]["deps"] if any(k["kind"] != "dev" for k in d["dep_kinds"]))
        p = packages[key]
        if p["source"] is None:
            continue
        source = Path(p["manifest_path"]).parent
        files = license_files(source, p.get("license_file"))
        if not files:
            fail(f"original license text missing for {p['name']} {p['version']}")
        for file in files:
            save(file, f"rust-crates/{p['name']}-{p['version']}/{file.relative_to(source).as_posix()}", f"{p['name']} {p['version']} ({p.get('license')})", f"{p['source']} {p.get('repository') or ''}")
    vendor = ROOT / "vendor/llama.cpp"
    for file in ["LICENSE", "licenses/LICENSE-jsonhpp", "vendor/cpp-httplib/LICENSE", "vendor/hash/rotate-bits/LICENSE.md", "vendor/hash/xxhash/LICENSE", "vendor/hash/sha256/LICENSE"]:
        save(vendor / file, "llama.cpp/" + file, "llama.cpp and bundled CPU/common components", f"https://github.com/ggml-org/llama.cpp/blob/{LLAMA_COMMIT}/{file}")
    source = vendor / "vendor/sheredom/subprocess.h"
    raw = regular(source).read_bytes()
    match = re.search(rb"/\*\s+This is free and unencumbered software.*?\*/", raw, re.S)
    if not match:
        fail("sheredom original embedded license not found")
    out = stage / "licenses/llama.cpp/sheredom-LICENSE.txt"
    out.write_bytes(match.group(0))
    records.append({"component": "sheredom/subprocess.h", "source": f"https://github.com/ggml-org/llama.cpp/blob/{LLAMA_COMMIT}/vendor/sheredom/subprocess.h", "source_sha256": digest(source), "path": "licenses/llama.cpp/sheredom-LICENSE.txt", "sha256": digest(out)})
    source = vendor / "vendor/hash/sha1/sha1.h"
    raw = regular(source).read_bytes()
    match = re.search(rb"/\*\s+SHA-1 in C.*?100% Public Domain.*?\*/", raw, re.S)
    if not match:
        fail("SHA-1 original public-domain notice not found")
    out = stage / "licenses/llama.cpp/sha1-NOTICE.txt"
    out.write_bytes(match.group(0))
    records.append({"component": "SHA-1 Steve Reid", "source": f"https://github.com/ggml-org/llama.cpp/blob/{LLAMA_COMMIT}/vendor/hash/sha1/sha1.h", "source_sha256": digest(source), "path": "licenses/llama.cpp/sha1-NOTICE.txt", "sha256": digest(out)})
    sysroot = Path(command(["rustc", "--print", "sysroot"], env))
    rust_docs = sysroot / "share/doc/rust"
    save(rust_docs / "COPYRIGHT-library.html", "rust-std/COPYRIGHT-library.html", "Rust standard library", "installed pinned Rust toolchain/share/doc/rust/COPYRIGHT-library.html")
    for file in sorted((rust_docs / "licenses").glob("*")):
        if file.is_file():
            save(file, "rust-std/licenses/" + file.name, "Rust standard library license texts", "installed pinned Rust toolchain/share/doc/rust/licenses/" + file.name)
    write_json(stage / "licenses/index.json", {"scope": "product normal/build dependency closure; build-time macro licenses included conservatively", "files": records})
    return records


def promote(stage, zip_path, sha_path, dist, extras=()):
    """All validation precedes promotion; rollback handled failures to prior outputs."""
    changes, backups = [], []
    try:
        for source, name in [(stage, "windows-x64-cpu"), (zip_path, "windows-x64-cpu.zip"), (sha_path, "windows-x64-cpu.zip.sha256"), *extras]:
            dest = dist / name
            if dest.exists() or dest.is_symlink():
                if dest.is_symlink() or getattr(dest.lstat(), "st_file_attributes", 0) & 0x400:
                    fail("final package destination is a symlink/reparse point")
                backup = dist / (".previous-" + uuid.uuid4().hex + "-" + name)
                dest.rename(backup)
                backups.append((backup, dest))
            source.rename(dest)
            changes.append(dest)
    except BaseException:
        for dest in reversed(changes):
            shutil.rmtree(dest) if dest.is_dir() else dest.unlink()
        for backup, dest in reversed(backups):
            backup.rename(dest)
        raise
    for backup, _ in backups:
        shutil.rmtree(backup) if backup.is_dir() else backup.unlink()


def build():
    if sys.platform != "win32":
        fail("native Windows x64 MSVC host required; Linux cannot build or certify this package")
    if sys.maxsize <= 2**32:
        fail("64-bit Python is required for the x64 Windows packaging host")
    for key in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS", "CL", "_CL_", "CFLAGS", "CXXFLAGS"):
        if os.environ.get(key):
            fail(f"packaging refuses implicit {key}; no Rust flags are silently changed")
    selected, vs, env, folded, redist = selected_visual_studio()
    git = lambda *args: command(["git", *args], env)
    vendor_commit = git("-C", "vendor/llama.cpp", "rev-parse", "HEAD")
    if vendor_commit != LLAMA_COMMIT or git("-C", "vendor/llama.cpp", "status", "--porcelain", "--untracked-files=all"):
        fail("llama.cpp vendor source differs from its locked clean commit")
    project = {"commit": git("rev-parse", "HEAD"), "tree": git("rev-parse", "HEAD^{tree}"), "dirty": bool(git("status", "--porcelain", "--untracked-files=all")), "diff_sha256": hashlib.sha256(command(["git", "diff", "--binary", "HEAD"], env).encode()).hexdigest()}
    source_files = []
    for name in git("ls-files", "-z", "--cached", "--others", "--exclude-standard").split("\0"):
        if name and (ROOT / name).is_file():
            relative(name)
            source_files.append({"path": name, "sha256": digest(ROOT / name)})
    project["worktree_files_sha256"] = hashlib.sha256(json.dumps(source_files, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    # Native identity and compiler outputs may contain developer paths; they live
    # in the private build manifest, never in runtime logs or API smoke reports.
    rust = command(["rustc", "-vV"], env)
    if "release: 1.98.1\n" not in rust + "\n" or "host: x86_64-pc-windows-msvc" not in rust:
        fail("pinned Rust 1.98.1 native x86_64 MSVC toolchain is required")
    native, configure_native = native_build_settings(selected, vs, env)
    cmake = checked_cmake(env, configure_native[configure_native.index("-G") + 1])
    build_root = ROOT / "build/windows-x64-cpu"
    build_root.mkdir(parents=True, exist_ok=True)
    # cc-rs build outputs can otherwise remain cached after switching VS/MSVC.
    cargo_target = build_root / ("cargo-" + native.name.removeprefix("native-"))
    env["CARGO_TARGET_DIR"] = str(cargo_target)
    # Prove the API/CLI has no native linkage in a separate Release build first.
    env["AIR_NATIVE_DIR"] = str(build_root / "deliberately-absent-native")
    tree = command(["cargo", "tree", "--locked", "-p", "runtime-cli", "--target", TARGET, "--edges", "normal"], env)
    if re.search(r"\b(engine-host|llama-adapter|runtime-worker)\b", tree):
        fail("management CLI has an unexpected native inference dependency")
    command(["cargo", "build", "--locked", "--release", "--target", TARGET, "-p", "runtime-cli", "--bin", "ai-runtime"], env)
    command(configure_native, env)
    native_build_settings(selected, vs, env)
    command(["cmake", "--build", native, "--config", "Release", "--target", "air_llama", "--parallel", env.get("CMAKE_BUILD_PARALLEL_LEVEL", "2")], env)
    env["AIR_NATIVE_DIR"] = str(native)
    command(["cargo", "build", "--locked", "--release", "--target", TARGET, "-p", "runtime-worker", "--bin", "ai-runtime-worker"], env)
    env["AIR_NATIVE_DIR"] = str(build_root / "deliberately-absent-native")
    helper_tree = command(["cargo", "tree", "--locked", "-p", "xtask", "--target", TARGET, "--edges", "normal"], env)
    if re.search(r"\b(engine-host|llama-adapter|runtime-worker)\b", helper_tree):
        fail("acceptance tool has an unexpected native inference dependency")
    command(["cargo", "build", "--locked", "--release", "--target", TARGET, "-p", "xtask", "--bin", "nexa-acceptance"], env)
    metadata = json.loads(command(["cargo", "metadata", "--locked", "--format-version", "1", "--filter-platform", TARGET], env))
    identity = dict(line.split("=", 1) for line in (native / "air-native-Release.txt").read_text(encoding="utf-8").splitlines())
    if identity.get("system") != "Windows" or identity.get("configuration") != "Release" or identity.get("crt") != "MD":
        fail("native Release /MD build identity mismatch")
    dist = ROOT / "dist"
    dist.mkdir(exist_ok=True)
    if dist.is_symlink() or getattr(dist.lstat(), "st_file_attributes", 0) & 0x400:
        fail("dist must not be a symlink/reparse point")
    with tempfile.TemporaryDirectory(prefix=".windows-stage-", dir=dist) as staging:
        staging = Path(staging)
        stage = staging / "windows-x64-cpu"
        stage.mkdir()
        for binary in ("ai-runtime.exe", "ai-runtime-worker.exe"):
            shutil.copyfile(regular(cargo_target / TARGET / "release" / binary), stage / binary)
        templates = ROOT / "packaging/windows-x64-cpu"
        for name in ("README.md", "config.example.toml", "THIRD_PARTY_NOTICES.md"):
            shutil.copyfile(regular(templates / name), stage / name)
        crt_sources = []
        pe_evidence = {}
        dumpbin = selected_msvc_tool(vs, env, "dumpbin.exe")
        def inspect(path):
            pe_machine(path)
            headers = command([dumpbin, "/nologo", "/headers", path], env)
            dependencies = command([dumpbin, "/nologo", "/dependents", path], env)
            pe_evidence[path.name] = {"headers": headers, "dependents": dependencies}
            return parse_dependents(dependencies)
        def copy_crt(source, dest):
            regular(source)
            # Version/signature must describe the original DLL before copying.
            info = authenticode_info(source, env)
            pe_machine(source)
            shutil.copyfile(source, dest)
            if digest(source) != digest(dest):
                fail("CRT DLL changed while copying")
            crt_sources.append({"path": dest.name, "source_relative_to_visual_studio": source.relative_to(vs).as_posix(), "sha256": digest(source), "size_bytes": source.stat().st_size, **info})
        dependencies = collect_dependencies(stage, redist, inspect, copy_crt)
        copy_licenses(stage, metadata, env)
        consolidate_licenses(stage)
        replacements = [(str(ROOT), "<project>"), (str(vs), "<visual-studio>"), (os.environ.get("USERPROFILE", ""), "<user-profile>"), (str(Path(command(["rustc", "--print", "sysroot"], env))), "<rust-toolchain>")]
        public_identity = sanitize(identity, replacements)
        manifest = {
            "schema_version": 1, "product": "nexa-runtime", "package_version": tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]["package"]["version"],
            "platform": "windows-x64", "backend": "cpu", "target": TARGET, "configuration": "Release", "architecture": "x86_64", "crt": "MD",
            "protocol_version": 1, "worker_protocol_version": 2, "shim_version": 3,
            "project_commit": project["commit"], "project_tree": project["tree"], "project_dirty": project["dirty"], "source": project,
            "llama_commit": LLAMA_COMMIT, "cargo_lock_sha256": digest(ROOT / "Cargo.lock"),
            "cpu_baseline": [k.removeprefix("GGML_").lower() for k in ("GGML_SSE42", "GGML_AVX", "GGML_AVX2", "GGML_F16C", "GGML_FMA", "GGML_BMI2", "GGML_AVX512") if identity.get(k) == "ON"],
            "native_build": public_identity,
            "native_archives": [{"name": k.removeprefix("library."), "sha256": digest(Path(v)), "size_bytes": Path(v).stat().st_size} for k,v in identity.items() if k.startswith("library.")],
            "toolchain": {"rustc": rust, "cargo": command(["cargo", "--version"], env), "cmake": cmake, "visual_studio": {key: selected.get(key) for key in ("instanceId", "installationVersion", "productId", "displayName", "channelId", "isPrerelease")}, "msvc": identity.get("compiler_version"), "vc_tools_version": folded.get("VCTOOLSVERSION"), "windows_sdk_version": identity.get("windows_sdk_version"), "vsdevcmd_windows_sdk_version": folded.get("WINDOWSSDKVERSION"), "vctools_redist_dir": folded.get("VCTOOLSREDISTDIR"), "host_os": powershell("[Environment]::OSVersion.VersionString", env), "image_os": env.get("IMAGEOS"), "image_version": env.get("IMAGEVERSION")},
            "management_without_native": {"verified": True, "profile": "release", "dependencies": tree},
            "dependencies": dependencies, "crt_sources": crt_sources,
            "redistribution": visual_studio_redistribution(selected),
            "acceptance": {"windows10_clean_machine": "not_run", "windows11": "not_run", "real_model": "separate acceptance report required", "target": "Windows 10 x64 first; Server 2022 build does not establish Windows 10 compatibility"},
            "files": entries(stage),
        }
        manifest = sanitize(manifest, replacements)
        write_json(stage / "manifest.json", manifest)
        (stage / "SHA256SUMS").write_text("".join(f"{x['sha256']}  {x['path']}\n" for x in entries(stage)), encoding="utf-8")
        verify_package(stage, manifest)
        archive = staging / "windows-x64-cpu.zip"
        with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as zipped:
            for file in entries(stage):
                zipped.write(stage / file["path"], "windows-x64-cpu/" + file["path"])
        with zipfile.ZipFile(archive) as zipped:
            if zipped.testzip() is not None or len(zipped.namelist()) != len(entries(stage)):
                fail("zip validation failed")
        checksum = staging / "windows-x64-cpu.zip.sha256"
        checksum.write_text(f"{digest(archive)}  windows-x64-cpu.zip\n", encoding="utf-8")
        # The verifier is a separate tool bundle with its own actual CRT closure.
        helper = staging / "acceptance-tools"
        helper.mkdir()
        shutil.copyfile(regular(cargo_target / TARGET / "release/nexa-acceptance.exe"), helper / "nexa-acceptance.exe")
        shutil.copyfile(templates / "THIRD_PARTY_NOTICES.md", helper / "THIRD_PARTY_NOTICES.md")
        crt_sources = []
        helper_dependencies = collect_dependencies(helper, redist, inspect, copy_crt, ("nexa-acceptance.exe",))
        copy_licenses(helper, metadata, env, roots=("xtask",))
        consolidate_licenses(helper)
        helper_manifest = {k: v for k, v in manifest.items() if k not in ("files", "native_build", "native_archives", "cpu_baseline", "acceptance", "management_without_native")}
        helper_manifest.update(product="nexa-acceptance-tools", dependencies=helper_dependencies, crt_sources=crt_sources, files=entries(helper), native_inference_linkage=False, helper_without_native={"verified": True, "profile": "release", "dependencies": helper_tree})
        helper_manifest = sanitize(helper_manifest, replacements)
        write_json(helper / "manifest.json", helper_manifest)
        (helper / "SHA256SUMS").write_text("".join(f"{x['sha256']}  {x['path']}\n" for x in entries(helper)), encoding="utf-8")
        verify_package(helper, helper_manifest)
        helper_zip = staging / "acceptance-tools.zip"
        with zipfile.ZipFile(helper_zip, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as zipped:
            for file in entries(helper):
                zipped.write(helper / file["path"], "acceptance-tools/" + file["path"])
        with zipfile.ZipFile(helper_zip) as zipped:
            if zipped.testzip() is not None or len(zipped.namelist()) != len(entries(helper)):
                fail("acceptance tool ZIP validation failed")
        helper_sha = staging / "acceptance-tools.zip.sha256"
        helper_sha.write_text(f"{digest(helper_zip)}  acceptance-tools.zip\n", encoding="utf-8")
        evidence = ROOT / "artifacts/verification/windows-package"
        evidence.mkdir(parents=True, exist_ok=True)
        write_json(evidence / "pe-inspection.json", sanitize(pe_evidence, replacements))
        build_result = {"build_status": "pass", "project_commit": project["commit"], "package_sha256": digest(archive), "compressed_bytes": archive.stat().st_size, "installed_bytes": sum(x["size_bytes"] for x in entries(stage)), "runtime_executables_bytes": sum((stage / name).stat().st_size for name in ("ai-runtime.exe", "ai-runtime-worker.exe")), "app_local_crt_bytes": sum(x["size_bytes"] for x in manifest["crt_sources"]), "ui_bytes": 0, "model_bytes": 0, "symbols_in_package": False, "manifest_sha256": digest(stage / "manifest.json"), "runtime_acceptance": "not_run_by_packager", "acceptance_tools_sha256": digest(helper_zip), "acceptance_tools_compressed_bytes": helper_zip.stat().st_size}
        if git("rev-parse", "HEAD") != project["commit"] or git("rev-parse", "HEAD^{tree}") != project["tree"]:
            fail("project commit/tree changed during packaging")
        if git("-C", "vendor/llama.cpp", "rev-parse", "HEAD") != LLAMA_COMMIT or git("-C", "vendor/llama.cpp", "status", "--porcelain", "--untracked-files=all"):
            fail("locked vendor source changed during packaging")
        for source_file in source_files:
            if not (ROOT / source_file["path"]).is_file() or digest(ROOT / source_file["path"]) != source_file["sha256"]:
                fail("source files changed during packaging; staged outputs will not be published")
        promote(stage, archive, checksum, dist, [(helper, "acceptance-tools"), (helper_zip, "acceptance-tools.zip"), (helper_sha, "acceptance-tools.zip.sha256")])
        write_json(evidence / "build-result.json", build_result)
    print("Built and integrity-checked dist/windows-x64-cpu and windows-x64-cpu.zip; real-model and Windows 10 clean-machine acceptance are separate gates")


if __name__ == "__main__":
    try:
        if len(sys.argv) != 1:
            fail("package_windows.py takes no options; use xtask build --platform windows-x64 --backend cpu")
        build()
    except (ValueError, OSError, KeyError, json.JSONDecodeError, struct.error) as error:
        print(f"Windows package failed: {error}", file=sys.stderr)
        sys.exit(1)
