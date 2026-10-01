import importlib.util
import json
import os
import struct
from pathlib import Path
import tempfile
import unittest
from unittest import mock

spec = importlib.util.spec_from_file_location("package_windows", Path(__file__).with_name("package_windows.py"))
pack = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pack)


class PackageTests(unittest.TestCase):
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
        for name in ("ai-runtime.exe", "ai-runtime-worker.exe", "config.example.toml", "README.md", "THIRD_PARTY_NOTICES.md", "licenses/index.json"):
            path = stage / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("fixture", encoding="utf-8")
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
