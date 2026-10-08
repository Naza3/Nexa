#!/usr/bin/env python3
"""Package a verified Windows payload with the locked Tauri WiX/NSIS bundlers.

The temporary Cargo project supplies packaging metadata only. No Rust is built,
no source manifest is rewritten, and no binary is patched or signed here.
"""
from __future__ import annotations

import argparse
import json
import os
import re
from pathlib import Path
import shutil
import subprocess
import sys

import package_windows_msi as legacy
import package_windows as base
from release_version import repository_version, validate_version
from tauri_wix import generate_fragment, UPGRADE_CODE

ROOT = base.ROOT
TEMPLATES = ROOT / "packaging/tauri/windows"
TAURI_VERSION = "2.12.1"
TARGET = "x86_64-pc-windows-msvc"
BUILD_CHECKS = {"payload_verified", "tauri_bundles_verified", "input_bytes_unchanged"}
HELPER_IMPORTS = {"kernel32.dll", "user32.dll", "msi.dll", "shell32.dll", "ole32.dll",
                  "bcrypt.dll", "comctl32.dll", "advapi32.dll", "gdi32.dll", "version.dll"}
IMPLICIT_COMPILER_FLAGS = ("CL", "_CL_", "LINK", "_LINK_", "CFLAGS", "CXXFLAGS")


def nsis_string(value):
    value = str(value)
    if any(ord(c) < 32 for c in value):
        raise ValueError("control character in NSIS path")
    return value.replace("$", "$$").replace('"', '$\\"')


def write_inventory(work, files):
    paths = [legacy.checked_relative(item["path"]) for item in files]
    if not paths or len(paths) > legacy.MAX_FILES or len({p.casefold() for p in paths}) != len(paths):
        raise ValueError("invalid installer guard inventory")
    paths.append("uninstall.exe")
    literals = ["L" + json.dumps(p.replace("/", "\\")) for p in sorted(set(paths))]
    (work / "inventory.h").write_text(
        "/* Generated from the verified program-file inventory. */\n"
        "static const WCHAR *const NEXA_OWNED_PATHS[] = {\n" + ",\n".join(literals) + "\n};\n"
        "#define NEXA_OWNED_COUNT " + str(len(literals)) + "\n", encoding="ascii")


def verify_helper_imports(output, env, dumpbin=None):
    if dumpbin is not None:
        imports = base.parse_dependents(base.command([dumpbin, "/nologo", "/dependents", output], env))
    else:
        # Native Linux objdump can inspect PE imports without executing Windows code.
        listing = base.command(["objdump", "-p", output], env)
        names = re.findall(r"^\s*DLL Name:\s*([^\r\n]+)", listing, re.MULTILINE)
        imports = base.parse_dependents("\n".join(names))
    if not set(imports) <= HELPER_IMPORTS:
        raise ValueError("installer helper has non-inbox/CRT dependencies: " + str(imports))
    return imports


def compile_helpers(work, files):
    """Build bounded guards and MSI-owned startup cleanup; no custom Setup EXE."""
    for name in IMPLICIT_COMPILER_FLAGS:
        if os.environ.get(name):
            raise ValueError("installer refuses implicit " + name)
    work = base.resolve_checked_path(work)
    work.mkdir(parents=True, exist_ok=True)
    write_inventory(work, files)
    source = ROOT / "packaging/windows-installer"
    if sys.platform == "win32":
        _, vs, env, _, _ = base.selected_visual_studio()
        compiler = base.selected_msvc_tool(vs, env, "cl.exe")
        dumpbin = base.selected_msvc_tool(vs, env, "dumpbin.exe")
        rc = Path(env["WINDOWSSDKDIR"]) / "bin" / env["WINDOWSSDKVERSION"].rstrip("\\/") / "x64/rc.exe"
        compiler_flags, linker_flags = [], []
    else:
        # Explicitly prepared MSVC-compatible SDK, not a claim of Windows execution.
        sdk = Path(os.environ["NEXA_XWIN_ROOT"]).resolve()
        env = os.environ.copy()
        compiler, rc = "clang-cl", "llvm-rc"
        dumpbin = None
        compiler_flags = ["--target=" + TARGET, "-fuse-ld=lld", *["/imsvc" + str(sdk / part) for part in
                          ("crt/include", "sdk/include/ucrt", "sdk/include/um", "sdk/include/shared", "sdk/include/winrt")]]
        linker_flags = ["/libpath:" + str(sdk / part) for part in
                        ("crt/lib/x86_64", "sdk/lib/um/x86_64", "sdk/lib/ucrt/x86_64")]
    common = [compiler, *compiler_flags, "/nologo", "/W4", "/WX", "/O1", "/GS-", "/Zl",
              "/DUNICODE", "/D_UNICODE", "/D_WIN32_WINNT=0x0A00", "/I" + str(source), "/I" + str(work)]
    resource = work / "check.rc"
    manifest = str(source / "setup.manifest").replace("\\", "\\\\")
    resource.write_text('1 24 "' + manifest + '"\n', encoding="utf-8")
    base.command([rc, "/nologo", "/fo", work / "check.res", resource], env, cwd=ROOT)
    result = {}
    for role, filename, entry in (("guard", "guard.c", None), ("os", "os_check.c", "OsCheckEntry"),
                                   ("check", "install_check.c", "CheckEntry")):
        output = work / ("nexa-installer-guard.dll" if role == "guard" else "nexa-os-check.exe" if role == "os" else "nexa-install-check.exe")
        inputs = [str((source / filename).relative_to(ROOT))]
        base.command([*common, *([] if entry else ["/LD"]), *inputs, "/Fo" + str(work / (role + ".obj")),
                      "/link", *linker_flags, *([str(work / "check.res")] if entry else []), "/NODEFAULTLIB", "/MACHINE:X64", "/DYNAMICBASE", "/NXCOMPAT",
                      *( ["/ENTRY:" + entry, "/SUBSYSTEM:WINDOWS"] if entry else ["/NOENTRY", "/IMPLIB:" + str(work / "guard.lib")]),
                      "/OUT:" + str(output), "kernel32.lib", "user32.lib", "msi.lib", "shell32.lib", "ole32.lib", "uuid.lib", "advapi32.lib"], env, cwd=ROOT)
        base.pe_machine(output)
        verify_helper_imports(output, env, dumpbin)
        result[role] = output
    return result


