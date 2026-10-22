#!/usr/bin/env python3
"""Explicit opt-in setup of isolated, pinned PI serializer validation inputs."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import urllib.request


def sha(data):
    return hashlib.sha256(data).hexdigest()


def deduplicate_patch(data):
    """The official patch repeats exactly one deepseek.json edit twice."""
    kept = {}
    for part in data.decode("utf-8").split("diff --git ")[1:]:
        key = part.splitlines()[0]
        if key in kept:
            changes = lambda text: [line for line in text.splitlines() if line.startswith(("+", "-", "@"))]
            if changes(kept[key]) != changes(part):
                raise ValueError("conflicting duplicate patch")
        else:
            kept[key] = part
    return "".join("diff --git " + part for part in kept.values()).encode("utf-8")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    args = parser.parse_args()
    root = args.directory.resolve()
    if root.exists():
        parser.error("choose a new external directory; existing installations are not overwritten")
    here = Path(__file__).resolve().parent
    lock = json.loads((here / "upstream-lock.json").read_text(encoding="utf-8"))
    source_lock = (here / "client/package-lock.json").read_bytes()
    if sha(source_lock) != lock["pi_ai"]["package_lock_sha256"]:
        raise ValueError("npm lock identity mismatch")
    for package in json.loads(source_lock)["packages"].values():
        if "resolved" in package and not package["resolved"].startswith("https://registry.npmjs.org/"):
            raise ValueError("non-registry dependency")
    (root / "upstream").mkdir(parents=True)
    # A minimal package marker avoids TypeScript type-stripping warnings.
    (root / "upstream/package.json").write_text('{"type":"module"}\n', encoding="utf-8")
    for entry in lock["source_files"]:
        with urllib.request.urlopen(entry["url"], timeout=60) as response:
            data = response.read(2 * 1024 * 1024 + 1)
        if len(data) > 2 * 1024 * 1024 or sha(data) != entry["sha256"]:
            raise ValueError("upstream source identity mismatch")
        (root / "upstream" / entry["file"]).write_bytes(data)
    for name in ("package.json", "package-lock.json"):
        shutil.copyfile(here / "client" / name, root / name)
    subprocess.run(["npm", "ci", "--ignore-scripts", "--no-audit", "--no-fund", "--registry=https://registry.npmjs.org", "--cache", str(root / ".npm-cache")], cwd=root, check=True)
    patch = deduplicate_patch((root / "upstream/pi-ai.patch").read_bytes())
    if sha(patch) != lock["applied_patch_sha256"]:
        raise ValueError("normalized patch identity mismatch")
    patch_path = root / "upstream/pi-ai-deduplicated.patch"
    patch_path.write_bytes(patch)
    package = root / "node_modules/@earendil-works/pi-ai"
    subprocess.run(["git", "apply", "--check", str(patch_path)], cwd=package, check=True)
    subprocess.run(["git", "apply", str(patch_path)], cwd=package, check=True)
    print("Pinned PI serializer inputs prepared; no application or system tools executed.")


if __name__ == "__main__":
    main()
