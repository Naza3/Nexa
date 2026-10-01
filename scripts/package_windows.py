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
    return command(["powershell.exe", "-NoLogo", "-NoProfile", "-NonInteractive", "-Command", "$ErrorActionPreference='Stop';" + script], env)


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
    required = {"nexa-acceptance.exe", "THIRD_PARTY_NOTICES.md", "licenses/index.json"} if helper else {"ai-runtime.exe", "ai-runtime-worker.exe", "config.example.toml", "README.md", "THIRD_PARTY_NOTICES.md", "licenses/index.json"}
    if not required <= names:
        fail("required package content missing")
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


def selected_visual_studio():
    env = windows_environment(os.environ, "")
    base = Path(env.get("PROGRAMFILES(X86)", "C:/Program Files (x86)"))
    vswhere = regular(base / "Microsoft Visual Studio/Installer/vswhere.exe")
    choices = json.loads(command([vswhere, "-latest", "-version", "[17.0,18.0)", "-products", "*", "-requires", "Microsoft.VisualStudio.Component.VC.Tools.x86.x64", "-format", "json", "-utf8"], env))
    if len(choices) != 1:
        fail("one existing Visual Studio C++ instance is required; no installation is attempted")
    selected = choices[0]
    if selected.get("isPrerelease") or not selected.get("isComplete", False):
        fail("pre-release or incomplete Visual Studio instances are not distribution sources")
    vs = Path(selected["installationPath"]).resolve()
    dev = regular(vs / "Common7/Tools/VsDevCmd.bat")
    output = command(["cmd.exe", "/d", "/s", "/c", f'""{dev}" -no_logo -arch=x64 -host_arch=x64 >nul && set"'], env)
    env = windows_environment(env, output)
    folded = env.copy()
    value = folded.get("VCTOOLSREDISTDIR")
    if not value:
        fail("selected VS has no VCToolsRedistDir; installing a redistributable requires separate authorization")
    redist_dir = Path(value).resolve() / "x64/Microsoft.VC143.CRT"
    try:
        redist_dir.relative_to(vs / "VC/Redist/MSVC")
    except ValueError:
        fail("CRT source is outside the selected Visual Studio redistribution directory")
    if not redist_dir.is_dir() or any("debug" in p.lower() for p in redist_dir.parts):
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
    return selected, vs, env, folded, redist


