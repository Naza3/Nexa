import importlib.util
import contextlib
import io
import json
import os
import struct
import sys
from pathlib import Path
import tempfile
import unittest
from unittest import mock
from test_license_bundle import compact_fixture

spec = importlib.util.spec_from_file_location("package_windows", Path(__file__).with_name("package_windows.py"))
pack = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pack)


class PackageTests(unittest.TestCase):
    def test_structured_command_separates_dependency_progress_from_json(self):
        child = [sys.executable, "-c", "import sys; print('Downloading crates ...', file=sys.stderr); print('{\"packages\": []}')"]
        diagnostics = io.StringIO()
        with contextlib.redirect_stderr(diagnostics):
            metadata = json.loads(pack.command(child, merge_stderr=False))
        self.assertEqual(metadata, {"packages": []})
        self.assertIn("Downloading crates ...", diagnostics.getvalue())

    def test_structured_command_rejects_nonzero_exit_with_valid_json(self):
        child = [sys.executable, "-c", "import sys; print('{}'); print('dependency download failed', file=sys.stderr); sys.exit(7)"]
        with self.assertRaises(ValueError) as failure:
            json.loads(pack.command(child, merge_stderr=False))
        self.assertIn("command failed (7)", str(failure.exception))
        self.assertIn("dependency download failed", str(failure.exception))

    def test_structured_command_does_not_recover_invalid_stdout_from_stderr(self):
        child = [sys.executable, "-c", "import sys; print('not JSON'); print('{}', file=sys.stderr)"]
        diagnostics = io.StringIO()
        with contextlib.redirect_stderr(diagnostics), self.assertRaises(json.JSONDecodeError):
            json.loads(pack.command(child, merge_stderr=False))
        self.assertEqual(diagnostics.getvalue().strip(), "{}")

    def test_default_command_still_returns_combined_output(self):
        child = [sys.executable, "-c", "import sys; print('stdout diagnostic'); print('stderr diagnostic', file=sys.stderr)"]
        output = pack.command(child)
        self.assertIn("stdout diagnostic", output)
        self.assertIn("stderr diagnostic", output)

    def test_native_archive_contract_matches_locked_native_consumer(self):
        import re
        consumer = (pack.ROOT / "crates/llama-adapter/native_identity.rs").read_text(encoding="utf-8")
        names = re.search(r"pub const LIBRARIES: \[&str; 10\] = \[(.*?)\];", consumer, re.S)
        self.assertIsNotNone(names)
        self.assertEqual(tuple(re.findall(r'"([^"\n]+)"', names[1])), pack.NATIVE_ARCHIVES)
        self.assertEqual((pack.WORKER_PROTOCOL_VERSION, pack.SHIM_VERSION), (4, 4))

    def test_native_archive_closure_rejects_missing_extra_empty_and_video(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            fields = {"MTMD_VIDEO": "OFF"}
            for name in pack.NATIVE_ARCHIVES:
                path = root / (name + ".lib")
                path.write_bytes(b"synthetic archive metadata")
                fields["library." + name] = str(path)
            records = pack.native_archive_records(root, fields)
            self.assertEqual({record["name"] for record in records}, set(pack.NATIVE_ARCHIVES))
            for missing in ("mtmd", "vendor-hash"):
                bad = fields.copy()
                del bad["library." + missing]
                with self.assertRaisesRegex(ValueError, "closure"):
                    pack.native_archive_records(root, bad)
            for value in ("ON", None):
                with self.assertRaisesRegex(ValueError, "MTMD_VIDEO"):
                    pack.native_archive_records(root, {**fields, "MTMD_VIDEO": value})
            with self.assertRaisesRegex(ValueError, "closure"):
                pack.native_archive_records(root, {**fields, "library.extra": fields["library.mtmd"]})
            (root / "mtmd.lib").write_bytes(b"")
            with self.assertRaisesRegex(ValueError, "empty"):
                pack.native_archive_records(root, fields)

    def test_multimodal_embedded_licenses_survive_bundle_byte_for_byte(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            records = []
            pack.copy_embedded_vendor_licenses(root, records)
            self.assertEqual(len(records), 2)
            original = {}
            for record in records:
                source = pack.ROOT / "vendor/llama.cpp" / record["source"].split(pack.LLAMA_COMMIT + "/")[1]
                raw = (root / record["path"]).read_bytes()
                self.assertIn(raw, source.read_bytes())
                self.assertEqual(record["source_sha256"], pack.digest(source))
                self.assertEqual(record["sha256"], pack.digest(root / record["path"]))
                self.assertIn(b"Permission is hereby granted", raw)
                self.assertIn(b"Public Domain", raw)
                self.assertIn(b"THE SOFTWARE IS PROVIDED", raw)
                self.assertTrue(raw.endswith(b"*/"))
                original[record["path"]] = raw
            self.assertIn(b"Copyright (c) 2017 Sean Barrett", original["licenses/llama.cpp/stb-image-LICENSE.txt"])
            self.assertIn(b"Copyright 2026 David Reid", original["licenses/llama.cpp/miniaudio-LICENSE.txt"])
            pack.write_json(root / "licenses/index.json", {"files": records})
            pack.consolidate_licenses(root)
            restored = pack.verify_license_bundle(root)
            for name, raw in original.items():
                self.assertEqual(restored[name]["raw"], raw)
                self.assertEqual(restored[name]["document"]["attributions"][0]["source_sha256"], next(record["source_sha256"] for record in records if record["path"] == name))

    def test_missing_embedded_license_fails_closed(self):
        files = ("vendor/stb/stb_image.h", "vendor/miniaudio/miniaudio.h")
        for bad in files:
            with self.subTest(file=bad), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                for file in files:
                    source = root / "vendor/llama.cpp" / file
                    source.parent.mkdir(parents=True, exist_ok=True)
                    source.write_bytes(b"source without license" if file == bad else (pack.ROOT / "vendor/llama.cpp" / file).read_bytes())
                with mock.patch.object(pack, "ROOT", root), self.assertRaisesRegex(ValueError, "original embedded"):
                    pack.copy_embedded_vendor_licenses(root / "stage", [])

    def vs_fixture(self, root, major=18, edition="Community", version=None, tools=None, redist=None):
        version = version or ("18.0.10000.0" if major == 18 else "17.14.37710.0")
        tools = tools or ("14.50.35707" if major == 18 else "14.44.35207")
        redist = redist or ("14.50.35710" if major == 18 else "14.44.35112")
        vs = root / f"VS{major}" / edition
        sdk = root / "Windows Kits/10"
        sdk_version = "10.0.26100.0"
        tool_dir = vs / "VC/Tools/MSVC" / tools
        redist_dir = vs / "VC/Redist/MSVC" / redist
        crt = redist_dir / "x64" / ("Microsoft.VC" + pack.msvc_toolset(redist)[1:] + ".CRT")
        files = [vs / "Common7/Tools/VsDevCmd.bat", crt / "vcruntime140.dll",
                 root / "Microsoft Visual Studio/Installer/vswhere.exe",
                 sdk / f"Include/{sdk_version}/um/Windows.h", sdk / f"Lib/{sdk_version}/um/x64/kernel32.lib",
                 sdk / f"Lib/{sdk_version}/ucrt/x64/ucrt.lib"]
        files.extend(tool_dir / "bin/Hostx64/x64" / name for name in ("cl.exe", "link.exe", "lib.exe", "dumpbin.exe"))
        for path in files:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b"fixture, never executed")
        selected = {"installationVersion": version, "installationPath": str(vs), "instanceId": f"vs{major}-{edition}",
                    "productId": "Microsoft.VisualStudio.Product." + edition, "isPrerelease": False, "isComplete": True}
        env = {"VSINSTALLDIR": str(vs), "VCTOOLSVERSION": tools, "VCTOOLSINSTALLDIR": str(tool_dir),
               "VCTOOLSREDISTDIR": str(redist_dir), "WINDOWSSDKVERSION": sdk_version + "\\", "WINDOWSSDKDIR": str(sdk), "PATH": str(tool_dir / "bin/Hostx64/x64")}
        return selected, env

    def vs_where(self, args, env):
        self.assertEqual(args[0], "where.exe")
        return str(Path(env["VCTOOLSINSTALLDIR"]) / "bin/Hostx64/x64" / args[1])

    def test_existing_vs2026_vs2022_and_build_tools_are_used_without_installing(self):
        for major, edition in ((18, "Community"), (17, "Enterprise"), (18, "BuildTools"), (17, "BuildTools")):
            with self.subTest(major=major, edition=edition), tempfile.TemporaryDirectory() as folder:
                selected, env = self.vs_fixture(Path(folder), major, edition)
                def run(args, actual_env):
                    if args[0] == "where.exe":
                        return self.vs_where(args, actual_env)
                    self.assertEqual(Path(args[0]).name, "vswhere.exe")
                    self.assertNotIn("-latest", args)
                    self.assertNotIn("-version", args)
                    self.assertNotIn("-prerelease", args)
                    self.assertEqual(args[args.index("-products") + 1], "*")
                    self.assertNotIn("-requires", args)
                    return json.dumps([selected])
                with mock.patch.dict(pack.os.environ, {"PROGRAMFILES(X86)": folder}, clear=True), mock.patch.object(pack, "command", side_effect=run), mock.patch.object(pack, "devcmd_environment", return_value=env) as dev:
                    actual, vs, child, folded, crt = pack.selected_visual_studio()
                self.assertEqual(actual, selected)
                self.assertEqual(vs, Path(selected["installationPath"]).resolve())
                self.assertEqual(dev.call_args.args[0], Path(selected["installationPath"]) / "Common7/Tools/VsDevCmd.bat")
                self.assertEqual(child, folded)
                self.assertIsNot(child, folded)
                self.assertEqual(set(crt), {"vcruntime140.dll"})
                self.assertIn("Microsoft.VC145.CRT" if major == 18 else "Microsoft.VC143.CRT", str(crt["vcruntime140.dll"]))

    def test_explicit_cmake_selection_survives_every_vs_environment_reentry(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            selected, developer_env = self.vs_fixture(root)
            selected_bin, bundled_bin = root / "Python Scripts", root / "VS bundled CMake"
            for directory in (selected_bin, bundled_bin):
                directory.mkdir()
                for name in ("cmake.exe", "ctest.exe"):
                    (directory / name).write_bytes(b"tool selection fixture, never executed")
            developer_env["PATH"] = str(bundled_bin) + os.pathsep + developer_env["PATH"]
            initial = {"NEXA_CMAKE_BIN": str(selected_bin), "PATH": str(selected_bin)}
            with mock.patch.object(pack, "devcmd_environment", side_effect=lambda *_: developer_env.copy()):
                first = pack.visual_studio_paths(selected, initial)[2]
                second = pack.visual_studio_paths(selected, first)[2]
            for env in (first, second):
                self.assertEqual(env["NEXA_CMAKE_BIN"], str(selected_bin))
                self.assertEqual(env["PATH"].split(os.pathsep)[0], str(selected_bin))
                for name in ("cmake.exe", "ctest.exe"):
                    self.assertEqual(pack.shutil.which(name, path=env["PATH"], mode=os.F_OK), str(selected_bin / name))

    def test_resolved_directory_aliases_preserve_vs_tools_and_crt_identity(self):
        # This is a real filesystem alias on Linux too, not a mocked 8.3 map.
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder).resolve()
            selected, env = self.vs_fixture(root)
            (root / "existing parent").mkdir()
            alias = root / "existing parent" / ".."
            fields = ("installationPath", "VSINSTALLDIR", "VCTOOLSINSTALLDIR", "VCTOOLSREDISTDIR", "PATH")
            for changed in (*[(field,) for field in fields], fields):
                candidate, child_env = selected.copy(), env.copy()
                for field in changed:
                    target = candidate if field == "installationPath" else child_env
                    target[field] = str(alias / Path(target[field]).relative_to(root))
                with self.subTest(changed=changed), mock.patch.object(pack, "devcmd_environment", return_value=child_env):
                    _, vs, _, _, crt = pack.visual_studio_paths(candidate, {})
                self.assertEqual(vs, Path(selected["installationPath"]))
                self.assertEqual(crt["vcruntime140.dll"], Path(env["VCTOOLSREDISTDIR"]) / "x64/Microsoft.VC145.CRT/vcruntime140.dll")
                self.assertEqual(crt["vcruntime140.dll"].relative_to(vs).parts[:3], ("VC", "Redist", "MSVC"))

    def test_missing_parent_is_not_an_existing_directory_alias(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder).resolve()
            selected, env = self.vs_fixture(root)
            for field in ("VSINSTALLDIR", "VCTOOLSINSTALLDIR", "VCTOOLSREDISTDIR"):
                child_env = env.copy()
                child_env[field] = str(root / "missing parent" / ".." / Path(env[field]).relative_to(root))
                with self.subTest(field=field), mock.patch.object(pack, "devcmd_environment", return_value=child_env), self.assertRaisesRegex(ValueError, "does not exist before resolution"):
                    pack.visual_studio_paths(selected, {})

    def test_external_existing_crt_remains_outside_after_alias_resolution(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder).resolve()
            selected, env = self.vs_fixture(root)
            _, external = self.vs_fixture(root / "other installation")
            (root / "existing parent").mkdir()
            for source in (Path(external["VCTOOLSREDISTDIR"]), root / "existing parent" / ".." / Path(external["VCTOOLSREDISTDIR"]).relative_to(root)):
                with self.subTest(source=source), mock.patch.object(pack, "devcmd_environment", return_value=dict(env, VCTOOLSREDISTDIR=str(source))), self.assertRaisesRegex(ValueError, "outside the selected Visual Studio redistribution"):
                    pack.visual_studio_paths(selected, {})

    @unittest.skipUnless(os.name == "nt", "requires real Windows 8.3 filesystem aliases")
    def test_real_windows_short_and_long_directory_aliases(self):
        import ctypes
        from ctypes import wintypes
        get_short_path = ctypes.WinDLL("kernel32", use_last_error=True).GetShortPathNameW
        get_short_path.argtypes = (wintypes.LPCWSTR, wintypes.LPWSTR, wintypes.DWORD)
        get_short_path.restype = wintypes.DWORD
        def short_path(path):
            needed = get_short_path(str(path), None, 0)
            if not needed:
                raise ctypes.WinError(ctypes.get_last_error())
            result = ctypes.create_unicode_buffer(needed)
            written = get_short_path(str(path), result, needed)
            if not written or written >= needed:
                raise ctypes.WinError(ctypes.get_last_error())
            return Path(result.value)
        with tempfile.TemporaryDirectory(prefix="nexa long directory alias-") as folder:
            root = Path(folder).resolve()
            selected, env = self.vs_fixture(root)
            if short_path(root) == root:
                self.skipTest("filesystem does not provide a distinct 8.3 name for the test directory")
            self.assertTrue(short_path(root).samefile(root))
            for mode in ("selected-short", "environment-short", "crt-short", "all-short"):
                candidate, child_env = selected.copy(), env.copy()
                if mode in ("selected-short", "all-short"):
                    candidate["installationPath"] = str(short_path(Path(candidate["installationPath"])))
                fields = ("VCTOOLSREDISTDIR",) if mode == "crt-short" else ("VSINSTALLDIR", "VCTOOLSINSTALLDIR", "VCTOOLSREDISTDIR", "WINDOWSSDKDIR", "PATH")
                if mode != "selected-short":
                    for field in fields:
                        child_env[field] = str(short_path(Path(child_env[field])))
                with self.subTest(mode=mode), mock.patch.object(pack, "devcmd_environment", return_value=child_env):
                    _, vs, _, _, crt = pack.visual_studio_paths(candidate, {})
                self.assertEqual(vs, Path(selected["installationPath"]))
                self.assertTrue(crt["vcruntime140.dll"].is_relative_to(vs))
                self.assertEqual(crt["vcruntime140.dll"], Path(env["VCTOOLSREDISTDIR"]) / "x64/Microsoft.VC145.CRT/vcruntime140.dll")

    def assert_linked_directory_sources_rejected(self, make_link):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder).resolve()
            selected, env = self.vs_fixture(root)
            for field in ("installationPath", "VSINSTALLDIR", "VCTOOLSINSTALLDIR", "VCTOOLSREDISTDIR"):
                candidate, child_env = selected.copy(), env.copy()
                target = candidate if field == "installationPath" else child_env
                link = root / "linked directory"
                make_link(link, Path(target[field]))
                target[field] = str(link)
                try:
                    with self.subTest(field=field), mock.patch.object(pack, "devcmd_environment", return_value=child_env), self.assertRaisesRegex(ValueError, "symlink/reparse"):
                        pack.visual_studio_paths(candidate, {})
                finally:
                    if link.is_symlink():
                        link.unlink()
                    else:
                        link.rmdir()

    @unittest.skipIf(os.name == "nt", "real Windows junction coverage does not require symlink privileges")
    def test_original_symlink_directory_aliases_are_rejected_before_resolution(self):
        self.assert_linked_directory_sources_rejected(lambda link, target: link.symlink_to(target, target_is_directory=True))

    @unittest.skipUnless(os.name == "nt", "requires real Windows junctions")
    def test_real_windows_junction_directory_aliases_are_rejected_before_resolution(self):
        def junction(link, target):
            host = Path(os.environ["SYSTEMROOT"]) / "System32/cmd.exe"
            pack.command([host, "/d", "/c", "mklink", "/J", link, target])
        self.assert_linked_directory_sources_rejected(junction)

    def test_explicit_cmake_directory_is_absolute_and_contains_both_tools(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            selected, env = self.vs_fixture(root)
            with mock.patch.object(pack, "devcmd_environment", return_value=env.copy()), self.assertRaisesRegex(ValueError, "must be absolute"):
                pack.visual_studio_paths(selected, {"NEXA_CMAKE_BIN": "relative-cmake"})
            for present in ((), ("cmake.exe",)):
                directory = root / ("empty" if not present else "missing-ctest")
                directory.mkdir()
                for name in present:
                    (directory / name).write_bytes(b"fixture")
                with self.subTest(present=present), mock.patch.object(pack, "devcmd_environment", return_value=env.copy()), self.assertRaises((OSError, ValueError)):
                    pack.visual_studio_paths(selected, {"NEXA_CMAKE_BIN": str(directory)})

    def test_newest_usable_stable_instance_wins_and_broken_newer_instance_falls_back(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            old, old_env = self.vs_fixture(root, 17)
            new, new_env = self.vs_fixture(root, 18, version="18.10.10000.0")
            older18, _ = self.vs_fixture(root, 18, "Professional", version="18.9.10000.0")
            ignored = [dict(new, installationVersion="19.0.0.0"), dict(new, isPrerelease=True, installationVersion="18.11.0.0"), dict(new, isComplete=False, installationVersion="18.12.0.0")]
            choices = [old, older18, *ignored, new]
            with mock.patch.object(Path, "exists", return_value=True), mock.patch.object(pack, "regular", side_effect=lambda p: p), mock.patch.object(pack, "command", return_value=json.dumps(choices)), mock.patch.object(pack, "visual_studio_paths", return_value="newest") as initialize:
                self.assertEqual(pack.selected_visual_studio(), "newest")
                self.assertEqual(initialize.call_args.args[0], new)
            # The newest installed C++ environment may be unusable (e.g. missing
            # SDK/CRT). Try the next stable installed environment before advising setup.
            with mock.patch.object(Path, "exists", return_value=True), mock.patch.object(pack, "regular", side_effect=lambda p: p), mock.patch.object(pack, "command", return_value=json.dumps([old, new])), mock.patch.object(pack, "visual_studio_paths", side_effect=[ValueError("missing SDK"), (old, old_env)]) as initialize:
                self.assertEqual(pack.selected_visual_studio(), (old, old_env))
                self.assertEqual([c.args[0] for c in initialize.call_args_list], [new, old])

    def test_missing_cpp_no_vs_and_missing_discovery_have_actionable_setup_messages(self):
        for installed, message in (([{"installationVersion": "18.0.0.0"}], "Existing Visual Studio"), ([], "No existing Visual Studio")):
            with self.subTest(installed=installed), mock.patch.object(Path, "exists", return_value=True), mock.patch.object(pack, "regular", side_effect=lambda p: p), mock.patch.object(pack, "command", side_effect=["[]", json.dumps(installed)]) as run, mock.patch.object(pack, "devcmd_environment") as dev:
                with self.assertRaisesRegex(ValueError, message) as error:
                    pack.selected_visual_studio()
                self.assertIn("Visual Studio Installer > Modify", str(error.exception))
                self.assertIn("https://aka.ms/vs/17/release/vs_buildtools.exe", str(error.exception))
                self.assertIn("does not download or install", str(error.exception))
                self.assertIn("-all", run.call_args.args[0])
                dev.assert_not_called()
        with mock.patch.object(Path, "exists", return_value=False), mock.patch.object(pack, "command") as run, self.assertRaisesRegex(ValueError, "vswhere.exe is missing"):
            pack.selected_visual_studio()
        run.assert_not_called()

    def test_preview_incomplete_unknown_and_invalid_vs_versions_are_never_initialized(self):
        entries = [{"installationVersion": version, "isComplete": complete, "isPrerelease": preview}
                   for version, complete, preview in (("18.0.0.0", True, True), ("18.0.0.0", False, False), ("19.0.0.0", True, False), ("bad", True, False), ("17.0.0.0-preview", True, False))]
        with mock.patch.object(Path, "exists", return_value=True), mock.patch.object(pack, "regular", side_effect=lambda p: p), mock.patch.object(pack, "command", return_value=json.dumps(entries)), mock.patch.object(pack, "visual_studio_paths") as initialize, self.assertRaisesRegex(ValueError, "preview/incomplete/unknown"):
            pack.selected_visual_studio()
        initialize.assert_not_called()

    def test_old_developer_prompt_state_is_cleared_only_in_child_environment(self):
        env = {"PATH": "old VS;user bin", "__VSCMD_PREINIT_PATH": "user bin", "VSCMD_VER": "17.0", "VSINSTALLDIR": "other VS", "VCTOOLSREDISTDIR": "stale", "VCTOOLSVERSION": "14.44.0", "LIB": "stale", "INCLUDE": "stale", "WINDOWSSDKDIR": "stale", "KEEP": "value"}
        before = env.copy()
        self.assertEqual(pack.visual_studio_environment(env), {"PATH": "user bin", "KEEP": "value"})
        self.assertEqual(env, before)

    def test_redistribution_references_follow_the_selected_visual_studio(self):
        for version, year in (("17.14.37710.0", "2022"), ("18.0.10000.0", "2026")):
            for edition in ("Community", "BuildTools"):
                selected = {"installationVersion": version, "productId": "Microsoft.VisualStudio.Product." + edition}
                metadata = pack.visual_studio_redistribution(selected)
                self.assertIn("/" + year + "/redistribution", metadata["redist_list"])
                self.assertIn(year, metadata["community_terms_reference"])
                self.assertEqual(metadata["edition"], selected["productId"])
                self.assertEqual(metadata["license_terms_directory"], "https://visualstudio.microsoft.com/license-terms/")
                self.assertFalse(metadata["license_acceptance_performed"])
        with self.assertRaises(ValueError):
            pack.visual_studio_redistribution({"installationVersion": "19.0.0.0"})

    def test_selected_tools_sdk_and_crt_cannot_be_inherited_from_another_instance(self):
        for change, message in (({"VSINSTALLDIR": "other"}, "different Visual Studio"), ({"VCTOOLSINSTALLDIR": "other"}, "outside the selected"), ({"VCTOOLSREDISTDIR": "other"}, "outside the selected"), ({"VCTOOLSREDISTDIR": ""}, "no VCToolsRedistDir"), ({"WINDOWSSDKVERSION": ""}, "no Windows 10/11 SDK"), ({"VCTOOLSVERSION": "14.60.12345"}, "unsupported MSVC")):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as folder:
                selected, env = self.vs_fixture(Path(folder))
                env.update(change)
                with mock.patch.object(pack, "devcmd_environment", return_value=env), mock.patch.object(pack, "command", side_effect=self.vs_where), self.assertRaisesRegex(ValueError, message):
                    pack.visual_studio_paths(selected, {})
        with tempfile.TemporaryDirectory() as folder:
            selected, env = self.vs_fixture(Path(folder))
            with mock.patch.object(pack, "devcmd_environment", return_value=dict(env, PATH=str(Path(folder) / "other"))), self.assertRaisesRegex(ValueError, "cl.exe is not from"):
                pack.visual_studio_paths(selected, {})
            (Path(env["WINDOWSSDKDIR"]) / "Lib/10.0.26100.0/um/x64/kernel32.lib").unlink()
            with mock.patch.object(pack, "devcmd_environment", return_value=env), mock.patch.object(pack, "command", side_effect=self.vs_where), self.assertRaises(FileNotFoundError):
                pack.visual_studio_paths(selected, {})

    def test_unicode_tool_paths_use_child_path_and_reject_current_directory_shadowing(self):
        with tempfile.TemporaryDirectory(prefix="nexa-中文 tools-") as folder:
            root = Path(folder)
            selected, env = self.vs_fixture(root)
            vs = Path(selected["installationPath"])
            expected = Path(env["PATH"]) / "cl.exe"
            with mock.patch.object(pack, "command") as command:
                self.assertEqual(pack.selected_msvc_tool(vs, env, "cl.exe"), expected)
                command.assert_not_called()
            project = root / "project"
            project.mkdir()
            (project / "cl.exe").write_bytes(b"shadow, never executed")
            with mock.patch.object(pack, "ROOT", project), self.assertRaisesRegex(ValueError, "shadowed"):
                pack.selected_msvc_tool(vs, env, "cl.exe")

    def test_vs2026_v143_side_by_side_tools_can_use_its_newer_release_crt(self):
        with tempfile.TemporaryDirectory() as folder:
            selected, env = self.vs_fixture(Path(folder), tools="14.44.35207", redist="14.50.35710")
            with mock.patch.object(pack, "devcmd_environment", return_value=env), mock.patch.object(pack, "command", side_effect=self.vs_where):
                _, _, _, _, crt = pack.visual_studio_paths(selected, {})
            self.assertIn("Microsoft.VC145.CRT", str(crt["vcruntime140.dll"]))
        for version, expected in (("14.39.33519", "v143"), ("14.44.35207", "v143"), ("14.50.35707", "v145"), ("14.51.36231", "v145")):
            self.assertEqual(pack.msvc_toolset(version), expected)
        for version in ("14.29.0", "14.60.0", "14.50.1-preview", "14.50/../1", ""):
            with self.subTest(version=version), self.assertRaises(ValueError):
                pack.msvc_toolset(version)

    def test_crt_directory_must_be_regular_release_x64_and_current(self):
        for mode, message in (("debug", "missing/invalid"), ("onecore", "missing/invalid"), ("old", "older than"), ("no_dll", "no existing Release CRT"), ("wrong_arch", "missing/invalid")):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as folder:
                selected, env = self.vs_fixture(Path(folder))
                root = Path(env["VCTOOLSREDISTDIR"])
                crt = root / "x64/Microsoft.VC145.CRT"
                if mode in ("debug", "onecore"):
                    env["VCTOOLSREDISTDIR"] = str(root / mode)
                elif mode == "old":
                    env["VCTOOLSREDISTDIR"] = str(root.parent / "14.44.35112")
                elif mode == "no_dll":
                    (crt / "vcruntime140.dll").unlink()
                else:
                    (root / "x64").rename(root / "arm64")
                with mock.patch.object(pack, "devcmd_environment", return_value=env), mock.patch.object(pack, "command", side_effect=self.vs_where), self.assertRaisesRegex(ValueError, message):
                    pack.visual_studio_paths(selected, {})

    @unittest.skipIf(os.name == "nt", "symlink fixtures require developer-mode/admin permission on Windows")
    def test_crt_rejects_symlink_ancestors_and_case_duplicate_dlls(self):
        with tempfile.TemporaryDirectory() as folder:
            selected, env = self.vs_fixture(Path(folder))
            root = Path(env["VCTOOLSREDISTDIR"])
            crt = root / "x64/Microsoft.VC145.CRT"
            (crt / "VCRUNTIME140.dll").write_bytes(b"duplicate")
            with mock.patch.object(pack, "devcmd_environment", return_value=env), mock.patch.object(pack, "command", side_effect=self.vs_where), self.assertRaisesRegex(ValueError, "duplicate CRT"):
                pack.visual_studio_paths(selected, {})
            (crt / "VCRUNTIME140.dll").unlink()
            outside = Path(folder) / "outside"
            root.rename(outside)
            root.symlink_to(outside, target_is_directory=True)
            with mock.patch.object(pack, "devcmd_environment", return_value=env), mock.patch.object(pack, "command", side_effect=self.vs_where), self.assertRaisesRegex(ValueError, "symlink/reparse"):
                pack.visual_studio_paths(selected, {})

    def test_cmake_generator_toolset_instance_and_build_cache_are_aligned(self):
        with tempfile.TemporaryDirectory() as folder, mock.patch.object(pack, "ROOT", Path(folder)):
            roots = []
            for major, version, toolset in ((17, "14.44.35207", "v143"), (18, "14.50.35707", "v145"), (18, "14.44.35207", "v143")):
                selected, env = self.vs_fixture(Path(folder), major, tools=version)
                vs = Path(selected["installationPath"])
                native, args = pack.native_build_settings(selected, vs, env)
                roots.append(native)
                self.assertEqual(args[args.index("-G") + 1], pack.VS_GENERATORS[major])
                self.assertEqual(args[args.index("-A") + 1], "x64")
                self.assertEqual(args[args.index("-T") + 1], f"{toolset},host=x64,version={version}")
                self.assertIn(f"-DCMAKE_GENERATOR_INSTANCE={vs}", args)
                self.assertIn("-DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreadedDLL", args)
                self.assertNotEqual(native, Path(folder) / "build/native-release")
                native.mkdir(parents=True)
                values = {"CMAKE_GENERATOR": pack.VS_GENERATORS[major], "CMAKE_GENERATOR_PLATFORM": "x64", "CMAKE_GENERATOR_TOOLSET": args[args.index("-T") + 1], "CMAKE_GENERATOR_INSTANCE": str(vs)}
                def save(data):
                    text = "// Fixture CMake cache\n" + "".join(f"{key}:INTERNAL={value}\n" for key, value in data.items())
                    (native / "CMakeCache.txt").write_text(text, encoding="utf-8")
                    return text
                save(values)
                self.assertEqual(pack.native_build_settings(selected, vs, env), (native, args))
                for key, value in (("CMAKE_GENERATOR", "Ninja"), ("CMAKE_GENERATOR_PLATFORM", "Win32"), ("CMAKE_GENERATOR_TOOLSET", "v143"), ("CMAKE_GENERATOR_INSTANCE", str(vs.parent / "Other"))):
                    expected = save(dict(values, **{key: value}))
                    with self.subTest(key=key), self.assertRaisesRegex(ValueError, "no files were deleted"):
                        pack.native_build_settings(selected, vs, env)
                    self.assertEqual((native / "CMakeCache.txt").read_text(encoding="utf-8"), expected)
            self.assertEqual(len(set(roots)), len(roots))
            selected, env = self.vs_fixture(Path(folder), 18, "Other")
            other, _ = pack.native_build_settings(selected, Path(selected["installationPath"]), env)
            self.assertNotIn(other, roots)
            for version in ("bad", "19.0.0.0"):
                with self.assertRaisesRegex(ValueError, "unsupported Visual Studio CMake generator"):
                    pack.native_build_settings(dict(selected, installationVersion=version), Path(selected["installationPath"]), env)

    def test_cmake_minimum_accepts_42_and_newer_versions_with_selected_generator(self):
        env = {"PATH": "selected toolchain"}
        for generator in pack.VS_GENERATORS.values():
            for version in ("4.2.0", "4.4.3", "4.4.4", "4.4.30", "4.10.0", "5.0.0", "4.4.4-vendor1"):
                output = f"cmake version {version}\n\nCMake suite maintained and supported by Kitware (kitware.com/cmake)."
                capabilities = {"generators": [{"name": "Ninja"}, {"name": generator}]}
                with self.subTest(generator=generator, version=version), mock.patch.object(pack, "command", side_effect=[output, json.dumps(capabilities)]) as run:
                    # The existing manifest field keeps the complete actual output.
                    self.assertEqual(pack.checked_cmake(env, generator), output)
                    self.assertEqual(run.call_args_list, [mock.call(["cmake", "--version"], env), mock.call(["cmake", "-E", "capabilities"], env)])

    def test_cmake_below_minimum_is_rejected_before_capability_or_build_commands(self):
        for generator in pack.VS_GENERATORS.values():
            for version in ("3.24.0", "3.99.99", "4.0.9", "4.1.99"):
                with self.subTest(generator=generator, version=version), mock.patch.object(pack, "command", return_value=f"cmake version {version}") as run, self.assertRaisesRegex(ValueError, "CMake 4.2 or newer"):
                    pack.checked_cmake({}, generator)
                run.assert_called_once_with(["cmake", "--version"], {})

    def test_cmake_malformed_version_is_rejected(self):
        for output in ("", "4.4.4", "cmake version 4.4", "cmake version 4.4.4.1", "cmake version 4.4.4oops", "cmake version ４.４.４", "cmake version 4.4.4 extra", "warning\ncmake version 4.4.4"):
            with self.subTest(output=output), mock.patch.object(pack, "command", return_value=output) as run, self.assertRaisesRegex(ValueError, "cannot parse CMake version"):
                pack.checked_cmake({}, pack.VS_GENERATORS[18])
            run.assert_called_once_with(["cmake", "--version"], {})

    def test_cmake_newer_version_still_requires_exact_selected_generator(self):
        for generators in ([], [{"name": "Visual Studio 17 2022"}], [{"name": "Visual Studio 18 2026 Preview"}]):
            with self.subTest(generators=generators), mock.patch.object(pack, "command", side_effect=["cmake version 4.4.4", json.dumps({"generators": generators})]), self.assertRaisesRegex(ValueError, "required generator: Visual Studio 18 2026"):
                pack.checked_cmake({}, pack.VS_GENERATORS[18])

    def test_cmake_malformed_capabilities_is_rejected(self):
        for capabilities in ("not JSON", "null", "[]", "{}", '{"generators": {}}', '{"generators": [null]}', '{"generators": [{}]}', '{"generators": [{"name": 18}]}'):
            with self.subTest(capabilities=capabilities), mock.patch.object(pack, "command", side_effect=["cmake version 4.4.4", capabilities]), self.assertRaisesRegex(ValueError, "CMake capabilities"):
                pack.checked_cmake({}, pack.VS_GENERATORS[18])

    def test_cmake_command_failure_is_not_hidden(self):
        with mock.patch.object(pack, "command", side_effect=["cmake version 4.4.4", ValueError("command failed (1): cmake")]), self.assertRaisesRegex(ValueError, "command failed"):
            pack.checked_cmake({}, pack.VS_GENERATORS[18])

    def test_build_wires_selected_vs2026_to_cmake_and_isolates_native_and_cargo_caches(self):
        with tempfile.TemporaryDirectory() as folder, mock.patch.object(pack, "ROOT", Path(folder)):
            root = Path(folder)
            selected, env = self.vs_fixture(root)
            vs = Path(selected["installationPath"])
            native, expected_args = pack.native_build_settings(selected, vs, env)
            legacy = root / "build/native-release/CMakeCache.txt"
            legacy.parent.mkdir(parents=True)
            legacy.write_text("CMAKE_GENERATOR:INTERNAL=Visual Studio 17 2022\n", encoding="utf-8")
            cargo_envs = []
            def run(args, child_env):
                if args[:2] == ["rustc", "-vV"]:
                    return "release: 1.98.1\nhost: x86_64-pc-windows-msvc"
                if args[:2] == ["cmake", "--version"]:
                    return "cmake version 4.4.4"
                if args == ["cmake", "-E", "capabilities"]:
                    return json.dumps({"generators": [{"name": pack.VS_GENERATORS[18]}]})
                if args[:4] == ["git", "-C", "vendor/llama.cpp", "rev-parse"]:
                    return pack.LLAMA_COMMIT
                if args[:2] == ["git", "rev-parse"]:
                    return "a" * 40
                if args[0] == "cargo":
                    cargo_envs.append(child_env.copy())
                if args[:2] == ["cmake", "-S"]:
                    self.assertEqual(args, expected_args)
                if args[:2] == ["cmake", "--build"]:
                    self.assertEqual(args[2], native)
                    raise RuntimeError("stop before any real compilation")
                return ""
            with mock.patch.object(pack.sys, "platform", "win32"), mock.patch.dict(pack.os.environ, {}, clear=True), mock.patch.object(pack, "selected_visual_studio", return_value=(selected, vs, env, env.copy(), {})), mock.patch.object(pack, "command", side_effect=run), self.assertRaisesRegex(RuntimeError, "stop before"):
                pack.build()
            self.assertTrue(cargo_envs)
            for child in cargo_envs:
                self.assertEqual(child["CARGO_TARGET_DIR"], str(root / "build/windows-x64-cpu" / ("cargo-" + native.name.removeprefix("native-"))))
            self.assertEqual(legacy.read_text(encoding="utf-8"), "CMAKE_GENERATOR:INTERNAL=Visual Studio 17 2022\n")

    def test_aws_lc_native_notices_are_retained_without_source_payload(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            names = ("LICENSE", "aws-lc/LICENSE", "aws-lc/third_party/fiat/LICENSE",
                     "aws-lc/third_party/NOTICE.txt", "aws-lc/crypto/source.c", "unrelated/src/LICENSE")
            for name in names:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("fixture", encoding="utf-8")
            self.assertEqual({path.relative_to(root).as_posix() for path in pack.license_files(root)}, set(names[:4]))

    def test_windows_environment_overrides_case_insensitively(self):
        base = {"PATH": "old path", "VCTOOLSREDISTDIR": "old redist", "ImageOS": "windows2022", "ImageVersion": "20260929", "SystemRoot": "C:/Windows"}
        output = "Path=new path\r\nVCToolsRedistDir=new redist\r\n= C:=ignored\r\nCOMPLEX=a=b=c\r\n"
        env = pack.windows_environment(base, output)
        self.assertEqual(env["PATH"], "new path")
        self.assertEqual(env["VCTOOLSREDISTDIR"], "new redist")
        self.assertEqual(env["IMAGEOS"], "windows2022")
        self.assertEqual(env["IMAGEVERSION"], "20260929")
        self.assertEqual(env["SYSTEMROOT"], "C:/Windows")
        self.assertEqual(env["COMPLEX"], "a=b=c")
        self.assertEqual(len(env), 6)
        self.assertTrue(all(key == key.upper() for key in env))
        self.assertEqual(base["PATH"], "old path")

    def test_windows_environment_removes_preexisting_case_duplicates(self):
        env = pack.windows_environment({"PATH": "first", "Path": "second", "VCToolsRedistDir": "first", "vctoolsredistdir": "second"}, "pAtH=selected\nvCtOoLsReDiStDiR=selected CRT")
        self.assertEqual(env, {"PATH": "selected", "VCTOOLSREDISTDIR": "selected CRT"})

    def test_devcmd_uses_raw_cmd_quoting_instead_of_crt_argument_escaping(self):
        cmd = r"C:\Windows\System32\cmd.exe"
        dev = r"C:\Program Files\Microsoft Visual Studio\2022\Enterprise\Common7\Tools\VsDevCmd.bat"
        line = pack.devcmd_command_line(cmd, dev)
        self.assertEqual(line, f'"{cmd}" /d /s /u /v:off /c ""{dev}" -no_logo -arch=x64 -host_arch=x64 >nul && set"')
        legacy = pack.subprocess.list2cmdline(["cmd.exe", "/d", "/s", "/c", f'""{dev}" -no_logo -arch=x64 -host_arch=x64 >nul && set"'])
        self.assertIn('\\"', legacy)
        self.assertNotIn('\\"', line)
        result = mock.Mock(returncode=0, stdout="Path=selected compiler path\r\nVCToolsRedistDir=selected redist\r\n")
        env = {"SYSTEMROOT": r"C:\Windows", "PATH": "old path"}
        with mock.patch.object(pack, "regular", side_effect=lambda path: path), mock.patch.object(pack.subprocess, "run", return_value=result) as run:
            actual = pack.devcmd_environment(dev, env)
        self.assertIsInstance(run.call_args.args[0], str)
        self.assertFalse(run.call_args.kwargs["shell"])
        self.assertEqual(run.call_args.kwargs["encoding"], "utf-16-le")
        self.assertEqual(actual["PATH"], "selected compiler path")
        self.assertEqual(actual["VCTOOLSREDISTDIR"], "selected redist")

    def test_devcmd_failure_cannot_reuse_inherited_redist_environment(self):
        result = mock.Mock(returncode=7, stdout="fixture initialization failed")
        env = {"SYSTEMROOT": r"C:\Windows", "VCTOOLSREDISTDIR": "inherited stale redist"}
        with mock.patch.object(pack, "regular", side_effect=lambda path: path), mock.patch.object(pack.subprocess, "run", return_value=result), self.assertRaisesRegex(ValueError, r"initialization failed \(7\)"):
            pack.devcmd_environment("VsDevCmd.bat", env)
        for path in ('C:/bad"path/VsDevCmd.bat', 'C:/%UNEXPECTED%/VsDevCmd.bat', 'C:/bad\npath/VsDevCmd.bat'):
            with self.subTest(path=path), self.assertRaises(ValueError):
                pack.devcmd_command_line(r"C:\Windows\System32\cmd.exe", path)

    @unittest.skipUnless(os.name == "nt", "requires the real Windows cmd.exe parser")
    def test_real_windows_devcmd_with_spaces_unicode_and_failure(self):
        with tempfile.TemporaryDirectory(prefix="nexa-devcmd-") as folder:
            directory = Path(folder) / "VS 中文 开发 (Tools) & literal!"
            directory.mkdir()
            dev = directory / "VsDevCmd.bat"
            # cmd batch slots split unquoted '=' delimiters. Preserve %* as
            # well as the actual slots so a failed assertion reports evidence,
            # rather than hiding parameter values behind an exit code.
            dev.write_bytes(b'@echo off\r\nset "NEXA_FIXTURE_ARGS=%*"\r\nset "NEXA_FIXTURE_ARG_1=%1"\r\nset "NEXA_FIXTURE_ARG_2=%2"\r\nset "NEXA_FIXTURE_ARG_3=%3"\r\nset "NEXA_FIXTURE_ARG_4=%4"\r\nset "NEXA_FIXTURE_ARG_5=%5"\r\nset "Path=%~dp0compiler"\r\nset "VCToolsRedistDir=%~dp0Redist"\r\nexit /b 0\r\n')
            env = pack.windows_environment(os.environ, "")
            actual = pack.devcmd_environment(dev, env)
            self.assertEqual(actual.get("NEXA_FIXTURE_ARGS", "").strip(), "-no_logo -arch=x64 -host_arch=x64")
            self.assertEqual([actual.get(f"NEXA_FIXTURE_ARG_{n}") for n in range(1, 6)], ["-no_logo", "-arch", "x64", "-host_arch", "x64"])
            self.assertEqual(actual["PATH"], str(directory / "compiler"))
            self.assertEqual(actual["VCTOOLSREDISTDIR"], str(directory / "Redist"))
            dev.write_bytes(b'@echo off\r\nexit /b 7\r\n')
            with self.assertRaisesRegex(ValueError, r"initialization failed \(7\)"):
                pack.devcmd_environment(dev, env)

    def test_windows_powershell_uses_only_system_host_modules_and_child_env_fix(self):
        env = {"SYSTEMROOT": "C:/Windows", "PSMODULEPATH": "C:/Program Files/PowerShell/7/Modules", "PATH": "selected VS path", "VCTOOLSREDISTDIR": "selected CRT", "KEEP_SETTING": "unchanged"}
        before = dict(env)
        with mock.patch.object(pack, "regular", side_effect=lambda path: path), mock.patch.object(pack, "command", return_value="fixture") as run:
            self.assertEqual(pack.powershell("'fixture'", env), "fixture")
        args, actual_env = run.call_args.args
        self.assertEqual(args[0], Path("C:/Windows/System32/WindowsPowerShell/v1.0/powershell.exe"))
        self.assertEqual(actual_env, {key: value for key, value in env.items() if key != "PSMODULEPATH"})
        self.assertEqual(env, before)
        script = args[-1]
        for name in ("Microsoft.PowerShell.Security", "Microsoft.PowerShell.Utility", "Microsoft.PowerShell.Management"):
            self.assertIn("Modules/" + name + "/" + name + ".psd1", script.replace("\\", "/"))
        self.assertIn("Import-Module -Name", script)
        self.assertIn("$_.FullyQualifiedErrorId", script)
        self.assertIn("$_.Exception.Message", script)
        self.assertIn("exit 1", script)
        self.assertNotIn("Set-ExecutionPolicy", script)

    def test_authenticode_requires_valid_microsoft_signature_without_fallback(self):
        for status, signer in (("NotSigned", ""), ("UnknownError", "Microsoft Corporation"), ("HashMismatch", "Microsoft Corporation"), ("Valid", "Unrelated publisher")):
            with self.subTest(status=status, signer=signer), mock.patch.object(pack, "powershell", return_value=json.dumps({"signature_status": status, "signer": signer})), self.assertRaisesRegex(ValueError, "valid Microsoft signature"):
                pack.authenticode_info(Path("fixture.dll"), {})
        with mock.patch.object(pack, "powershell", side_effect=ValueError("specific system module import failure")), self.assertRaisesRegex(ValueError, "specific system module import failure"):
            pack.authenticode_info(Path("fixture.dll"), {})

    @unittest.skipUnless(os.name == "nt", "requires real Windows PowerShell and Authenticode")
    def test_real_windows_authenticode_with_inherited_module_path(self):
        env = pack.windows_environment(os.environ, "")
        before = dict(env)
        host = pack.regular(Path(env["SYSTEMROOT"]) / "System32/WindowsPowerShell/v1.0/powershell.exe")
        # Read-only diagnostic of the inherited lookup, not an acceptance bypass.
        # The fixed-host signature checks below must still succeed independently.
        diagnostic_script = ("$ErrorActionPreference='Stop';"
                             "[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false);"
                             "try {Import-Module Microsoft.PowerShell.Security -ErrorAction Stop;"
                             "[Console]::WriteLine('inherited Security module import: success; host=' + $PSHOME + '; version=' + $PSVersionTable.PSVersion);"
                             "} catch {[Console]::WriteLine('inherited Security module import: failed; host=' + $PSHOME + '; version=' + $PSVersionTable.PSVersion + '; type=' + $_.Exception.GetType().FullName + '; id=' + $_.FullyQualifiedErrorId + '; message=' + $_.Exception.Message);}")
        diagnostic = pack.command([host, "-NoLogo", "-NoProfile", "-NonInteractive", "-Command", diagnostic_script], env)
        replacements = [(str(pack.ROOT), "<project>"), (env.get("USERPROFILE", ""), "<user-profile>"), (env["SYSTEMROOT"], "<windows>"), (env.get("PROGRAMFILES", ""), "<program-files>")]
        print(pack.sanitize(diagnostic, replacements))
        info = pack.authenticode_info(host, env)
        self.assertEqual(info["signature_status"], "Valid")
        self.assertIn("Microsoft Corporation", info["signer"])
        self.assertEqual(Path(info["powershell_host"]).resolve(), host.parent.resolve())
        self.assertTrue(info["powershell_version"].startswith("5.1."))
        with tempfile.TemporaryDirectory(prefix="nexa-unsigned-") as folder:
            unsigned = Path(folder) / "unsigned-fixture.ps1"
            unsigned.write_text("# Unsigned test data; never executed.\n", encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "valid Microsoft signature"):
                pack.authenticode_info(unsigned, env)
        self.assertEqual(env, before)

    def test_relative_paths_are_strict(self):
        for name in ("../escape", "/absolute", "C:/absolute", "a\\b", "a//b", "a/./b", "a/../b", "bad.", "bad ", "a\x00b", "", "\ud800"):
            with self.subTest(name=repr(name)), self.assertRaises((ValueError, UnicodeError)):
                pack.relative(name)
        self.assertEqual(pack.relative("licenses/test/LICENSE"), "licenses/test/LICENSE")

    def test_parse_imports_includes_delay_loads_and_deduplicates(self):
        text = "Image has the following dependencies:\n KERNEL32.dll\n VCRUNTIME140.dll\nImage has the following delay load dependencies:\n msvcp140.dll\n KERNEL32.dll\nSummary"
        self.assertEqual(pack.parse_dependents(text), ["kernel32.dll", "msvcp140.dll", "vcruntime140.dll"])
        with self.assertRaises(ValueError):
            pack.parse_dependents("not a successful dumpbin response")

    def test_os_contract_debug_and_unknown_dependencies(self):
        self.assertEqual(pack.dependency_kind("kernel32.dll", {}), "os")
        self.assertEqual(pack.dependency_kind("api-ms-win-crt-runtime-l1-1-0.dll", {}), "os")
        self.assertEqual(pack.dependency_kind("vcruntime140.dll", {"vcruntime140.dll": 1}), "app-local")
        for name in ("ucrtbased.dll", "vcruntime140d.dll", "vcruntime140_1d.dll", "msvcp140d.dll", "concrt140d.dll", "unknown.dll", "api-ms-win-evil/dll.dll"):
            with self.subTest(name=name), self.assertRaises(ValueError):
                pack.dependency_kind(name, {name: 1} if "d.dll" in name else {})

    def test_recursive_app_local_closure_is_exact(self):
        with tempfile.TemporaryDirectory() as folder:
            stage = Path(folder)
            copied = []
            deps = {"ai-runtime.exe": ["kernel32.dll", "vcruntime140.dll"], "ai-runtime-worker.exe": ["msvcp140.dll"], "msvcp140.dll": ["vcruntime140.dll", "vcruntime140_1.dll"], "vcruntime140.dll": ["kernel32.dll"], "vcruntime140_1.dll": ["vcruntime140.dll"]}
            result = pack.collect_dependencies(stage, {x: x for x in deps if x.endswith(".dll")}, lambda p: deps[p.name], lambda s,d: copied.append(d.name))
            self.assertEqual(set(result), set(deps))
            self.assertEqual(sorted(copied), ["msvcp140.dll", "vcruntime140.dll", "vcruntime140_1.dll"])

    def fixture(self, stage):
        for name in ("ai-runtime.exe", "ai-runtime-worker.exe", "config.example.toml", "README.md", "THIRD_PARTY_NOTICES.md"):
            path = stage / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("fixture", encoding="utf-8")
        compact_fixture(stage)
        return {"product": "nexa-runtime", "files": pack.entries(stage), "dependencies": {"ai-runtime.exe": {"imports": [{"name":"kernel32.dll", "kind":"os"}]}, "ai-runtime-worker.exe": {"imports": [{"name":"kernel32.dll", "kind":"os"}]}}}

    def test_manifest_rejects_missing_changed_extra_duplicate_and_pollution(self):
        for mode in ("missing", "changed", "extra", "duplicate", "traversal", "pollution", "unresolved"):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as folder:
                stage = Path(folder)
                manifest = self.fixture(stage)
                pack.verify_package(stage, manifest)
                if mode == "missing":
                    (stage / "ai-runtime.exe").unlink()
                elif mode == "changed":
                    (stage / "ai-runtime.exe").write_text("changed", encoding="utf-8")
                elif mode == "extra":
                    (stage / "unexpected.txt").write_text("extra", encoding="utf-8")
                elif mode == "duplicate":
                    manifest["files"].append(dict(manifest["files"][0]))
                elif mode == "traversal":
                    manifest["files"][0]["path"] = "../escape"
                elif mode == "unresolved":
                    manifest["dependencies"]["ai-runtime.exe"]["imports"].append({"name":"vcruntime140.dll", "kind":"app-local"})
                else:
                    (stage / "test-fault-worker.exe").write_text("pollution", encoding="utf-8")
                    manifest["files"] = pack.entries(stage)
                with self.assertRaises(ValueError):
                    pack.verify_package(stage, manifest)

    def test_rejects_symlink_and_case_duplicate(self):
        with tempfile.TemporaryDirectory() as folder:
            stage = Path(folder)
            (stage / "original").write_text("data", encoding="utf-8")
            if os.name != "nt":
                (stage / "link").symlink_to(stage / "original")
                with self.assertRaises(ValueError):
                    pack.entries(stage)
                (stage / "link").unlink()
                (stage / "Original").write_text("duplicate", encoding="utf-8")
                with self.assertRaises(ValueError):
                    pack.entries(stage)

    def test_hash_inventory_covers_manifest_without_self_reference(self):
        with tempfile.TemporaryDirectory() as folder:
            stage = Path(folder)
            manifest = self.fixture(stage)
            pack.write_json(stage / "manifest.json", manifest)
            sums = "".join(f"{x['sha256']}  {x['path']}\n" for x in pack.entries(stage))
            (stage / "SHA256SUMS").write_text(sums, encoding="utf-8")
            pack.verify_package(stage, manifest)
            self.assertIn("  manifest.json\n", sums)
            self.assertNotIn("  SHA256SUMS\n", sums)
            (stage / "manifest.json").write_text("tampered", encoding="utf-8")
            with self.assertRaises(ValueError):
                pack.verify_package(stage, manifest)

    def test_failed_promotion_rolls_back_previous_package(self):
        with tempfile.TemporaryDirectory() as folder:
            dist = Path(folder)
            old = dist / "windows-x64-cpu"
            old.mkdir()
            (old / "old").write_text("previous", encoding="utf-8")
            stage = dist / "stage"
            stage.mkdir()
            (stage / "new").write_text("new", encoding="utf-8")
            with self.assertRaises(OSError):
                pack.promote(stage, dist / "missing.zip", dist / "missing.sha256", dist)
            self.assertTrue((old / "old").is_file())
            self.assertFalse((old / "new").exists())
            self.assertEqual(sorted(p.name for p in dist.iterdir()), ["windows-x64-cpu"])

    def test_windows_reparse_source_is_rejected_even_without_symlink_flag(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "runtime.dll"
            path.write_text("fixture", encoding="utf-8")
            real_lstat = Path.lstat
            def reparse_lstat(item):
                result = real_lstat(item)
                if item == path:
                    class Metadata:
                        st_file_attributes = 0x400
                    return Metadata()
                return result
            with mock.patch.object(Path, "lstat", reparse_lstat), mock.patch.object(Path, "is_symlink", return_value=False), self.assertRaisesRegex(ValueError, "reparse"):
                pack.regular(path)

    def test_pe_machine_rejects_wrong_architecture_and_non_pe(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "program.exe"
            data = bytearray(128)
            data[:2] = b"MZ"
            struct.pack_into("<I", data, 0x3C, 64)
            data[64:68] = b"PE\0\0"
            struct.pack_into("<H", data, 68, 0x8664)
            path.write_bytes(data)
            pack.pe_machine(path)
            struct.pack_into("<H", data, 68, 0xAA64)
            path.write_bytes(data)
            with self.assertRaises(ValueError):
                pack.pe_machine(path)
            path.write_bytes(b"fake executable")
            with self.assertRaises(ValueError):
                pack.pe_machine(path)

    def test_license_subtree_cannot_hide_executable_or_model(self):
        for name in ("licenses/hidden.gguf", "licenses/test-fault-worker.exe", "licenses/secrets/api-token"):
            with tempfile.TemporaryDirectory() as folder, self.subTest(name=name):
                stage = Path(folder)
                manifest = self.fixture(stage)
                path = stage / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("pollution", encoding="utf-8")
                manifest["files"] = pack.entries(stage)
                with self.assertRaises(ValueError):
                    pack.verify_package(stage, manifest)

    def test_evidence_sanitizes_user_and_project_paths_recursively(self):
        data = {"native": [r"C:\Users\Person\repo\build\file", "C:/Users/Person/repo/file"], "sha256": "abcdef"}
        cleaned = pack.sanitize(data, [(r"C:\Users\Person\repo", "<project>"), (r"C:\Users\Person", "<user-profile>")])
        self.assertEqual(cleaned["native"], [r"<project>\build\file", "<project>/file"])
        self.assertEqual(cleaned["sha256"], "abcdef")

    def test_non_windows_host_cannot_claim_windows_build(self):
        with mock.patch.object(pack.sys, "platform", "linux"), self.assertRaisesRegex(ValueError, "native Windows"):
            pack.build()


if __name__ == "__main__":
    unittest.main()
