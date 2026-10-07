import copy
import contextlib
import io
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock
import urllib.parse
import zipfile

import release_windows as release

COMMIT = "a" * 40
VERSION = "0.1.0"
ENV = {"GITHUB_SHA": COMMIT, "GITHUB_REF": "refs/tags/v" + VERSION, "GITHUB_EVENT_NAME": "push", "GITHUB_REPOSITORY": "Naza3/Nexa"}


def encoded(value):
    return (json.dumps(value, sort_keys=True) + "\n").encode()


def record(path, data):
    return {"path": path, "sha256": hashlib.sha256(data).hexdigest(), "size_bytes": len(data)}


def portable(path, commit=COMMIT, version=VERSION, extra=None):
    files = {"nexa-desktop.exe": b"MZfixture", "download/" + release.SOURCE: b"exact corresponding source"}
    for name, product in (("runtime/manifest.json", "nexa-runtime"), ("download/manifest.json", "nexa-download")):
        files[name] = encoded({"product": product, "project_commit": commit, "project_dirty": False, "package_version": version})
    manifest = {"product": "nexa-desktop", "project_commit": commit, "project_dirty": False, "package_version": version,
                "files": [record(name, data) for name, data in sorted(files.items())]}
    files["manifest.json"] = encoded(manifest)
    files["SHA256SUMS"] = "".join(f"{hashlib.sha256(data).hexdigest()}  {name}\n" for name, data in sorted(files.items())).encode()
    with zipfile.ZipFile(path, "w") as zipped:
        for name, data in files.items():
            zipped.writestr("desktop-windows/" + name, data)
        if extra:
            zipped.writestr(*extra)
    return files


def fixture(root):
    names = release.artifact_names(VERSION)
    files = portable(root / names["portable"])
    (root / names["source"]).write_bytes(files["download/" + release.SOURCE])
    (root / names["msi"]).write_bytes(bytes.fromhex("d0cf11e0a1b11ae1") + b"fixture")
    (root / names["setup"]).write_bytes(b"MZfixture")
    identity = {"project_commit": COMMIT, "version": VERSION, "payload_manifest_sha256": hashlib.sha256(files["manifest.json"]).hexdigest(),
                "msi_sha256": release.digest(root / names["msi"]), "setup_sha256": release.digest(root / names["setup"])}
    report = {"schema_version": 1, "product": "nexa-windows-release", "version": VERSION, "tag": "v" + VERSION,
              "project_commit": COMMIT, "payload_manifest_sha256": identity["payload_manifest_sha256"], "payload_file_count": len(files),
              "artifacts": [record(name, (root / name).read_bytes()) for name in sorted(names.values())], "installer_identity": identity,
              "verification": {"native_runner": "windows-2022", "desktop_bridge": "pass", "unsigned": True,
                               "native_window_tested": False, "target_windows10_tested": False, "clean_machine_offline_tested": False,
                               "installer_build": dict.fromkeys(release.BUILD_CHECKS, True), "installer_lifecycle": dict.fromkeys(release.LIFECYCLE_CHECKS, True)}}
    release.write_json(root / "release-manifest.json", report)
    resums(root)
    return report


def resums(root):
    (root / "SHA256SUMS").write_text("".join(f"{release.digest(path)}  {path.name}\n" for path in sorted(root.iterdir()) if path.name != "SHA256SUMS"), encoding="utf-8")


