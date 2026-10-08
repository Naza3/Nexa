#!/usr/bin/env python3
"""Validate and publish a closed, same-commit Windows release asset set.

A tag push is the only publishing entry point. Branch/manual builds stage the
same artifacts without creating tags/releases. Existing bytes are never replaced.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import tempfile
import urllib.error
import urllib.parse
import urllib.request
import zipfile

from release_version import repository_version, tag_version, validate_version

ROOT = Path(__file__).resolve().parents[1]
SHA = re.compile(r"[0-9a-f]{40}", re.ASCII)
HASH = re.compile(r"[0-9a-f]{64}", re.ASCII)
SOURCE = "aria2-1.37.0-nexa-corresponding-source.tar.gz"
BUILD_CHECKS = {"payload_verified", "tauri_bundles_verified", "input_bytes_unchanged"}
LIFECYCLE_CHECKS = {"msi_install", "msi_repair", "msi_same_version_upgrade", "msi_rollback", "msi_upgrade",
                    "downgrade_rejected", "msi_uninstall", "legacy_msi_migration", "nsis_install",
                    "nsis_repeat_install", "nsis_same_version_upgrade", "nsis_uninstall",
                    "running_process_blocked", "cross_format_blocked", "user_data_preserved", "per_user_scope", "autostart_lifecycle"}


def regular(path: Path) -> Path:
    path = Path(path).absolute()
    for part in (path, *path.parents):
        info = part.lstat()
        if part.is_symlink() or getattr(info, "st_file_attributes", 0) & 0x400:
            raise ValueError("release symlink/reparse point rejected")
    if not path.is_file():
        raise ValueError("release source is not an ordinary file")
    return path


def digest(path: Path) -> str:
    with regular(path).open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate JSON key")
        result[key] = value
    return result


def load(path: Path):
    with regular(path).open("rb") as source:
        raw = source.read(8 * 1024 * 1024 + 1)
    if len(raw) > 8 * 1024 * 1024:
        raise ValueError("release metadata exceeds bounded size")
    return json.loads(raw.decode("utf-8-sig"), object_pairs_hook=unique_object,
                      parse_constant=lambda _: (_ for _ in ()).throw(ValueError("non-finite JSON")))


def write_json(path: Path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8", newline="\n")


def command(args, root=ROOT):
    return subprocess.check_output(args, cwd=root, encoding="utf-8", stderr=subprocess.PIPE).strip()


def validate(root=ROOT, environment=None):
    env = os.environ if environment is None else environment
    version = repository_version(root)
    commit = env.get("GITHUB_SHA", "")
    if not SHA.fullmatch(commit) or command(["git", "rev-parse", "HEAD"], root) != commit:
        raise ValueError("checkout is not the exact event commit")
    if command(["git", "status", "--porcelain", "--untracked-files=all"], root):
        raise ValueError("release requires clean committed source")
    ref = env.get("GITHUB_REF", "")
    tag = ref.removeprefix("refs/tags/") if ref.startswith("refs/tags/") else ""
    if tag and tag_version(tag) != version:
        raise ValueError("release tag does not match all committed package versions")
    if not tag and not ref.startswith("refs/heads/"):
        raise ValueError("only branch or tag refs are supported")
    return {"version": version, "tag": tag, "project_commit": commit}


def artifact_names(version):
    validate_version(version)
    prefix = f"Nexa-{version}-windows-x64"
    return {"portable": prefix + "-portable.zip", "msi": prefix + "-setup.msi",
            "setup": prefix + "-setup.exe", "source": f"Nexa-{version}-{SOURCE}"}


def zip_inventory(archive, expected_commit, expected_version):
    """Verify byte closure without extracting paths supplied by a ZIP."""
    with zipfile.ZipFile(regular(archive)) as zipped:
        infos = zipped.infolist()
        names = [item.filename for item in infos]
        if len(names) != len({name.casefold() for name in names}) or len(names) > 10000:
            raise ValueError("duplicate or oversized portable ZIP inventory")
        for item in infos:
            path = PurePosixPath(item.filename)
            if (not item.filename.startswith("desktop-windows/") or path.as_posix() != item.filename or path.is_absolute()
                    or any(part in {".", ".."} for part in path.parts) or "\\" in item.filename
                    or ":" in item.filename or item.is_dir() or (item.external_attr >> 16) & 0o170000 == 0o120000
                    or item.flag_bits & 1 or item.file_size > 1024**3):
                raise ValueError("unsafe portable ZIP entry")
        if sum(item.file_size for item in infos) > 2 * 1024**3:
            raise ValueError("portable ZIP is unexpectedly large")
        if any(zipped.getinfo("desktop-windows/" + name).file_size > 8 * 1024 * 1024
               for name in ("manifest.json", "SHA256SUMS", "runtime/manifest.json", "download/manifest.json")):
            raise ValueError("portable ZIP metadata is unexpectedly large")
        raw = zipped.read("desktop-windows/manifest.json")
        manifest = json.loads(raw.decode("utf-8"), object_pairs_hook=unique_object)
        if (manifest.get("product") != "nexa-desktop" or manifest.get("project_commit") != expected_commit
                or manifest.get("project_dirty") is not False or manifest.get("package_version") != expected_version):
            raise ValueError("portable source/version identity mismatch")
        records = manifest.get("files", [])
        expected = {"desktop-windows/" + item["path"]: item for item in records}
        if len(expected) != len(records):
            raise ValueError("duplicate portable manifest entry")
        if set(names) != set(expected) | {"desktop-windows/manifest.json", "desktop-windows/SHA256SUMS"}:
            raise ValueError("portable ZIP differs from closed manifest inventory")
        actual = []
        for item in infos:
            with zipped.open(item) as source:
                sha = hashlib.file_digest(source, "sha256").hexdigest()
            relative = item.filename.removeprefix("desktop-windows/")
            record = {"path": relative, "size_bytes": item.file_size, "sha256": sha}
            if item.filename in expected and expected[item.filename] != record:
                raise ValueError("portable ZIP file hash/size mismatch")
            actual.append(record)
        # Existing Windows packagers write CRLF and sort WindowsPath objects.
        # Verify a unique byte inventory, not the verifier host's sort order.
        sums = zipped.read("desktop-windows/SHA256SUMS").decode("utf-8")
        canonical = sums.replace("\r\n", "\n") if "\r\n" in sums else sums
        if (not canonical.endswith("\n") or "\r" in canonical
                or ("\r\n" in sums and "\n" in sums.replace("\r\n", ""))):
            raise ValueError("portable ZIP checksum newline format mismatch")
        parsed = {}
        for line in canonical.splitlines():
            match = re.fullmatch(r"([0-9a-f]{64})  (.+)", line)
            if not match or match[2] in parsed:
                raise ValueError("invalid or duplicate portable checksum record")
            parsed[match[2]] = match[1]
        if parsed != {item["path"]: item["sha256"] for item in actual if item["path"] != "SHA256SUMS"}:
            raise ValueError("portable ZIP checksum inventory mismatch")
        for name, product in (("runtime/manifest.json", "nexa-runtime"), ("download/manifest.json", "nexa-download")):
            nested = json.loads(zipped.read("desktop-windows/" + name).decode("utf-8"), object_pairs_hook=unique_object)
            if (nested.get("product") != product or nested.get("project_commit") != expected_commit
                    or nested.get("project_dirty") is not False or (product == "nexa-runtime" and nested.get("package_version") != expected_version)):
                raise ValueError("nested portable payload identity mismatch")
        source_record = expected.get("desktop-windows/download/" + SOURCE)
        if source_record is None:
            raise ValueError("exact aria2 corresponding source missing")
        return manifest, hashlib.sha256(raw).hexdigest(), actual, source_record


def installer_report(path, identity, required):
    value = load(path)
    if (value.get("schema_version") != 1 or value.get("status") != "pass"
            or any(value.get(key) != wanted for key, wanted in identity.items())):
        raise ValueError("installer report identity/status mismatch")
    checks = value.get("checks")
    if not isinstance(checks, dict) or set(checks) != required or any(item is not True for item in checks.values()):
        raise ValueError("installer report lacks required passing checks")
    return {key: checks[key] for key in sorted(checks)}


def stage(root, output, msi, setup, build_report, lifecycle_report, environment=None):
    env = os.environ if environment is None else environment
    identity = validate(root, env)
    version, commit = identity["version"], identity["project_commit"]
    payload = root / "dist/desktop-windows"
    archive = root / "dist/desktop-windows.zip"
    # Reuse full dependency/license/PE validators; do not build a second payload.
    import package_desktop_windows as desktop
    desktop.verify(payload, load(payload / "manifest.json"))
    manifest, manifest_hash, inventory, source = zip_inventory(archive, commit, version)
    if {item["path"]: item for item in desktop.base.entries(payload)} != {item["path"]: item for item in inventory}:
        raise ValueError("portable ZIP and validated MSI payload differ")
    desktop_acceptance = load(root / "artifacts/verification/windows-desktop/acceptance.json")
    if (desktop_acceptance.get("result") != "pass" or desktop_acceptance.get("project_commit") != commit
            or desktop_acceptance.get("desktop_package_sha256") != digest(archive)
            or desktop_acceptance.get("package_unchanged") is not True or desktop_acceptance.get("bridge_result") != "pass"):
        raise ValueError("current portable real-bridge acceptance is missing or stale")
    installer_identity = {"project_commit": commit, "version": version, "payload_manifest_sha256": manifest_hash,
                          "msi_sha256": digest(msi), "setup_sha256": digest(setup)}
    build_checks = installer_report(build_report, installer_identity, BUILD_CHECKS)
    lifecycle_checks = installer_report(lifecycle_report, installer_identity, LIFECYCLE_CHECKS)
    if load(build_report).get("unsigned") is not True:
        raise ValueError("unexpected installer signing claim")
    if regular(msi).read_bytes()[:8] != bytes.fromhex("d0cf11e0a1b11ae1") or regular(setup).read_bytes()[:2] != b"MZ":
        raise ValueError("installer file format mismatch")
    names = artifact_names(version)
    if output.exists():
        raise ValueError("release destination already exists; never merge stale assets")
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".release-stage-", dir=output.parent) as temporary:
        work = Path(temporary) / "release"
        work.mkdir()
        for key, path in {"portable": archive, "msi": msi, "setup": setup, "source": payload / "download" / SOURCE}.items():
            shutil.copyfile(regular(path), work / names[key])
        if digest(work / names["source"]) != source["sha256"]:
            raise ValueError("standalone corresponding source differs from portable payload")
        records = [{"path": name, "sha256": digest(work / name), "size_bytes": (work / name).stat().st_size} for name in sorted(names.values())]
        report = {"schema_version": 1, "product": "nexa-windows-release", "version": version,
                  "tag": "v" + version, "project_commit": commit, "payload_manifest_sha256": manifest_hash,
                  "payload_file_count": len(inventory), "license_file_count": len(desktop.base.license_related_files(payload)),
                  "artifacts": records, "installer_identity": installer_identity,
                  "verification": {"native_runner": "windows-2022", "desktop_bridge": "pass", "installer_build": build_checks,
                                   "installer_lifecycle": lifecycle_checks, "unsigned": True, "native_window_tested": False,
                                   "target_windows10_tested": False, "clean_machine_offline_tested": False}}
        write_json(work / "release-manifest.json", report)
        (work / "SHA256SUMS").write_text("".join(f"{digest(path)}  {path.name}\n" for path in sorted(work.iterdir())), encoding="utf-8", newline="\n")
        verify(work, commit, version)
        work.rename(output)
    return report


def verify(directory, commit, version):
    if not SHA.fullmatch(commit):
        raise ValueError("invalid release commit")
    names = artifact_names(version)
    expected = set(names.values()) | {"release-manifest.json", "SHA256SUMS"}
    if {path.name for path in directory.iterdir()} != expected:
        raise ValueError("release assets are missing, duplicated or unexpected")
    report = load(directory / "release-manifest.json")
    if (report.get("schema_version") != 1 or report.get("product") != "nexa-windows-release"
            or report.get("version") != version or report.get("tag") != "v" + version or report.get("project_commit") != commit):
        raise ValueError("release manifest source/version identity mismatch")
    records = [{"path": name, "sha256": digest(directory / name), "size_bytes": (directory / name).stat().st_size} for name in sorted(names.values())]
    if report.get("artifacts") != records:
        raise ValueError("release asset hash/size inventory mismatch")
    sums = "".join(f"{digest(directory / name)}  {name}\n" for name in sorted(expected - {"SHA256SUMS"}))
    if regular(directory / "SHA256SUMS").read_text(encoding="utf-8") != sums:
        raise ValueError("release checksum inventory mismatch")
    _, manifest_hash, inventory, source = zip_inventory(directory / names["portable"], commit, version)
    if report.get("payload_manifest_sha256") != manifest_hash or report.get("payload_file_count") != len(inventory):
        raise ValueError("release portable manifest binding mismatch")
    if digest(directory / names["source"]) != source["sha256"] or (directory / names["source"]).stat().st_size != source["size_bytes"]:
        raise ValueError("release corresponding source differs from included source")
    expected_identity = {"project_commit": commit, "version": version, "payload_manifest_sha256": manifest_hash,
                         "msi_sha256": digest(directory / names["msi"]), "setup_sha256": digest(directory / names["setup"])}
    proof = report.get("verification", {})
    if (report.get("installer_identity") != expected_identity or proof.get("native_runner") != "windows-2022"
            or proof.get("desktop_bridge") != "pass" or proof.get("unsigned") is not True
            or any(proof.get(key) is not False for key in ("native_window_tested", "target_windows10_tested", "clean_machine_offline_tested"))):
        raise ValueError("release proof/installer identity mismatch")
    for name, required in (("installer_build", BUILD_CHECKS), ("installer_lifecycle", LIFECYCLE_CHECKS)):
        checks = proof.get(name)
        if not isinstance(checks, dict) or set(checks) != required or any(value is not True for value in checks.values()):
            raise ValueError("release installer proof is incomplete")
    return report


class GitHub:
    def __init__(self, repository, token):
        if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository) or not token:
            raise ValueError("missing verified repository/token")
        self.repository, self.token = repository, token
        self.base = f"https://api.github.com/repos/{repository}"

    def request(self, method, path, value=None, data=None):
        url = path if path.startswith("https://") else self.base + path
        parsed = urllib.parse.urlsplit(url)
        if parsed.scheme != "https" or parsed.netloc not in {"api.github.com", "uploads.github.com"} or not parsed.path.startswith(f"/repos/{self.repository}/"):
            raise ValueError("unexpected GitHub API destination")
        headers = {"Accept": "application/vnd.github+json", "Authorization": "Bearer " + self.token,
                   "X-GitHub-Api-Version": "2022-11-28", "User-Agent": "Nexa-release"}
        if value is not None:
            data = json.dumps(value).encode("utf-8")
            headers["Content-Type"] = "application/json"
        elif data is not None:
            headers["Content-Type"] = "application/octet-stream"
        request = urllib.request.Request(url, data=data, headers=headers, method=method)
        # Do not forward the token to a redirect target.
        class NoRedirect(urllib.request.HTTPRedirectHandler):
            def redirect_request(self, req, fp, code, msg, hdrs, newurl):
                return None
        try:
            with urllib.request.build_opener(NoRedirect).open(request, timeout=120) as response:
                return json.loads(response.read().decode("utf-8"))
        except urllib.error.HTTPError as error:
            if error.code == 404 and method == "GET":
                return None
            raise ValueError(f"GitHub API {method} failed with HTTP {error.code}; existing assets were not overwritten") from None

    def require_tag(self, tag, commit):
        obj = self.request("GET", "/git/ref/tags/" + urllib.parse.quote(tag, safe=""))
        if not obj:
            raise ValueError("tag no longer exists; release never creates tags")
        obj = obj["object"]
        for _ in range(8):
            if obj["type"] == "commit":
                if obj["sha"] != commit:
                    raise ValueError("tag moved away from the validated event commit")
                return
            if obj["type"] != "tag" or not SHA.fullmatch(obj["sha"]):
                break
            obj = self.request("GET", "/git/tags/" + obj["sha"])["object"]
        raise ValueError("unsupported tag object chain")

    def assets(self, release_id):
        assets = []
        for page in range(1, 11):
            batch = self.request("GET", f"/releases/{release_id}/assets?per_page=100&page={page}")
            assets.extend(batch)
            if len(batch) < 100:
                return assets
        raise ValueError("too many release assets")


def check_remote_asset(asset, path):
    if (asset.get("name") != path.name or asset.get("state") != "uploaded"
            or asset.get("size") != regular(path).stat().st_size or asset.get("digest") != "sha256:" + digest(path)):
        raise ValueError("existing/uploaded release asset differs; never overwrite a published version")


def publish(directory, environment=None, client=None):
    env = os.environ if environment is None else environment
    if env.get("GITHUB_EVENT_NAME") != "push" or not env.get("GITHUB_REF", "").startswith("refs/tags/"):
        raise ValueError("publishing requires an actual tag push, not branch/dispatch/PR")
    tag = env["GITHUB_REF"].removeprefix("refs/tags/")
    version, commit = tag_version(tag), env.get("GITHUB_SHA", "")
    verify(directory, commit, version)
    api = client or GitHub(env.get("GITHUB_REPOSITORY", ""), env.get("GH_TOKEN", ""))
    api.require_tag(tag, commit)
    marker = f"<!-- nexa-release commit={commit} manifest={digest(directory / 'release-manifest.json')} -->"
    release = api.request("GET", "/releases/tags/" + urllib.parse.quote(tag, safe=""))
    if release is None:
        release = api.request("POST", "/releases", {"tag_name": tag, "target_commitish": commit, "name": "Nexa " + tag,
            "draft": True, "prerelease": False, "make_latest": "false",
            "body": f"{marker}\n\nWindows x64 CPU：便携 ZIP、MSI 和 Setup EXE，来自同一已验证 payload。\n\n"
                    f"源码提交：{commit}。SHA256SUMS 与 release-manifest.json 记录完整身份；对应 aria2 源码随包并单独提供。\n\n"
                    "MSI/Setup 为当前用户安装，升级和卸载保留模型与用户配置。需要系统已有 Evergreen WebView2。\n\n"
                    "安装程序尚未代码签名，Windows 可能提示未知发布者。标准 Windows Server 2022 CI 已完成原生和安装生命周期验证；"
                    "Windows 10 目标机原生窗口、无开发工具/离线及长期稳定性验收仍独立待验。"})
    if release.get("tag_name") != tag or marker not in release.get("body", "") or release.get("prerelease") is not False:
        raise ValueError("existing release was not created for this exact validated manifest")
    paths = {path.name: path for path in directory.iterdir()}
    existing = api.assets(release["id"])
    if len({asset["name"] for asset in existing}) != len(existing) or any(asset["name"] not in paths for asset in existing):
        raise ValueError("existing release has duplicate or unexpected assets")
    for asset in existing:
        check_remote_asset(asset, paths[asset["name"]])
    missing = set(paths) - {asset["name"] for asset in existing}
    if release.get("draft") is not True and missing:
        raise ValueError("published release is incomplete; never mutate published assets")
    if release.get("draft") is True:
        upload = f"https://uploads.github.com/repos/{api.repository}/releases/{release['id']}/assets"
        for name in sorted(missing):
            asset = api.request("POST", upload + "?name=" + urllib.parse.quote(name, safe=""), data=regular(paths[name]).read_bytes())
            check_remote_asset(asset, paths[name])
        final = api.assets(release["id"])
        if {asset["name"] for asset in final} != set(paths) or len(final) != len(paths):
            raise ValueError("draft asset inventory incomplete; left unpublished")
        for asset in final:
            check_remote_asset(asset, paths[asset["name"]])
        api.require_tag(tag, commit)
        release = api.request("PATCH", f"/releases/{release['id']}", {"draft": False, "make_latest": "legacy"})
        if release.get("draft") is not False:
            raise ValueError("release publication was not confirmed")
    api.require_tag(tag, commit)
    print(f"Verified published release {tag} at exact commit {commit}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="action", required=True)
    check = commands.add_parser("validate")
    check.add_argument("--github-output", type=Path)
    staging = commands.add_parser("stage")
    staging.add_argument("--output", type=Path, default=ROOT / "dist/release")
    for name in ("msi", "setup", "build-report", "lifecycle-report"):
        staging.add_argument("--" + name, type=Path, required=True)
    verification = commands.add_parser("verify")
    verification.add_argument("--directory", type=Path, required=True)
    verification.add_argument("--commit", required=True)
    verification.add_argument("--version", required=True)
    publishing = commands.add_parser("publish")
    publishing.add_argument("--directory", type=Path, required=True)
    args = parser.parse_args()
    if args.action == "validate":
        result = validate()
        if args.github_output:
            with args.github_output.open("a", encoding="utf-8", newline="\n") as output:
                output.writelines(f"{key}={value}\n" for key, value in result.items())
        print(json.dumps(result))
    elif args.action == "stage":
        stage(ROOT, args.output, args.msi, args.setup, args.build_report, args.lifecycle_report)
    elif args.action == "verify":
        verify(args.directory, args.commit, args.version)
    else:
        publish(args.directory)


if __name__ == "__main__":
    main()
