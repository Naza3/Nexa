"""Portable checks for private fixture authoring and native-only entry guards."""
from contextlib import contextmanager
from pathlib import Path
import tempfile
import io
from types import SimpleNamespace
import unittest
from unittest import mock

import package_windows as base
import test_tauri_windows_lifecycle as lifecycle
import windows_installer_diagnostics as diagnostics


class FakeMsi:
    def __init__(self, actions=()):
        self.package = "{11111111-1111-4111-8111-111111111111}"
        self.actions = list(actions)
        self.operations = []
        self.product = "{22222222-2222-4222-8222-222222222222}"

    @contextmanager
    def database(self, path, mode=0):
        yield 1

    def summary_string(self, database, field):
        return self.package

    def rows(self, database, query):
        if "`Property`" in query:
            return [["ProductCode", self.product], ["UpgradeCode", lifecycle.legacy.UPGRADE_CODE],
                    ["ProductVersion", "0.2.3"]]
        return self.actions

    def execute(self, database, sql, values):
        self.operations.append((sql, values))

    def summary(self, database, values):
        self.package = values[9]

    def MsiDatabaseCommit(self, database):
        return 0

    def check(self, code, message):
        if code:
            raise ValueError(message)


class TauriLifecycleContractTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="nexa-lifecycle-unit-")
        self.addCleanup(self.temporary.cleanup)
        self.work = Path(self.temporary.name)

    def test_runtime_startup_diagnostic_is_fixed_bounded_and_redacted(self):
        for code in lifecycle.STARTUP_CODES:
            for newline in (b"\n", b"\r\n"):
                report = b"nexa-startup-v1:" + code.encode("ascii") + newline
                self.assertEqual(lifecycle.startup_failure_code(io.BytesIO(report)), code)
        for raw in (b"", b"secret token /private/path", b"x" * 100000,
                    b"nexa-startup-v1:unknown\n",
                    b"nexa-startup-v1:configuration_busy\nsecret"):
            self.assertEqual(lifecycle.startup_failure_code(io.BytesIO(raw)), "runtime_start_failed")
        output = mock.Mock()
        output.read.return_value = b""
        lifecycle.startup_failure_code(output)
        output.read.assert_called_once_with(257)

    def test_readiness_waits_for_discovery_then_requires_authenticated_status(self):
        root, work = self.work / "installed", self.work / "busy"
        discovery = work / "runtime-data/runtime/instance.json"
        process = mock.Mock()
        process.poll.side_effect = [None, None, None, None]
        process.wait.return_value = 0
        def publish(_):
            discovery.parent.mkdir(parents=True, exist_ok=True)
            discovery.write_text("{}", encoding="utf-8")
        def prove(*args, **kwargs):
            self.assertTrue(discovery.is_file())
            return SimpleNamespace(returncode=0)
        with mock.patch.object(lifecycle.common, "invoke") as invoke, \
                mock.patch.object(lifecycle.common, "trace"), \
                mock.patch.object(lifecycle.common, "short_path", side_effect=lambda p: p), \
                mock.patch.object(lifecycle.subprocess, "Popen", return_value=process), \
                mock.patch.object(lifecycle.subprocess, "run", side_effect=prove) as status, \
                mock.patch.object(lifecycle.time, "sleep", side_effect=publish) as sleep:
            with lifecycle.live_runtime(root, work) as child:
                self.assertIs(child, process)
            sleep.assert_called_once_with(0.2)
            status.assert_called_once()
            self.assertEqual(invoke.call_args.args[0][-1], "stop")
            process.wait.assert_called_once_with(timeout=30)

    def test_readiness_rejects_reused_data_before_launch(self):
        work = self.work / "busy"
        (work / "runtime-data").mkdir(parents=True)
        with mock.patch.object(lifecycle.subprocess, "Popen") as launch:
            with self.assertRaisesRegex(ValueError, "fresh"):
                with lifecycle.live_runtime(self.work, work):
                    self.fail("must not yield")
            launch.assert_not_called()

    def test_exited_runtime_reports_only_fixed_code_and_preserves_failure(self):
        process = mock.Mock()
        process.poll.return_value = 1
        with mock.patch.object(lifecycle.common, "invoke"), \
                mock.patch.object(lifecycle.common, "trace") as trace, \
                mock.patch.object(lifecycle.common, "short_path", side_effect=lambda p: p), \
                mock.patch.object(lifecycle.subprocess, "Popen", return_value=process), \
                mock.patch.object(lifecycle.subprocess, "run") as status, \
                mock.patch("builtins.print") as output:
            with self.assertRaisesRegex(ValueError, "exited"):
                with lifecycle.live_runtime(self.work, self.work / "busy"):
                    self.fail("must not yield")
            status.assert_not_called()
            trace.assert_any_call("runtime_ready", "error", exit_code=1)
            self.assertIn('"startup_error": "runtime_start_failed"', output.call_args.args[0])

    def payload(self):
        folder = self.work / "payload"
        folder.mkdir()
        for name in ("README.md", "SHA256SUMS", "nexa-desktop.exe", "manifest.json"):
            (folder / name).write_bytes(("fixture " + name).encode("ascii"))
        return folder

    def test_installed_diagnostic_targets_installed_exe_and_checks_identity(self):
        manifest = {"project_commit": "a" * 40, "project_dirty": False}
        installed = self.work / "installed"
        installed.mkdir()
        (installed / "uninstall.exe").write_bytes(b"synthetic installer extra")
        for installer in ("msi", "nsis"):
            with mock.patch.object(lifecycle.legacy.base, "windows_environment", return_value={"SYSTEMROOT": "C:\\Windows"}), mock.patch.object(lifecycle.desktop_smoke, "checked_json", return_value=manifest) as check:
                lifecycle.diagnose_installed(installed, manifest, self.work, installer)
                args, kwargs = check.call_args
                self.assertEqual(args[0], [installed / "nexa-desktop.exe", "--diagnose"])
                self.assertNotEqual(args[1], installed)
                self.assertEqual(kwargs["phase"], "desktop_diagnose")
                self.assertEqual(kwargs["diagnostic_file"].name, f"diagnostics-installed-{installer}.json")
                self.assertIn(kwargs["diagnostic_file"].name, lifecycle.desktop_smoke.DIAGNOSTIC_REPORTS)
        for observed in ({"project_commit": "b" * 40, "project_dirty": False},
                         {"project_commit": "a" * 40, "project_dirty": True}):
            with mock.patch.object(lifecycle.legacy.base, "windows_environment", return_value={"SYSTEMROOT": "C:\\Windows"}), mock.patch.object(lifecycle.desktop_smoke, "checked_json", return_value=observed):
                with self.assertRaisesRegex(ValueError, "source identity mismatch"):
                    lifecycle.diagnose_installed(installed, manifest, self.work, "nsis")
        with mock.patch.object(lifecycle.legacy.base, "windows_environment", return_value={"SYSTEMROOT": "C:\\Windows"}), mock.patch.object(lifecycle.desktop_smoke, "checked_json", side_effect=ValueError("diagnostic failed")):
            with self.assertRaisesRegex(ValueError, "diagnostic failed"):
                lifecycle.diagnose_installed(installed, manifest, self.work, "msi")

    def test_changed_fixture_replaces_bytes_adds_and_removes_only_private_copy(self):
        payload = self.payload()
        original = base.entries(payload)
        changed = self.work / "changed"
        files = lifecycle.variant_payload(payload, changed)
        self.assertEqual(base.entries(payload), original)
        self.assertFalse((changed / "SHA256SUMS").exists())
        self.assertTrue((changed / "private-fixture/added.txt").is_file())
        self.assertNotEqual((changed / "README.md").read_bytes(), (payload / "README.md").read_bytes())
        self.assertEqual((changed / "nexa-desktop.exe").read_bytes(), (payload / "nexa-desktop.exe").read_bytes())
        self.assertEqual(files, base.entries(changed))

    def test_legacy_fixture_has_obsolete_owned_file_without_modifying_source(self):
        payload = self.payload()
        original = base.entries(payload)
        destination = self.work / "legacy"
        files = lifecycle.variant_payload(payload, destination, legacy_fixture=True)
        self.assertIn("nexa-private-obsolete.txt", {item["path"] for item in files})
        self.assertTrue((destination / "SHA256SUMS").is_file())
        self.assertEqual(base.entries(payload), original)

    def test_product_identity_is_read_from_msi_not_derived_from_version(self):
        api = FakeMsi()
        product, version, package = lifecycle.msi_identity(api, self.work / "fixture.msi")
        self.assertEqual(product, api.product)
        self.assertEqual(version, "0.2.3")
        self.assertEqual(package, api.package)
        self.assertNotEqual(product, lifecycle.legacy.product_code(version))

    def test_released_legacy_fixture_keeps_023_identity_with_private_inventory(self):
        files = [{"path": "private-inventory.txt"}]
        cabinet, guard = self.work / "private.cab", self.work / "private-guard.dll"
        with mock.patch.object(lifecycle.legacy, "write_msi") as author:
            destination = lifecycle.build_released_legacy_fixture(files, "0.3.0", cabinet, guard, self.work)
        author.assert_called_once_with(files, "0.2.3", cabinet, guard, destination)
        self.assertEqual(destination, self.work / "legacy-release-0.2.3.msi")

    def test_released_legacy_fixture_does_not_attempt_equal_version_or_downgrade(self):
        with mock.patch.object(lifecycle.legacy, "write_msi") as author:
            for version in ("0.1.9", "0.2.2", "0.2.3"):
                with self.subTest(version=version):
                    self.assertIsNone(lifecycle.build_released_legacy_fixture([], version, None, None, self.work))
        author.assert_not_called()

    def test_fixture_workspace_uses_checkout_volume_instead_of_system_temp(self):
        checkout = (self.work / "checkout with spaces").resolve()
        system_temp = self.work / "system-temp"
        checkout.mkdir()
        system_temp.mkdir()
        with mock.patch.object(lifecycle.legacy, "ROOT", checkout), \
                mock.patch.object(lifecycle.tempfile, "tempdir", str(system_temp)):
            first = lifecycle.fixture_workspace()
            second = lifecycle.fixture_workspace()
        self.assertEqual(first.parent, checkout / "build")
        self.assertEqual(second.parent, checkout / "build")
        self.assertNotEqual(first, second)
        self.assertTrue(first.is_dir() and second.is_dir())
        self.assertEqual(list(system_temp.iterdir()), [])

    def test_native_lifecycle_stages_are_accepted_by_sanitized_diagnostics(self):
        self.assertLessEqual(lifecycle.STAGES, diagnostics.STAGES)

    def test_product_identity_rejects_malformed_package_identity(self):
        api = FakeMsi()
        api.package = "not-a-guid"
        with self.assertRaisesRegex(ValueError, "identity"):
            lifecycle.msi_identity(api, self.work / "fixture.msi")

    def test_nsis_user_registration_is_not_duplicated_by_shared_wow64_views(self):
        @contextmanager
        def opened_key(hive, path, reserved, access):
            if hive != 1:
                raise FileNotFoundError
            yield "user-key"
        # OpenKey raises before returning a handle, rather than on __enter__.
        def open_key(hive, path, reserved, access):
            if hive != 1:
                raise FileNotFoundError
            return opened_key(hive, path, reserved, access)
        registry = SimpleNamespace(HKEY_CURRENT_USER=1, HKEY_LOCAL_MACHINE=2,
                                   KEY_WOW64_64KEY=0x100, KEY_WOW64_32KEY=0x200, KEY_READ=1,
                                   OpenKey=open_key,
                                   QueryValueEx=lambda key, name: ({"UninstallString": '"C:\\Nexa\\uninstall.exe"',
                                                                  "DisplayVersion": "0.2.3"}[name], 1))
        with mock.patch.dict(lifecycle.sys.modules, {"winreg": registry}):
            self.assertEqual(lifecycle.nsis_registrations(), [("user", '"C:\\Nexa\\uninstall.exe"', "0.2.3")])
        registry.QueryValueEx = mock.Mock(side_effect=FileNotFoundError)
        with mock.patch.dict(lifecycle.sys.modules, {"winreg": registry}):
            with self.assertRaisesRegex(ValueError, "incomplete"):
                lifecycle.nsis_registrations()

    def test_rollback_updates_only_private_copy_and_executes_before_failure(self):
        source, destination = self.work / "source.msi", self.work / "rollback.msi"
        source.write_bytes(b"original fixture MSI bytes")
        api = FakeMsi()
        previous = api.package
        self.assertEqual(lifecycle.add_rollback_fixture(source, destination, api), destination)
        self.assertEqual(source.read_bytes(), b"original fixture MSI bytes")
        self.assertNotEqual(api.package, previous)
        rows = [values for sql, values in api.operations if "`InstallExecuteSequence`" in sql]
        self.assertEqual(rows, [["InstallExecute", None, 6490], ["NexaTestRollback", "NOT Installed", 6590]])
        self.assertTrue(any(values[:2] == ["NexaTestRollback", 19] for _, values in api.operations))

    def test_rollback_supports_existing_standard_installexecute_action(self):
        source, destination = self.work / "source.msi", self.work / "rollback.msi"
        source.write_bytes(b"fixture")
        api = FakeMsi([["InstallExecute", "6500"]])
        lifecycle.add_rollback_fixture(source, destination, api)
        self.assertFalse(any(values[0] == "InstallExecute" for _, values in api.operations))
        self.assertTrue(any(values == ["NexaTestRollback", "NOT Installed", 6590] for _, values in api.operations))

    def test_rollback_refuses_in_place_or_existing_destination(self):
        source, destination = self.work / "source.msi", self.work / "rollback.msi"
        source.write_bytes(b"original")
        destination.write_bytes(b"keep")
        with self.assertRaises(ValueError):
            lifecycle.add_rollback_fixture(source, source, FakeMsi())
        with self.assertRaises(ValueError):
            lifecycle.add_rollback_fixture(source, destination, FakeMsi())
        self.assertEqual(source.read_bytes(), b"original")
        self.assertEqual(destination.read_bytes(), b"keep")

    def test_native_lifecycle_refuses_non_disposable_environment_before_msi_access(self):
        with mock.patch.object(lifecycle.sys, "platform", "linux"), mock.patch.object(lifecycle, "Msi") as api:
            with self.assertRaisesRegex(ValueError, "disposable"):
                lifecycle.lifecycle(*(self.work / name for name in ("input.msi", "setup.exe", "payload", "report.json")))
            api.assert_not_called()
        with mock.patch.object(lifecycle.sys, "platform", "win32"), mock.patch.dict(lifecycle.os.environ, {}, clear=True), \
                mock.patch.object(lifecycle, "Msi") as api:
            with self.assertRaisesRegex(ValueError, "disposable"):
                lifecycle.lifecycle(*(self.work / name for name in ("input.msi", "setup.exe", "payload", "report.json")))
            api.assert_not_called()


if __name__ == "__main__":
    unittest.main()
