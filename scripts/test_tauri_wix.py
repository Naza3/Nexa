"""Portable tests for Tauri WiX authoring; Windows lifecycle is tested separately."""
from pathlib import Path
import tempfile
import unittest
import xml.etree.ElementTree as ET

import tauri_wix as wix


NS = {"w": wix.WIX_NS}
TEMPLATE = Path(__file__).resolve().parents[1] / "packaging/tauri/windows/main.wxs"


class TauriWixTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="nexa-wix-xml-")
        self.addCleanup(self.temporary.cleanup)
        self.work = Path(self.temporary.name)
        self.payload = self.work / "payload with spaces & marks"
        self.payload.mkdir()
        self.names = ["nexa-desktop.exe", "README.md", "runtime/ai-runtime.exe",
                      "runtime/nested/notice.txt", "licenses/a&b.txt"]
        self.files = [{"path": path} for path in self.names]
        for path in self.names:
            target = self.payload / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(path.encode("ascii"))
        self.guard = self.work / "guard.dll"
        self.os_guard = self.work / "os.exe"
        self.guard.write_bytes(b"guard fixture")
        self.os_guard.write_bytes(b"os fixture")

    def generate(self, files=None, output="payload.wxs"):
        destination = self.work / output
        result = wix.generate_fragment(self.payload, self.files if files is None else files,
                                       self.guard, self.os_guard, destination)
        self.assertEqual(result, destination)
        return ET.parse(result).getroot()

    def test_exact_inventory_destinations_and_input_bytes_are_preserved(self):
        root = self.generate()
        self.assertIsNotNone(root.find(".//w:ComponentGroup[@Id='NexaPayloadGroup']", NS))
        rows = root.findall(".//w:CustomTable[@Id='NexaPayload']/w:Row/w:Data", NS)
        self.assertEqual(sorted(row.text for row in rows),
                         sorted(path.replace("/", "\\") for path in self.names))
        files = root.findall(".//w:File", NS)
        self.assertEqual({Path(row.get("Source")).relative_to(self.payload).as_posix() for row in files},
                         set(self.names) - {wix.MAIN_BINARY})
        for row in files:
            source = Path(row.get("Source"))
            self.assertEqual(row.get("Name"), source.name)
            self.assertEqual(source.read_bytes(), source.relative_to(self.payload).as_posix().encode("ascii"))

    def test_every_component_has_its_own_hkcu_keypath(self):
        root = self.generate()
        components = root.findall(".//w:Component", NS)
        keys, identities = set(), set()
        for component in components:
            self.assertEqual(component.get("Win64"), "yes")
            keypaths = component.findall("w:RegistryValue[@KeyPath='yes']", NS)
            self.assertEqual(len(keypaths), 1)
            keypath = keypaths[0]
            self.assertEqual(keypath.get("Root"), "HKCU")
            self.assertNotIn(keypath.get("Name"), keys)
            self.assertNotIn(component.get("Guid"), identities)
            keys.add(keypath.get("Name"))
            identities.add(component.get("Guid"))
            self.assertFalse(component.findall("w:File[@KeyPath='yes']", NS))

    def test_user_profile_directories_have_empty_only_cleanup(self):
        root = self.generate()
        directory_ids = {item.get("Id") for item in root.findall(".//w:Directory", NS)}
        removed = set()
        for component in root.findall(".//w:Component", NS):
            for removal in component.findall("w:RemoveFolder", NS):
                self.assertEqual(removal.get("On"), "uninstall")
                removed.add(component.get("Directory"))
        self.assertEqual(directory_ids, removed)
        self.assertFalse(root.findall(".//w:RemoveFile", NS))
        self.assertFalse(root.findall(".//w:RemoveFolderEx", NS))

    def test_native_guards_keep_aliases_and_pretransaction_order(self):
        root = self.generate()
        actions = {row.get("Id"): row for row in root.findall(".//w:CustomAction", NS)}
        self.assertEqual(set(actions), {"NexaOsGuard", "NexaGuard", "NexaRemoveAutostart"})
        self.assertEqual(actions["NexaGuard"].get("DllEntry"), "NexaGuard")
        for action in (actions["NexaOsGuard"], actions["NexaGuard"]):
            self.assertEqual(action.get("Execute"), "immediate")
            self.assertEqual(action.get("Return"), "check")
        aliases = {row.get("Id"): row for row in root.findall(".//w:SetProperty", NS)}
        self.assertEqual(aliases["INSTALLFOLDER"].get("Value"), "[INSTALLDIR]")
        self.assertEqual(aliases["INSTALLFOLDER"].get("After"), "CostFinalize")
        self.assertEqual(aliases["NexaMenuFolder"].get("Value"), "[ApplicationProgramsFolder]")
        self.assertEqual(aliases["NexaMenuFolder"].get("After"), "SetINSTALLFOLDER")
        for row in aliases.values():
            self.assertEqual(row.get("Sequence"), "both")
        for name in ("InstallUISequence", "InstallExecuteSequence"):
            rows = root.find(".//w:" + name, NS).findall("w:Custom", NS)
            self.assertEqual([(row.get("Action"), row.get("After"), row.text) for row in rows[:2]],
                             [("NexaOsGuard", "SetNexaMenuFolder", "1"), ("NexaGuard", "NexaOsGuard", "1")])
        cleanup = actions["NexaRemoveAutostart"]
        self.assertEqual(cleanup.get("Execute"), "commit")
        self.assertEqual(cleanup.get("Impersonate"), "yes")
        self.assertEqual(cleanup.get("Return"), "check")
        row = root.find(".//w:InstallExecuteSequence/w:Custom[@Action='NexaRemoveAutostart']", NS)
        self.assertEqual(row.get("Before"), "InstallFinalize")
        self.assertEqual(row.text, 'REMOVE="ALL" AND NOT UPGRADINGPRODUCTCODE')
        self.assertIsNone(root.find(".//w:InstallUISequence/w:Custom[@Action='NexaRemoveAutostart']", NS))

    def test_xml_is_stable_when_inventory_order_changes(self):
        self.generate()
        self.generate(list(reversed(self.files)), "second.wxs")
        self.assertEqual((self.work / "payload.wxs").read_bytes(),
                         (self.work / "second.wxs").read_bytes())

    def test_rejects_missing_main_duplicates_traversal_and_payload_output(self):
        for files in ([{"path": "README.md"}], self.files + [{"path": "README.md"}],
                      self.files + [{"path": "../guard.dll"}]):
            with self.subTest(files=files), self.assertRaises(ValueError):
                self.generate(files)
        with self.assertRaises(ValueError):
            wix.generate_fragment(self.payload, self.files, self.guard, self.os_guard,
                                  self.payload / "generated.wxs")

    def test_template_preserves_official_major_upgrade_and_user_scope(self):
        template = TEMPLATE.read_text(encoding="utf-8")
        self.assertIn('Id="*"', template)
        self.assertIn('UpgradeCode="{{upgrade_code}}"', template)
        self.assertIn('InstallScope="perUser"', template)
        self.assertIn('InstallPrivileges="limited"', template)
        self.assertIn('AllowSameVersionUpgrades="yes"', template)
        self.assertIn('Schedule="afterInstallInitialize"', template)
        self.assertIn('Directory Id="LocalAppDataFolder"', template)
        self.assertIn('Directory Id="NexaProgramsFolder" Name="Programs"', template)
        self.assertIn('Directory Id="INSTALLDIR" Name="Nexa"', template)
        self.assertIn('Value="Disable"', template)
        self.assertIn('Value="ReallySuppress"', template)
        self.assertIn('Installed OR NOT NEXA_NSIS_INSTALLED', template)
        self.assertNotIn('Id="LaunchApplication"', template)
        self.assertNotIn('Id="ARPNOREPAIR"', template)
        self.assertNotIn('{{resources}}', template)
        self.assertIn('Name="Desktop" Type="integer" Value="1" KeyPath="yes"', template)
        self.assertIn('<UIRef Id="WixUI_InstallDir" />', template)


if __name__ == "__main__":
    unittest.main()
