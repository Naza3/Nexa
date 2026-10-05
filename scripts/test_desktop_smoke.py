import json
from pathlib import Path
import subprocess
import sys
import time
import re
import tempfile
import unittest
from unittest import mock

import run_desktop_smoke as smoke


class DesktopSmokeFailureTests(unittest.TestCase):
    def test_onboarding_requires_new_exact_pass_observations(self):
        report = {"success": True, "local_text_validation": True, "repeat_text_validation": True, "offline_inventory": True, "manual_load_stop": True, "reload_after_stop": True}
        self.assertEqual(smoke.onboarding_report(report), report)
        for field in report:
            for value in (False, None, 1, "true"):
                with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                    smoke.onboarding_report(report | {field: value})
            with self.assertRaises(ValueError):
                smoke.onboarding_report({key: value for key, value in report.items() if key != field})

    def test_external_acceptance_requires_exact_booleans_and_all_windows_checks(self):
        report = {name: True for name in smoke.EXTERNAL_LIBRARY_KEYS}
        self.assertEqual(smoke.external_library_report(report, require_windows=True), report)
        for key in smoke.EXTERNAL_LIBRARY_KEYS:
            unsupported = {**report, key: False}
            self.assertEqual(smoke.external_library_report(unsupported), unsupported)
            with self.assertRaisesRegex(ValueError, "incomplete"):
                smoke.external_library_report(unsupported, require_windows=True)
            for value in (1, "true", None):
                with self.assertRaisesRegex(ValueError, "schema rejected"):
                    smoke.external_library_report({**report, key: value})
        for value in (None, {}, {"success": True}, {**report, "failed_file_name": "private"}, {**report, "extra": True}):
            with self.assertRaisesRegex(ValueError, "schema rejected"):
                smoke.external_library_report(value)

    def test_failure_protocol_enums_match_rust_source(self):
        source = (smoke.desktop.ROOT / "crates/desktop-bridge/src/bin/harness/report.rs").read_text(encoding="utf-8")
        for name, expected in (("STAGES", smoke.FAILURE_STAGES), ("CODES", smoke.FAILURE_CODES), ("BRIDGE_CODES", smoke.BRIDGE_CODES),
                               ("CLEANUP_STATUSES", smoke.CLEANUP_STATUSES), ("LOCK_STATES", smoke.LOCK_STATES),
                               ("DISCOVERY_STATES", smoke.DISCOVERY_STATES), ("PROBE_CODES", smoke.PROBE_CODES),
                               ("SIGNAL_STATES", smoke.SIGNAL_STATES), ("FAILURE_KEYS", smoke.FAILURE_KEYS), ("CLEANUP_KEYS", smoke.CLEANUP_KEYS),
                               ("PROBE_KEYS", smoke.PROBE_KEYS), ("LAUNCH_STRATEGIES", smoke.LAUNCH_STRATEGIES)):
            block = re.search(r"pub const " + name + r": &\[&str\] = &\[(.*?)\];", source, re.DOTALL)
            self.assertIsNotNone(block, name)
            self.assertEqual(set(re.findall(r'"([a-z_]+)"', block[1])), expected)
        self.assertIn(f"pub const MAX_REPORT_BYTES: usize = {smoke.MAX_FAILURE_BYTES};", source)

    def failure(self):
        return {
            "schema_version": 1,
            "kind": "nexa-desktop-bridge-acceptance",
            "success": False,
            "stage": sorted(smoke.FAILURE_STAGES)[0],
            "code": sorted(smoke.FAILURE_CODES)[0],
            "bridge_code": None,
            "os_error": 5,
            "runtime_exit_code": 0,
            "child_stage": None,
            "child_exit_code": 1,
            "cleanup": {
                "status": "unconfirmed", "code": None, "bridge_code": None,
                "os_error": None, "instance_lock": "held", "discovery": "present",
                "temporary_data_retained": True,
            },
        }

    def call(self, result, failure_file, phase="bridge_harness"):
        with mock.patch.object(smoke, "bounded_process", return_value=result):
            return smoke.checked_json(["fixed-executable"], Path("."), {}, 10,
                                      phase=phase, failure_file=failure_file)

    def test_nonzero_valid_failure_is_saved_and_remains_failure(self):
        report = self.failure()
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "bridge-failure.json"
            result = subprocess.CompletedProcess([], 1, json.dumps(report), "private stderr")
            with self.assertRaisesRegex(ValueError, "bridge_harness process failed: exit 1"):
                self.call(result, path)
            self.assertEqual(json.loads(path.read_text(encoding="utf-8")), report)

    def test_success_has_no_failure_evidence(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "bridge-failure.json"
            result = subprocess.CompletedProcess([], 0, '{"success":true}', "")
            self.assertEqual(self.call(result, path), {"success": True})
            self.assertFalse(path.exists())

    def test_unknown_sensitive_and_wrong_typed_failure_fields_are_rejected(self):
        changes = [
            ("success", True), ("schema_version", True), ("schema_version", 2),
            ("stage", "unknown"), ("code", "unknown"), ("bridge_code", "Bearer PRIVATE"),
            ("os_error", True), ("os_error", 2 ** 31), ("os_error", -(2 ** 31) - 1),
            ("runtime_exit_code", True), ("runtime_exit_code", 2 ** 31),
            ("child_stage", "unknown"), ("child_exit_code", "1"),
            ("message", "private error"), ("token", "private credential"),
            ("path", r"C:\Users\Private\file"), ("cleanup", []),
        ]
        reports = []
        for field, value in changes:
            report = self.failure()
            report[field] = value
            reports.append(report)
        for field, value in [("status", "unknown"), ("code", "private"),
                             ("os_error", True), ("instance_lock", "unknown"),
                             ("discovery", "unknown"), ("temporary_data_retained", 1),
                             ("messages", ["private body"])]:
            report = self.failure()
            report["cleanup"][field] = value
            reports.append(report)
        report = self.failure()
        del report["code"]
        reports.append(report)
        for report in reports:
            with self.subTest(report_fields=sorted(report)), tempfile.TemporaryDirectory() as temporary:
                path = Path(temporary) / "bridge-failure.json"
                with self.assertRaisesRegex(ValueError, "structured failure report rejected") as error:
                    self.call(subprocess.CompletedProcess([], 1, json.dumps(report), "private stderr"), path)
                self.assertNotIn("private", str(error.exception).lower())
                self.assertFalse(path.exists())

    def test_oversized_duplicate_nonfinite_and_malformed_failure_rejected(self):
        valid = json.dumps(self.failure())
        invalid = [valid + " " * smoke.MAX_FAILURE_BYTES, valid[:-1] + ',"success":false}',
                   valid.replace('"os_error": 5', '"os_error": NaN'), "{private", "[]", "null", "", "[" * 1500 + "]" * 1500]
        for raw in invalid:
            with self.subTest(length=len(raw)), tempfile.TemporaryDirectory() as temporary:
                path = Path(temporary) / "bridge-failure.json"
                with self.assertRaisesRegex(ValueError, "structured failure report rejected"):
                    self.call(subprocess.CompletedProcess([], 1, raw, "private stderr"), path)
                self.assertFalse(path.exists())

    def test_diagnostic_failure_is_separate_and_never_writes_bridge_report(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "bridge-failure.json"
            with self.assertRaisesRegex(ValueError, "desktop_diagnose structured diagnostic report rejected"):
                self.call(subprocess.CompletedProcess([], 1, "private stdout", "private stderr"), path, "desktop_diagnose")
            self.assertFalse(path.exists())

    def diagnostic(self, verified=False):
        return {"schema_version": 2, "package_verified": verified,
                "package_error_code": None if verified else "package_unlisted_file",
                "project_commit": "a" * 40 if verified else None,
                "project_dirty": False if verified else None,
                "webview2_version": "131.0.2903.86", "native_window_tested": False}

    def test_diagnostic_code_whitelist_matches_native_and_covers_validation_errors(self):
        root = smoke.desktop.ROOT / "apps/desktop/src-tauri/src"
        source = (root / "diagnostics.rs").read_text(encoding="utf-8")
        block = re.search(r"pub const PACKAGE_ERROR_CODES: &\[&str\] = &\[(.*?)\];", source, re.DOTALL)
        self.assertIsNotNone(block)
        self.assertEqual(set(re.findall(r'"([a-z_]+)"', block[1])), smoke.PACKAGE_ERROR_CODES)
        for name in ("layout.rs", "selection.rs"):
            emitted = set(re.findall(r'(?:Err|ok_or)\("([a-z_]+)"\)|map_err\(\|_\| "([a-z_]+)"\)',
                                     (root / name).read_text(encoding="utf-8")))
            emitted = {code for pair in emitted for code in pair if code}
            # Selection-only UI codes cannot reach layout validation.
            emitted -= {"selected_file_not_gguf", "selected_file_name_invalid", "selection_expired"}
            self.assertTrue(emitted <= smoke.PACKAGE_ERROR_CODES, emitted - smoke.PACKAGE_ERROR_CODES)

    def test_valid_nonzero_diagnostic_saved_without_overwriting_bridge_failure(self):
        report = self.diagnostic()
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "diagnostics.json"
            bridge_path = Path(temporary) / "bridge-failure.json"
            with mock.patch.object(smoke, "bounded_process", return_value=subprocess.CompletedProcess([], 1, json.dumps(report))):
                with self.assertRaisesRegex(ValueError, "desktop_diagnose process failed: exit 1"):
                    smoke.checked_json([], Path("."), {}, 1, phase="desktop_diagnose", diagnostic_file=path, failure_file=bridge_path)
            self.assertEqual(json.loads(path.read_text(encoding="utf-8")), report)
            self.assertFalse(bridge_path.exists())

    def test_diagnostic_strict_schema_types_and_sensitive_data(self):
        changes = [("schema_version", True), ("schema_version", 1), ("package_verified", 1),
                   ("package_error_code", "private path"), ("project_commit", "a" * 40),
                   ("project_dirty", False), ("native_window_tested", True),
                   ("webview2_version", "Bearer PRIVATE"), ("path", "private"), ("message", "private")]
        raws = []
        for field, value in changes:
            report = self.diagnostic()
            report[field] = value
            raws.append(json.dumps(report))
        raw = json.dumps(self.diagnostic())
        raws.extend([raw + " " * smoke.MAX_FAILURE_BYTES, raw[:-1] + ',"schema_version":2}', "{}", "[]"])
        for raw in raws:
            with self.subTest(length=len(raw)), tempfile.TemporaryDirectory() as temporary:
                path = Path(temporary) / "diagnostics.json"
                with mock.patch.object(smoke, "bounded_process", return_value=subprocess.CompletedProcess([], 1, raw)):
                    with self.assertRaisesRegex(ValueError, "structured diagnostic report rejected") as error:
                        smoke.checked_json([], Path("."), {}, 1, phase="desktop_diagnose", diagnostic_file=path)
                self.assertFalse(path.exists())
                self.assertNotIn("private", str(error.exception))

    def test_diagnostic_success_and_missing_webview_remain_distinct(self):
        report = self.diagnostic(True)
        self.assertEqual(smoke.startup_diagnostic(json.dumps(report)), report)
        with mock.patch.object(smoke, "bounded_process", return_value=subprocess.CompletedProcess([], 0, json.dumps(report))):
            self.assertEqual(smoke.checked_json([], Path("."), {}, 1, phase="desktop_diagnose"), report)
        report["webview2_version"] = None
        with mock.patch.object(smoke, "bounded_process", return_value=subprocess.CompletedProcess([], 0, json.dumps(report))):
            with self.assertRaisesRegex(ValueError, "success declaration rejected"):
                smoke.checked_json([], Path("."), {}, 1, phase="desktop_diagnose")

    def test_input_compatibility_checks_create_owned_inputs_and_restore_payload(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            package = root / "package"
            package.mkdir()
            evidence = root / "evidence"
            evidence.mkdir()
            model = root / "source.gguf"
            model.write_bytes(b"GGUFsynthetic model")
            original = b'{"synthetic":"manifest"}'
            (package / "manifest.json").write_bytes(original)
            checks = {}

            def positive(*args, **kwargs):
                paths = [package / "验收 同级外置模型.GGUF", package / "model/验收 中文 模型.GGUF", package / "models/验收 中文 模型.GGUF"]
                existing = [path for path in paths if path.exists()]
                self.assertEqual(len(existing), 1)
                self.assertEqual(existing[0].read_bytes(), model.read_bytes())
                self.assertIn(kwargs["diagnostic_file"].name, smoke.DIAGNOSTIC_REPORTS)
                return self.diagnostic(True)

            def negative(executable, cwd, env, code, output):
                self.assertTrue((package / "验收 同级外置模型.GGUF").exists())
                if code == "package_unlisted_file":
                    self.assertTrue((package / "nexa-test-unlisted.dll").exists())
                else:
                    self.assertEqual(code, "package_checksum_mismatch")
                    self.assertEqual((package / "manifest.json").read_bytes(), original + b"\n ")

            with mock.patch.object(smoke, "checked_json", side_effect=positive), mock.patch.object(smoke, "expect_package_rejection", side_effect=negative), mock.patch.object(smoke.desktop, "verify") as verify:
                smoke.package_input_checks(package, model, root, {}, evidence, {"project_commit": "a" * 40, "project_dirty": False}, checks)
            self.assertEqual(set(checks), {"root_gguf_accepted", "model_directory_gguf_accepted", "models_directory_gguf_accepted", "unlisted_dll_rejected", "tampered_manifest_rejected", "owned_model_input_removed", "package_payload_restored"})
            self.assertTrue(all(checks.values()))
            self.assertEqual((package / "manifest.json").read_bytes(), original)
            self.assertEqual(set(item.name for item in package.iterdir()), {"manifest.json"})
            self.assertTrue(model.exists())
            verify.assert_called_once()

    def test_input_probe_never_overwrites_or_removes_preexisting_file(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            existing = root / "验收 同级外置模型.GGUF"
            existing.write_bytes(b"GGUFexisting user input")
            with self.assertRaises(FileExistsError):
                smoke.package_input_checks(root, root / "unused", root, {}, root, {}, {})
            self.assertEqual(existing.read_bytes(), b"GGUFexisting user input")

    def test_input_probe_restores_manifest_on_unexpected_negative_result(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            model = root / "source.gguf"
            model.write_bytes(b"GGUFsynthetic")
            original = b"original manifest bytes"
            (root / "manifest.json").write_bytes(original)
            with mock.patch.object(smoke, "checked_json", return_value=self.diagnostic(True)), mock.patch.object(smoke, "expect_package_rejection", side_effect=[None, ValueError("synthetic probe failure")]):
                with self.assertRaisesRegex(ValueError, "synthetic probe failure"):
                    smoke.package_input_checks(root, model, root, {}, root, {"project_commit": "a" * 40, "project_dirty": False}, {})
            self.assertEqual((root / "manifest.json").read_bytes(), original)
            self.assertFalse((root / "验收 同级外置模型.GGUF").exists())
            self.assertFalse((root / "nexa-test-unlisted.dll").exists())
            self.assertTrue(model.exists())

    def test_model_directory_probe_failure_removes_only_its_owned_input(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            model = root / "source.gguf"
            model.write_bytes(b"GGUFsynthetic")
            manifest = {"project_commit": "a" * 40, "project_dirty": False}
            with mock.patch.object(smoke, "checked_json", side_effect=[self.diagnostic(True), ValueError("synthetic directory diagnostic failure")]):
                with self.assertRaisesRegex(ValueError, "directory diagnostic failure"):
                    smoke.package_input_checks(root, model, root, {}, root, manifest, {})
            self.assertEqual({entry.name for entry in root.iterdir()}, {"source.gguf"})
            self.assertEqual(model.read_bytes(), b"GGUFsynthetic")

    def test_input_probe_does_not_adopt_preexisting_model_directory(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            model = root / "source.gguf"
            model.write_bytes(b"GGUFsynthetic")
            directory = root / "model"
            directory.mkdir()
            existing = directory / "already present.gguf"
            existing.write_bytes(b"GGUFexisting")
            with mock.patch.object(smoke, "checked_json", return_value=self.diagnostic(True)):
                with self.assertRaises(FileExistsError):
                    smoke.package_input_checks(root, model, root, {}, root, {"project_commit": "a" * 40, "project_dirty": False}, {})
            self.assertEqual(existing.read_bytes(), b"GGUFexisting")
            self.assertFalse((root / "验收 同级外置模型.GGUF").exists())

    def test_negative_package_probe_requires_exit_one_and_exact_code(self):
        for code, exit_code, accepted in (("package_unlisted_file", 1, True), ("package_unlisted_file", 0, False), ("package_checksum_mismatch", 1, False)):
            with self.subTest(code=code, exit_code=exit_code), tempfile.TemporaryDirectory() as temporary:
                report = self.diagnostic()
                report["package_error_code"] = code
                path = Path(temporary) / "diagnostic.json"
                with mock.patch.object(smoke, "bounded_process", return_value=subprocess.CompletedProcess([], exit_code, json.dumps(report))):
                    if accepted:
                        smoke.expect_package_rejection("fixed", Path("."), {}, "package_unlisted_file", path)
                    else:
                        with self.assertRaisesRegex(ValueError, "did not reject the expected condition"):
                            smoke.expect_package_rejection("fixed", Path("."), {}, "package_unlisted_file", path)
                self.assertEqual(json.loads(path.read_text(encoding="utf-8")), report)

    def test_timeout_launch_and_decode_errors_are_phase_labeled_without_raw_output(self):
        failures = [subprocess.TimeoutExpired(["private path"], 10, output="private output"),
                    OSError("private path"), UnicodeDecodeError("utf-8", b"\xff", 0, 1, "private output")]
        for phase in ("desktop_diagnose", "bridge_harness"):
            for failure in failures:
                with self.subTest(phase=phase, error_type=type(failure).__name__), mock.patch.object(smoke, "bounded_process", side_effect=failure):
                    with self.assertRaisesRegex(ValueError, phase) as error:
                        smoke.checked_json(["fixed-executable"], Path("."), {}, 10, phase=phase)
                    self.assertNotIn("private", str(error.exception))

    def test_real_child_exit_does_not_wait_for_descendant_stdout_writer(self):
        # The grandchild intentionally inherits the regular stdout handle and
        # exits by itself after this assertion's bound. It creates no runtime.
        script = ("import subprocess,sys; "
                  "subprocess.Popen([sys.executable,'-c','import time;time.sleep(3)'], "
                  "stdin=subprocess.DEVNULL,stdout=sys.stdout,stderr=subprocess.DEVNULL,close_fds=True); "
                  "print('{\"success\":true}',flush=True)")
        started = time.monotonic()
        observed = smoke.checked_json([sys.executable, "-c", script], Path.cwd(), None, 5, phase="bridge_harness")
        self.assertEqual(observed, {"success": True})
        self.assertLess(time.monotonic() - started, 2.5)

    def test_real_timeout_does_not_drain_descendant_stdout_writer(self):
        script = ("import subprocess,sys,time; "
                  "subprocess.Popen([sys.executable,'-c','import time;time.sleep(3)'], "
                  "stdin=subprocess.DEVNULL,stdout=sys.stdout,stderr=subprocess.DEVNULL,close_fds=True); "
                  "print('{\"success\":true}',flush=True);time.sleep(10)")
        started = time.monotonic()
        with self.assertRaisesRegex(ValueError, "desktop_diagnose timed out; cleanup not confirmed"):
            smoke.checked_json([sys.executable, "-c", script], Path.cwd(), None, 0.5, phase="desktop_diagnose")
        self.assertLess(time.monotonic() - started, 2.5)

    def test_timeout_targets_owned_child_and_has_finite_reap_wait(self):
        process = mock.Mock()
        process.wait.side_effect = [subprocess.TimeoutExpired("private", 2), subprocess.TimeoutExpired("private", 1)]
        with mock.patch.object(smoke.subprocess, "Popen", return_value=process) as popen:
            with self.assertRaisesRegex(ValueError, "owned child exit unconfirmed; cleanup not confirmed"):
                smoke.checked_json(["fixed-harness"], Path("."), {}, 2, phase="bridge_harness")
        process.kill.assert_called_once_with()
        self.assertEqual(process.wait.call_args_list, [mock.call(timeout=2), mock.call(timeout=1)])
        self.assertEqual(popen.call_args.kwargs["stdin"], subprocess.DEVNULL)
        self.assertEqual(popen.call_args.kwargs["stderr"], subprocess.DEVNULL)
        self.assertTrue(popen.call_args.kwargs["close_fds"])
        self.assertNotEqual(popen.call_args.kwargs["stdout"], subprocess.PIPE)

    def test_regular_file_capture_rejects_oversize_without_unbounded_read(self):
        class FakeProcess:
            returncode = 0

            def wait(self, timeout):
                return self.returncode

        def spawn(*args, **kwargs):
            kwargs["stdout"].write(b"x" * 65)
            return FakeProcess()

        with mock.patch.object(smoke, "MAX_JSON_BYTES", 64), mock.patch.object(smoke.subprocess, "Popen", side_effect=spawn):
            with self.assertRaisesRegex(ValueError, "desktop_diagnose report exceeded bounded size"):
                smoke.checked_json(["fixed-harness"], Path("."), {}, 1, phase="desktop_diagnose")

    def test_numeric_limits_and_known_optional_values_are_preserved(self):
        report = self.failure()
        report["bridge_code"] = sorted(smoke.BRIDGE_CODES)[0]
        report["child_stage"] = sorted(smoke.FAILURE_STAGES)[0]
        report["os_error"] = -(2 ** 31)
        report["child_exit_code"] = 2 ** 31 - 1
        report["cleanup"].update(code=sorted(smoke.FAILURE_CODES)[0], bridge_code=report["bridge_code"], os_error=5)
        self.assertEqual(smoke.bridge_failure(json.dumps(report)), report)

    def test_success_report_must_be_strict_bounded_json(self):
        for raw in ("[]", "{private", '{"a":1,"a":2}', '{"a":NaN}', " " * (smoke.MAX_JSON_BYTES + 1)):
            with self.subTest(length=len(raw)), self.assertRaisesRegex(ValueError, "success report rejected"):
                self.call(subprocess.CompletedProcess([], 0, raw, ""), None, "bridge_harness")

    def probe(self):
        return {"schema_version": 2, "kind": "nexa-desktop-launch-probe", "success": False,
                "code": "spawn_failed", "os_error": None, "spawn_os_error": 5,
                "child_exit_code": None, "signal_state": "not_observed", "signal_os_error": None,
                "cleanup_confirmed": True, "strategy": "breakaway", "parent_in_job": True,
                "child_in_job": None, "parent_job_os_error": None, "child_job_os_error": None}

    def test_launch_negative_observation_is_saved_without_claiming_product_pass(self):
        report = self.probe()
        with tempfile.TemporaryDirectory() as temporary, mock.patch.object(smoke, "bounded_process", return_value=subprocess.CompletedProcess([], 0, json.dumps(report), "")):
            smoke.run_launch_probe(Path("fixed-harness"), Path(temporary))
            saved = json.loads((Path(temporary) / "launch-probe-breakaway.json").read_text(encoding="utf-8"))
            self.assertEqual(saved, report)
            self.assertFalse(saved["success"])
            self.assertFalse((Path(temporary) / "acceptance.json").exists())

    def test_launch_report_sensitive_unknown_numeric_and_false_success_are_rejected(self):
        for field, value in [("stage", "private"), ("message", "private"), ("code", "private"),
                             ("signal_state", []), ("success", True), ("success", 1),
                             ("os_error", True), ("spawn_os_error", 2 ** 31),
                             ("child_exit_code", -2 ** 31 - 1), ("signal_os_error", 0.5),
                             ("cleanup_confirmed", "true"), ("schema_version", 1), ("strategy", "unknown"),
                             ("parent_in_job", 1), ("child_in_job", "true"), ("parent_job_os_error", 5),
                             ("child_job_os_error", 2 ** 31)]:
            report = self.probe()
            report[field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                smoke.launch_probe_report(json.dumps(report))
        with self.assertRaises(ValueError):
            smoke.launch_probe_report(json.dumps(self.probe()) + " " * smoke.MAX_FAILURE_BYTES)
        with self.assertRaises(ValueError):
            smoke.launch_probe_report('{"success":true,"success":false}')

    def test_launch_pending_observation_requires_confirmed_clean_child_exit(self):
        report = self.probe()
        report.update(success=True, code="observed_pending", spawn_os_error=None, child_exit_code=0, signal_state="pending", child_in_job=False)
        self.assertEqual(smoke.launch_probe_report(json.dumps(report)), report)
        report["cleanup_confirmed"] = False
        with self.assertRaises(ValueError):
            smoke.launch_probe_report(json.dumps(report))

    def test_launch_strategy_cannot_be_mixed_or_silently_fallback(self):
        report = self.probe()
        with tempfile.TemporaryDirectory() as temporary, mock.patch.object(smoke, "bounded_process", return_value=subprocess.CompletedProcess([], 0, json.dumps(report), "")) as run:
            with self.assertRaisesRegex(ValueError, "schema rejected"):
                smoke.run_launch_probe(Path("fixed-harness"), Path(temporary), "inherit_job")
            self.assertEqual(run.call_args.args[0], ["fixed-harness", "--probe-launch", "inherit_job"])
            self.assertEqual(list(Path(temporary).iterdir()), [])

    def test_launch_artifact_precedes_tauri_without_replacing_final_gate(self):
        workflow = (smoke.desktop.ROOT / ".github/workflows/native-windows.yml").read_text(encoding="utf-8")
        self.assertLess(workflow.index("name: Preserve early desktop launch observation"),
                        workflow.index("name: Check independent desktop Rust graph and build actual Tauri Release"))
        self.assertIn("name: nexa-desktop-launch-probe-${{ github.sha }}", workflow)
        self.assertIn("artifacts/verification/windows-desktop/launch-probe-breakaway.json", workflow)
        self.assertIn("artifacts/verification/windows-desktop/launch-probe-inherit-job.json", workflow)
        self.assertIn("success() && steps.desktop_acceptance.outcome == 'success'", workflow)


if __name__ == "__main__":
    unittest.main()
