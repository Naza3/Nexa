"""Host-independent safety contracts for the pinned Tauri NSIS template.

These checks validate authoring, not execution of a Windows installer.
"""
from pathlib import Path
import re
import unittest


ROOT = Path(__file__).resolve().parents[1]
TEMPLATE = ROOT / "packaging/tauri/windows/installer.nsi"


def function(source, name):
    match = re.search(r"^Function " + re.escape(name) + r"\n(.*?)^FunctionEnd$", source, re.M | re.S)
    if not match:
        raise AssertionError("missing NSIS function: " + name)
    return match[1]


def section(source, name):
    match = re.search(r"^Section " + re.escape(name) + r"\n(.*?)^SectionEnd$", source, re.M | re.S)
    if not match:
        raise AssertionError("missing NSIS section: " + name)
    return match[1]


class TauriNsisTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.source = TEMPLATE.read_text(encoding="utf-8")

    def test_scope_and_actual_destination_are_checked_before_helper_extraction(self):
        self.assertIn('!if "${INSTALLMODE}" != "currentUser"', self.source)
        init = function(self.source, ".onInit")
        self.assertIn('StrCpy $INSTDIR "$LOCALAPPDATA\\Programs\\Nexa"', init)
        self.assertNotIn("RestorePreviousInstallLocation", init)
        self.assertLess(init.index("!insertmacro SetContext"), init.index("Call NexaCheck"))
        check = function(self.source, "${prefix}NexaCheck")
        self.assertLess(check.index('$INSTDIR != "$LOCALAPPDATA\\Programs\\Nexa"'), check.index("InitPluginsDir"))
        self.assertLess(check.index("SetErrorLevel 1603"), check.index("InitPluginsDir"))
        self.assertNotIn("MUI_PAGE_DIRECTORY", self.source)

    def test_every_mutation_entrypoint_has_preflight_including_silent_install(self):
        self.assertIn("Call NexaCheck", function(self.source, ".onInit"))
        self.assertIn("Call NexaDetectExisting", function(self.source, ".onInit"))
        self.assertIn("Call NexaCheck", section(self.source, "EarlyChecks"))
        install = section(self.source, "Install")
        self.assertLess(install.index("Call NexaReplaceExisting"), install.index("SetOutPath $INSTDIR"))
        self.assertLess(install.index("NSIS_HOOK_PREINSTALL"), install.index("Call NexaCheck"))
        self.assertLess(install.index("Call NexaCheck"), install.index("SetOutPath $INSTDIR"))
        self.assertIn("Call un.NexaCheck", function(self.source, "un.onInit"))
        uninstall = section(self.source, "Uninstall")
        self.assertEqual(uninstall.count("Call un.NexaCheck"), 2)
        self.assertLess(uninstall.rindex("Call un.NexaCheck"), uninstall.index('Delete "$INSTDIR'))
        self.assertNotIn("${Silent}", function(self.source, "NexaReplaceExisting"))

    def test_helper_embedded_for_installer_and_uninstaller_and_errors_fail_closed(self):
        self.assertIn('!insertmacro NexaCheckFunction "" "/nsis"', self.source)
        self.assertIn('!insertmacro NexaCheckFunction "un." "/check"', self.source)
        check = function(self.source, "${prefix}NexaCheck")
        self.assertIn('File "/oname=$PLUGINSDIR\\nexa-install-check.exe" "${NEXA_CHECK_SOURCE}"', check)
        self.assertIn("${If} ${Errors}", check)
        self.assertIn("$NexaCheckResult != 0", check)
        self.assertIn("SetErrorLevel 1638", check)
        self.assertIn("Use the new MSI to upgrade", check)
        self.assertGreaterEqual(check.count("Quit"), 4)

    def test_no_force_shutdown_or_msi_auto_uninstall(self):
        for forbidden in ("!insertmacro CheckIfAppIsRunning", "RmShutdown", "RmForceShutdown", "TerminateProcess", "taskkill", "$WixMode", "wix_loop", "ExecWait '$R1'"):
            with self.subTest(forbidden=forbidden):
                self.assertNotIn(forbidden, self.source)
        self.assertNotIn("ExecWait", function(self.source, "PageLeaveReinstall"))

    def test_replacement_uses_verified_same_format_uninstaller_and_waits(self):
        detect = function(self.source, "NexaDetectExisting")
        self.assertIn('$R0 != \'$\\"$INSTDIR\\uninstall.exe$\\"\'', detect)
        self.assertIn('${IfNot} ${FileExists} "$INSTDIR\\uninstall.exe"', detect)
        self.assertIn('ReadRegStr $R0 SHCTX "${UNINSTKEY}" "UninstallString"', detect)
        replace = function(self.source, "NexaReplaceExisting")
        self.assertLess(replace.index("Call NexaCheck"), replace.index("ExecWait"))
        self.assertLess(replace.index("Call NexaDetectExisting"), replace.index("ExecWait"))
        self.assertIn('/S /UPDATE _?=$INSTDIR', replace)
        self.assertLess(replace.index("$NexaCheckResult != 0"), replace.index('Delete "$INSTDIR\\uninstall.exe"'))
        self.assertIn('ReadRegStr $R0 SHCTX "${UNINSTKEY}" "UninstallString"', replace)
        self.assertTrue(replace.rstrip().endswith("Call NexaCheck"))
        self.assertIn("cannot roll back", function(self.source, "PageReinstall"))

    def test_same_version_is_accepted_and_newer_or_unknown_is_rejected_early(self):
        detect = function(self.source, "NexaDetectExisting")
        self.assertIn('nsis_tauri_utils::SemverCompare "${VERSION}" $R4', detect)
        self.assertIn('nsis_tauri_utils::SemverCompare $R4 "0.0.0"', detect)
        self.assertLess(detect.index('nsis_tauri_utils::SemverCompare $R4 "0.0.0"'), detect.index('nsis_tauri_utils::SemverCompare "${VERSION}"'))
        self.assertIn('${If} $R0 = -1', detect)
        self.assertIn('${If} $R0 != 0\n  ${AndIf} $R0 != 1', detect)
        self.assertLess(detect.index("SetErrorLevel 1638"), detect.index("StrCpy $NexaExisting 1"))
        self.assertNotIn("${Silent}", detect)
        self.assertNotIn("$UpdateMode", detect)

    def test_uninstall_is_owned_inventory_only_and_keeps_all_user_data(self):
        uninstall = section(self.source, "Uninstall")
        self.assertIn('{{#each resources}}\n    Delete "$INSTDIR\\\\{{this.[1]}}"', uninstall)
        self.assertIn('{{#each binaries}}\n    Delete "$INSTDIR\\\\{{this}}"', uninstall)
        self.assertNotRegex(self.source, r"(?i)rmdir\s+/r(?:\s|$)")
        self.assertNotIn("DeleteAppDataCheckbox", self.source)
        self.assertNotIn('Delete "$INSTDIR\\*', self.source)
        for protected in ("models", "settings", "token", "config.toml"):
            self.assertNotRegex(uninstall, r'(?im)^\s*(?:Delete|RMDir) .*' + protected)

    def test_shortcuts_use_only_the_guarded_fixed_start_menu_path(self):
        for name in (".onInit", "un.onInit"):
            self.assertIn('StrCpy $AppStartMenuFolder "Nexa"', function(self.source, name))
        for forbidden in ("MUI_PAGE_STARTMENU", "MUI_STARTMENU_GETFOLDER", "$DESKTOP", "CreateOrUpdateDesktopShortcut", "MUI_FINISHPAGE_SHOWREADME", "$OldMainBinaryName"):
            with self.subTest(forbidden=forbidden):
                self.assertNotIn(forbidden, self.source)
        self.assertIn('!if "${STARTMENUFOLDER}" != "Nexa"', self.source)
        self.assertIn('!if "${PRODUCTNAME}" != "Nexa"', self.source)
        self.assertNotIn('$SMPROGRAMS\\${PRODUCTNAME}.lnk', self.source)
        self.assertIn('CreateShortcut "$SMPROGRAMS\\$AppStartMenuFolder\\${PRODUCTNAME}.lnk"', self.source)
        self.assertIn('Delete "$SMPROGRAMS\\$AppStartMenuFolder\\${PRODUCTNAME}.lnk"', self.source)

    def test_autorun_removal_is_exact_and_preserved_during_replacement(self):
        uninstall = section(self.source, "Uninstall")
        marker = '; Preserve autorun during replacement'
        own = uninstall[uninstall.index(marker):uninstall.index('; Models, settings')]
        self.assertIn('${If} $UpdateMode <> 1', own)
        self.assertIn('ReadRegStr $R0 HKCU "Software\\Microsoft\\Windows\\CurrentVersion\\Run" "Nexa"', own)
        self.assertIn('${IfNot} ${Errors}', own)
        self.assertIn('$R0 == \'$\\"$INSTDIR\\${MAINBINARYNAME}.exe$\\"\'', own)
        self.assertLess(own.index('$R0 =='), own.index('DeleteRegValue'))
        self.assertNotIn('DeleteRegKey', own)
        self.assertNotRegex(section(self.source, "Install"), r'WriteReg.*CurrentVersion.*Run')
        self.assertIn('/S /UPDATE _?=$INSTDIR', function(self.source, "NexaReplaceExisting"))

    def test_pinned_source_and_license_are_documented(self):
        self.assertIn("30da1fd6e17de6107ecc850c95dfb16b5729f2dd", self.source)
        self.assertIn("SPDX-License-Identifier: Apache-2.0 OR MIT", self.source)
        self.assertIn("crates/tauri-bundler/src/bundle/windows/nsis/installer.nsi", self.source)


if __name__ == "__main__":
    unittest.main()
