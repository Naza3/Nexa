import json
from pathlib import Path
import subprocess
import re
import tempfile
import unittest
from unittest import mock

import run_desktop_smoke as smoke


class DesktopSmokeFailureTests(unittest.TestCase):
    def test_failure_protocol_enums_match_rust_source(self):
        source = (smoke.desktop.ROOT / "crates/desktop-bridge/src/bin/harness/report.rs").read_text(encoding="utf-8")
        for name, expected in (("STAGES", smoke.FAILURE_STAGES), ("CODES", smoke.FAILURE_CODES), ("BRIDGE_CODES", smoke.BRIDGE_CODES),
                               ("CLEANUP_STATUSES", smoke.CLEANUP_STATUSES), ("LOCK_STATES", smoke.LOCK_STATES),
                               ("DISCOVERY_STATES", smoke.DISCOVERY_STATES), ("PROBE_CODES", smoke.PROBE_CODES),
                               ("SIGNAL_STATES", smoke.SIGNAL_STATES), ("FAILURE_KEYS", smoke.FAILURE_KEYS), ("CLEANUP_KEYS", smoke.CLEANUP_KEYS)):
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
        with mock.patch.object(smoke.subprocess, "run", return_value=result) as run:
            try:
                return smoke.checked_json(["fixed-executable"], Path("."), {}, 10,
                                          phase=phase, failure_file=failure_file)
            finally:
                self.assertEqual(run.call_args.kwargs["stderr"], subprocess.DEVNULL)
                self.assertFalse(run.call_args.kwargs["check"])

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
            with self.assertRaisesRegex(ValueError, "desktop_diagnose process failed: exit 1"):
                self.call(subprocess.CompletedProcess([], 1, "private stdout", "private stderr"), path, "desktop_diagnose")
            self.assertFalse(path.exists())

    def test_timeout_launch_and_decode_errors_are_phase_labeled_without_raw_output(self):
        failures = [subprocess.TimeoutExpired(["private path"], 10, output="private output"),
                    OSError("private path"), UnicodeDecodeError("utf-8", b"\xff", 0, 1, "private output")]
        for phase in ("desktop_diagnose", "bridge_harness"):
            for failure in failures:
                with self.subTest(phase=phase, error_type=type(failure).__name__), mock.patch.object(smoke.subprocess, "run", side_effect=failure):
                    with self.assertRaisesRegex(ValueError, phase) as error:
                        smoke.checked_json(["fixed-executable"], Path("."), {}, 10, phase=phase)
                    self.assertNotIn("private", str(error.exception))

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
                self.call(subprocess.CompletedProcess([], 0, raw, ""), None, "desktop_diagnose")

    def probe(self):
        return {"schema_version": 1, "kind": "nexa-desktop-launch-probe", "success": False,
                "code": "spawn_failed", "os_error": None, "spawn_os_error": 5,
                "child_exit_code": None, "signal_state": "not_observed", "signal_os_error": None,
                "cleanup_confirmed": True}

    def test_launch_negative_observation_is_saved_without_claiming_product_pass(self):
        report = self.probe()
        with tempfile.TemporaryDirectory() as temporary, mock.patch.object(smoke.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, json.dumps(report), "")):
            smoke.run_launch_probe(Path("fixed-harness"), Path(temporary))
            saved = json.loads((Path(temporary) / "launch-probe.json").read_text(encoding="utf-8"))
            self.assertEqual(saved, report)
            self.assertFalse(saved["success"])
            self.assertFalse((Path(temporary) / "acceptance.json").exists())

    def test_launch_report_sensitive_unknown_numeric_and_false_success_are_rejected(self):
        for field, value in [("stage", "private"), ("message", "private"), ("code", "private"),
                             ("signal_state", []), ("success", True), ("success", 1),
                             ("os_error", True), ("spawn_os_error", 2 ** 31),
                             ("child_exit_code", -2 ** 31 - 1), ("signal_os_error", 0.5),
                             ("cleanup_confirmed", "true")]:
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
        report.update(success=True, code="observed_pending", spawn_os_error=None, child_exit_code=0, signal_state="pending")
        self.assertEqual(smoke.launch_probe_report(json.dumps(report)), report)
        report["cleanup_confirmed"] = False
        with self.assertRaises(ValueError):
            smoke.launch_probe_report(json.dumps(report))

    def test_launch_artifact_precedes_tauri_without_replacing_final_gate(self):
        workflow = (smoke.desktop.ROOT / ".github/workflows/native-windows.yml").read_text(encoding="utf-8")
        self.assertLess(workflow.index("name: Preserve early private desktop launch observation"),
                        workflow.index("name: Check independent desktop Rust graph and build actual Tauri Release"))
        self.assertIn("name: nexa-desktop-launch-probe-${{ github.sha }}", workflow)
        self.assertIn("path: artifacts/verification/windows-desktop/launch-probe.json", workflow)
        self.assertIn("success() && steps.desktop_acceptance.outcome == 'success'", workflow)


if __name__ == "__main__":
    unittest.main()
