#!/usr/bin/env python3
"""Host-independent authoring and safety-contract regression tests."""
import copy
from pathlib import Path
import tempfile
import unittest
import uuid
from unittest import mock

import package_windows_msi as pack


def files():
    return [{"path": path, "size_bytes": index + 1, "sha256": "a" * 64}
            for index, path in enumerate(["nexa-desktop.exe", "manifest.json", "SHA256SUMS", "runtime/ai-runtime.exe", "runtime/licenses/index.json", "download/nexa-aria2.exe"])]


class WindowsInstallerTests(unittest.TestCase):
    def test_component_and_product_identity(self):
        first = pack.author_tables(files(), "1.2.3")
        second = pack.author_tables(list(reversed(files())), "1.2.4")
        self.assertEqual(first["Component"], second["Component"])
        self.assertEqual(first["File"], second["File"])
        self.assertNotEqual(pack.product_code("1.2.3"), pack.product_code("1.2.4"))
        self.assertEqual(pack.product_code("1.2.3"), pack.product_code("1.2.3"))
        self.assertEqual(uuid.UUID(pack.UPGRADE_CODE.strip("{}")), pack.NAMESPACE)

    def test_scope_and_owned_files_only(self):
        tables = pack.author_tables(files(), "0.1.0")
        pack.validate_tables(tables)
        self.assertNotIn("ALLUSERS", dict(tables["Property"]))
        self.assertEqual(dict(tables["Property"])["LIMITUI"], "1")
        self.assertEqual(dict(tables["Property"])["ARPNOMODIFY"], "1")
        self.assertEqual(dict(tables["Property"])["MSIRESTARTMANAGERCONTROL"], "Disable")
        self.assertEqual(len(tables["NexaPayload"]), len(files()))
        self.assertTrue(all(row[2] is None and row[4] == 2 for row in tables["RemoveFile"]))
        self.assertTrue(all(row[1] == 1 for row in tables["Registry"]))
        self.assertTrue(all(row[3] == 260 for row in tables["Component"]))
        self.assertFalse(any("model" in row[0].lower() or "token" in row[0].lower() for row in tables["NexaPayload"]))
        self.assertNotIn("ServiceControl", tables)
        self.assertNotIn("ServiceInstall", tables)
        self.assertIn(["ExecuteAction", None, 1300], tables["InstallUISequence"])

    def test_os_guard_is_mandatory_and_precedes_all_mutation(self):
        tables = pack.production_tables(files(), "0.1.0", Path("guard.dll"), Path("os-check.exe"))
        pack.validate_tables(tables)
        self.assertIn(["NexaOsGuard", 2, "NexaOsGuardBinary", None], tables["CustomAction"])
        self.assertEqual(tables["Binary"], [["NexaGuardBinary", Path("guard.dll")], ["NexaOsGuardBinary", Path("os-check.exe")]])
        for name in ("InstallExecuteSequence", "InstallUISequence"):
            rows = {row[0]: row for row in tables[name]}
            self.assertIsNone(rows["NexaOsGuard"][1])
            self.assertLess(rows["NexaOsGuard"][2], rows["NexaGuard"][2])
        self.assertLess(dict((row[0], row[2]) for row in tables["InstallExecuteSequence"])["NexaOsGuard"], 1500)
        self.assertNotIn("NexaLegacyOsProbe", str(tables))
        self.assertNotIn("path_test", str(tables))

    def test_private_preflight_runs_only_read_only_actions_with_new_identity(self):
        production = pack.production_tables(files(), "0.1.0", Path("guard.dll"), Path("os-check.exe"))
        probe = pack.preflight_tables(production, Path("legacy-probe.dll"))
        pack.validate_tables(probe)
        self.assertEqual(tuple(row[0] for row in probe["InstallExecuteSequence"]), pack.PREFLIGHT_ACTIONS)
        self.assertEqual(probe["InstallUISequence"], [])
        self.assertEqual(probe["Upgrade"], [])
        for name in ("InstallInitialize", "InstallFinalize", "InstallFiles", "WriteRegistryValues", "RegisterProduct", "PublishProduct"):
            self.assertNotIn(name, [row[0] for row in probe["InstallExecuteSequence"]])
        before, after = dict(production["Property"]), dict(probe["Property"])
        self.assertNotEqual(before["ProductCode"], after["ProductCode"])
        self.assertNotEqual(before["UpgradeCode"], after["UpgradeCode"])
        self.assertEqual(production["CustomAction"], probe["CustomAction"][:-1])
        self.assertEqual(production["Binary"], probe["Binary"][:-1])
        self.assertEqual(len({row[1] for row in production["Component"]} & {row[1] for row in probe["Component"]}), 0)

    def test_version_check_uses_shared_manifested_exe_and_msi_probe(self):
        setup = (pack.AUTHORING / "setup.c").read_text(encoding="utf-8")
        os_check = (pack.AUTHORING / "os_check.c").read_text(encoding="utf-8")
        header = (pack.AUTHORING / "os_version.h").read_text(encoding="utf-8")
        guard = (pack.AUTHORING / "guard.c").read_text(encoding="utf-8") + (pack.AUTHORING / "guard_paths.h").read_text(encoding="utf-8")
        source = Path(pack.__file__).read_text(encoding="utf-8")
        self.assertIn('#include "os_version.h"', setup)
        self.assertIn('#include "os_version.h"', os_check)
        self.assertIn("VerifyVersionInfoW", header)
        self.assertIn("version.dwMajorVersion = 10", header)
        self.assertIn("VER_SERVICEPACKMAJOR | VER_SERVICEPACKMINOR", header)
        self.assertIn("ERROR_OLD_WIN_VERSION", os_check)
        self.assertNotIn("GetFileVersionInfo", guard)
        self.assertNotIn("RtlGetVersion", guard)
        self.assertIn('"System32/msiexec.exe"', source)
        self.assertIn('process.wait(timeout=60)', source)
        self.assertIn('"NexaLegacyOsProbe", "NexaOsGuard", "NexaGuard"', source)
        self.assertIn('item["return_code"] == 1', source)

    def test_upgrade_is_transactional_and_downgrade_blocked(self):
        tables = pack.author_tables(files(), "2.0.1")
        order = {name: sequence for name, _, sequence in tables["InstallExecuteSequence"]}
        self.assertLess(order["NexaGuard"], order["InstallInitialize"])
        self.assertLess(order["InstallInitialize"], order["RemoveExistingProducts"])
        self.assertLess(order["RemoveExistingProducts"], order["ProcessComponents"])
        self.assertLess(order["RemoveExistingProducts"], order["InstallFinalize"])
        self.assertEqual(tables["Upgrade"][0][1:5], [None, "2.0.1", None, 0])
        self.assertEqual(tables["Upgrade"][1][1:5], ["2.0.1", None, None, 2])
        self.assertIn("Installed OR NOT NEXA_NEWER", dict(tables["LaunchCondition"]))
        self.assertNotIn("VersionNT >= 1000", str(tables))
        self.assertFalse(any("Test" in row[0] for row in tables["CustomAction"]))

    def test_rollback_fixture_is_explicit_and_unexposed(self):
        tables = pack.author_tables(files(), "2.0.2", rollback_fixture=True)
        pack.validate_tables(tables)
        self.assertEqual(tables["CustomAction"][-1][:2], ["NexaTestRollback", 19])
        source = (pack.ROOT / "scripts/package_windows_msi.py").read_text(encoding="utf-8")
        self.assertNotIn('add_argument("--rollback', source)
        self.assertNotIn('add_argument("--fixture', source)

    def test_unsafe_and_alias_paths_rejected(self):
        for name in ("../x", "/x", "C:/x", "a\\b", "a//b", "a/./b", "con", "NUL.exe", "x/COM1.txt", "lpt9",
                     "x ", "x.", "x/evil?.dll", "x|a", "x\nname", "x\x00name", "x\x7fname", "文件.dll", "LONGNA~1.DLL"):
            with self.subTest(name=name), self.assertRaises(ValueError):
                pack.checked_relative(name)
        self.assertEqual(pack.checked_relative("runtime/licenses/index.json"), "runtime/licenses/index.json")

    def test_duplicate_paths_rejected(self):
        for duplicate in ("nexa-desktop.exe", "NEXA-DESKTOP.EXE"):
            with self.assertRaisesRegex(ValueError, "duplicate"):
                pack.author_tables(files() + [{"path": duplicate, "size_bytes": 1}], "1.0.0")

    def test_version_bounds(self):
        for version in ("v1.0.0", "1.0", "1.0.0.1", "01.0.0", "1.0.0-beta", "1.0.0+build", "256.0.0", "0.256.0", "0.0.65536", "0.0.0"):
            with self.subTest(version=version), self.assertRaises(ValueError):
                pack.product_code(version)
        pack.product_code("255.255.65535")

    def test_table_schema_rejects_mutations(self):
        for mutation in ("required", "width", "duplicate", "integer", "directory", "component", "feature", "removal", "guid"):
            tables = pack.author_tables(files(), "1.0.0")
            if mutation == "required": tables["File"][0][0] = None
            if mutation == "width": tables["File"][0][0] = "f" * 73
            if mutation == "duplicate": tables["File"].append(tables["File"][0])
            if mutation == "integer": tables["File"][0][3] = 2**31
            if mutation == "directory": tables["Component"][0][2] = "absent"
            if mutation == "component": tables["File"][0][1] = "absent"
            if mutation == "feature": tables["FeatureComponents"][0][0] = "absent"
            if mutation == "removal": tables["RemoveFile"][0][2] = "*"
            if mutation == "guid": tables["Component"][0][1] = tables["Component"][1][1]
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                pack.validate_tables(tables)

    def test_native_contract_has_no_force_kill_download_or_crt(self):
        guard = (pack.AUTHORING / "guard.c").read_text(encoding="utf-8") + (pack.AUTHORING / "guard_paths.h").read_text(encoding="utf-8")
        setup = (pack.AUTHORING / "setup.c").read_text(encoding="utf-8")
        for forbidden in ("TerminateProcess(", "ShellExecute", "URLDownload", "WinHttp", "system(", "taskkill"):
            self.assertNotIn(forbidden, guard + setup)
        self.assertIn("GetFinalPathNameByHandleW", guard)
        self.assertIn("GetLongPathNameW", guard)
        self.assertIn("!same_target_path(root, expected)", guard)
        self.assertIn("!same_target_path(path, expected)", guard)
        self.assertIn("FILE_ATTRIBUTE_REPARSE_POINT", (pack.AUTHORING / "native.h").read_text(encoding="utf-8"))
        self.assertIn("NEXA_MSI_SHA256", setup)
        self.assertIn("ERROR_SUCCESS_REBOOT_REQUIRED", setup)
        self.assertIn("ERROR_INSTALL_USEREXIT", setup)
        self.assertIn("/norestart REBOOT=ReallySuppress", setup)
        self.assertIn("CREATE_NEW", setup)
        self.assertIn("FILE_SHARE_READ", setup)
        self.assertIn("GetSystemDirectoryW(system_exe", setup)
        lifecycle = (pack.ROOT / "scripts/test_windows_msi_lifecycle.py").read_text(encoding="utf-8")
        self.assertIn('if text.value == "Finish" and user.IsWindowEnabled', lifecycle)

    def test_sql_is_static_and_all_rows_have_schema(self):
        tables = pack.author_tables(files(), "1.0.0")
        for table in tables:
            self.assertTrue(pack.create_sql(table).startswith("CREATE TABLE `" + table + "`"))
            for row in tables[table]:
                self.assertEqual(len(row), len(pack.columns(table)))


if __name__ == "__main__":
    unittest.main()
