#!/usr/bin/env python3
"""Reuse the native packager's selected VS/toolset and build tree in Windows CI.

Does not install Visual Studio, relax build identities, or certify a package.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tomllib

import package_windows as base

# Only compiler/SDK variables are carried into later Actions steps. Never copy
# an entire environment (which also contains runner credentials and control keys).
BUILD_ENVIRONMENT = (
    "PATH", "INCLUDE", "LIB", "LIBPATH", "VSINSTALLDIR", "VCINSTALLDIR",
    "VCTOOLSINSTALLDIR", "VCTOOLSREDISTDIR", "VCTOOLSVERSION",
    "WINDOWSSDKDIR", "WINDOWSSDKVERSION", "WINDOWSSDKBINPATH", "WINDOWSSDKVERBINPATH",
    "UNIVERSALCRTSDKDIR", "UCRTVERSION", "VISUALSTUDIOVERSION",
    "VSCMD_ARG_HOST_ARCH", "VSCMD_ARG_TGT_ARCH", "NEXA_CMAKE_BIN",
)
NATIVE_TARGETS = ("air_llama", "air-stream-test", "air-template-test",
                  "air-tool-parser-test", "air-ocr-template-test", "llama-completion", "llama-bench")
# Keep cache partitioning independent of credentials, the event commit and
# unrelated runner variables. Cargo locks/source inputs are added by the caller.
CACHE_IDENTITY_ENVIRONMENT = (
    "VCTOOLSINSTALLDIR", "VCTOOLSVERSION", "WINDOWSSDKDIR", "WINDOWSSDKVERSION",
    "UNIVERSALCRTSDKDIR", "UCRTVERSION", "NEXA_CMAKE_BIN", "IMAGEOS", "IMAGEVERSION",
)


def windows_cache_key(selected, vs, env, generator, rust, cmake):
    """Partition compiled dependencies by the already validated native tools.

    This is a cache hint, never proof that a restored product was built here.
    The normal source, native-library and final-package checks still run.
    """
    identity = {
        "schema": 1,
        "target": base.TARGET,
        "source_root": str(base.ROOT.resolve()).casefold(),
        "visual_studio_path": str(vs.resolve()).casefold(),
        "visual_studio": {key: selected.get(key, "") for key in
                          ("instanceId", "installationVersion", "productId")},
        "environment": {key: env.get(key, "") for key in CACHE_IDENTITY_ENVIRONMENT},
        "generator": generator,
        "rust": rust,
        "cmake": cmake,
    }
    encoded = json.dumps(identity, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def windows_handoff_key(env, rust, cmake, redist):
    """Bind cross-job packages to compatible tools and identical Release CRT.

    Runner images and VS instance IDs may differ across otherwise compatible
    hosts. Keep them in the cache partition, not the artifact compatibility key.
    """
    if not redist:
        base.fail("Windows handoff requires the selected Release CRT files")
    identity = {
        "schema": 1,
        "target": base.TARGET,
        "rust": rust,
        "cmake": cmake,
        "msvc_toolset": env["VCTOOLSVERSION"].strip(),
        "windows_sdk": env["WINDOWSSDKVERSION"].rstrip("\\/"),
        "ucrt": env.get("UCRTVERSION", "").strip(),
        "release_crt": {name: base.digest(base.regular(path)) for name, path in sorted(redist.items())},
    }
    encoded = json.dumps(identity, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def checked_rust(env):
    pin = tomllib.loads((base.ROOT / "rust-toolchain.toml").read_text(encoding="utf-8"))["toolchain"]["channel"]
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", pin):
        base.fail("CI requires a fixed Rust release, not a floating toolchain alias")
    rust = base.command(["rustc", "-vV"], env)
    fields = dict(line.split(": ", 1) for line in rust.splitlines() if ": " in line)
    if fields.get("release") != pin or fields.get("host") != base.TARGET:
        base.fail("actual Rust release/host differs from the pinned native Windows toolchain")
    return rust


def export_environment(path, values):
    lines = []
    for key, value in values.items():
        if not re.fullmatch(r"[A-Z_][A-Z0-9_]*", key) or any(c in value for c in "\r\n\0"):
            base.fail("invalid single-line CI build environment value")
        lines.append(f"{key}={value}\n")
    with Path(path).open("a", encoding="utf-8", newline="\n") as output:
        output.writelines(lines)


def run_logged(args, env, name):
    log = base.ROOT / "artifacts/verification" / name
    log.parent.mkdir(parents=True, exist_ok=True)
    with log.open("w", encoding="utf-8", newline="\n") as output:
        result = subprocess.run([str(arg) for arg in args], cwd=base.ROOT, env=env,
                                stdin=subprocess.DEVNULL, stdout=output, stderr=subprocess.STDOUT,
                                check=False)
    print(log.read_text(encoding="utf-8", errors="replace"), end="", flush=True)
    if result.returncode:
        base.fail(f"native CI command failed ({result.returncode}); see {name}")


def run(action):
    if sys.platform != "win32" or sys.maxsize <= 2**32:
        base.fail("native Windows x64 Python host required")
    selected, vs, env, _, redist = base.selected_visual_studio()
    native, configure = base.native_build_settings(selected, vs, env)
    generator = configure[configure.index("-G") + 1]
    rust = checked_rust(env)
    cmake = base.checked_cmake(env, generator)
    print(rust)
    print(cmake)
    if action == "prepare":
        expected = os.environ.get("GITHUB_SHA", "")
        if not re.fullmatch(r"[a-f0-9]{40}", expected) or base.command(["git", "rev-parse", "HEAD"], env) != expected:
            base.fail("Windows checkout differs from the requested CI commit")
        if base.command(["git", "status", "--porcelain", "--untracked-files=all"], env):
            base.fail("Windows CI requires a clean source checkout")
        if (base.command(["git", "-C", "vendor/llama.cpp", "rev-parse", "HEAD"], env) != base.LLAMA_COMMIT or
                base.command(["git", "-C", "vendor/llama.cpp", "status", "--porcelain", "--untracked-files=all"], env)):
            base.fail("Windows CI vendor differs from its locked clean commit")
        values = {key: env[key] for key in BUILD_ENVIRONMENT if key in env}
        values["AIR_NATIVE_DIR"] = str(native)
        # Match package_windows.build's selected Release Cargo tree, without
        # changing the separate root/debug or desktop workspace output paths.
        values["NEXA_RUNTIME_CARGO_TARGET_DIR"] = str(
            native.with_name("cargo-" + native.name.removeprefix("native-")).resolve())
        values["NEXA_WINDOWS_CACHE_KEY"] = windows_cache_key(selected, vs, env, generator, rust, cmake)
        values["NEXA_WINDOWS_HANDOFF_KEY"] = windows_handoff_key(env, rust, cmake, redist)
        export_environment(os.environ["GITHUB_ENV"], values)
        print(f"Selected {generator} / {env['VCTOOLSVERSION']}")
        return
    if Path(os.environ.get("AIR_NATIVE_DIR", "")).resolve() != native.resolve():
        base.fail("native build selection changed after CI preparation")
    run_logged(configure, env, "windows-configure.log")
    base.native_build_settings(selected, vs, env)
    run_logged(["cmake", "--build", native, "--config", "Release", "--target", *NATIVE_TARGETS,
                "--parallel", env.get("CMAKE_BUILD_PARALLEL_LEVEL", "4")], env, "windows-native-build.log")
    run_logged(["ctest", "--test-dir", native, "-C", "Release", "--output-on-failure"], env, "windows-ctest.log")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("action", choices=("prepare", "build"))
    run(parser.parse_args().action)
