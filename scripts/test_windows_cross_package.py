import hashlib
import json
from pathlib import Path
import struct
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock
import zipfile
import package_windows_cross_test as pack


class CrossPackageTests(unittest.TestCase):
    def pe_output(self, delay=False):
        return '''ImageFileHeader {
  Machine: IMAGE_FILE_MACHINE_AMD64 (0x8664)
}
ImageOptionalHeader {
  Magic: 0x20B
  DataDirectory {
    ImportTableRVA: 0x1000
    ImportTableSize: 0x28
    DelayImportDescriptorRVA: DELAY_RVA
    DelayImportDescriptorSize: DELAY_SIZE
  }
}
Import {
  Name: KERNEL32.dll
  Symbol: ExitProcess (1)
}
'''.replace("DELAY_RVA", "0x2000" if delay else "0x0").replace("DELAY_SIZE", "0x40" if delay else "0x0") + ("DelayImport {\n  Name: vcruntime140.dll\n}\n" if delay else "")

    def test_pe_normal_and_delay_dependencies(self):
        self.assertEqual(pack.parse_pe_inspection(self.pe_output()), {"normal": ["kernel32.dll"], "delay": []})
        self.assertEqual(pack.parse_pe_inspection(self.pe_output(True)), {"normal": ["kernel32.dll"], "delay": ["vcruntime140.dll"]})

    def test_pe_rejects_unsupported_machine_missing_or_undecoded_directories(self):
        source = self.pe_output(True)
        for bad in (source.replace("AMD64", "ARM64"), source.replace("Magic: 0x20B", "Magic: 0x10B"),
                    source.replace("    DelayImportDescriptorSize: 0x40\n", ""),
                    source.replace("DelayImport {\n  Name: vcruntime140.dll\n}\n", ""),
                    source.replace("ImportTableRVA: 0x1000", "ImportTableRVA: 0x0"),
                    source.replace("Name: KERNEL32.dll", "Name: ../KERNEL32.dll"),
                    source.replace("Name: KERNEL32.dll", "Name: KERNEL32.dll\n  Name: other.dll")):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                pack.parse_pe_inspection(bad)

    def fixture(self, root):
        dll = root / "vcruntime140.dll"
        # Header fixture only, never executed or used as a deliverable.
        data = bytearray(128)
        data[:2] = b"MZ"
        struct.pack_into("<I", data, 0x3c, 64)
        data[64:70] = b"PE\0\0\x64\x86"
        dll.write_bytes(data)
        archive = root / "redist.vsix"
        member = "Contents/VC/Redist/MSVC/14.44.1/x64/Microsoft.VC143.CRT/vcruntime140.dll"
        with zipfile.ZipFile(archive, "w") as zipped:
            zipped.write(dll, member)
        license_file = root / "License.txt"
        license_file.write_text("License fixture, not an actual Microsoft license", encoding="utf-8")
        def record(path):
            return {"path": str(path), "sha256": pack.base.digest(path), "size_bytes": path.stat().st_size}
        value = {"schema_version": 1,
                 "archives": [{**record(archive), "url": "https://download.visualstudio.microsoft.com/test/redist.vsix"}],
                 "dlls": [{**record(dll), "archive_sha256": pack.base.digest(archive), "package_path": member}],
                 "licenses": [{**record(license_file), "source_url": "https://visualstudio.microsoft.com/license-terms/"}]}
        path = root / "crt.json"
        pack.base.write_json(path, value)
        return path, value

    def test_crt_requires_exact_original_archive_member(self):
        with tempfile.TemporaryDirectory() as temporary:
            path, value = self.fixture(Path(temporary))
            _, records = pack.load_crt_provenance(path)
            self.assertEqual(set(records), {"vcruntime140.dll"})
            value["dlls"][0]["package_path"] = "Contents/no-such.dll"
            pack.base.write_json(path, value)
            with self.assertRaisesRegex(ValueError, "member"):
                pack.load_crt_provenance(path)

    def test_crt_rejects_unrelated_archive_even_with_self_consistent_hashes(self):
        with tempfile.TemporaryDirectory() as temporary:
            path, value = self.fixture(Path(temporary))
            archive = Path(value["archives"][0]["path"])
            with zipfile.ZipFile(archive, "w") as zipped:
                zipped.writestr(value["dlls"][0]["package_path"], b"x" * value["dlls"][0]["size_bytes"])
            value["archives"][0].update(sha256=pack.base.digest(archive), size_bytes=archive.stat().st_size)
            value["dlls"][0]["archive_sha256"] = pack.base.digest(archive)
            pack.base.write_json(path, value)
            with self.assertRaisesRegex(ValueError, "differs from"):
                pack.load_crt_provenance(path)

    def test_crt_rejects_bad_origin_hash_debug_and_missing_licenses(self):
        for mutation in ("url", "hash", "size", "license", "archive-ref", "duplicate", "debug"):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as temporary:
                path, value = self.fixture(Path(temporary))
                if mutation == "url":
                    value["archives"][0]["url"] = "https://untrusted.example/redist.vsix"
                elif mutation == "hash":
                    value["dlls"][0]["sha256"] = "0" * 64
                elif mutation == "size":
                    value["dlls"][0]["size_bytes"] += 1
                elif mutation == "license":
                    value["licenses"] = []
                elif mutation == "archive-ref":
                    value["dlls"][0]["archive_sha256"] = "0" * 64
                elif mutation == "duplicate":
                    value["dlls"].append(value["dlls"][0].copy())
                elif mutation == "debug":
                    original = Path(value["dlls"][0]["path"])
                    renamed = original.with_name("vcruntime140d.dll")
                    original.rename(renamed)
                    value["dlls"][0]["path"] = str(renamed)
                pack.base.write_json(path, value)
                with self.assertRaises(ValueError):
                    pack.load_crt_provenance(path)

    def test_signature_requires_success_timestamp_and_crl_without_ignore_flags(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            path, value = self.fixture(root)
            dll = Path(value["dlls"][0]["path"])
            tool, ca, tsa = (root / name for name in ("osslsigncode", "ca.pem", "tsa.pem"))
            for item in (tool, ca, tsa):
                item.write_text("signature tool or CA fixture, never executed", encoding="utf-8")
            valid = "O=Microsoft Corporation\nSignature verification: ok\nSignature CRL verification: ok\nTimestamp Server Signature verification: ok\nTimestamp Server Signature CRL verification: ok\nNumber of verified signatures: 1\nSucceeded\n"
            with mock.patch.object(pack.subprocess, "run", return_value=SimpleNamespace(returncode=0, stdout=valid)) as run:
                result = pack.verify_linux_signature(dll, tool, ca, tsa, root / "evidence")
                self.assertEqual(result["status"], "pass")
                self.assertFalse(result["windows_policy_equivalence"])
                self.assertEqual(run.call_args.args[0], [str(tool), "verify", "-CAfile", str(ca), "-TSA-CAfile", str(tsa), "-in", str(dll)])
                self.assertEqual(run.call_args.kwargs["timeout"], 180)
            for output, code in ((valid, 1), (valid.replace("Signature CRL verification: ok\n", ""), 0),
                                 (valid.replace("Timestamp Server Signature verification: ok\n", ""), 0),
                                 (valid.replace("Number of verified signatures: 1", "Number of verified signatures: 2"), 0),
                                 (valid.replace("Microsoft Corporation", "untrusted signer"), 0)):
                with mock.patch.object(pack.subprocess, "run", return_value=SimpleNamespace(returncode=code, stdout=output)), self.assertRaises(ValueError):
                    pack.verify_linux_signature(dll, tool, ca, tsa, root / "evidence")

    def test_native_identity_rejects_lexical_parent_escape(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            native = root / "native"
            native.mkdir()
            outside = root / "outside"
            outside.mkdir()
            fields = {"schema": "1", "system": "Windows", "configuration": "Release", "pointer_bytes": "8", "processor": "AMD64",
                      "crt": "MD", "llama_commit": pack.base.LLAMA_COMMIT, "profile": pack.PROFILE, "host_system": "Linux", "cross_compiling": "TRUE",
                      "compiler_id": "Clang", "compiler_frontend": "MSVC", "compiler_simulate_id": "MSVC", "compiler_target": pack.base.TARGET,
                      "c_compiler_id": "Clang", "c_compiler_frontend": "MSVC", "c_compiler_simulate_id": "MSVC", "c_compiler_target": pack.base.TARGET,
                      "msvc_runtime_library": "MultiThreadedDLL", "cross_abi_verified": "1", "cross_cpu_baseline_verified": "1", "GGML_SSE42": "ON", "GGML_AVX": "ON", "GGML_AVX2": "ON", "GGML_FMA": "ON", "GGML_F16C": "ON", "GGML_BMI2": "ON", "GGML_AVX512": "OFF"}
            fields.update(dict.fromkeys(("GGML_NATIVE", "GGML_BACKEND_DL", "GGML_OPENMP", "GGML_CUDA", "GGML_VULKAN", "GGML_METAL", "LLAMA_OPENSSL", "BUILD_SHARED_LIBS"), "OFF"))
            for name in ("air_llama", "llama-common", "llama-common-base", "cpp-httplib", "llama", "ggml", "ggml-cpu", "ggml-base"):
                path = native / (name + ".lib")
                path.write_bytes(b"archive path fixture")
                fields["library." + name] = str(path)
            identity = native / "air-native-Release.txt"
            def write():
                identity.write_text("\n".join(key + "=" + value for key, value in fields.items()) + "\n", encoding="utf-8")
            write()
            self.assertEqual(len(pack.native_identity(native)[1]), 8)
            (outside / "air_llama.lib").write_bytes(b"outside archive path fixture")
            fields["library.air_llama"] = str(native / ".." / "outside" / "air_llama.lib")
            write()
            with self.assertRaisesRegex(ValueError, "escaped"):
                pack.native_identity(native)

    def test_source_receipt_cannot_be_reused_after_changes(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "receipt.json"
            original = {"commit": "a" * 40, "dirty": True, "worktree_files_sha256": "b" * 64}
            with mock.patch.object(pack, "source_identity", return_value=original), mock.patch.object(pack.platform, "system", return_value="Linux"):
                pack.capture_source(path)
                self.assertEqual(pack.check_receipt(path), original)
                with self.assertRaises(ValueError):
                    pack.capture_source(path)
            with mock.patch.object(pack, "source_identity", return_value={**original, "dirty": False}), self.assertRaisesRegex(ValueError, "source differs"):
                pack.check_receipt(path)

    def test_cross_documents_remove_native_authenticode_and_actions_claims(self):
        source_root = Path(__file__).resolve().parents[1]
        templates = source_root / "packaging"
        if not templates.exists():
            self.skipTest("requires package document templates")
        with tempfile.TemporaryDirectory() as temporary:
            for role, directory in (("runtime", "windows-x64-cpu"), ("desktop", "desktop-windows")):
                for name in ("README.md", "THIRD_PARTY_NOTICES.md"):
                    output = Path(temporary) / (role + name)
                    pack.copy_cross_document(templates / directory / name, output, role)
                    text = output.read_text(encoding="utf-8")
                    for old in ("本次开发 Actions 产物", "验证 Microsoft Authenticode 并记录", "所选 Visual Studio 的合法", "实际 `vswhere` 所选"):
                        self.assertNotIn(old, text)
                    self.assertIn("Linux", text)


if __name__ == "__main__":
    unittest.main()
