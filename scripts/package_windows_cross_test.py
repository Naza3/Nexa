#!/usr/bin/env python3
"""Package explicit Linux-built Windows test inputs without native-Windows claims.

Existing native Windows packagers are unchanged. Capture source before building;
this entry verifies that snapshot, actual PE imports and source-matched aria2,
then copies original licenses and only the required official Release CRT files.
It does not download tools, run Windows executables or perform Windows-system
Authenticode verification. Linux osslsigncode verification is recorded separately.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib
from urllib.parse import urlsplit
import zipfile
import package_windows as base
import package_desktop_windows as desktop
import verify_cross_package_consumers as consumers

ROOT = base.ROOT
PROFILE = "linux-clang-cl-msvc"
ACCEPTANCE = {
    "windows_execution": "not_run", "native_window": "not_run",
    "real_model_inference": "not_run", "download_runtime": "not_run",
    "windows10_target_machine": "not_run", "clean_machine": "not_run",
    "windows_authenticode_verification": "not_run",
}


def source_identity():
    env = os.environ.copy()
    git = lambda *args: base.command(["git", *args], env)
    if git("-C", "vendor/llama.cpp", "rev-parse", "HEAD") != base.LLAMA_COMMIT or git("-C", "vendor/llama.cpp", "status", "--porcelain", "--untracked-files=all"):
        base.fail("locked llama.cpp requires its real clean Git checkout")
    names = sorted(set(git("ls-files", "-z", "--cached", "--others", "--exclude-standard").split("\0")) - {""})
    records, absent = [], []
    for name in names:
        base.relative(name)
        path = ROOT / name
        if path.is_file():
            records.append({"path": name, "sha256": base.digest(base.regular(path))})
        elif not path.is_dir():
            absent.append(name)
    status = git("status", "--porcelain", "--untracked-files=all")
    sparse = git("config", "--bool", "core.sparseCheckout") if absent else "false"
    if absent and sparse != "true":
        base.fail("missing tracked source outside an explicit sparse checkout")
    return {"commit": git("rev-parse", "HEAD"), "tree": git("rev-parse", "HEAD^{tree}"),
            "dirty": bool(status), "git_status": status,
            "diff_sha256": hashlib.sha256(git("diff", "--binary", "HEAD").encode()).hexdigest(),
            "worktree_files_sha256": hashlib.sha256(json.dumps(records, sort_keys=True, separators=(",", ":")).encode()).hexdigest(),
            "materialized_source_files": len(records), "sparse_checkout": sparse == "true",
            "absent_tracked_paths": len(absent), "llama_commit": base.LLAMA_COMMIT}


def capture_source(destination):
    if platform.system() != "Linux" or destination.exists():
        base.fail("capture requires Linux and a new receipt path")
    source = source_identity()
    base.write_json(destination, {"schema_version": 1, "purpose": "cross-test-build-source-receipt",
                                "profile": PROFILE, "source": source})
    return source


def check_receipt(path):
    receipt = json.loads(base.regular(path).read_text(encoding="utf-8"))
    if (receipt.get("schema_version") != 1 or receipt.get("purpose") != "cross-test-build-source-receipt"
            or receipt.get("profile") != PROFILE or receipt.get("source") != source_identity()):
        base.fail("source differs from the captured pre-build receipt")
    return receipt["source"]


def parse_pe_inspection(output):
    if not re.search(r"^  Machine: IMAGE_FILE_MACHINE_AMD64 \(0x8664\)$", output, re.M):
        base.fail("LLVM did not report AMD64 PE")
    if not re.search(r"^  Magic: 0x20B$", output, re.M):
        base.fail("LLVM did not report PE32+ optional headers")
    imports = {"normal": [], "delay": []}
    for kind, body in re.findall(r"^(Import|DelayImport) \{\n(.*?)^\}", output, re.M | re.S):
        names = re.findall(r"^  Name: ([^\r\n]+)$", body, re.M)
        if len(names) != 1 or not re.fullmatch(r"[A-Za-z0-9_.-]+\.dll", names[0], re.I):
            base.fail("malformed LLVM PE import name")
        imports["normal" if kind == "Import" else "delay"].append(names[0].lower())
    for field, kind in (("ImportTable", "normal"), ("DelayImportDescriptor", "delay")):
        values = re.findall(r"^    " + field + r"(RVA|Size): (0x[0-9A-Fa-f]+)$", output, re.M)
        if len(values) != 2 or {key for key, _ in values} != {"RVA", "Size"}:
            base.fail("PE import directory evidence missing")
        populated = [int(value, 16) != 0 for _, value in values]
        if populated[0] != populated[1] or bool(imports[kind]) != populated[0]:
            base.fail("PE import directory and decoded imports disagree")
    if not imports["normal"]:
        base.fail("PE has no verified normal imports")
    return {key: sorted(set(value)) for key, value in imports.items()}


def check_file_record(record):
    path = base.regular(Path(record["path"]))
    if path.stat().st_size != record["size_bytes"] or base.digest(path) != record["sha256"]:
        base.fail("provenance file hash/size mismatch")
    return path


def verify_archive_member(archive, member, record):
    base.relative(member)
    try:
        with zipfile.ZipFile(archive) as zipped:
            matches = [info for info in zipped.infolist() if info.filename == member]
            if len(matches) != 1 or matches[0].is_dir() or matches[0].file_size != record["size_bytes"]:
                base.fail("CRT original archive member missing/duplicate/size mismatch")
            with zipped.open(matches[0]) as original:
                actual = hashlib.file_digest(original, "sha256").hexdigest()
            if actual != record["sha256"]:
                base.fail("CRT DLL differs from its original official archive member")
    except zipfile.BadZipFile:
        base.fail("CRT source must be the original inspectable ZIP/VSIX archive")


def verify_linux_signature(path, verifier, ca, tsa_ca, evidence_directory):
    for file in (path, verifier, ca, tsa_ca):
        base.regular(file)
    result = subprocess.run([str(verifier), "verify", "-CAfile", str(ca), "-TSA-CAfile", str(tsa_ca), "-in", str(path)],
                            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, check=False, timeout=180)
    output = result.stdout
    count = re.findall(r"^Number of verified signatures: ([0-9]+)$", output, re.M)
    if (result.returncode != 0 or len(count) != 1 or int(count[0]) < 1 or not output.rstrip().endswith("Succeeded")
            or len(re.findall(r"^Signature verification: ok$", output, re.M)) != int(count[0])
            or len(re.findall(r"^Signature CRL verification: ok$", output, re.M)) != int(count[0])
            or len(re.findall(r"^Timestamp Server Signature verification: ok$", output, re.M)) != int(count[0])
            or len(re.findall(r"^Timestamp Server Signature CRL verification: ok$", output, re.M)) != int(count[0])
            or "O=Microsoft Corporation" not in output):
        base.fail("Linux CRT signature/timestamp/CRL validation failed: " + output[-3000:])
    evidence_directory.mkdir(parents=True, exist_ok=True)
    report = evidence_directory / (path.name + ".txt")
    report.write_text(base.sanitize(output, [(str(path), "<copied-crt>"), (str(ca), "<Microsoft-Root-2011>"), (str(tsa_ca), "<Microsoft-Root-2010>")]), encoding="utf-8")
    return {"status": "pass", "method": "osslsigncode verify on Linux", "tool_sha256": base.digest(verifier),
            "crt_sha256": base.digest(path), "code_signing_root_sha256": base.digest(ca), "timestamp_root_sha256": base.digest(tsa_ca),
            "verified_signatures": int(count[0]), "timestamp_verification": "pass", "crl_verification": "pass",
            "warnings": sorted(set(line.strip() for line in output.splitlines() if "Warning:" in line)),
            "log_sha256": base.digest(report), "windows_policy_equivalence": False}


def load_crt_provenance(path):
    value = json.loads(base.regular(path).read_text(encoding="utf-8"))
    if value.get("schema_version") != 1 or not value.get("archives") or not value.get("dlls") or not value.get("licenses"):
        base.fail("incomplete official CRT provenance/license inventory")
    archives = {}
    for record in value["archives"]:
        url = urlsplit(record["url"])
        if url.scheme != "https" or url.hostname not in {"download.visualstudio.microsoft.com", "download.microsoft.com"} or url.username or url.password:
            base.fail("CRT archive must have its official Microsoft HTTPS source")
        check_file_record(record)
        if record["sha256"] in archives:
            base.fail("duplicate CRT archive provenance")
        archives[record["sha256"]] = record
    dlls = {}
    for record in value["dlls"]:
        source = check_file_record(record)
        base.relative(record["package_path"])
        name = source.name.lower()
        if record["archive_sha256"] not in archives or name in dlls or not re.fullmatch(r"(?:vcruntime|msvcp|concrt|vcomp|vccorlib)\d+(?:_[A-Za-z0-9_]+)?\.dll", name) or base.DEBUG_CRT.fullmatch(name):
            base.fail("invalid, duplicate or non-Release CRT origin")
        archive = archives[record["archive_sha256"]]
        verify_archive_member(Path(archive["path"]), record["package_path"], record)
        base.pe_machine(source)
        dlls[name] = record
    for record in value["licenses"]:
        check_file_record(record)
        if urlsplit(record["source_url"]).scheme != "https":
            base.fail("CRT license original requires source URL")
    return value, dlls


def native_identity(native):
    fields = {}
    for line in base.regular(native / "air-native-Release.txt").read_text(encoding="utf-8").splitlines():
        key, sep, value = line.partition("=")
        if not sep or not key or key in fields:
            base.fail("invalid/duplicate native identity field")
        fields[key] = value
    expected = {"schema": "1", "system": "Windows", "configuration": "Release", "pointer_bytes": "8",
                "crt": "MD", "llama_commit": base.LLAMA_COMMIT, "profile": PROFILE, "host_system": "Linux",
                "cross_compiling": "TRUE", "compiler_id": "Clang", "compiler_frontend": "MSVC",
                "compiler_simulate_id": "MSVC", "compiler_target": base.TARGET,
                "c_compiler_id": "Clang", "c_compiler_frontend": "MSVC", "c_compiler_simulate_id": "MSVC",
                "c_compiler_target": base.TARGET, "msvc_runtime_library": "MultiThreadedDLL", "cross_abi_verified": "1", "cross_cpu_baseline_verified": "1", "GGML_SSE42": "ON", "GGML_AVX": "ON", "GGML_AVX2": "ON", "GGML_FMA": "ON", "GGML_F16C": "ON", "GGML_BMI2": "ON", "GGML_AVX512": "OFF"}
    expected.update(dict.fromkeys(("GGML_NATIVE", "GGML_BACKEND_DL", "GGML_OPENMP", "GGML_CUDA", "GGML_VULKAN", "GGML_METAL", "LLAMA_OPENSSL", "BUILD_SHARED_LIBS", "MTMD_VIDEO"), "OFF"))
    if any(fields.get(key) != value for key, value in expected.items()) or fields.get("processor", "").lower() not in {"amd64", "x86_64"}:
        base.fail("native Clang MSVC-ABI Release cross profile mismatch")
    return fields, base.native_archive_records(native, fields)


def seal(stage, manifest, verify):
    manifest["files"] = base.entries(stage)
    base.write_json(stage / "manifest.json", manifest)
    (stage / "SHA256SUMS").write_text("".join(f"{item['sha256']}  {item['path']}\n" for item in base.entries(stage)), encoding="utf-8")
    verify(stage, manifest)


def write_crt_licenses(stage, provenance):
    records = []
    for index, record in enumerate(provenance["licenses"]):
        path = check_file_record(record)
        # Originals retain their native format, including DOCX/PDF. Consolidation
        # keeps non-text originals standalone; it never extracts or converts them.
        name = f"original-{index + 1}-{path.name}"
        if path.suffix.lower() not in {".txt", ".rtf", ".html", ".htm", ".md", ".pdf", ".docx"}:
            base.fail("unsupported CRT original-license file type")
        destination = stage / "licenses/microsoft-crt" / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(path, destination)
        records.append({"path": destination.relative_to(stage).as_posix(), "sha256": base.digest(destination), "source_url": record["source_url"]})
    base.write_json(stage / "licenses/microsoft-crt/index.json", {"windows_authenticode_verification": "not_run", "files": records})


def copy_cross_document(source, destination, role):
    text = base.regular(source).read_text(encoding="utf-8")
    if source.name == "README.md":
        text = text.replace("均来自构建时所选 Visual Studio 的合法 VC/Redist/MSVC 源，保持未修改", "取自已核验官方 Microsoft 下载包的 Release x64 原文件，保持未修改")
        text = text.replace("DLL 来源/版本/签名", "DLL 官方下载包来源/原成员路径、Linux 签名校验与 Windows 未验状态")
        text = text.replace("请从可信的本次开发 Actions 产物获取本包。", "本包由 Linux 云端交叉构建，没有使用 GitHub Actions；请核对本次交付的 ZIP SHA-256。")
        text = text.replace("另一个 `acceptance-tools.zip` 包含", "外层同级 `acceptance-tools/` 目录包含")
        text = "# Linux 交叉构建测试包说明\n\n本包在 Linux 构建，目标 Windows x64；C++ 使用 Clang 的 MSVC ABI 与 Release 动态 CRT（/MD）。尚未在 Windows 执行，原生窗口、下载、真实模型推理、目标机和干净机器验收均未验证。本包 CRT 原文件通过 Linux osslsigncode 签名、时间戳与吊销列表校验，但未执行 Windows Get-AuthenticodeSignature，不宣称原生 MSVC 编译器构建或 Windows 已通过。下文为既有操作步骤与产品约束，不是本包运行验收结果。\n\n" + text
    elif role in {"runtime", "helper"}:
        start = text.index("## Microsoft Visual C++ Release x64 runtime")
        text = text[:start].replace("未新增或接受任何许可协议；", "本次 Linux 构建使用用户已确认的 Microsoft Build Tools/SDK 适用条款；")
        text += "## Microsoft Visual C++ Release x64 runtime\n\n仅复制实际 PE 导入闭包需要的官方 Release x64 CRT 原文件。每个文件的 Microsoft 下载 URL、原包 SHA-256、精确包内路径、文件 SHA-256 与大小记录在 manifest 的 crt_sources。打包逐字节核对原包成员，没有修改 DLL。CRT 原文件通过 Linux osslsigncode 签名、时间戳与吊销列表校验，结果与警告保留在 manifest；Windows Get-AuthenticodeSignature 未执行，Linux 检查不等价于 Windows 验签策略。hash 表示来源闭合与内容一致性，不是数字签名结论。\n\n适用 Microsoft 条款原文与来源保存在 licenses/microsoft-crt/；本说明不替代原条款，不授予额外公开分发权。本包不含 Debug CRT、系统 UCRT、Windows inbox DLL 或模型权重，不安装软件。\n"
    else:
        text = text.replace("完全保留匹配源版本的 T05 CPU 产品", "包含匹配源版本的 Linux 交叉构建 CPU 测试产品")
        text = text.replace("桌面 EXE 新增 app-local CRT 仅取所选 Visual Studio 已有 Release x64 Redist 闭包，验证 Microsoft Authenticode 并记录实际 DLL 版本、来源与 hash。许可规则和微软原件入口沿用 `runtime/THIRD_PARTY_NOTICES.md`", "桌面 EXE 的 app-local CRT 仅取官方 Microsoft Release x64 下载包，按实际 PE 导入闭包选择，校验原包成员与 DLL 字节、来源和 hash。CRT 原文件通过 Linux osslsigncode 签名、时间戳与吊销列表校验，Windows Get-AuthenticodeSignature 未执行。原条款和来源见 `licenses/microsoft-crt/` 与 `runtime/THIRD_PARTY_NOTICES.md`")
    destination.write_text(text, encoding="utf-8")


def package(args):
    if platform.system() != "Linux" or args.output.exists():
        base.fail("cross-test packaging requires Linux and a new output directory")
    source = check_receipt(args.source_receipt)
    identity, archives = native_identity(args.native_dir)
    provenance, redist_records = load_crt_provenance(args.crt_provenance)
    toolchain_provenance = json.loads(base.regular(args.toolchain_provenance).read_text(encoding="utf-8"))
    redist = {name: Path(record["path"]) for name, record in redist_records.items()}
    env = os.environ.copy()
    rust = base.command(["rustc", "-vV"], env)
    if "release: 1.98.1\n" not in rust + "\n" or "host: x86_64-unknown-linux-gnu" not in rust:
        base.fail("pinned Rust 1.98.1 Linux host is required")
    inspector_version = base.command([args.llvm_readobj, "--version"], env)
    evidence, copied, signatures = {}, [], {}
    def inspect(path):
        base.pe_machine(path)
        output = base.command([args.llvm_readobj, "--file-headers", "--coff-imports", path], env)
        imports = parse_pe_inspection(output)
        evidence[str(path.relative_to(temporary))] = {"imports": imports, "sha256": base.digest(path), "size_bytes": path.stat().st_size}
        return sorted(set(imports["normal"] + imports["delay"]))
    def copy_crt(path, destination):
        record = redist_records[path.name.lower()]
        check_file_record(record)
        shutil.copyfile(base.regular(path), destination)
        if base.digest(destination) != record["sha256"]:
            base.fail("CRT changed during copy")
        if record["sha256"] not in signatures:
            signatures[record["sha256"]] = verify_linux_signature(destination, args.osslsigncode, args.authenticode_ca, args.authenticode_tsa_ca, temporary / "linux-signature-evidence")
        origin = next(item for item in provenance["archives"] if item["sha256"] == record["archive_sha256"])
        copied.append({"path": destination.name, "sha256": record["sha256"], "size_bytes": record["size_bytes"],
                       "source_archive_url": origin["url"], "source_archive_sha256": origin["sha256"],
                       "source_package_path": record["package_path"], "linux_authenticode": signatures[record["sha256"]], "windows_authenticode_verification": "not_run"})
    metadata = json.loads(base.command(["cargo", "metadata", "--locked", "--format-version", "1", "--filter-platform", base.TARGET], env))
    shell_metadata = json.loads(base.command(["cargo", "metadata", "--locked", "--manifest-path", ROOT / "apps/desktop/src-tauri/Cargo.toml", "--format-version", "1", "--filter-platform", base.TARGET], env))
    graphs = {}
    for key, extra in (("management", ["-p", "runtime-cli"]), ("acceptance_tools", ["-p", "xtask"]), ("desktop", ["--manifest-path", ROOT / "apps/desktop/src-tauri/Cargo.toml"])):
        graph = base.command(["cargo", "tree", "--locked", *extra, "--target", base.TARGET, "--edges", "normal"], env)
        if re.search(r"\b(engine-host|llama-adapter|runtime-worker)\b", graph):
            base.fail(key + " unexpectedly links native inference")
        graphs[key] = graph
    replacements = [(str(ROOT), "<project>"), (str(args.toolchain_provenance.parent), "<cross-tools>"), (str(args.native_dir), "<native-build>"),
                    (str(Path(base.command(["rustc", "--print", "sysroot"], env))), "<rust-toolchain>"),
                    (str(Path.home()), "<home>")]
    common = {"schema_version": 1, "package_version": tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]["package"]["version"],
              "platform": "windows-x64", "backend": "cpu", "target": base.TARGET, "configuration": "Release",
              "project_commit": source["commit"], "project_dirty": source["dirty"], "source": source,
              "build_mode": "linux-cross-test", "acceptance": ACCEPTANCE,
              "toolchain": {"rustc": rust, "cargo": base.command(["cargo", "--version"], env), "llvm_readobj": inspector_version,
                            "host_os": platform.platform(), "compiler_id": identity["compiler_id"],
                            "compiler_version": identity["compiler_version"], "native_profile": PROFILE,
                            "toolchain_provenance": toolchain_provenance, "toolchain_provenance_sha256": base.digest(args.toolchain_provenance),
                            "crt_provenance_sha256": base.digest(args.crt_provenance),
                            "crt_source_validation": {key: provenance[key] for key in ("channel_manifest_hash_verified", "channel_manifest_limitation") if key in provenance}}}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".cross-test-", dir=args.output.parent) as temp:
        temporary = Path(temp)
        payload = temporary / "Nexa-Windows-cross-test"
        stage = payload / "desktop-windows"
        runtime, helper = stage / "runtime", payload / "acceptance-tools"
        runtime.mkdir(parents=True)
        helper.mkdir()
        for name, source_path in (("ai-runtime.exe", args.runtime_exe), ("ai-runtime-worker.exe", args.worker_exe)):
            shutil.copyfile(base.regular(source_path), runtime / name)
        for name in ("README.md", "config.example.toml", "THIRD_PARTY_NOTICES.md"):
            if name == "config.example.toml":
                shutil.copyfile(base.regular(ROOT / "packaging/windows-x64-cpu" / name), runtime / name)
            else:
                copy_cross_document(ROOT / "packaging/windows-x64-cpu" / name, runtime / name, "runtime")
        copied.clear()
        dependencies = base.collect_dependencies(runtime, redist, inspect, copy_crt)
        base.copy_licenses(runtime, metadata, env)
        write_crt_licenses(runtime, provenance)
        base.consolidate_licenses(runtime)
        manifest = {**common, "product": "nexa-runtime", "crt": "MD", "architecture": "x86_64", "protocol_version": 1,
                    "worker_protocol_version": base.WORKER_PROTOCOL_VERSION, "shim_version": base.SHIM_VERSION, "project_tree": source["tree"],
                    "llama_commit": base.LLAMA_COMMIT, "cargo_lock_sha256": base.digest(ROOT / "Cargo.lock"),
                    "native_build": base.sanitize(identity, replacements), "native_archives": archives,
                    "cpu_baseline": [key.removeprefix("GGML_").lower() for key in ("GGML_SSE42", "GGML_AVX", "GGML_AVX2", "GGML_F16C", "GGML_FMA", "GGML_BMI2", "GGML_AVX512") if identity.get(key) == "ON"],
                    "dependencies": dependencies, "crt_sources": copied.copy(),
                    "management_without_native": {"verified": True, "method": "locked-normal-dependency-graph", "dependencies": graphs["management"]}}
        seal(runtime, base.sanitize(manifest, replacements), base.verify_package)
        shutil.copyfile(base.regular(args.acceptance_exe), helper / "nexa-acceptance.exe")
        copy_cross_document(ROOT / "packaging/windows-x64-cpu/THIRD_PARTY_NOTICES.md", helper / "THIRD_PARTY_NOTICES.md", "helper")
        copied.clear()
        dependencies = base.collect_dependencies(helper, redist, inspect, copy_crt, ("nexa-acceptance.exe",))
        base.copy_licenses(helper, metadata, env, roots=("xtask",))
        write_crt_licenses(helper, provenance)
        base.consolidate_licenses(helper)
        seal(helper, base.sanitize({**common, "product": "nexa-acceptance-tools", "dependencies": dependencies,
             "crt_sources": copied.copy(), "native_inference_linkage": False,
             "helper_without_native": {"verified": True, "method": "locked-normal-dependency-graph", "dependencies": graphs["acceptance_tools"]}}, replacements), base.verify_package)
        shutil.copyfile(base.regular(args.desktop_exe), stage / "nexa-desktop.exe")
        desktop.verify_download(args.component_dir, source["commit"], source["dirty"])
        shutil.copytree(args.component_dir, stage / "download")
        download_build = desktop.verify_download(stage / "download", source["commit"], source["dirty"])
        imports = set(inspect(stage / "download/nexa-aria2.exe"))
        if imports != {name.lower() for name in download_build["pe"]["imports"]} or any(name not in desktop.aria2_build.SYSTEM_IMPORTS and not re.fullmatch(r"api-ms-win-crt-[a-z0-9-]+-l1-1-0\.dll", name) for name in imports):
            base.fail("aria2 PE imports differ from locked component/system closure")
        for name in ("README.md", "THIRD_PARTY_NOTICES.md"):
            copy_cross_document(ROOT / "packaging/desktop-windows" / name, stage / name, "desktop")
        with (stage / "THIRD_PARTY_NOTICES.md").open("a", encoding="utf-8") as notices:
            notices.write("\n\n## Nexa download component\n\nModified aria2 1.37.0 is GPL-2.0-or-later. Exact patched corresponding source, build materials and original dependency licenses are in download/, including " + desktop.DOWNLOAD_SOURCE + ".\n")
        copied.clear()
        dependencies = desktop.collect_dependencies(stage, redist, inspect, copy_crt)
        desktop.copy_rust_licenses(stage, shell_metadata, env)
        desktop.npm_licenses(stage)
        write_crt_licenses(stage, provenance)
        base.consolidate_licenses(stage)
        seal(stage, base.sanitize({**common, "product": "nexa-desktop", "dependencies": dependencies, "crt_sources": copied.copy(),
             "runtime_manifest_sha256": base.digest(runtime / "manifest.json"), "desktop_cargo_lock_sha256": base.digest(ROOT / "apps/desktop/src-tauri/Cargo.lock"),
             "npm_lock_sha256": base.digest(ROOT / "apps/desktop/package-lock.json"),
             "desktop_without_native": {"verified": True, "method": "locked-normal-dependency-graph", "dependencies": graphs["desktop"]},
             "webview2": {"mode": "installed-evergreen", "bundled": False, "auto_install": False}}, replacements), desktop.verify)
        consumer_report = consumers.verify((stage / "nexa-desktop.exe").absolute(), temporary / "consumer-verification.json")
        (payload / "BUILD-STATUS.txt").write_text("Nexa Windows x64 私有测试包，由 Linux 交叉构建。\n在已安装 WebView2 的 Windows 运行 desktop-windows/nexa-desktop.exe。\n本包尚未在 Windows 执行；原生窗口、真实模型推理、下载、干净机器与 Windows Authenticode 验签均未验证。\n不含模型权重。acceptance-tools 是独立验收工具目录，不是已完成验收的报告。\n", encoding="utf-8")
        base.write_json(payload / "manifest.json", base.sanitize({**common, "product": "nexa-windows-cross-test", "native_window_tested": False,
            "files": base.entries(payload), "pe_inspection": evidence, "consumer_verification": consumer_report}, replacements))
        (payload / "SHA256SUMS").write_text("".join(f"{item['sha256']}  {item['path']}\n" for item in base.entries(payload)), encoding="utf-8")
        if check_receipt(args.source_receipt) != source:
            base.fail("source changed during packaging")
        archive = temporary / "Nexa-Windows-cross-test.zip"
        inventory = base.entries(payload)
        with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as zipped:
            for item in inventory:
                zipped.write(payload / item["path"], payload.name + "/" + item["path"])
        with zipfile.ZipFile(archive) as zipped:
            if zipped.testzip() is not None or len(zipped.namelist()) != len(inventory):
                base.fail("cross-test ZIP validation failed")
            for item in inventory:
                raw = zipped.read(payload.name + "/" + item["path"])
                if len(raw) != item["size_bytes"] or hashlib.sha256(raw).hexdigest() != item["sha256"]:
                    base.fail("ZIP bytes differ from inspected package")
        (temporary / "Nexa-Windows-cross-test.zip.sha256").write_text(base.digest(archive) + "  Nexa-Windows-cross-test.zip\n", encoding="utf-8")
        base.write_json(temporary / "build-result.json", {"schema_version": 1, "status": "cross_build_inputs_packaged_and_inspected",
            "source": source, "sha256": base.digest(archive), "size_bytes": archive.stat().st_size, "files": len(inventory),
            "acceptance": ACCEPTANCE, "source_receipt_sha256": base.digest(args.source_receipt)})
        temporary.rename(args.output)
    print("Created a Linux-cross-built Windows test ZIP; Windows execution remains unverified")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture-source", type=Path)
    for name in ("source-receipt", "native-dir", "runtime-exe", "worker-exe", "acceptance-exe", "desktop-exe", "component-dir", "crt-provenance", "toolchain-provenance", "osslsigncode", "authenticode-ca", "authenticode-tsa-ca", "output"):
        parser.add_argument("--" + name, type=Path)
    parser.add_argument("--llvm-readobj", type=Path, default=Path("llvm-readobj"))
    args = parser.parse_args()
    if args.capture_source:
        capture_source(args.capture_source)
    else:
        required = ("source_receipt", "native_dir", "runtime_exe", "worker_exe", "acceptance_exe", "desktop_exe", "component_dir", "crt_provenance", "toolchain_provenance", "osslsigncode", "authenticode_ca", "authenticode_tsa_ca", "output")
        if any(getattr(args, name) is None for name in required):
            parser.error("packaging requires all explicit input paths")
        for name in required:
            setattr(args, name, getattr(args, name).absolute())
        package(args)


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, KeyError, json.JSONDecodeError, subprocess.TimeoutExpired) as error:
        print(f"Cross-test package failed: {error}", file=sys.stderr)
        raise SystemExit(1)
