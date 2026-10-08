"""Packaging contracts that keep Tauri's installers bound to the verified payload."""
import json
import itertools
import os
from pathlib import Path
import tempfile
import unittest
from contextlib import ExitStack
from unittest import mock

import package_tauri_windows as pack


class TauriPackageTests(unittest.TestCase):
    def test_metadata_scaffold_preserves_binary_and_explicit_resource_destinations(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            payload, work = root / "payload", root / "work"
            payload.mkdir(); work.mkdir()
            for name, data in (("nexa-desktop.exe", b"MZ:exact main bytes"), ("runtime/worker.exe", b"MZ:worker"),
                               ("licenses/THIRD_PARTY_LICENSES.txt", b"original copyright"), ("config.example.toml", b"# template")):
                path = payload / name
                path.parent.mkdir(parents=True, exist_ok=True); path.write_bytes(data)
            helpers = {key: root / (key + ".bin") for key in ("guard", "os", "check")}
            for path in helpers.values(): path.write_bytes(b"test guard")
            before = pack.base.entries(payload)
            project = pack.prepare_project(payload, before, "0.2.3", work, helpers)
            self.assertEqual(pack.base.entries(payload), before)
            self.assertEqual((work / "target" / pack.TARGET / "release/nexa-desktop.exe").read_bytes(), b"MZ:exact main bytes")
            config = json.loads((project / "tauri.conf.json").read_text(encoding="utf-8"))
            self.assertFalse(config["bundle"]["windows"]["allowDowngrades"])
            self.assertFalse(config["bundle"]["windows"]["bundleVCRuntime"])
            self.assertEqual(config["bundle"]["windows"]["webviewInstallMode"], {"type": "skip"})
            nsis = json.loads((project / "nsis.json").read_text(encoding="utf-8"))
            self.assertEqual(set(nsis["bundle"]["resources"].values()), {item["path"] for item in before} - {"nexa-desktop.exe"})
            msi = json.loads((project / "msi.json").read_text(encoding="utf-8"))
            self.assertEqual(msi["bundle"]["resources"], {})
            self.assertEqual(msi["bundle"]["windows"]["wix"]["componentGroupRefs"], ["NexaPayloadGroup"])
            self.assertNotIn("[dependencies]", (project / "Cargo.toml").read_text(encoding="utf-8"))

    def test_guard_inventory_is_bounded_and_rejects_alias_or_traversal(self):
        with tempfile.TemporaryDirectory() as temporary:
            work = Path(temporary)
            for paths in ([], ["../user-data"], ["models/../config.toml"], ["main.exe", "MAIN.EXE"]):
                with self.subTest(paths=paths), self.assertRaises(ValueError):
                    pack.write_inventory(work, [{"path": p} for p in paths])
            pack.write_inventory(work, [{"path": "runtime/worker.exe"}])
            text = (work / "inventory.h").read_text(encoding="ascii")
            self.assertIn('L"runtime\\\\worker.exe"', text)
            self.assertIn('L"uninstall.exe"', text)
            self.assertNotIn("models", text)

    def test_nsis_source_paths_are_quoted_without_expanding_variables(self):
        self.assertEqual(pack.nsis_string('C:\\path $HOME\\a"b'), 'C:\\path $$HOME\\a$\\"b')
        for value in ("x\ny", "x\x00y", "x\ry"):
            with self.assertRaises(ValueError): pack.nsis_string(value)

    def test_release_requires_both_tauri_formats_and_native_lifecycle(self):
        import release_windows as release
        self.assertEqual(pack.BUILD_CHECKS, release.BUILD_CHECKS)
        self.assertTrue({"legacy_msi_migration", "msi_same_version_upgrade", "nsis_same_version_upgrade",
                         "cross_format_blocked", "autostart_lifecycle"} <= release.LIFECYCLE_CHECKS)


class TauriPublicationTests(unittest.TestCase):
    def fixture(self, root):
        payload = root / "payload"
        payload.mkdir()
        (payload / "nexa-desktop.exe").write_bytes(b"MZ exact source bytes")
        (payload / "manifest.json").write_text('{"fixture":true}', encoding="utf-8")
        files = pack.base.entries(payload)
        paths = dict(payload=payload, work=root / "work", report=root / "out/report.json",
                     msi=root / "out/Nexa.msi", setup=root / "out/Nexa.exe")
        return paths, files

    def run_create(self, paths, files, bundle_failure=None, mutate=False):
        observed = []
        def helpers(work, inventory):
            work.mkdir()
            outputs = {role: work / (role + ".bin") for role in ("guard", "os", "check")}
            for path in outputs.values(): path.write_bytes(b"helper fixture")
            return outputs
        def bundle(project, kind, work, version):
            for name in ("msi", "setup", "report"):
                self.assertFalse(paths[name].exists(), "published before all bundles passed")
            observed.append(kind)
            if kind == bundle_failure:
                raise ValueError("simulated bundler failure")
            output = work / (kind + ".built")
            output.write_bytes(("built " + kind).encode("ascii"))
            if mutate:
                (paths["payload"] / "nexa-desktop.exe").write_bytes(b"unexpected change")
            return output
        with mock.patch.object(pack.sys, "platform", "win32"), \
             mock.patch.object(pack.legacy, "validate_payload", return_value=(paths["payload"], {"project_commit": "a" * 40}, files)), \
             mock.patch.object(pack, "compile_helpers", side_effect=helpers), \
             mock.patch.object(pack, "bundle", side_effect=bundle):
            result = pack.create(version="0.2.3", **paths)
        return result, observed

    def test_every_path_overlap_is_rejected_before_build_or_writes(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            paths, files = self.fixture(root)
            before = pack.base.entries(root)
            for left, right in itertools.combinations(paths, 2):
                for contained in (False, True):
                    candidates = dict(paths)
                    candidates[right] = paths[left] / "child" if contained else paths[left]
                    with self.subTest(left=left, right=right, contained=contained), \
                         mock.patch.object(pack.sys, "platform", "win32"), \
                         mock.patch.object(pack.legacy, "validate_payload") as validate, \
                         mock.patch.object(pack, "compile_helpers") as compile_helpers:
                        with self.assertRaisesRegex(ValueError, "distinct and non-overlapping"):
                            pack.create(version="0.2.3", **candidates)
                        validate.assert_not_called()
                        compile_helpers.assert_not_called()
                        self.assertEqual(pack.base.entries(root), before)

    def test_existing_targets_and_symlink_parents_are_not_modified(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            paths, _ = self.fixture(root)
            for name in ("msi", "setup", "report", "work"):
                target = paths[name]
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(b"user-owned sentinel")
                with self.subTest(name=name), self.assertRaisesRegex(ValueError, "must be new"):
                    pack.checked_layout(paths["payload"], paths["work"], paths["report"], paths["msi"], paths["setup"], ("msi", "nsis"))
                self.assertEqual(target.read_bytes(), b"user-owned sentinel")
                target.unlink()
            alias = root / "alias"
            try:
                alias.symlink_to(paths["payload"], target_is_directory=True)
            except OSError:
                return  # Windows without symlink privileges still runs sentinel checks.
            with self.assertRaisesRegex(ValueError, "symlink/reparse"):
                pack.checked_layout(paths["payload"], paths["work"], alias / "report.json", paths["msi"], paths["setup"], ("msi", "nsis"))

    def test_bundles_and_input_verification_finish_before_publishing(self):
        with tempfile.TemporaryDirectory() as temp:
            paths, files = self.fixture(Path(temp))
            result, observed = self.run_create(paths, files)
            self.assertEqual(observed, ["msi", "nsis"])
            self.assertEqual(paths["msi"].read_bytes(), b"built msi")
            self.assertEqual(paths["setup"].read_bytes(), b"built nsis")
            self.assertEqual(pack.base.entries(paths["payload"]), files)
            self.assertEqual(json.loads(paths["report"].read_text(encoding="utf-8")), result)
            self.assertEqual(result["setup_sha256"], pack.base.digest(paths["setup"]))

    def test_second_bundle_or_input_check_failure_publishes_nothing(self):
        for failure in ("nsis", "payload"):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as temp:
                paths, files = self.fixture(Path(temp))
                with self.assertRaises(ValueError):
                    self.run_create(paths, files, bundle_failure="nsis" if failure == "nsis" else None,
                                    mutate=failure == "payload")
                for name in ("msi", "setup", "report"):
                    self.assertFalse(paths[name].exists())

    def test_failed_publish_cleans_partial_outputs_and_never_overwrites_racing_file(self):
        for failure in ("copy", "race"):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                first, second = root / "first.source", root / "second.source"
                first.write_bytes(b"first"); second.write_bytes(b"second")
                targets = [root / "out/first", root / "out/second"]
                records = [(source, dest, pack.base.digest(source)) for source, dest in zip((first, second), targets)]
                if failure == "race":
                    targets[1].parent.mkdir()
                    targets[1].write_bytes(b"external writer")
                copier = pack.shutil.copyfileobj
                calls = 0
                def partial_copy(source, destination):
                    nonlocal calls
                    calls += 1
                    if calls == 2:
                        destination.write(b"partial")
                        raise OSError("simulated disk error")
                    return copier(source, destination)
                with ExitStack() as stack:
                    if failure == "copy": stack.enter_context(mock.patch.object(pack.shutil, "copyfileobj", side_effect=partial_copy))
                    with self.assertRaises(OSError): pack.publish_outputs(records)
                self.assertFalse(targets[0].exists())
                if failure == "copy": self.assertFalse(targets[1].exists())
                else: self.assertEqual(targets[1].read_bytes(), b"external writer")

    def test_compiler_environment_is_rejected_before_creating_work(self):
        with tempfile.TemporaryDirectory() as temp:
            work = Path(temp) / "must-not-exist"
            for key in pack.IMPLICIT_COMPILER_FLAGS:
                with self.subTest(key=key), mock.patch.dict(os.environ, {key: "/injected"}, clear=True):
                    with self.assertRaisesRegex(ValueError, "installer refuses implicit"):
                        pack.compile_helpers(work, [{"path": "nexa-desktop.exe"}])
                    self.assertFalse(work.exists())

    def test_import_inspection_rejects_crt_unknown_and_empty_for_both_toolchains(self):
        for dumpbin in (None, "dumpbin.exe"):
            for names, accepted in ((["KERNEL32.dll", "msi.dll"], True),
                                    (["KERNEL32.dll", "VCRUNTIME140.dll"], False),
                                    (["KERNEL32.dll", "unexpected.dll"], False), ([], False)):
                text = "\n".join(names if dumpbin else ["    DLL Name: " + name for name in names])
                with self.subTest(dumpbin=dumpbin, names=names), mock.patch.object(pack.base, "command", return_value=text):
                    if accepted:
                        self.assertEqual(pack.verify_helper_imports(Path("helper.exe"), {}, dumpbin), ["kernel32.dll", "msi.dll"])
                    else:
                        with self.assertRaises(ValueError): pack.verify_helper_imports(Path("helper.exe"), {}, dumpbin)


if __name__ == "__main__": unittest.main()
