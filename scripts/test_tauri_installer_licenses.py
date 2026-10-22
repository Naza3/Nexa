"""Installer original-license preservation in the existing compact desktop bundle."""
import hashlib
import json
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest import mock

import package_desktop_windows as pack


ORIGINALS = pack.ROOT / "packaging/tauri/windows/licenses"


class TauriInstallerLicenseTests(unittest.TestCase):
    def fixture(self, directory):
        root, stage, sysroot = directory / "repo", directory / "stage", directory / "sysroot"
        supplements = root / "packaging/desktop-windows/third-party"
        supplements.mkdir(parents=True)
        (supplements / "sources.json").write_text('{"files": []}', encoding="utf-8")
        (supplements / "native-components.json").write_text('{"components": []}', encoding="utf-8")
        (supplements / "SOURCES.md").write_text("Fixture source inventory\n", encoding="utf-8")
        shutil.copytree(ORIGINALS, root / "packaging/tauri/windows/licenses")
        docs = sysroot / "share/doc/rust"
        docs.mkdir(parents=True)
        (docs / "COPYRIGHT-library.html").write_text("<html>Fixture Rust copyright</html>\n", encoding="utf-8")
        return root, stage, sysroot

    def test_original_hashes_and_required_separate_notices(self):
        sources = json.loads((ORIGINALS / "sources.json").read_text(encoding="utf-8"))
        names = set()
        for record in sources["files"]:
            self.assertNotIn(record["path"], names)
            names.add(record["path"])
            raw = (ORIGINALS / record["path"]).read_bytes()
            self.assertEqual(hashlib.sha256(raw).hexdigest(), record["sha256"])
            raw.decode("utf-8", errors="strict")
        for path in ("nsis-3.11/COPYING", "nsis-3.11/System-Call.S", "nsis-3.11/Modern-UI-2-Readme.html",
                     "wix-3.14.1/LICENSE.TXT", "nsis-tauri-utils-0.5.3/LICENSE_MIT", "tauri-2.12.1/LICENSE-MIT"):
            self.assertIn(path, names)
        self.assertIn("SPECIAL EXCEPTION FOR LZMA", (ORIGINALS / "nsis-3.11/COPYING").read_text(encoding="utf-8"))
        self.assertIn("Thomas Gaugler", (ORIGINALS / "nsis-3.11/System-Call.S").read_text(encoding="utf-8"))
        self.assertFalse(sources["plugin_binary"]["cargo_lock_published"])
        self.assertEqual(sources["plugin_binary"]["rust_compiler_identity"], "not published")

    def test_real_originals_survive_copy_and_lossless_consolidation(self):
        with tempfile.TemporaryDirectory() as temp:
            root, stage, sysroot = self.fixture(Path(temp))
            with mock.patch.object(pack, "ROOT", root), mock.patch.object(pack.base, "command", return_value=str(sysroot)):
                records = pack.copy_rust_licenses(stage, {"packages": [], "resolve": {"nodes": []}}, {})
            sources = json.loads((ORIGINALS / "sources.json").read_text(encoding="utf-8"))
            expected = {"licenses/installers/" + r["path"]: (ORIGINALS / r["path"]).read_bytes() for r in sources["files"]}
            expected["licenses/installers/sources.json"] = (ORIGINALS / "sources.json").read_bytes()
            self.assertTrue(set(expected) <= {r["path"] for r in records})
            pack.base.consolidate_licenses(stage)
            recovered = pack.base.verify_license_bundle(stage)
            for path, raw in expected.items():
                self.assertEqual(recovered[path]["raw"], raw)
                self.assertTrue(recovered[path]["document"]["attributions"])
            self.assertLessEqual(len(pack.base.license_related_files(stage)), 3)
            self.assertFalse((stage / "LICENSE").exists())

    def test_tampered_original_is_rejected_before_package_creation(self):
        with tempfile.TemporaryDirectory() as temp:
            root, stage, sysroot = self.fixture(Path(temp))
            target = root / "packaging/tauri/windows/licenses/nsis-3.11/System-Call.S"
            target.write_bytes(target.read_bytes() + b"altered")
            with mock.patch.object(pack, "ROOT", root), mock.patch.object(pack.base, "command", return_value=str(sysroot)):
                with self.assertRaisesRegex(ValueError, "installer original license hash mismatch"):
                    pack.copy_rust_licenses(stage, {"packages": [], "resolve": {"nodes": []}}, {})
            self.assertFalse((stage / "licenses/index.json").exists())


if __name__ == "__main__":
    unittest.main()