class FakeGitHub:
    repository = "Naza3/Nexa"

    def __init__(self, release_value=None):
        self.release = release_value
        self.uploaded = []
        self.calls = []
        self.tag_checks = 0
        self.tag_moves_at = None
        self.bad_digest = False

    def require_tag(self, tag, commit):
        self.tag_checks += 1
        if self.tag_checks == self.tag_moves_at:
            raise ValueError("tag moved")
        assert tag == "v" + VERSION and commit == COMMIT

    def assets(self, _):
        return copy.deepcopy(self.uploaded)

    def request(self, method, path, value=None, data=None):
        self.calls.append((method, path))
        if method == "GET":
            return self.release
        if method == "POST" and path == "/releases":
            self.release = value | {"id": 12}
            return self.release
        if method == "POST":
            name = urllib.parse.parse_qs(urllib.parse.urlsplit(path).query)["name"][0]
            item = {"name": name, "size": len(data), "digest": "sha256:" + ("0" * 64 if self.bad_digest else hashlib.sha256(data).hexdigest()), "state": "uploaded"}
            self.uploaded.append(item)
            return item
        self.release.update(value)
        return self.release


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        # Unit-test fake publication must not look like an actual release log.
        output = contextlib.redirect_stdout(io.StringIO())
        output.__enter__()
        self.addCleanup(output.__exit__, None, None, None)

    def test_validate_exact_committed_tag(self):
        with mock.patch.object(release, "repository_version", return_value=VERSION), mock.patch.object(release, "command", side_effect=[COMMIT, ""]):
            self.assertEqual(release.validate(environment=ENV), {"version": VERSION, "tag": "v" + VERSION, "project_commit": COMMIT})

    def test_checkout_drift_dirty_and_wrong_tag_fail(self):
        for output, env in ((["b" * 40], ENV), ([COMMIT, " M Cargo.toml"], ENV), ([COMMIT, ""], ENV | {"GITHUB_REF": "refs/tags/v0.2.0"})):
            with self.subTest(output=output, env=env), mock.patch.object(release, "repository_version", return_value=VERSION), mock.patch.object(release, "command", side_effect=output), self.assertRaises(ValueError):
                release.validate(environment=env)

    def test_branch_validation_does_not_invent_tag(self):
        with mock.patch.object(release, "repository_version", return_value=VERSION), mock.patch.object(release, "command", side_effect=[COMMIT, ""]):
            self.assertEqual(release.validate(environment=ENV | {"GITHUB_REF": "refs/heads/codex/dev"})["tag"], "")

    def staging_fixture(self, root):
        payload = root / "dist/desktop-windows"
        payload.mkdir(parents=True)
        archive = root / "dist/desktop-windows.zip"
        files = portable(archive)
        for name, data in files.items():
            path = payload / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
        msi, setup = root / "dist/setup.msi", root / "dist/setup.exe"
        msi.write_bytes(bytes.fromhex("d0cf11e0a1b11ae1") + b"synthetic test fixture")
        setup.write_bytes(b"MZsynthetic test fixture")
        identity = {"project_commit": COMMIT, "version": VERSION, "payload_manifest_sha256": release.digest(payload / "manifest.json"),
                    "msi_sha256": release.digest(msi), "setup_sha256": release.digest(setup)}
        build, lifecycle = root / "build-report.json", root / "lifecycle-report.json"
        release.write_json(build, identity | {"schema_version": 1, "status": "pass", "unsigned": True, "checks": dict.fromkeys(release.BUILD_CHECKS, True)})
        release.write_json(lifecycle, identity | {"schema_version": 1, "status": "pass", "checks": dict.fromkeys(release.LIFECYCLE_CHECKS, True)})
        evidence = root / "artifacts/verification/windows-desktop/acceptance.json"
        evidence.parent.mkdir(parents=True)
        release.write_json(evidence, {"result": "pass", "project_commit": COMMIT, "desktop_package_sha256": release.digest(archive), "package_unchanged": True, "bridge_result": "pass"})
        return msi, setup, build, lifecycle

    def test_staging_orchestration_keeps_portable_bytes_and_complete_assets(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            args = self.staging_fixture(root)
            import package_desktop_windows as desktop
            # Payload's native validator has its own exhaustive suite. Only it
            # is mocked here; orchestration compares the real synthetic bytes.
            with mock.patch.object(desktop, "verify"), mock.patch.object(release, "validate", return_value={"version": VERSION, "project_commit": COMMIT, "tag": "v" + VERSION}):
                release.stage(root, root / "dist/release", *args, ENV)
            value = release.verify(root / "dist/release", COMMIT, VERSION)
            self.assertEqual(value["payload_file_count"], 6)
            self.assertEqual(release.digest(root / "dist/desktop-windows.zip"), release.digest(root / "dist/release" / release.artifact_names(VERSION)["portable"]))

    def test_staging_rejects_stale_bridge_report_and_keeps_destination_absent(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            args = self.staging_fixture(root)
            path = root / "artifacts/verification/windows-desktop/acceptance.json"
            value = release.load(path)
            value["desktop_package_sha256"] = "b" * 64
            release.write_json(path, value)
            import package_desktop_windows as desktop
            with mock.patch.object(desktop, "verify"), mock.patch.object(release, "validate", return_value={"version": VERSION, "project_commit": COMMIT, "tag": "v" + VERSION}), self.assertRaises(ValueError):
                release.stage(root, root / "dist/release", *args, ENV)
            self.assertFalse((root / "dist/release").exists())

    def test_complete_asset_set_verifies(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            report = fixture(root)
            self.assertEqual(release.verify(root, COMMIT, VERSION), report)

    def test_no_missing_additional_or_changed_assets(self):
        for action in ("delete", "add", "change"):
            with self.subTest(action=action), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                fixture(root)
                path = root / release.artifact_names(VERSION)["msi"]
                if action == "delete":
                    path.unlink()
                elif action == "add":
                    (root / "unexpected.log").write_bytes(b"no")
                else:
                    path.write_bytes(b"tampered")
                    resums(root)
                with self.assertRaises(ValueError):
                    release.verify(root, COMMIT, VERSION)

    def test_same_hash_inventory_cannot_hide_source_or_proof_mismatch(self):
        for change in ("commit", "source", "empty_checks", "false_checks", "installer_identity", "target_claim"):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                value = fixture(root)
                if change == "commit":
                    value["project_commit"] = "b" * 40
                elif change == "source":
                    name = release.artifact_names(VERSION)["source"]
                    (root / name).write_bytes(b"wrong corresponding source")
                    value["artifacts"] = [record(item["path"], (root / item["path"]).read_bytes()) for item in value["artifacts"]]
                elif change == "empty_checks":
                    value["verification"]["installer_lifecycle"] = {}
                elif change == "false_checks":
                    value["verification"]["installer_lifecycle"]["rollback"] = False
                elif change == "installer_identity":
                    value["installer_identity"]["setup_sha256"] = "b" * 64
                else:
                    value["verification"]["target_windows10_tested"] = True
                release.write_json(root / "release-manifest.json", value)
                resums(root)
                with self.assertRaises(ValueError):
                    release.verify(root, COMMIT, VERSION)

    def test_rejects_unsafe_zip_and_wrong_payload_identity(self):
        for extra in (("../escape", b"bad"), ("desktop-windows/../escape", b"bad"), ("desktop-windows/extra", b"bad")):
            with self.subTest(extra=extra), tempfile.TemporaryDirectory() as temp:
                path = Path(temp) / "portable.zip"
                portable(path, extra=extra)
                with self.assertRaises(ValueError):
                    release.zip_inventory(path, COMMIT, VERSION)
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "portable.zip"
            portable(path, commit="b" * 40)
            with self.assertRaises(ValueError):
                release.zip_inventory(path, COMMIT, VERSION)

    def test_windows_checksum_order_crlf_and_mixed_newlines(self):
        for mode in ("windows", "mixed", "duplicate"):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as temp:
                path = Path(temp) / "portable.zip"
                files = portable(path)
                lines = files["SHA256SUMS"].decode().splitlines()
                content = "\r\n".join(reversed(lines)) + "\r\n"
                if mode == "mixed":
                    content = content.replace("\r\n", "\n", 1)
                elif mode == "duplicate":
                    content += lines[0] + "\r\n"
                files["SHA256SUMS"] = content.encode()
                with zipfile.ZipFile(path, "w") as zipped:
                    for name, data in files.items():
                        zipped.writestr("desktop-windows/" + name, data)
                if mode == "windows":
                    release.zip_inventory(path, COMMIT, VERSION)
                else:
                    with self.assertRaises(ValueError):
                        release.zip_inventory(path, COMMIT, VERSION)

    def test_installer_report_requires_all_true_checks_and_exact_identity(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "report.json"
            for checks in ({}, {"install": True}, dict.fromkeys(release.LIFECYCLE_CHECKS, 1), dict.fromkeys(release.LIFECYCLE_CHECKS | {"unreviewed_field"}, True)):
                release.write_json(path, {"schema_version": 1, "status": "pass", "project_commit": COMMIT, "checks": checks})
                with self.assertRaises(ValueError):
                    release.installer_report(path, {"project_commit": COMMIT}, release.LIFECYCLE_CHECKS)

    def test_publish_draft_upload_verify_then_publish(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            fixture(root)
            api = FakeGitHub()
            release.publish(root, ENV, api)
            self.assertFalse(api.release["draft"])
            self.assertEqual(len(api.uploaded), 6)
            self.assertEqual(api.tag_checks, 3)
            self.assertEqual(api.calls[-1], ("PATCH", "/releases/12"))
            self.assertEqual(api.release["target_commitish"], COMMIT)
            # Exact rerun is read-only, with no upload or release edit.
            api.calls.clear()
            release.publish(root, ENV, api)
            self.assertEqual(api.calls, [("GET", "/releases/tags/v" + VERSION)])

    def test_partial_draft_retries_only_missing_bytes(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            fixture(root)
            api = FakeGitHub()
            api.tag_moves_at = 2
            with self.assertRaises(ValueError):
                release.publish(root, ENV, api)
            self.assertTrue(api.release["draft"])
            api.uploaded.pop()
            api.calls.clear()
            api.tag_moves_at = None
            release.publish(root, ENV, api)
            self.assertEqual(len([call for call in api.calls if call[0] == "POST"]), 1)

    def test_tag_moved_or_bad_upload_never_publishes(self):
        for mode in ("moved_before", "moved_after_upload", "bad_digest"):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                fixture(root)
                api = FakeGitHub()
                api.tag_moves_at = {"moved_before": 1, "moved_after_upload": 2}.get(mode)
                api.bad_digest = mode == "bad_digest"
                with self.assertRaises(ValueError):
                    release.publish(root, ENV, api)
                self.assertFalse(any(call[0] == "PATCH" for call in api.calls))

    def test_published_wrong_or_missing_asset_never_overwritten(self):
        for mode in ("missing", "changed", "unowned"):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                fixture(root)
                api = FakeGitHub()
                release.publish(root, ENV, api)
                if mode == "missing":
                    api.uploaded.pop()
                elif mode == "changed":
                    api.uploaded[0]["digest"] = "sha256:" + "b" * 64
                else:
                    api.release["body"] = "someone else's release"
                api.calls.clear()
                with self.assertRaises(ValueError):
                    release.publish(root, ENV, api)
                self.assertTrue(all(method == "GET" for method, _ in api.calls))

    def test_branch_dispatch_and_pr_cannot_publish(self):
        for env in (ENV | {"GITHUB_REF": "refs/heads/codex/dev"}, ENV | {"GITHUB_EVENT_NAME": "workflow_dispatch"}, ENV | {"GITHUB_EVENT_NAME": "pull_request"}):
            with self.subTest(env=env), self.assertRaises(ValueError):
                release.publish(Path("not-read"), env, FakeGitHub())

    def test_api_rejects_unexpected_destinations_before_network(self):
        api = release.GitHub("Naza3/Nexa", "fake-test-token")
        for url in ("https://example.com/repos/Naza3/Nexa/releases", "https://api.github.com/repos/other/repo/releases", "https://api.github.com.evil/repos/Naza3/Nexa/releases"):
            with self.subTest(url=url), self.assertRaises(ValueError):
                api.request("GET", url)

    def test_annotated_tag_is_peeled_and_missing_or_moved_tags_fail(self):
        api = release.GitHub("Naza3/Nexa", "fake-test-token")
        with mock.patch.object(api, "request", side_effect=[{"object": {"type": "tag", "sha": "b" * 40}}, {"object": {"type": "commit", "sha": COMMIT}}]):
            api.require_tag("v" + VERSION, COMMIT)
        for value in (None, {"object": {"type": "commit", "sha": "c" * 40}}):
            with mock.patch.object(api, "request", return_value=value), self.assertRaises(ValueError):
                api.require_tag("v" + VERSION, COMMIT)

    def test_installer_failure_upload_uses_only_validated_sanitized_sidecar(self):
        workflow = (release.ROOT / ".github/workflows/native-windows.yml").read_text(encoding="utf-8")
        stage = workflow.split("      - name: Stage only validated sanitized installer diagnostics\n", 1)[1]
        stage, remainder = stage.split("      - name: Preserve sanitized installer diagnostics on success or failure\n", 1)
        upload = remainder.split("      - name: Preserve unverified installer binaries only for failed lifecycle diagnosis\n", 1)[0]
        self.assertIn("        if: always()", stage)
        self.assertIn("        timeout-minutes: 2", stage)
        self.assertIn("windows_installer_diagnostics.py --input dist/windows-msi-diagnostics.json --output artifacts/upload-installer-evidence --commit $env:GITHUB_SHA", stage)
        self.assertIn("if ($LASTEXITCODE -ne 0)", stage)
        self.assertIn("always() && steps.installer_evidence.outcome == 'success'", upload)
        self.assertIn("          path: artifacts/upload-installer-evidence/", upload)
        self.assertIn("          if-no-files-found: error", upload)
        self.assertNotIn("dist/", upload)
        self.assertNotIn("windows-msi-logs", upload)
        self.assertNotIn("*.log", upload)
        self.assertLess(workflow.index("id: installer_acceptance"), workflow.index("id: installer_evidence"))
        self.assertLess(workflow.index("id: installer_evidence"), workflow.index("Stage closed three-format release assets"))

    def test_failed_lifecycle_binary_artifact_is_closed_unverified_and_not_a_release_input(self):
        workflow = (release.ROOT / ".github/workflows/native-windows.yml").read_text(encoding="utf-8")
        upload = workflow.split("      - name: Preserve unverified installer binaries only for failed lifecycle diagnosis\n", 1)[1].split("      - name: Preserve desktop package\n", 1)[0]
        self.assertIn("always() && steps.windows_installers.outcome == 'success' && steps.installer_acceptance.outcome != 'success'", upload)
        self.assertIn("name: nexa-windows-installers-UNVERIFIED-${{ github.sha }}", upload)
        paths = upload.split("          path: |\n", 1)[1].split("          if-no-files-found:", 1)[0]
        self.assertEqual([line.strip() for line in paths.splitlines()], [
            "dist/Nexa-${{ env.NEXA_RELEASE_VERSION }}-windows-x64-setup.msi",
            "dist/Nexa-${{ env.NEXA_RELEASE_VERSION }}-windows-x64-setup.exe",
            "dist/windows-msi-build-report.json"])
        self.assertNotIn("*", paths)
        publishing = workflow.split("\n  release:\n", 1)[1]
        self.assertNotIn("UNVERIFIED", publishing)
        self.assertIn("name: nexa-windows-release-${{ github.sha }}", publishing)

    def test_all_workflow_actions_are_sha_pinned_and_release_is_isolated(self):
        text = (release.ROOT / ".github/workflows/native-windows.yml").read_text(encoding="utf-8")
        import re
        for action in re.findall(r"uses: ([^\s]+)", text):
            self.assertRegex(action, r"^[A-Za-z0-9_./-]+@[0-9a-f]{40}$")
        self.assertIn("branches: ['codex/dev', 'main']", text)
        self.assertIn("tags: ['v*']", text)
        self.assertEqual(text.count("contents: write"), 1)
        self.assertNotIn("pull_request_target", text)
        self.assertIn("needs: [release-identity, download-component, native]", text)
        self.assertIn("github.event_name == 'push' && github.ref_type == 'tag'", text)
        self.assertIn("scripts/test_windows_msi_lifecycle.py", text)
