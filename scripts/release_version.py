#!/usr/bin/env python3
"""Read one committed, stable version shared by Cargo, Tauri, npm and MSI.

Never rewrite source versions in CI. Prerelease/build suffixes are deliberately
unsupported because Windows Installer only compares three numeric fields.
"""
from __future__ import annotations

import json
from pathlib import Path
import re
import tomllib

VERSION = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", re.ASCII)


def validate_version(value: str) -> str:
    match = VERSION.fullmatch(value) if isinstance(value, str) else None
    if match is None:
        raise ValueError("version must be canonical MAJOR.MINOR.PATCH; prereleases/build suffixes are unsupported")
    if any(int(part) > limit for part, limit in zip(match.groups(), (255, 255, 65535))):
        raise ValueError("version exceeds Windows Installer limits (255.255.65535)")
    if value == "0.0.0":
        raise ValueError("0.0.0 is reserved; choose a positive release version")
    return value


def tag_version(tag: str) -> str:
    if not isinstance(tag, str) or not tag.startswith("v"):
        raise ValueError("release tag must be vMAJOR.MINOR.PATCH")
    return validate_version(tag[1:])


def repository_version(root: Path) -> str:
    root = Path(root)
    workspace = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]
    version = validate_version(workspace["package"]["version"])
    local = {}
    for member in workspace["members"]:
        package = tomllib.loads((root / member / "Cargo.toml").read_text(encoding="utf-8"))["package"]
        actual = version if package["version"] == {"workspace": True} else package["version"]
        if actual != version or package["name"] in local:
            raise ValueError(f"workspace package version/name mismatch: {member}")
        local[package["name"]] = actual
    shell = root / "apps/desktop/src-tauri"
    package = tomllib.loads((shell / "Cargo.toml").read_text(encoding="utf-8"))["package"]
    tauri = json.loads((shell / "tauri.conf.json").read_text(encoding="utf-8"))
    npm = json.loads((root / "apps/desktop/package.json").read_text(encoding="utf-8"))
    npm_lock = json.loads((root / "apps/desktop/package-lock.json").read_text(encoding="utf-8"))
    versions = (package["version"], tauri["version"], npm["version"], npm_lock["version"], npm_lock["packages"][""]["version"])
    if any(actual != version for actual in versions):
        raise ValueError("Cargo/Tauri/npm/package-lock versions must all match the committed workspace version")
    if any(value != "nexa-desktop" for value in (package["name"], npm["name"], npm_lock["name"], npm_lock["packages"][""]["name"])):
        raise ValueError("desktop package identity mismatch")
    for path, required, permitted in (
        (root / "Cargo.lock", set(local), local),
        (shell / "Cargo.lock", {package["name"]}, local | {package["name"]: version}),
    ):
        entries = [item for item in tomllib.loads(path.read_text(encoding="utf-8"))["package"] if "source" not in item]
        names = [item["name"] for item in entries]
        if len(names) != len(set(names)) or not required <= set(names):
            raise ValueError("Cargo.lock local package inventory mismatch")
        if any(item["name"] not in permitted or item["version"] != version for item in entries):
            raise ValueError("Cargo.lock has an unknown or stale local package version")
    return version
