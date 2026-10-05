#!/usr/bin/env python3
"""Read-only old-ZIP corpus check; creates only temporary validation fixtures.

Never builds, executes, publishes, rewrites the input, or creates an output ZIP.
This verifies packaging source behavior, not new binaries or Windows execution.
"""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
import tempfile
import zipfile
import package_windows as base
import package_desktop_windows as desktop


def verify_old_inventory(stage):
    actual = base.entries(stage)
    manifest = json.loads(base.regular(stage / "manifest.json").read_text(encoding="utf-8"))
    # WindowsPath sorts case-insensitively; LinuxPath does not. Compare the
    # closed mapping, preserving the original manifest/checksum bytes below.
    payload = [item for item in actual if item["path"] not in {"manifest.json", "SHA256SUMS"}]
    declared = {item["path"]: item for item in manifest["files"]}
    if len(declared) != len(manifest["files"]) or declared != {item["path"]: item for item in payload}:
        base.fail("input corpus inventory/hash/size mismatch")
    sums = base.regular(stage / "SHA256SUMS").read_text(encoding="utf-8").splitlines()
    parsed = [line.split("  ") for line in sums]
    if any(len(item) != 2 for item in parsed):
        base.fail("invalid input corpus SHA256SUMS record")
    checksums = {path: sha for sha, path in parsed}
    if len(checksums) != len(parsed) or checksums != {item["path"]: item["sha256"] for item in actual if item["path"] != "SHA256SUMS"}:
        base.fail("input corpus SHA256SUMS mismatch")
    return manifest


def seal_fixture(stage, manifest):
    for name in ("manifest.json", "SHA256SUMS"):
        (stage / name).unlink()
    manifest["files"] = base.entries(stage)
    base.write_json(stage / "manifest.json", manifest)
    (stage / "SHA256SUMS").write_text("".join(f"{item['sha256']}  {item['path']}\n" for item in base.entries(stage)), encoding="utf-8")


def verify_corpus(archive, expected_sha256):
    if base.digest(base.regular(archive)) != expected_sha256:
        base.fail("input corpus ZIP differs from expected SHA256")
    with tempfile.TemporaryDirectory(prefix="nexa-license-corpus-") as temporary:
        stage = Path(temporary) / "desktop-windows"
        stage.mkdir()
        with zipfile.ZipFile(archive) as zipped:
            roots = [prefix for prefix in ("desktop-windows/", "Nexa-Windows-cross-test/desktop-windows/")
                     if prefix + "manifest.json" in zipped.namelist()]
            if len(roots) != 1:
                base.fail("input corpus must have exactly one recognized desktop root")
            archive_prefix = roots[0]
            seen = set()
            for info in zipped.infolist():
                # This input is data, not a trusted extraction instruction.
                name = base.relative(info.filename)
                if (info.is_dir() or
                        name.casefold() in seen or (info.external_attr >> 16) & 0o170000 == 0o120000):
                    base.fail("unexpected/indirect ZIP corpus member")
                seen.add(name.casefold())
                if not name.startswith(archive_prefix):
                    continue  # A cross ZIP also carries a separate acceptance-tools package.
                target = stage / name.removeprefix(archive_prefix)
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(zipped.read(info))
        roles = {"desktop": stage, "runtime": stage / "runtime", "download": stage / "download"}
        manifests = {role: verify_old_inventory(root) for role, root in roles.items()}
        if (base.digest(stage / "runtime/manifest.json") != manifests["desktop"]["runtime_manifest_sha256"] or
                any(manifests[role]["project_commit"] != manifests["desktop"]["project_commit"] for role in roles)):
            base.fail("input corpus source identity mismatch")
        before = base.entries(stage)
        notice_names = base.license_related_files(stage)
        notices = {name: (stage / name).read_bytes() for name in notice_names}
        source_hash = base.digest(stage / "download" / desktop.DOWNLOAD_SOURCE)
        build_hash = base.digest(stage / "download/build-manifest.json")
        lock = json.loads((base.ROOT / "third_party/aria2/source-lock.json").read_text(encoding="utf-8"))
        recovered = {}
        role_counts = {}
        for role, root in roles.items():
            supplied = desktop.download_attributions(lock) if role == "download" else None
            base.consolidate_licenses(root, supplied=supplied)
            originals = base.verify_license_bundle(root, supplied=supplied)
            prefix = "" if role == "desktop" else role + "/"
            recovered.update({prefix + name: item["raw"] for name, item in originals.items()})
            role_counts[role] = len(originals)
        for name in notice_names:
            if name not in recovered and "/licenses/" not in name and not name.startswith("licenses/"):
                recovered[name] = (stage / name).read_bytes()
        if recovered != notices:
            base.fail("not every original license/notice/index byte survived")
        seal_fixture(stage / "runtime", manifests["runtime"])
        seal_fixture(stage / "download", manifests["download"])
        manifests["desktop"]["runtime_manifest_sha256"] = base.digest(stage / "runtime/manifest.json")
        seal_fixture(stage, manifests["desktop"])
        desktop.verify(stage, manifests["desktop"])
        unchanged = [item for item in before if item["path"] not in notice_names and
                     Path(item["path"]).name not in {"manifest.json", "SHA256SUMS"}]
        if any(base.digest(stage / item["path"]) != item["sha256"] for item in unchanged):
            base.fail("a non-license payload changed in the temporary fixture")
        if source_hash != base.digest(stage / "download" / desktop.DOWNLOAD_SOURCE) or build_hash != base.digest(stage / "download/build-manifest.json"):
            base.fail("corresponding source archive or build provenance was modified")
        result = {"verification": "temporary source-level corpus fixture; no new binaries or distribution",
                  "input_sha256": expected_sha256, "input_project_commit": manifests["desktop"]["project_commit"],
                  "input_desktop_prefix": archive_prefix,
                  "input_total_files": len(before), "input_license_notice_files": len(notices),
                  "recovered_original_files": len(recovered), "original_license_files_by_role": role_counts,
                  "fixture_license_notice_files": base.license_related_files(stage),
                  "fixture_total_files": len(base.entries(stage)), "unchanged_non_license_payload_files": len(unchanged),
                  "corresponding_source_sha256": source_hash, "source_build_manifest_sha256": build_hash}
    if base.digest(archive) != expected_sha256:
        base.fail("read-only corpus input changed during verification")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--expected-sha256", required=True)
    args = parser.parse_args()
    print(json.dumps(verify_corpus(args.archive, args.expected_sha256), ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
