from contextlib import redirect_stdout
import hashlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from stage_ci_evidence import MAX_REPORT_BYTES, catalog_download_report, read_regular, stage
from run_desktop_smoke import DIAGNOSTIC_REPORTS, EXTERNAL_LIBRARY_KEYS


class EvidenceStagingTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.source = self.root / "source"
        self.source.mkdir()
        self.out = self.root / "out"

    def put(self, name, contents):
        file = self.source / name
        file.parent.mkdir(parents=True, exist_ok=True)
        file.write_bytes(contents if isinstance(contents, bytes) else contents.encode("utf-8"))

    def run_stage(self):
        return stage(self.source, self.out, self.root, {"GITHUB_SHA": "a" * 40, "USERPROFILE": r"C:\Users\Private Name"})

    def test_allowlist_omits_model_credentials_data_symbols_and_unknown_logs(self):
        for name in ("model.gguf", "api-token", "data/config.json", "runtime.pdb", "unknown.json", "windows-unreviewed.log"):
            self.put(name, "private data")
        self.put("windows-rust-tests.log", "test result: ok. 1 passed; 0 failed")
        result = self.run_stage()
        self.assertEqual([x["path"] for x in result["files"]], ["windows-rust-tests.log"])
        self.assertEqual(sorted(p.name for p in self.out.iterdir()), ["evidence-index.json", "windows-rust-tests.log"])

    def test_failure_command_parameters_identity_and_hash_chain_preserved(self):
        report = {"result": "failed", "exit_code": 2, "elapsed_ms": 42, "command": {"program": "native-smoke", "args": ["--threads", "2"]}, "model_sha256": "b" * 64, "chat_template_sha256": "c" * 64, "toolchain": "rustc 1.98.1", "path": r"C:\Users\Private Name\模型 目录\qa.gguf"}
        self.put("windows-baseline.json", json.dumps(report))
        self.put("upstream-zh.txt", "synthetic generated response")
        result = self.run_stage()
        output = json.loads((self.out / "windows-baseline.json").read_text(encoding="utf-8"))
        self.assertEqual(output["result"], "failed")
        self.assertEqual(output["command"], report["command"])
        self.assertEqual(output["model_sha256"], report["model_sha256"])
        self.assertNotIn("Private Name", output["path"])
        self.assertEqual(result["hash_only_synthetic_outputs"][0]["sha256"], hashlib.sha256(b"synthetic generated response").hexdigest())
        self.assertFalse((self.out / "upstream-zh.txt").exists())
        self.assertEqual(result["project_commit"], "a" * 40)

    def assert_rejected(self, name):
        result = self.run_stage()
        self.assertEqual(result["result"], "failed")
        self.assertFalse((self.out / name).exists())
        self.assertTrue((self.out / "staging-failure.json").is_file())
        return result

    def test_forbidden_json_bodies_and_secrets_never_uploaded(self):
        self.put("windows-baseline.json", json.dumps({"api_token": "sensitive"}))
        self.put("windows-rust-tests.log", "test result: FAILED. 0 passed; 1 failed")
        result = self.assert_rejected("windows-baseline.json")
        self.assertEqual(result["rejected_reports"][0]["path"], "windows-baseline.json")
        self.assertIn("FAILED", (self.out / "windows-rust-tests.log").read_text(encoding="utf-8"))
        self.assertNotIn("sensitive", (self.out / "staging-failure.json").read_text(encoding="utf-8"))

    def test_aria2_retry_history_survives_staging_and_path_redaction(self):
        # Use the real probe/report classifier; only native execution and delay
        # are replaced. Staging must retain the failed observation after recovery.
        from test_aria2_build_policy_windows import probe, certificate_log, timeout_log
        import subprocess

        name, (url, errors) = next(iter(probe.CERTIFICATE_CASES.items()))
        diagnostic = timeout_log(url) + r"C:\Users\Private Name\work\native.log" + "\n"
        outcomes = [subprocess.CompletedProcess([], 2, diagnostic.encode(), b""),
                    subprocess.CompletedProcess([], 1, certificate_log(url, sorted(errors)[0]).encode(), b"")]
        with patch.object(probe.subprocess, "run", side_effect=outcomes), patch.object(probe.time, "sleep"), \
                redirect_stdout(io.StringIO()) as summary:
            case = probe.run_download_case(Path("aria2.exe"), self.root / "probe", name, url, "certificate")
        self.assertTrue(case["passed"])
        self.assertNotIn("Private Name", summary.getvalue())
        self.assertIn("earlier failures retained", summary.getvalue())
        self.put("windows-aria2-policy.json", json.dumps({"passed": True, "cases": [case]}))
        self.assertEqual(self.run_stage()["result"], "pass")
        staged_text = (self.out / "windows-aria2-policy.json").read_text(encoding="utf-8")
        self.assertNotIn("Private Name", staged_text)
        output = json.loads(staged_text)["cases"][0]
        self.assertTrue(output["passed"])
        self.assertTrue(output["recovered_after_retry"])
        self.assertEqual(output["attempt_count"], 2)
        self.assertEqual([item["passed"] for item in output["attempts"]], [False, True])
        self.assertEqual(output["attempts"][0]["transport_failure"], "network_timeout")
        self.assertEqual(output["attempts"][0]["exit"], 2)
        self.assertIn("Timeout.", output["attempts"][0]["diagnostic"])
        self.assertIn(sorted(errors)[0], output["attempts"][1]["diagnostic"])

    def test_explicit_credential_text_is_not_blindly_scrubbed(self):
        self.put("windows-rust-tests.log", "Authorization: Bearer ABCDEFGHIJKLMNOPQRSTUVWXYZ")
        self.assert_rejected("windows-rust-tests.log")

    def test_binary_magic_rejected_under_allowed_filename(self):
        self.put("windows-rust-tests.log", b"MZhidden bytes")
        self.assert_rejected("windows-rust-tests.log")

    def test_duplicate_json_key_rejected(self):
        self.put("windows-baseline.json", '{"status":"fail","status":"pass"}')
        self.assert_rejected("windows-baseline.json")

    def test_symlink_cannot_smuggle_allowed_report(self):
        outside = self.root / "secret.txt"
        outside.write_text("private", encoding="utf-8")
        try:
            (self.source / "windows-rust-tests.log").symlink_to(outside)
        except OSError:
            self.skipTest("symlink creation is unavailable on this test host")
        self.assert_rejected("windows-rust-tests.log")

    def test_t05_report_and_ci_outcomes_preserved(self):
        self.put("windows-package/package-acceptance.json", json.dumps({"schema_version": 1, "short_package_checks_passed": False, "a20_target_evidence": "unverified", "checks": [{"id": "product_integrity", "status": "fail"}], "environment": {"machine_role_user_declared": "ci"}, "http_cli": {"checks": [{"id": "cancel", "status": "pass"}]}}))
        result = stage(self.source, self.out, self.root, {"NEXA_CI_PACKAGE_ACCEPTANCE": "failure"})
        self.assertEqual(result["result"], "pass")
        self.assertEqual(result["ci_step_outcomes"], {"package_acceptance": "failure"})
        output = json.loads((self.out / "windows-package/package-acceptance.json").read_text(encoding="utf-8"))
        self.assertFalse(output["short_package_checks_passed"])
        self.assertEqual(output["checks"][0]["status"], "fail")

    def test_desktop_structured_failure_preserves_cause_and_cleanup(self):
        report = {
            "schema_version": 1, "kind": "nexa-desktop-bridge-acceptance", "success": False,
            "stage": "launch_initial_child", "code": "bridge_error", "bridge_code": "runtime_start_failed",
            "os_error": 5, "runtime_exit_code": None, "child_stage": "child_start", "child_exit_code": 1,
            "cleanup": {"status": "unconfirmed", "code": "bridge_error", "bridge_code": "connection_failed",
                        "os_error": None, "instance_lock": "held", "discovery": "present", "temporary_data_retained": True},
        }
        self.put("windows-desktop/bridge-failure.json", json.dumps(report))
        result = self.run_stage()
        self.assertEqual(result["result"], "pass")
        output = json.loads((self.out / "windows-desktop/bridge-failure.json").read_text(encoding="utf-8"))
        self.assertEqual(output, report)

    def test_desktop_failure_requires_its_exact_closed_schema(self):
        self.put("windows-desktop/bridge-failure.json", json.dumps({"stage": "private text", "success": False}))
        self.assert_rejected("windows-desktop/bridge-failure.json")

    def test_desktop_startup_failure_preserves_only_closed_diagnostic(self):
        report = {"schema_version": 2, "package_verified": False, "package_error_code": "package_file_hash_mismatch",
                  "project_commit": None, "project_dirty": None, "webview2_version": "131.0.2903.86", "native_window_tested": False}
        self.put("windows-desktop/diagnostics.json", json.dumps(report))
        self.assertEqual(self.run_stage()["result"], "pass")
        self.assertEqual(json.loads((self.out / "windows-desktop/diagnostics.json").read_text(encoding="utf-8")), report)

    def test_desktop_startup_diagnostic_unknown_field_is_rejected(self):
        report = {"schema_version": 2, "package_verified": False, "package_error_code": "package_file_hash_mismatch",
                  "project_commit": None, "project_dirty": None, "webview2_version": None, "native_window_tested": False,
                  "detail": "unreviewed private data"}
        self.put("windows-desktop/diagnostics.json", json.dumps(report))
        self.assert_rejected("windows-desktop/diagnostics.json")

    def test_native_ui_failure_filename_is_never_ci_evidence(self):
        self.put("windows-desktop/bridge-real.json", json.dumps({"success": False, "external_library": {name: True for name in EXTERNAL_LIBRARY_KEYS}, "failed_file_name": "private model name.gguf"}))
        self.assert_rejected("windows-desktop/bridge-real.json")
        self.assertNotIn("private model", (self.out / "staging-failure.json").read_text(encoding="utf-8"))

    def test_external_platform_negative_is_preserved_without_becoming_pass(self):
        report = {"success": True, "external_library": {name: False for name in EXTERNAL_LIBRARY_KEYS}}
        self.put("windows-desktop/bridge-real.json", json.dumps(report))
        self.assertEqual(self.run_stage()["result"], "pass")
        self.assertEqual(json.loads((self.out / "windows-desktop/bridge-real.json").read_text(encoding="utf-8")), report)

    def test_external_unknown_data_fields_are_not_staged(self):
        report = {"success": True, "external_library": {**{name: True for name in EXTERNAL_LIBRARY_KEYS}, "file_name": "private"}}
        self.put("windows-desktop/bridge-real.json", json.dumps(report))
        self.assert_rejected("windows-desktop/bridge-real.json")

    def test_all_fixed_input_diagnostics_are_staged_with_the_same_schema(self):
        report = {"schema_version": 2, "package_verified": True, "package_error_code": None,
                  "project_commit": "a" * 40, "project_dirty": False, "webview2_version": "131.0.2903.86", "native_window_tested": False}
        for name in DIAGNOSTIC_REPORTS:
            self.put("windows-desktop/" + name, json.dumps(report))
        result = self.run_stage()
        self.assertEqual(result["result"], "pass")
        self.assertEqual({item["path"] for item in result["files"]}, {"windows-desktop/" + name for name in DIAGNOSTIC_REPORTS})

    def test_launch_probe_negative_observation_is_preserved(self):
        report = {"schema_version": 2, "kind": "nexa-desktop-launch-probe", "success": False,
                  "code": "signal_registration_failed", "os_error": None, "spawn_os_error": None,
                  "child_exit_code": 0, "signal_state": "error", "signal_os_error": 6, "cleanup_confirmed": True,
                  "strategy": "inherit_job", "parent_in_job": True, "child_in_job": True,
                  "parent_job_os_error": None, "child_job_os_error": None}
        self.put("windows-desktop/launch-probe-inherit-job.json", json.dumps(report))
        self.assertEqual(self.run_stage()["result"], "pass")
        self.assertEqual(json.loads((self.out / "windows-desktop/launch-probe-inherit-job.json").read_text(encoding="utf-8")), report)

    def test_oversized_report_rejected_before_open(self):
        path = self.source / "windows-rust-tests.log"
        with path.open("wb") as file:
            file.truncate(MAX_REPORT_BYTES + 1)
        with patch.object(Path, "open", side_effect=AssertionError("oversized report must not be opened")):
            with self.assertRaisesRegex(ValueError, "bounded size"):
                read_regular(path)

    def test_report_growth_after_stat_has_bounded_read(self):
        path = self.source / "windows-rust-tests.log"
        path.write_bytes(b"small")
        original_open = Path.open
        reads = []

        class TrackedReader:
            def __init__(self, file):
                self.file = file

            def __enter__(self):
                return self

            def __exit__(self, *args):
                self.file.close()

            def read(self, amount):
                data = self.file.read(amount)
                reads.append((amount, len(data)))
                return data

        def grow_then_open(current, mode):
            self.assertEqual(current, path)
            self.assertEqual(mode, "rb")
            # Deterministically simulate another writer growing this same file
            # between the preflight stat and opening it for the actual read.
            with original_open(current, "ab") as writer:
                writer.truncate(4096)
            return TrackedReader(original_open(current, mode))

        with patch("stage_ci_evidence.MAX_REPORT_BYTES", 64), patch.object(Path, "open", grow_then_open):
            with self.assertRaisesRegex(ValueError, "bounded size"):
                read_regular(path)
        self.assertEqual(reads, [(65, 65)])
        self.assertEqual(path.stat().st_size, 4096)

    def test_existing_destination_is_never_merged(self):
        self.out.mkdir()
        (self.out / "stale-secret").write_text("old", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "already exists"):
            self.run_stage()

    def test_catalog_download_report_has_closed_non_sensitive_schema(self):
        report = {"schema_version": 1, "success": True, "source": "modelscope",
                  "catalog_id": "qwen3-0.6b-q8-0", "source_revision": "a" * 40,
                  "size_bytes": 639446688, "sha256": "b" * 64,
                  "downloaded_bytes": 639446688, "published": True,
                  "registered": True, "elapsed_ms": 1234}
        catalog_download_report(report)
        self.put("windows-catalog-download.json", json.dumps(report))
        result = self.run_stage()
        self.assertEqual(result["result"], "pass")
        self.assertEqual(json.loads((self.out / "windows-catalog-download.json").read_text(encoding="utf-8")), report)
        for changes in ({"path": "private"}, {"url": "https://private.invalid"},
                        {"source": "huggingface"}, {"catalog_id": "other"},
                        {"schema_version": True}, {"size_bytes": True},
                        {"downloaded_bytes": 1}, {"published": False},
                        {"registered": False}, {"sha256": "invalid"},
                        {"elapsed_ms": -1}, {"source_revision": {}}):
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                catalog_download_report(report | changes)

    def test_catalog_failure_keeps_only_fixed_error(self):
        report = {"schema_version": 1, "success": False, "source": "modelscope",
                  "catalog_id": "qwen3-0.6b-q8-0", "error": "catalog_download_verification_failed"}
        catalog_download_report(report)
        catalog_download_report(report | {"stage": "download_poll", "code": "model_download_timeout"})
        for changes in ({"error": "private exception body"}, {"url": "private"}, {"success": 0},
                        {"stage": "download_poll"}, {"stage": "download_poll", "code": "https://private.invalid"},
                        {"stage": {}, "code": "verification_failed"}):
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                catalog_download_report(report | changes)


if __name__ == "__main__":
    unittest.main()