def prepare_project(payload, files, version, work, helpers):
    """Create isolated Tauri metadata and exact resource mappings for both formats."""
    version = validate_version(version)
    project = work / "project"
    (project / "src").mkdir(parents=True)
    (project / "Cargo.toml").write_text(
        '[package]\nname = "nexa-desktop"\nversion = "' + version + '"\nedition = "2021"\n[workspace]\n', encoding="utf-8")
    (project / "src/main.rs").write_text("// Packaging metadata only; never compiled.\nfn main() {}\n", encoding="utf-8")
    target = work / "target" / TARGET / "release"
    target.mkdir(parents=True)
    shutil.copyfile(base.regular(payload / "nexa-desktop.exe"), target / "nexa-desktop.exe")
    config = json.loads((ROOT / "apps/desktop/src-tauri/tauri.conf.json").read_text(encoding="utf-8"))
    config = {key: config[key] for key in ("productName", "identifier")}
    config["version"] = version
    config["bundle"] = {
        "active": True, "targets": ["msi", "nsis"], "publisher": "Nexa",
        "homepage": "https://github.com/Naza3/Nexa", "createUpdaterArtifacts": False,
        "icon": [str(ROOT / "apps/desktop/src-tauri/icons/icon.ico")],
        "windows": {"allowDowngrades": False, "bundleVCRuntime": False,
                    "webviewInstallMode": {"type": "skip"},
                    "wix": {"upgradeCode": UPGRADE_CODE, "language": "en-US",
                            "template": str(TEMPLATES / "main.wxs"), "enableElevatedUpdateTask": False},
                    "nsis": {"installMode": "currentUser", "languages": ["SimpChinese", "English"],
                             "startMenuFolder": "Nexa", "template": str(TEMPLATES / "installer.nsi")}},
    }
    (project / "tauri.conf.json").write_text(json.dumps(config, indent=2) + "\n", encoding="utf-8")
    fragment = generate_fragment(payload, files, helpers["guard"], helpers["os"], work / "payload.wxs")
    msi_config = {"bundle": {"resources": {}, "windows": {"wix": {
        "fragmentPaths": [str(fragment)], "componentGroupRefs": ["NexaPayloadGroup"]}}}}
    hooks = work / "hooks.nsh"
    hooks.write_text('!define NEXA_CHECK_SOURCE "' + nsis_string(helpers["check"]) + '"\n', encoding="utf-8")
    resources = {str(payload / item["path"]): item["path"] for item in files if item["path"] != "nexa-desktop.exe"}
    nsis_config = {"bundle": {"resources": resources, "windows": {"nsis": {"installerHooks": str(hooks)}}}}
    for kind, value in (("msi", msi_config), ("nsis", nsis_config)):
        (project / (kind + ".json")).write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
    return project


def bundle(project, kind, work, version):
    cli_root = ROOT / "apps/desktop/node_modules/@tauri-apps/cli"
    if json.loads((cli_root / "package.json").read_text(encoding="utf-8"))["version"] != TAURI_VERSION:
        raise ValueError("Tauri CLI version differs from the package lock")
    env = os.environ.copy()
    env["CARGO_TARGET_DIR"] = str(work / "target")
    env["CARGO_NET_OFFLINE"] = "true"
    # Signature/URL credentials are never required for unsigned local releases.
    base.command(["node", cli_root / "tauri.js", "bundle", "--ci", "--no-sign", "--no-binary-patching",
                  "--target", TARGET, "--bundles", kind, "--config", project / (kind + ".json")], env, cwd=project)
    suffix = ".msi" if kind == "msi" else ".exe"
    directory = work / "target" / TARGET / "release/bundle" / kind
    candidates = list(directory.glob("*" + suffix))
    if len(candidates) != 1:
        raise ValueError("Tauri did not produce exactly one " + kind + " installer")
    return base.regular(candidates[0])


