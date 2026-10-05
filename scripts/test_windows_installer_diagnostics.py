"""Failure evidence is a closed, bounded data contract, never raw logs or UI text."""
from contextlib import redirect_stdout
import copy
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

import windows_installer_diagnostics as diag
import test_windows_msi_lifecycle as lifecycle


class InstallerDiagnosticsTests(unittest.TestCase):
    def document(self, root):
        with redirect_stdout(io.StringIO()):
            trace = diag.Trace(root / "diagnostics.json", "a" * 40, "0.1.0")
            trace.event("msi_default_ui_install", "start", expected=(0,))
        return trace.document

    def test_failure_round_trip_is_explicit_not_success(self):
        with tempfile.TemporaryDirectory() as temporary, redirect_stdout(io.StringIO()):
            root = Path(temporary)
            trace = diag.Trace(root / "diagnostics.json", "a" * 40, "0.1.0")
            trace.event("setup_uninstall_absent", "timeout", expected=(1605,))
            trace.finish(False)
            actual = json.loads((root / "diagnostics.json").read_text(encoding="utf-8"))
            self.assertEqual(diag.validate_document(actual, "a" * 40)["status"], "failed")
            self.assertEqual(actual["last_stage"], "setup_uninstall_absent")
            self.assertEqual(actual["events"][-1]["state"], "timeout")

    def test_log_filter_retains_codes_and_drops_paths_and_free_text(self):
        source = ("Action start 09:01:02: NexaGuard.\r\n"
                  "Action start 09:01:02: SECRET_PAYLOAD.\r\n"
                  "Property: token = VERY_PRIVATE_TOKEN\r\nPath C:\\Users\\PrivateName\\secret.gguf\r\n"
                  "Nexa must be installed in the current user's LocalAppData\\Programs\\Nexa directory.\r\n"
                  "Action ended 09:01:03: NexaGuard. Return value 3.\r\nError 1603.\r\n")
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "installer.log"
            for encoding in ("utf-8", "utf-16"):
                path.write_text(source, encoding=encoding)
                result = diag.summarize_log(path)
                self.assertEqual(result["actions"], [{"action": "NexaGuard", "phase": "start", "return_code": None}, {"action": "NexaGuard", "phase": "end", "return_code": 3}])
                self.assertEqual(result["error_codes"], [1603])
                self.assertIn("guard_install_root", result["reason_codes"])
                serialized = json.dumps(result)
                for forbidden in ("SECRET_PAYLOAD", "VERY_PRIVATE_TOKEN", "PrivateName", "secret.gguf", "C:"):
                    self.assertNotIn(forbidden, serialized)

    def test_unknown_and_oversized_fields_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            original = self.document(Path(temporary))
            mutations = [lambda d: d.update(raw_log="secret"), lambda d: d.update(project_commit="b" * 40),
                         lambda d: d.update(last_stage="C:\\Users\\secret"),
                         lambda d: d["events"][0].update(window_title="secret"),
                         lambda d: d["events"][0].update(elapsed_ms=True),
                         lambda d: d["events"][0]["msi_log"].update(reason_codes=["secret"]),
                         lambda d: d["events"][0]["msi_log"].update(actions=[{"action": "secret", "phase": "start", "return_code": None}]),
                         lambda d: d.update(events=d["events"] * 161)]
            for mutate in mutations:
                candidate = copy.deepcopy(original); mutate(candidate)
                with self.assertRaises(ValueError): diag.validate_document(candidate, "a" * 40)

    def test_public_stager_missing_and_atomic_failure(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            command = [sys.executable, str(Path(diag.__file__)), "--input", str(root / "input.json"), "--output", str(root / "output"), "--commit", "a" * 40]
            subprocess.run(command, check=True, capture_output=True)
            missing = json.loads((root / "output/windows-msi-diagnostics.json").read_text(encoding="utf-8"))
            self.assertEqual(missing["installer_diagnostics"], "missing")
            import shutil
            shutil.rmtree(root / "output")
            for payload in ('{"schema_version":1,"schema_version":2}', '{"secret":"user path"}', '{"secret":NaN}'):
                (root / "input.json").write_text(payload, encoding="utf-8")
                result = subprocess.run(command, capture_output=True)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse((root / "output").exists())

    def test_stager_rejects_linked_input_and_output_ancestor(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            actual = root / "actual"; actual.mkdir()
            try: (root / "linked").symlink_to(actual, target_is_directory=True)
            except OSError: self.skipTest("host cannot create test symlinks")
            command = [sys.executable, str(Path(diag.__file__)), "--input", str(root / "missing.json"), "--output", str(root / "linked/output"), "--commit", "a" * 40]
            result = subprocess.run(command, capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse((actual / "output").exists())

    def test_timeout_is_stage_specific_and_does_not_kill_installer(self):
        with tempfile.TemporaryDirectory() as temporary, redirect_stdout(io.StringIO()):
            trace = diag.Trace(Path(temporary) / "diagnostics.json", "a" * 40, "0.1.0")
            process = mock.Mock(pid=1234)
            process.wait.side_effect = subprocess.TimeoutExpired("msiexec", 240)
            with mock.patch.object(lifecycle, "_TRACE", trace), mock.patch.object(lifecycle.subprocess, "Popen", return_value=process), mock.patch.object(diag, "summarize_windows", return_value=[]):
                with self.assertRaisesRegex(ValueError, "msi_default_ui_install"):
                    lifecycle.invoke(["msiexec"], stage="msi_default_ui_install")
            process.kill.assert_not_called()
            process.terminate.assert_not_called()
            self.assertEqual(trace.document["status"], "failed")


if __name__ == "__main__": unittest.main()