def license_files(package_dir, explicit=None):
    files = []
    if explicit:
        files.append(package_dir / explicit)
    for path in sorted(package_dir.iterdir()):
        if path.name.lower().startswith(("license", "licence", "copying", "notice", "unlicense")):
            files.extend([path] if path.is_file() else [x for x in path.rglob("*") if x.is_file()])
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
    cmake = command(["cmake", "--version"], env)
    if not cmake.startswith("cmake version 4.4.3"):
        fail("CMake 4.4.3 is required by the development build lock")
    build_root = ROOT / "build/windows-x64-cpu"
    build_root.mkdir(parents=True, exist_ok=True)
    native = ROOT / "build/native-release"
    cargo_target = build_root / "cargo"
    env["CARGO_TARGET_DIR"] = str(cargo_target)
    # Prove the API/CLI has no native linkage in a separate Release build first.
    env["AIR_NATIVE_DIR"] = str(build_root / "deliberately-absent-native")
    tree = command(["cargo", "tree", "--locked", "-p", "runtime-cli", "--target", TARGET, "--edges", "normal"], env)
    if re.search(r"\b(engine-host|llama-adapter|runtime-worker)\b", tree):
        fail("management CLI has an unexpected native inference dependency")
    command(["cargo", "build", "--locked", "--release", "--target", TARGET, "-p", "runtime-cli", "--bin", "ai-runtime"], env)
    command(["cmake", "-S", ROOT / "native/llama-shim", "-B", native, "-G", "Visual Studio 17 2022", "-A", "x64", f"-DCMAKE_GENERATOR_INSTANCE={vs}", "-DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreadedDLL"], env)
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
        dumpbin = regular(Path(command(["where.exe", "dumpbin.exe"], env).splitlines()[0]))
        if not dumpbin.resolve().is_relative_to(vs):
            fail("dumpbin is not from the selected Visual Studio instance")
        def inspect(path):
            pe_machine(path)
            headers = command([dumpbin, "/nologo", "/headers", path], env)
            dependencies = command([dumpbin, "/nologo", "/dependents", path], env)
            pe_evidence[path.name] = {"headers": headers, "dependents": dependencies}
            return parse_dependents(dependencies)
        def copy_crt(source, dest):
            regular(source)
            # Version/signature must describe the original DLL before copying.
            info = json.loads(powershell("$p=" + ps_literal(source) + "; $s=Get-AuthenticodeSignature -LiteralPath $p; $f=Get-Item -LiteralPath $p; @{signature_status=[string]$s.Status; signer=[string]$s.SignerCertificate.Subject; file_version=$f.VersionInfo.FileVersion; product_version=$f.VersionInfo.ProductVersion} | ConvertTo-Json -Compress", env))
            if info["signature_status"] != "Valid" or "Microsoft Corporation" not in info["signer"]:
                fail("CRT DLL does not have a valid Microsoft signature")
            pe_machine(source)
            shutil.copyfile(source, dest)
            if digest(source) != digest(dest):
                fail("CRT DLL changed while copying")
            crt_sources.append({"path": dest.name, "source_relative_to_visual_studio": source.relative_to(vs).as_posix(), "sha256": digest(source), "size_bytes": source.stat().st_size, **info})
        dependencies = collect_dependencies(stage, redist, inspect, copy_crt)
        copy_licenses(stage, metadata, env)
        replacements = [(str(ROOT), "<project>"), (str(vs), "<visual-studio>"), (os.environ.get("USERPROFILE", ""), "<user-profile>"), (str(Path(command(["rustc", "--print", "sysroot"], env))), "<rust-toolchain>")]
        public_identity = sanitize(identity, replacements)
        manifest = {
            "schema_version": 1, "product": "nexa-runtime", "package_version": tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]["package"]["version"],
            "platform": "windows-x64", "backend": "cpu", "target": TARGET, "configuration": "Release", "architecture": "x86_64", "crt": "MD",
            "protocol_version": 1, "worker_protocol_version": 1, "shim_version": 2,
            "project_commit": project["commit"], "project_tree": project["tree"], "project_dirty": project["dirty"], "source": project,
            "llama_commit": LLAMA_COMMIT, "cargo_lock_sha256": digest(ROOT / "Cargo.lock"),
            "cpu_baseline": [k.removeprefix("GGML_").lower() for k in ("GGML_SSE42", "GGML_AVX", "GGML_AVX2", "GGML_F16C", "GGML_FMA", "GGML_BMI2", "GGML_AVX512") if identity.get(k) == "ON"],
            "native_build": public_identity,
            "native_archives": [{"name": k.removeprefix("library."), "sha256": digest(Path(v)), "size_bytes": Path(v).stat().st_size} for k,v in identity.items() if k.startswith("library.")],
            "toolchain": {"rustc": rust, "cargo": command(["cargo", "--version"], env), "cmake": cmake, "visual_studio": {key: selected.get(key) for key in ("instanceId", "installationVersion", "productId", "displayName", "channelId", "isPrerelease")}, "msvc": identity.get("compiler_version"), "vc_tools_version": folded.get("VCTOOLSVERSION"), "windows_sdk_version": identity.get("windows_sdk_version"), "vsdevcmd_windows_sdk_version": folded.get("WINDOWSSDKVERSION"), "vctools_redist_dir": folded.get("VCTOOLSREDISTDIR"), "host_os": powershell("[Environment]::OSVersion.VersionString", env), "image_os": env.get("IMAGEOS"), "image_version": env.get("IMAGEVERSION")},
            "management_without_native": {"verified": True, "profile": "release", "dependencies": tree},
            "dependencies": dependencies, "crt_sources": crt_sources,
            "redistribution": {"purpose": "private development acceptance", "source_rule": "unmodified Release x64 files from the selected Visual Studio VC/Redist/MSVC tree", "redist_list": "https://learn.microsoft.com/en-us/visualstudio/releases/2022/redistribution", "community_terms_reference": "https://visualstudio.microsoft.com/wp-content/uploads/2021/11/Visual-Studio-2022-Community-License-EN.docx", "edition": selected.get("productId"), "license_acceptance_performed": False},
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