def checked_layout(payload, work, report, msi, setup, targets):
    """Reject aliases, existing destinations and overlapping roots before writes."""
    paths = {"payload": payload, "work": work, "report": report, "msi": msi, "nsis": setup}
    for name in ("payload", "work", "report", *targets):
        if paths[name] is None:
            raise ValueError("missing installer path: " + name)
    paths = {name: base.resolve_checked_path(path) for name, path in paths.items() if path is not None}
    items = list(paths.items())
    for index, (name, path) in enumerate(items):
        for other_name, other in items[index + 1:]:
            if path.is_relative_to(other) or other.is_relative_to(path):
                raise ValueError("installer paths must be distinct and non-overlapping: " + name + "/" + other_name)
        if name != "payload" and path.exists():
            raise ValueError("installer destination must be new: " + name)
    return paths


def publish_outputs(artifacts):
    """Copy only completed artifacts; exclusive creation never replaces a file.

    A failed copy removes files created by this call, including a partial last
    file. The work directory remains available for diagnosis and retry.
    """
    created = []
    try:
        for source, destination, expected_hash in artifacts:
            # Recheck parent links after the build, before creating each output.
            if base.resolve_checked_path(destination) != destination:
                raise ValueError("installer output path changed during packaging")
            destination.parent.mkdir(parents=True, exist_ok=True)
            with destination.open("xb") as output:
                created.append(destination)
                with base.regular(source).open("rb") as input_file:
                    shutil.copyfileobj(input_file, output)
            if base.digest(destination) != expected_hash:
                raise ValueError("installer output copy differs from verified artifact")
    except BaseException:
        for destination in reversed(created):
            destination.unlink(missing_ok=True)
        raise


def create(payload, version, msi, setup, report, work, *, targets=("msi", "nsis")):
    if not targets or len(set(targets)) != len(targets) or not set(targets) <= {"msi", "nsis"}:
        raise ValueError("invalid installer target selection")
    if "msi" in targets and sys.platform != "win32":
        raise ValueError("Tauri WiX MSI packaging requires native Windows")
    paths = checked_layout(payload, work, report, msi, setup, targets)
    payload, manifest, files = legacy.validate_payload(paths["payload"], version)
    work = paths["work"]
    work.mkdir(parents=True)
    helpers = compile_helpers(work / "helpers", files)
    project = prepare_project(payload, files, version, work, helpers)
    built = {kind: bundle(project, kind, work, version) for kind in targets}
    if base.entries(payload) != files or base.digest(work / "target" / TARGET / "release/nexa-desktop.exe") != base.digest(payload / "nexa-desktop.exe"):
        raise ValueError("Tauri packaging changed the verified payload")
    value = {"schema_version": 1, "status": "pass", "version": version,
             "project_commit": manifest["project_commit"], "payload_manifest_sha256": base.digest(payload / "manifest.json"),
             "unsigned": True, "packager": "tauri", "tauri_cli": TAURI_VERSION, "scope": "per-user",
             "formats": list(targets), "checks": dict.fromkeys(BUILD_CHECKS, True)}
    artifacts = []
    for kind, source in built.items():
        digest = base.digest(source)
        value["msi_sha256" if kind == "msi" else "setup_sha256"] = digest
        artifacts.append((source, paths[kind], digest))
    staged_report = work / "build-report.json"
    base.write_json(staged_report, value)
    artifacts.append((staged_report, paths["report"], base.digest(staged_report)))
    publish_outputs(artifacts)
    return value


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--payload", type=Path, default=ROOT / "dist/desktop-windows")
    parser.add_argument("--version", default=repository_version(ROOT))
    parser.add_argument("--output", type=Path)
    parser.add_argument("--setup-output", type=Path)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--work", type=Path, default=ROOT / "build/tauri-installers")
    parser.add_argument("--nsis-only", action="store_true", help="Linux cross-package check; never a complete Windows release")
    parser.add_argument("--check-toolchain", action="store_true")
    args = parser.parse_args()
    if args.check_toolchain:
        outputs = compile_helpers(args.work / "preflight", [{"path": "nexa-desktop.exe"}])
        base.write_json(args.report, {"status": "compile-pass", "installation_tested": False,
                                    "helpers": {key: base.digest(value) for key, value in outputs.items()}})
        return
    if not args.setup_output or (not args.nsis_only and not args.output):
        parser.error("installer output paths are required")
    create(args.payload, args.version, args.output, args.setup_output, args.report, args.work,
           targets=("nsis",) if args.nsis_only else ("msi", "nsis"))


if __name__ == "__main__":
    main()
