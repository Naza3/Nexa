"""Keep Nexa-owned build targets desktop-only without editing upstream sources."""
from pathlib import Path
import re
import tomllib
import unittest


ROOT = Path(__file__).resolve().parents[1]
RETIRED_PATHS = (
    "apps/android-verifier",
    "mobile",
    "native/mnn-probe",
    "native/mnn-shim",
    "native/mnn-patches",
    "scripts/android_mnn",
    ".github/workflows/android-mnn-probe.yml",
    ".github/workflows/android-mnn-native.yml",
)
MOBILE_TARGET = re.compile(r"android|\bios\b|\bmnn\b|flutter|\bmobile\b", re.IGNORECASE)


class DesktopScopeTests(unittest.TestCase):
    def test_retired_project_targets_are_absent(self):
        for name in RETIRED_PATHS:
            with self.subTest(path=name):
                self.assertFalse((ROOT / name).exists(), name)

    def test_owned_rust_and_native_code_has_no_mobile_target_contract(self):
        # Deliberately exclude dependency locks, fixtures, and vendor/llama.cpp.
        # Upstream multi-platform content is part of the unchanged pinned source.
        roots = ("crates", "xtask", "native/llama-shim", "apps/desktop/src-tauri/src")
        suffixes = {".rs", ".cpp", ".h"}
        for name in roots:
            for path in (ROOT / name).rglob("*"):
                if path.is_file() and path.suffix in suffixes:
                    with self.subTest(path=path.relative_to(ROOT).as_posix()):
                        self.assertNotRegex(path.read_text(encoding="utf-8"), MOBILE_TARGET)

    def test_owned_build_manifests_have_no_mobile_targets_or_dependencies(self):
        manifests = [ROOT / "Cargo.toml", ROOT / "xtask/Cargo.toml",
                     ROOT / "apps/desktop/src-tauri/Cargo.toml"]
        manifests.extend((ROOT / "crates").glob("*/Cargo.toml"))
        for path in manifests:
            with self.subTest(path=path.relative_to(ROOT).as_posix()):
                content = path.read_text(encoding="utf-8")
                tomllib.loads(content)
                self.assertNotRegex(content, MOBILE_TARGET)
        workspace = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]
        for member in workspace["members"]:
            self.assertTrue((ROOT / member / "Cargo.toml").is_file(), member)

    def test_workflows_do_not_build_or_exempt_retired_targets(self):
        for path in (ROOT / ".github/workflows").glob("*.yml"):
            with self.subTest(path=path.name):
                self.assertNotRegex(path.read_text(encoding="utf-8"), MOBILE_TARGET)

    def test_desktop_scope_gate_runs_before_native_build(self):
        workflow = (ROOT / ".github/workflows/native-windows.yml").read_text(encoding="utf-8")
        self.assertIn("-p 'test_desktop_scope.py'", workflow)
        self.assertLess(workflow.index("-p 'test_desktop_scope.py'"), workflow.index("cargo build"))


if __name__ == "__main__":
    unittest.main()
