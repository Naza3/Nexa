"""Keep the two Rust workspaces' error-result lint policy in sync.

Behavioral error classification is tested in Rust, not by matching source text.
"""
from pathlib import Path
import tomllib
import unittest

ROOT = Path(__file__).resolve().parents[1]


def manifest(path):
    return tomllib.loads(path.read_text(encoding="utf-8"))


class RustErrorPolicyTests(unittest.TestCase):
    def test_all_workspace_members_inherit_error_lints(self):
        workspace = manifest(ROOT / "Cargo.toml")["workspace"]
        self.assertEqual(workspace["lints"]["rust"]["unused_must_use"], "deny")
        self.assertEqual(workspace["lints"]["clippy"]["dbg_macro"], "deny")
        for member in workspace["members"]:
            with self.subTest(member=member):
                self.assertTrue(manifest(ROOT / member / "Cargo.toml")["lints"]["workspace"])

    def test_standalone_desktop_has_same_error_lints(self):
        expected = manifest(ROOT / "Cargo.toml")["workspace"]["lints"]
        actual = manifest(ROOT / "apps/desktop/src-tauri/Cargo.toml")["lints"]
        for group, name in (("rust", "unused_must_use"), ("clippy", "dbg_macro")):
            with self.subTest(lint=name):
                self.assertEqual(actual[group][name], expected[group][name])


if __name__ == "__main__":
    unittest.main()
