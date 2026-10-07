#!/usr/bin/env python3
"""Synchronize Nexa product versions offline without resolving dependencies."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import sys
import tempfile

if sys.version_info < (3, 11):
    raise SystemExit("Python 3.11 or newer is required.")

import tomllib

from release_version import repository_version, tag_version, validate_version

ROOT = Path(__file__).resolve().parents[1]
VERSION_FILES = (
    "Cargo.toml", "Cargo.lock", "apps/desktop/package.json",
    "apps/desktop/package-lock.json", "apps/desktop/src-tauri/Cargo.toml",
    "apps/desktop/src-tauri/Cargo.lock", "apps/desktop/src-tauri/tauri.conf.json",
)


def replace_version_field(text: str, target: str) -> str:
    result, count = re.subn(
        r'(?m)^([ \t]*version[ \t]*=[ \t]*)[\"\'][^\"\'\r\n]*[\"\']',
        lambda match: match[1] + '"' + target + '"', text,
    )
    if count != 1:
        raise ValueError("expected exactly one product version in the TOML section")
    return result


def update_table(text: str, table: str, target: str) -> str:
    pattern = r'(?ms)(^\[' + re.escape(table) + r'\][^\n]*\n)(.*?)(?=^\[|\Z)'
    result, count = re.subn(pattern, lambda match: match[1] + replace_version_field(match[2], target), text)
    if count != 1:
        raise ValueError(f"expected one [{table}] section")
    return result


def update_lock(text: str, target: str) -> str:
    packages = tomllib.loads(text)["package"]
    local = {item["name"]: item["version"] for item in packages if "source" not in item}
    blocks = re.split(r'(?m)(?=^\[\[package\]\])', text)
    for index, block in enumerate(blocks):
        if block.startswith("[[package]]"):
            package = tomllib.loads(block)["package"][0]
            if "source" not in package:
                blocks[index] = replace_version_field(block, target)
    result = "".join(blocks)
    # Cargo sometimes qualifies dependency edges with a version. Registry/git
    # edges include a source suffix and must retain their original identity.
    for name, previous in local.items():
        pattern = r'(?m)^([ \t]*)"' + re.escape(name + " " + previous) + r'"([, \t]*\r?$)'
        result = re.sub(pattern, lambda match: match[1] + json.dumps(name + " " + target) + match[2], result)
    return result


def update_json(text: str, target: str, *, npm_lock: bool = False) -> str:
    value = json.loads(text)
    value["version"] = target
    if npm_lock:
        value["packages"][""]["version"] = target
    newline = "\r\n" if "\r\n" in text else "\n"
    return (json.dumps(value, ensure_ascii=False, indent=2) + "\n").replace("\n", newline)


def write_atomic(path: Path, data: bytes) -> None:
    """Replace one file only after its complete new contents have been written."""
    mode = path.stat().st_mode
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(dir=path.parent, prefix=".nexa-version-", delete=False) as stream:
            temporary = Path(stream.name)
            stream.write(data)
        temporary.chmod(mode)
        os.replace(temporary, path)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def update_version(root: Path, target: str, *, dry_run: bool = False) -> list[str]:
    target = tag_version(target) if isinstance(target, str) and target.startswith("v") else validate_version(target)
    root = Path(root)
    originals = {name: (root / name).read_bytes() for name in VERSION_FILES}
    workspace = tomllib.loads(originals["Cargo.toml"].decode("utf-8"))["workspace"]
    members = {}
    for member in workspace["members"]:
        path = str(Path(member) / "Cargo.toml")
        members[path] = (root / path).read_bytes()
        if tomllib.loads(members[path].decode("utf-8"))["package"]["version"] != {"workspace": True}:
            raise ValueError(f"workspace member must inherit version.workspace: {member}")
    updated = {}
    for name, original in originals.items():
        text = original.decode("utf-8")
        if name.endswith("Cargo.lock"):
            result = update_lock(text, target)
        elif name.endswith("Cargo.toml"):
            result = update_table(text, "workspace.package" if name == "Cargo.toml" else "package", target)
        else:
            result = update_json(text, target, npm_lock=name.endswith("package-lock.json"))
        updated[name] = result.encode("utf-8")

    # Permit repair of partially bumped versions, but validate the entire
    # proposed set with the production release gate before changing any file.
    with tempfile.TemporaryDirectory(prefix="nexa-version-") as temp:
        staged = Path(temp)
        for name, data in (members | updated).items():
            path = staged / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
        if repository_version(staged) != target:
            raise ValueError("staged product version differs from requested version")

    changed = [name for name in VERSION_FILES if originals[name] != updated[name]]
    if dry_run:
        return changed
    for name, data in (members | originals).items():
        if (root / name).read_bytes() != data:
            raise ValueError(f"file changed during version preparation: {name}")
    written = []
    try:
        for name in changed:
            write_atomic(root / name, updated[name])
            written.append(name)
        repository_version(root)
    except (Exception, KeyboardInterrupt):
        failures = []
        for name in reversed(written):
            try:
                write_atomic(root / name, originals[name])
            except OSError:
                failures.append(name)
        if failures:
            raise RuntimeError("could not restore original files; inspect git diff: " + ", ".join(failures))
        raise
    return changed


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("version", nargs="?", help="new version, e.g. 0.2.1 or v0.2.1; prompts when omitted")
    parser.add_argument("--dry-run", action="store_true", help="validate and list changes without writing")
    parser.add_argument("--check", action="store_true", help="check current version consistency without writing")
    args = parser.parse_args()
    if args.check and (args.version is not None or args.dry_run):
        parser.error("--check cannot be combined with a version or --dry-run")
    try:
        if args.check:
            print("Version check passed: " + repository_version(ROOT))
            return 0
        target = args.version if args.version is not None else input("New version (e.g. 0.2.1; Enter to cancel): ").strip()
        if not target:
            print("Cancelled; no files changed.")
            return 0
        changed = update_version(ROOT, target, dry_run=args.dry_run)
        print(("Preview: " if args.dry_run else "Version synchronized: ") + target)
        for name in changed:
            print("  " + name)
        print(f"{len(changed)} file(s) {'would change' if args.dry_run else 'changed'}.")
        if not args.dry_run:
            print("Review git diff and commit these files before creating the matching release tag.")
        return 0
    except (ValueError, OSError, KeyError, TypeError, RuntimeError) as error:
        print("Version update failed: " + str(error), file=sys.stderr)
        return 1
    except (EOFError, KeyboardInterrupt):
        print("Cancelled.", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
