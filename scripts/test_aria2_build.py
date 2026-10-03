"""Offline fail-closed tests; does not claim any Windows execution evidence."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

import build_aria2_windows as build

spec = importlib.util.spec_from_file_location("windows_probe", build.MATERIALS / "tests/windows_probe.py")
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)


class BuildContractTests(unittest.TestCase):
    def test_fixed_patch_identity(self):
        lock = json.loads(build.LOCK.read_text(encoding="utf-8"))
        build.verified(build.MATERIALS / "patches" / lock["patch"]["filename"], lock["patch"]["sha256"])

    def test_version_locked(self):
        lock = json.loads(build.LOCK.read_text(encoding="utf-8"))
        self.assertEqual(lock["aria2"]["version"], "1.37.0")
        self.assertEqual(lock["toolchain"]["sha256"], "27d33157cc252c29ad6f777a96a0d94176fea1b534ff09b5071485def143b90e")

    def test_schannel_required(self):
        for config in ("", "#define ENABLE_SSL 1\n", "#define SECURITY_WIN32 1\n"):
            with self.subTest(config=config), self.assertRaises(ValueError):
                build.check_config_text(config)

    def test_only_schannel(self):
        config = "#define ENABLE_SSL 1\n#define SECURITY_WIN32 1\n"
        self.assertTrue(build.check_config_text(config)["SECURITY_WIN32"])
        for feature in build.FORBIDDEN:
            with self.subTest(feature=feature), self.assertRaises(ValueError):
                build.check_config_text(config + "#define " + feature + " 1\n")

    def test_undefined_not_enabled(self):
        config = "#define ENABLE_SSL 1\n#define SECURITY_WIN32 1\n/* #undef ENABLE_BITTORRENT */\n"
        self.assertFalse(build.check_config_text(config)["ENABLE_BITTORRENT"])

    def test_pe_amd64_imports(self):
        text = "Machine: IMAGE_FILE_MACHINE_AMD64 (0x8664)\n  Name: KERNEL32.dll\n  Name: secur32.dll\n  Name: api-ms-win-crt-heap-l1-1-0.dll\n"
        self.assertEqual(build.pe_metadata(text)["machine"], "AMD64")
        with self.assertRaises(ValueError):
            build.pe_metadata(text.replace("IMAGE_FILE_MACHINE_AMD64", "IMAGE_FILE_MACHINE_I386"))
        with self.assertRaises(ValueError):
            build.pe_metadata(text + "  Name: libc++.dll\n")
        with self.assertRaises(ValueError):
            build.pe_metadata(text.replace("secur32.dll", "libssl-3.dll"))

    def test_hash_mismatch_is_fatal(self):
        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp) / "test"
            p.write_bytes(b"not the locked file")
            with self.assertRaises(ValueError):
                build.verified(p, "0" * 64)

    def test_source_snapshot_detects_change(self):
        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp) / "NexaNetworkPolicy.h"
            p.write_bytes(b"policy")
            before = build.source_identity(Path(tmp))
            p.write_bytes(b"different")
            self.assertNotEqual(before, build.source_identity(Path(tmp)))

    def test_tls_rejection_needs_certificate_evidence(self):
        for text in ("timeout", "connection refused", "Error: 80092013", "", "abc80090325xyz"):
            self.assertFalse(probe.certificate_rejected(1, text))
        self.assertTrue(probe.certificate_rejected(1, "Error: untrusted root (80090325)"))
        self.assertFalse(probe.certificate_rejected(0, "Error: 80090325"))

    def test_clean_environment(self):
        self.assertFalse(any("proxy" in k.lower() for k in probe.clean_env()))

    def test_artifact_not_product_workflow(self):
        text = (build.ROOT / ".github/workflows/aria2-build-probe.yml").read_text(encoding="utf-8")
        self.assertIn("runs-on: windows-2022", text)
        self.assertIn("needs: build", text)
        self.assertNotIn("package_windows", text)
        self.assertNotIn("check-certificate=false", text)

    def test_production_patch_has_no_test_bypass(self):
        text = (build.MATERIALS / "patches/nexa-network-policy.patch").read_text(encoding="utf-8")
        self.assertNotIn("getenv", text)
        self.assertNotIn("NEXA_TEST", text)
        self.assertIn("nexa::publicDestination(rp->ai_addr, rp->ai_addrlen)", text)
        self.assertIn("throw DL_ABORT_EX", text)


if __name__ == "__main__":
    unittest.main()
