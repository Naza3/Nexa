"""Offline setup invariants; does not install or download dependencies."""
import importlib.util
import unittest
import sys
sys.dont_write_bytecode = True
from pathlib import Path

spec = importlib.util.spec_from_file_location("pi_setup", Path(__file__).with_name("setup.py"))
setup = importlib.util.module_from_spec(spec)
spec.loader.exec_module(setup)


class PatchTests(unittest.TestCase):
    def test_exact_duplicate_change_applied_once(self):
        part = "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new\n"
        self.assertEqual(setup.deduplicate_patch((part + part).encode()), part.encode())

    def test_conflicting_duplicate_change_fails(self):
        part = "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new\n"
        with self.assertRaises(ValueError):
            setup.deduplicate_patch((part + part.replace("+new", "+other")).encode())

    def test_unique_file_order_preserved(self):
        a = "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new\n"
        b = a.replace("a/x", "a/y").replace("b/x", "b/y")
        self.assertEqual(setup.deduplicate_patch((a + b).encode()), (a + b).encode())


if __name__ == "__main__":
    unittest.main()
