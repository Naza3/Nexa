#!/usr/bin/env python3
"""Package an already built Tauri Release EXE plus an intact matching T05 runtime.

No model, user data, credentials, WebView2 installer, or updater is bundled.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import sys
import tempfile
import zipfile
import package_windows as base
import build_aria2_windows as aria2_build

ROOT = base.ROOT
SHELL = ROOT / "apps/desktop/src-tauri"
# These are Windows inbox components, never copied from System32. Unknown imports
# remain a hard failure and must be reviewed before adding an OS classification.
DESKTOP_OS = base.OS_DLLS | set("comctl32 d3d11 d3d12 d3dcompiler_47 dcomp dwmapi dxgi imm32 msimg32 propsys shcore uiautomationcore urlmon usp10 uxtheme windowscodecs wininet winspool wintrust wtsapi32".split())


DOWNLOAD_LICENSES = {"COPYING", "COPYING.MinGW-w64-runtime.txt", "COPYING.MinGW-w64.txt",
    "COPYING.winpthreads.txt", "COPYING.winstorecompat.txt", "aria2-COPYING",
    "compiler-rt-LICENSE.TXT", "libcxx-LICENSE.TXT", "libcxxabi-LICENSE.TXT",
    "libunwind-LICENSE.TXT", "llvm-mingw-LICENSE.TXT"}
DOWNLOAD_SOURCE = "aria2-1.37.0-nexa-corresponding-source.tar.gz"
DOWNLOAD_FILES = {"nexa-aria2.exe", DOWNLOAD_SOURCE, "build-manifest.json"} | {"licenses/" + name for name in DOWNLOAD_LICENSES}


def verify_download(stage, commit, dirty):
    manifest = json.loads(base.regular(stage / "manifest.json").read_text(encoding="utf-8"))
    entries = base.entries(stage)
    payload = [item for item in entries if item["path"] not in {"manifest.json", "SHA256SUMS"}]
    if (manifest.get("product") != "nexa-download" or manifest.get("project_commit") != commit or
            manifest.get("project_dirty") != dirty or manifest.get("files") != payload or
            {item["path"] for item in payload} != DOWNLOAD_FILES):
        base.fail("download component inventory/source mismatch")
    # No unknown empty directory is part of the closed component either.
    if {p.relative_to(stage).as_posix() for p in stage.rglob("*") if p.is_dir()} != {"licenses"}:
        base.fail("download component directory mismatch")
    build = json.loads(base.regular(stage / "build-manifest.json").read_text(encoding="utf-8"))
    lock = json.loads((ROOT / "third_party/aria2/source-lock.json").read_text(encoding="utf-8"))
    if (build.get("schema_version") != 1 or build.get("source_commit") != commit or
            build.get("source_lock") != lock or build.get("binary_name") != "nexa-aria2.exe" or
            build.get("product_relative_path") != "download/nexa-aria2.exe" or
            build.get("target") != "x86_64-w64-mingw32" or build.get("tls_backend") != "Schannel"):
        base.fail("download build provenance mismatch")
    if (build.get("features", {}).get("SECURITY_WIN32") is not True or build.get("features", {}).get("ENABLE_SSL") is not True or
            any(build.get("features", {}).get(name) is not False for name in aria2_build.FORBIDDEN)):
        base.fail("download build feature profile mismatch")
    for item in payload:
        if item["path"] == "build-manifest.json":
            continue
        expected = build["files"].get(item["path"])
        if expected != {"sha256": item["sha256"], "bytes": item["size_bytes"]}:
            base.fail("download payload differs from source-build artifact")
    base.pe_machine(stage / "nexa-aria2.exe")
    expected = "".join(f"{item['sha256']}  {item['path']}\n" for item in entries if item["path"] != "SHA256SUMS")
    if base.regular(stage / "SHA256SUMS").read_text(encoding="utf-8") != expected:
        base.fail("download component checksum mismatch")
    return build


def prepare_download(artifact, destination, commit, dirty):
    if not re.fullmatch(r"[a-f0-9]{40}", commit) or destination.exists():
        base.fail("download component source/destination invalid")
    build = json.loads(base.regular(artifact / "build-manifest.json").read_text(encoding="utf-8"))
    # Verify the original artifact's complete hash inventory before selecting roles.
    records = {item["path"]: {"sha256": item["sha256"], "bytes": item["size_bytes"]}
               for item in base.entries(artifact) if item["path"] != "build-manifest.json"}
    if records != build["files"]:
        base.fail("original download build artifact closure mismatch")
    destination.mkdir(parents=True)
    for name in sorted(DOWNLOAD_FILES):
        target = destination / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(base.regular(artifact / name), target)
    base.write_json(destination / "manifest.json", {"schema_version": 1, "product": "nexa-download",
        "project_commit": commit, "project_dirty": dirty, "files": base.entries(destination)})
    (destination / "SHA256SUMS").write_text("".join(f"{item['sha256']}  {item['path']}\n" for item in base.entries(destination)), encoding="utf-8")
    verify_download(destination, commit, dirty)


def dependency_kind(name, redist):
    name = name.lower()
    if base.DEBUG_CRT.fullmatch(name):
        base.fail("Debug CRT is forbidden in desktop package")
    if name.removesuffix(".dll") in DESKTOP_OS or base.API_SET.fullmatch(name):
        return "os"
    if name in redist:
        return "app-local"
    base.fail(f"unknown desktop non-OS dependency: {name}")


def collect_dependencies(stage, redist, inspect, copy_crt):
    queue, result = ["nexa-desktop.exe"], {}
    while queue:
        name = queue.pop(0)
        if name in result:
            continue
        imports = []
        for dll in inspect(stage / name):
            kind = dependency_kind(dll, redist)
            imports.append({"name": dll, "kind": kind})
            if kind == "app-local" and dll not in result and dll not in queue:
                copy_crt(redist[dll], stage / dll)
                queue.append(dll)
        result[name] = {"imports": imports}
    return result


def source_identity(env):
    records = []
    names = base.command(["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"], env).split("\0")
    for name in names:
        if name and (ROOT / name).is_file():
            base.relative(name)
            records.append({"path": name, "sha256": base.digest(ROOT / name)})
    digest = hashlib.sha256(json.dumps(records, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    return {"commit": base.command(["git", "rev-parse", "HEAD"], env),
            "dirty": bool(base.command(["git", "status", "--porcelain", "--untracked-files=all"], env)),
            "worktree_files_sha256": digest}


def copy_rust_licenses(stage, metadata, env):
    supplements_root = ROOT / "packaging/desktop-windows/third-party"
    supplements = json.loads((supplements_root / "sources.json").read_text(encoding="utf-8"))
    packages = {p["id"]: p for p in metadata["packages"]}
    nodes = {n["id"]: n for n in metadata["resolve"]["nodes"]}
    queue = [p["id"] for p in metadata["packages"] if p["name"] == "nexa-desktop" and p["source"] is None]
    visited, records = set(), []
    def save(source, destination, record):
        base.relative(destination)
        target = stage / destination
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(base.regular(source), target)
        records.append({**record, "path": destination, "sha256": base.digest(target)})
    while queue:
        key = queue.pop()
        if key in visited:
            continue
        visited.add(key)
        queue.extend(d["pkg"] for d in nodes[key]["deps"] if any(kind["kind"] != "dev" for kind in d["dep_kinds"]))
        package = packages[key]
        if package["source"] is None:
            continue
        source = Path(package["manifest_path"]).parent
        name = package["name"] + "-" + package["version"]
        files = base.license_files(source, package.get("license_file"))
        if files:
            for file in files:
                save(file, "licenses/rust-crates/" + name + "/" + file.relative_to(source).as_posix(),
                     {"component": name, "license": package.get("license"), "source": package["source"], "repository": package.get("repository")})
        else:
            originals = [record for record in supplements["files"] if record["package"] == name]
            vcs = json.loads(base.regular(source / ".cargo_vcs_info.json").read_text(encoding="utf-8"))
            if not originals:
                base.fail(f"original license unavailable for desktop crate {name}")
            for record in originals:
                base.relative(record["path"])
                original = supplements_root / record["path"]
                if record["revision"] != vcs["git"]["sha1"] or base.digest(original) != record["sha256"]:
                    base.fail("supplemental license hash or original crate revision mismatch")
                save(original, "licenses/rust-crates/" + record["path"],
                     {"component": name, "license": package.get("license"), "source": record["source"], "source_revision": record["revision"], "repository": package.get("repository"), "reason": record.get("reason", "Registry package omits upstream license original")})
    sysroot = Path(base.command(["rustc", "--print", "sysroot"], env))
    docs = sysroot / "share/doc/rust"
    for file in [docs / "COPYRIGHT-library.html", *sorted((docs / "licenses").glob("*"))]:
        if file.is_file():
            save(file, "licenses/rust-std/" + file.relative_to(docs).as_posix(), {"component": "Rust standard library", "source": "pinned Rust toolchain/share/doc/rust"})
    # The MIT wrapper license does not replace Microsoft's native loader license.
    native = json.loads((supplements_root / "native-components.json").read_text(encoding="utf-8"))
    for component in native["components"]:
        matches = [p for p in packages.values() if p["name"] == component["crate"] and p["version"] == component["crate_version"]]
        if len(matches) != 1:
            base.fail("native loader license identity no longer matches the Cargo graph")
        loader = Path(matches[0]["manifest_path"]).parent / component["crate_file"]
        if base.digest(loader) != component["sha256"]:
            base.fail("native loader differs from the verified SDK original")
        for original in component["license_files"]:
            base.relative(original["path"])
            file = supplements_root / original["path"]
            if base.digest(file) != original["sha256"]:
                base.fail("native SDK original license hash mismatch")
            save(file, "licenses/native/" + original["path"], {"component": component["name"], "version": component["version"], "source": component["source"], "package_sha256": component["package_sha256"], "loader_sha256": component["sha256"]})
    save(supplements_root / "SOURCES.md", "licenses/SUPPLEMENTAL_SOURCES.md", {"component": "Supplemental original-license source inventory", "source": "Nexa packaging source"})
    base.write_json(stage / "licenses/index.json", {"scope": "locked desktop normal/build closure, original license texts and native loader identity", "files": records})
    return records


def npm_licenses(stage):
    lock = json.loads((ROOT / "apps/desktop/package-lock.json").read_text(encoding="utf-8"))
    records = []
    for location, package in lock["packages"].items():
        if not location or package.get("dev"):
            continue
        base.relative(location)
        if not location.startswith("node_modules/") or not isinstance(package.get("version"), str):
            base.fail("invalid npm production package identity")
        directory = ROOT / "apps/desktop" / location
        if package.get("optional") and not directory.exists():
            continue
        manifest = json.loads(base.regular(directory / "package.json").read_text(encoding="utf-8"))
        if manifest["version"] != package["version"]:
            base.fail("npm installed version differs from lock")
        files = base.license_files(directory)
        if not files:
            base.fail(f"original npm license missing: {location}")
        for source in files:
            destination = "licenses/npm/" + location.removeprefix("node_modules/") + "/" + source.relative_to(directory).as_posix()
            base.relative(destination)
            target = stage / destination
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(base.regular(source), target)
            records.append({"component": manifest["name"], "version": manifest["version"], "source": package.get("resolved"), "integrity": package.get("integrity"), "path": destination, "sha256": base.digest(target)})
    if not records:
        base.fail("npm production license closure is empty")
    base.write_json(stage / "licenses/npm-index.json", {"scope": "locked embedded frontend production dependency license originals", "files": records})


def verify(stage, manifest):
    actual = base.entries(stage)
    payload = [item for item in actual if item["path"] not in ("manifest.json", "SHA256SUMS")]
    if manifest["product"] != "nexa-desktop" or manifest["files"] != payload:
        base.fail("desktop manifest inventory/hash/size mismatch")
    required = {"nexa-desktop.exe", "README.md", "THIRD_PARTY_NOTICES.md", "licenses/index.json", "licenses/npm-index.json", "runtime/manifest.json", "runtime/SHA256SUMS", "download/manifest.json", "download/SHA256SUMS", "download/nexa-aria2.exe"}
    names = {item["path"] for item in payload}
    if not required <= names:
        base.fail("required desktop package files missing")
    runtime_root = stage / "runtime"
    runtime = json.loads(base.regular(runtime_root / "manifest.json").read_text(encoding="utf-8"))
    base.verify_package(runtime_root, runtime)
    if runtime["project_commit"] != manifest["project_commit"] or runtime["project_dirty"] != manifest["project_dirty"]:
        base.fail("desktop and runtime source identity differ")
    if base.digest(runtime_root / "manifest.json") != manifest["runtime_manifest_sha256"]:
        base.fail("nested runtime manifest differs from package identity")
    verify_download(stage / "download", manifest["project_commit"], manifest["project_dirty"])
    for name in names:
        if name.startswith("download/"):
            continue
        if name.startswith("runtime/"):
            continue  # Entire nested closure was verified with the T05 verifier.
        if name.startswith("licenses/"):
            if Path(name).suffix.lower() in {".exe", ".dll", ".gguf", ".pdb", ".lib", ".a", ".so", ".log", ".bat", ".ps1", ".py"}:
                base.fail("binary/tool/user-data pollution under desktop licenses")
            continue
        if name in {"nexa-desktop.exe", "README.md", "THIRD_PARTY_NOTICES.md"}:
            continue
        if "/" in name or name not in manifest["dependencies"] or not name.endswith(".dll"):
            base.fail("unexpected desktop package content")
    for name, node in manifest["dependencies"].items():
        if name not in names:
            base.fail("desktop dependency node has no file")
        for imported in node["imports"]:
            dll, kind = imported["name"], imported["kind"]
            if dependency_kind(dll, manifest["dependencies"]) != kind:
                base.fail("desktop dependency classification mismatch")
            if kind == "app-local" and dll not in names:
                base.fail("incomplete desktop app-local dependency closure")
    if (stage / "SHA256SUMS").exists():
        expected = "".join(f"{item['sha256']}  {item['path']}\n" for item in actual if item["path"] != "SHA256SUMS")
        if (stage / "SHA256SUMS").read_text(encoding="utf-8") != expected:
            base.fail("desktop SHA256SUMS differs from exact file set")


def promote(stage, archive, checksum, dist):
    # Keep the known T05 publication routine unchanged; desktop names are a
    # separate transaction and failed outputs never replace a validated package.
    changes, backups = [], []
    try:
        for source in (stage, archive, checksum):
            destination = dist / source.name
            if destination.exists() or destination.is_symlink():
                if destination.is_symlink() or getattr(destination.lstat(), "st_file_attributes", 0) & 0x400:
                    base.fail("indirect desktop destination")
                backup = dist / (".previous-" + base.uuid.uuid4().hex + "-" + source.name)
                destination.rename(backup)
                backups.append((backup, destination))
            source.rename(destination)
            changes.append(destination)
    except BaseException:
        for destination in reversed(changes):
            shutil.rmtree(destination) if destination.is_dir() else destination.unlink()
        for backup, destination in reversed(backups):
            backup.rename(destination)
        raise
    for backup, _ in backups:
        shutil.rmtree(backup) if backup.is_dir() else backup.unlink()


def build(executable, component):
    if os.name != "nt":
        base.fail("desktop packaging requires actual Windows x64 PE inspection")
    selected, vs, env, _, redist = base.selected_visual_studio()
    source = source_identity(env)
    runtime_root = ROOT / "dist/windows-x64-cpu"
    runtime = json.loads(base.regular(runtime_root / "manifest.json").read_text(encoding="utf-8"))
    base.verify_package(runtime_root, runtime)
    if source != {key: runtime["source"][key] for key in source} or runtime["cargo_lock_sha256"] != base.digest(ROOT / "Cargo.lock"):
        base.fail("runtime package is stale or built from a different source/lock")
    graph = base.command(["cargo", "tree", "--locked", "--manifest-path", SHELL / "Cargo.toml", "--target", base.TARGET, "--edges", "normal"], env)
    if re.search(r"\b(engine-host|llama-adapter|runtime-worker)\b", graph):
        base.fail("desktop process links native inference dependencies")
    metadata = json.loads(base.command(["cargo", "metadata", "--locked", "--manifest-path", SHELL / "Cargo.toml", "--format-version", "1", "--filter-platform", base.TARGET], env))
    dumpbin = base.regular(Path(base.command(["where.exe", "dumpbin.exe"], env).splitlines()[0]))
    if not dumpbin.resolve().is_relative_to(vs):
        base.fail("desktop PE inspector is not from selected Visual Studio")
    evidence, crt_sources = {}, []
    replacements = [(str(ROOT), "<project>"), (str(vs), "<visual-studio>"), (os.environ.get("USERPROFILE", ""), "<user-profile>")]
    def inspect(path):
        base.pe_machine(path)
        output = base.command([dumpbin, "/nologo", "/dependents", path], env)
        evidence[path.name] = {"headers": base.command([dumpbin, "/nologo", "/headers", path], env), "dependents": output}
        return base.parse_dependents(output)
    def copy_crt(source_path, destination):
        info = base.authenticode_info(source_path, env)
        base.pe_machine(source_path)
        shutil.copyfile(base.regular(source_path), destination)
        if base.digest(source_path) != base.digest(destination):
            base.fail("desktop CRT changed during copy")
        crt_sources.append({"path": destination.name, "source_relative_to_visual_studio": source_path.relative_to(vs).as_posix(), "sha256": base.digest(destination), "size_bytes": destination.stat().st_size, **info})
    dist = ROOT / "dist"
    with tempfile.TemporaryDirectory(prefix=".desktop-stage-", dir=dist) as temporary:
        temporary = Path(temporary)
        stage = temporary / "desktop-windows"
        stage.mkdir()
        shutil.copyfile(base.regular(executable), stage / "nexa-desktop.exe")
        shutil.copytree(runtime_root, stage / "runtime")
        verify_download(component, source["commit"], source["dirty"])
        shutil.copytree(component, stage / "download")
        download_build = verify_download(stage / "download", source["commit"], source["dirty"])
        actual_imports = set(inspect(stage / "download/nexa-aria2.exe"))
        if actual_imports != {name.lower() for name in download_build["pe"]["imports"]}:
            base.fail("download PE imports differ from build manifest")
        if any(name not in aria2_build.SYSTEM_IMPORTS and not re.fullmatch(r"api-ms-win-crt-[a-z0-9-]+-l1-1-0\.dll", name) for name in actual_imports):
            base.fail("download PE has undeclared non-system dependency")
        for name in ("README.md", "THIRD_PARTY_NOTICES.md"):
            shutil.copyfile(base.regular(ROOT / "packaging/desktop-windows" / name), stage / name)
        with (stage / "THIRD_PARTY_NOTICES.md").open("a", encoding="utf-8") as notices:
            notices.write("\n\n## Nexa download component\n\n"
                          "The separately executed nexa-aria2.exe is a modified aria2 1.37.0, licensed under GPL-2.0-or-later. "
                          "The exact corresponding patched source, build materials and dependency license originals accompany it under download/. "
                          "See download/licenses/aria2-COPYING and download/" + DOWNLOAD_SOURCE + ". "
                          "LLVM/MinGW runtime notices are included in download/licenses/.\n")
        dependencies = collect_dependencies(stage, redist, inspect, copy_crt)
        copy_rust_licenses(stage, metadata, env)
        npm_licenses(stage)
        manifest = {"schema_version": 1, "product": "nexa-desktop", "package_version": "0.1.0", "platform": "windows-x64", "backend": "cpu", "target": base.TARGET, "configuration": "Release",
                    "project_commit": source["commit"], "project_dirty": source["dirty"], "source": source,
                    "desktop_cargo_lock_sha256": base.digest(SHELL / "Cargo.lock"), "npm_lock_sha256": base.digest(ROOT / "apps/desktop/package-lock.json"),
                    "runtime_manifest_sha256": base.digest(runtime_root / "manifest.json"), "dependencies": dependencies, "crt_sources": crt_sources,
                    "desktop_without_native": {"verified": True, "dependencies": graph},
                    "webview2": {"mode": "installed-evergreen", "bundled": False, "auto_install": False, "version": "observed by --diagnose on execution machine"},
                    "visual_studio_edition": selected.get("productId"),
                    "acceptance": {"native_window": "separate Windows UI acceptance required", "windows10": "not_run", "windows11": "not_run", "clean_machine_offline_long_run": "deferred by user"}, "files": base.entries(stage)}
        manifest = base.sanitize(manifest, replacements)
        base.write_json(stage / "manifest.json", manifest)
        (stage / "SHA256SUMS").write_text("".join(f"{item['sha256']}  {item['path']}\n" for item in base.entries(stage)), encoding="utf-8")
        verify(stage, manifest)
        archive = temporary / "desktop-windows.zip"
        with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as zipped:
            for item in base.entries(stage):
                zipped.write(stage / item["path"], "desktop-windows/" + item["path"])
        with zipfile.ZipFile(archive) as zipped:
            if zipped.testzip() is not None or len(zipped.namelist()) != len(base.entries(stage)):
                base.fail("desktop ZIP validation failed")
        checksum = temporary / "desktop-windows.zip.sha256"
        checksum.write_text(f"{base.digest(archive)}  desktop-windows.zip\n", encoding="utf-8")
        result = {"schema_version":1, "build_status":"pass", "project_commit":source["commit"], "package_sha256":base.digest(archive), "compressed_bytes":archive.stat().st_size,
                  "installed_bytes":sum(item["size_bytes"] for item in base.entries(stage)), "runtime_bytes":sum(item["size_bytes"] for item in base.entries(stage / "runtime")),
                  "download_component_bytes":sum(item["size_bytes"] for item in base.entries(stage / "download")),
                  "ui_executable_bytes":(stage / "nexa-desktop.exe").stat().st_size, "ui_app_local_crt_bytes":sum(item["size_bytes"] for item in crt_sources), "model_bytes":0, "manifest_sha256":base.digest(stage / "manifest.json"), "native_window_tested":False}
        if source_identity(env) != source:
            base.fail("source changed during desktop packaging")
        evidence_root = ROOT / "artifacts/verification/windows-desktop"
        evidence_root.mkdir(parents=True, exist_ok=True)
        base.write_json(evidence_root / "pe-inspection.json", base.sanitize(evidence, replacements))
        promote(stage, archive, checksum, dist)
        base.write_json(evidence_root / "build-result.json", result)
    print("Built verified desktop-windows.zip; native window and target-machine UI acceptance remain separate gates")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--desktop-exe", type=Path, default=ROOT / "build/desktop/cargo" / base.TARGET / "release/nexa-desktop.exe")
    parser.add_argument("--component-dir", type=Path, default=ROOT / "build/download")
    parser.add_argument("--prepare-download-component", type=Path)
    args = parser.parse_args()
    if args.prepare_download_component:
        commit = base.command(["git", "rev-parse", "HEAD"], os.environ.copy())
        dirty = bool(base.command(["git", "status", "--porcelain", "--untracked-files=all"], os.environ.copy()))
        prepare_download(args.prepare_download_component, args.component_dir, commit, dirty)
    else:
        build(args.desktop_exe, args.component_dir)

if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, KeyError, json.JSONDecodeError) as error:
        print(f"Desktop package failed: {error}", file=sys.stderr)
        raise SystemExit(1)
