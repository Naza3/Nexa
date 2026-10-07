"""Offline release version editing contracts, using real repository manifests."""
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import tomllib
import unittest
from unittest import mock

import release_version
import set_version


SOURCE = Path(__file__).resolve().parents[1]
PRODUCT_FILES = (
    "Cargo.toml", "Cargo.lock", "apps/desktop/package.json",
    "apps/desktop/package-lock.json", "apps/desktop/src-tauri/Cargo.toml",
    "apps/desktop/src-tauri/Cargo.lock", "apps/desktop/src-tauri/tauri.conf.json",
)
LOCK_FILES = ("Cargo.lock", "apps/desktop/src-tauri/Cargo.lock")


class SetVersionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / "repository with spaces"
        workspace = tomllib.loads((SOURCE / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]
        paths = list(PRODUCT_FILES) + [f"{member}/Cargo.toml" for member in workspace["members"]]
        for relative in paths:
            target = self.root / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(SOURCE / relative, target)
        self.current = release_version.repository_version(self.root)
        self.target = "0.2.1" if self.current != "0.2.1" else "0.2.2"

    def snapshot(self):
        return {path.relative_to(self.root).as_posix(): path.read_bytes()
                for path in self.root.rglob("*") if path.is_file()}

    def packages(self, relative):
        return tomllib.loads((self.root / relative).read_text(encoding="utf-8"))["package"]

    def run_cli(self, *args, stdin=None):
        scripts = self.root / "scripts"
        scripts.mkdir(exist_ok=True)
        for name in ("set_version.py", "release_version.py"):
            shutil.copyfile(SOURCE / "scripts" / name, scripts / name)
        return subprocess.run(
            [sys.executable, str(scripts / "set_version.py"), *args],
            cwd=self.temp.name, input=stdin, text=True, encoding="utf-8", capture_output=True, timeout=20,
        )

    def test_updates_all_seven_files_and_is_idempotent(self):
        before = self.snapshot()
        changed = set_version.update_version(self.root, self.target)
        self.assertCountEqual(changed, PRODUCT_FILES)
        self.assertEqual(release_version.repository_version(self.root), self.target)
        after = self.snapshot()
        self.assertEqual({path for path in before if before[path] != after[path]}, set(PRODUCT_FILES))
        self.assertEqual(set_version.update_version(self.root, self.target), [])
        self.assertEqual(self.snapshot(), after)

    def test_accepts_tag_and_preserves_dependencies_and_checksums(self):
        # A registry package intentionally collides with the old release number.
        for relative in LOCK_FILES:
            path = self.root / relative
            dependency = ('[[package]]\nname = "release-version-collision"\n'
                          f'version = "{self.current}"\n'
                          'source = "registry+https://github.com/rust-lang/crates.io-index"\n'
                          'checksum = "' + 'a' * 64 + '"\n\n')
            path.write_text(path.read_text(encoding="utf-8").replace("[[package]]", dependency + "[[package]]", 1), encoding="utf-8")
        before = {relative: self.packages(relative) for relative in LOCK_FILES}
        npm_path = self.root / "apps/desktop/package-lock.json"
        npm_before = json.loads(npm_path.read_text(encoding="utf-8"))
        set_version.update_version(self.root, "v" + self.target)
        for relative in LOCK_FILES:
            after = self.packages(relative)
            self.assertEqual(len(after), len(before[relative]))
            for old, new in zip(before[relative], after):
                expected = dict(old)
                if "source" not in old:
                    expected["version"] = self.target
                self.assertEqual(new, expected)
        npm_after = json.loads(npm_path.read_text(encoding="utf-8"))
        npm_before["version"] = self.target
        npm_before["packages"][""]["version"] = self.target
        self.assertEqual(npm_before, npm_after)
        self.assertEqual(release_version.repository_version(self.root), self.target)

    def test_invalid_target_never_changes_files(self):
        before = self.snapshot()
        for target in ("", "0.0.0", "01.2.3", "1.2", "1.2.3-rc.1", "1.2.3+build",
                       "256.0.0", "1.256.0", "1.0.65536", "V0.2.1", "vv0.2.1",
                       " 0.2.1", "0.2.1\n", "0.2.1;echo", "１.2.3", None):
            with self.subTest(target=target):
                with self.assertRaises((ValueError, TypeError)):
                    set_version.update_version(self.root, target)
                self.assertEqual(self.snapshot(), before)

    def test_dry_run_reports_changes_without_writing(self):
        before = self.snapshot()
        self.assertCountEqual(set_version.update_version(self.root, self.target, dry_run=True), PRODUCT_FILES)
        self.assertEqual(self.snapshot(), before)
        self.assertEqual(set_version.update_version(self.root, self.current, dry_run=True), [])

    def test_repairs_existing_product_version_mismatch(self):
        path = self.root / "apps/desktop/package.json"
        value = json.loads(path.read_text(encoding="utf-8"))
        value["version"] = "0.1.7"
        path.write_text(json.dumps(value) + "\n", encoding="utf-8")
        path = self.root / "apps/desktop/src-tauri/Cargo.lock"
        blocks = path.read_text(encoding="utf-8").split("[[package]]")
        for index, block in enumerate(blocks[1:], 1):
            package = tomllib.loads("[[package]]" + block)["package"][0]
            if "source" not in package:
                blocks[index] = block.replace(f'version = "{package["version"]}"', 'version = "0.1.8"', 1)
        path.write_text("[[package]]".join(blocks), encoding="utf-8")
        set_version.update_version(self.root, self.target)
        self.assertEqual(release_version.repository_version(self.root), self.target)

    def test_broken_input_and_invalid_inventory_have_no_partial_writes(self):
        cases = (
            ("apps/desktop/src-tauri/tauri.conf.json", b'{"version":'),
            ("apps/desktop/src-tauri/Cargo.lock", b'[[package]]\nname = "unknown-local"\nversion = "0.2.0"\n'),
            ("crates/runtime-core/Cargo.toml", b'[package]\nname = "runtime-core"\nversion = "0.0.9"\n'),
        )
        for relative, broken in cases:
            with self.subTest(path=relative):
                path = self.root / relative
                original = path.read_bytes()
                path.write_bytes(broken)
                before = self.snapshot()
                for dry_run in (False, True):
                    with self.assertRaises((ValueError, KeyError, TypeError)):
                        set_version.update_version(self.root, self.target, dry_run=dry_run)
                    self.assertEqual(self.snapshot(), before)
                path.write_bytes(original)

    def test_preserves_each_files_newline_style(self):
        for index, relative in enumerate(PRODUCT_FILES):
            path = self.root / relative
            content = path.read_bytes().replace(b"\r\n", b"\n")
            path.write_bytes(content.replace(b"\n", b"\r\n") if index % 2 else content)
        set_version.update_version(self.root, self.target)
        for index, relative in enumerate(PRODUCT_FILES):
            content = (self.root / relative).read_bytes()
            with self.subTest(path=relative):
                if index % 2:
                    self.assertIn(b"\r\n", content)
                    self.assertNotIn(b"\n", content.replace(b"\r\n", b""))
                else:
                    self.assertNotIn(b"\r\n", content)
        self.assertEqual(release_version.repository_version(self.root), self.target)

    def test_second_file_write_failure_rolls_back_first_file(self):
        before = self.snapshot()
        original_write = set_version.write_atomic
        attempts = []

        def fail_second_write(path, data):
            attempts.append(path)
            if len(attempts) == 2:
                raise OSError("injected second-file write failure")
            original_write(path, data)

        with mock.patch.object(set_version, "write_atomic", side_effect=fail_second_write):
            with self.assertRaisesRegex(OSError, "injected second-file"):
                set_version.update_version(self.root, self.target)
        self.assertGreaterEqual(len(attempts), 3)
        self.assertEqual(self.snapshot(), before)
        self.assertEqual(release_version.repository_version(self.root), self.current)

    def test_cli_locates_repository_from_script_and_supports_check(self):
        result = self.run_cli("v" + self.target)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(release_version.repository_version(self.root), self.target)
        result = self.run_cli("--check")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(self.target, result.stdout)
        path = self.root / "apps/desktop/package.json"
        path.write_text(path.read_text(encoding="utf-8").replace(self.target, "0.1.8"), encoding="utf-8")
        result = self.run_cli("--check")
        self.assertNotEqual(result.returncode, 0)

    def test_cli_dry_run_and_empty_interactive_input_do_not_write(self):
        before = {path: (self.root / path).read_bytes() for path in PRODUCT_FILES}
        for args, stdin in (((self.target, "--dry-run"), None), ((), "\n")):
            with self.subTest(args=args):
                result = self.run_cli(*args, stdin=stdin)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual({path: (self.root / path).read_bytes() for path in PRODUCT_FILES}, before)
        result = self.run_cli(stdin=self.target + "\n")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(release_version.repository_version(self.root), self.target)

    @unittest.skipUnless(sys.platform == "win32", "Windows cmd.exe entry point")
    def test_windows_entry_point_forwards_version_and_exit_status(self):
        result = self.run_cli("--check")  # Stage the scripts in the fixture.
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        helper = self.root / "update-version.cmd"
        shutil.copyfile(SOURCE / helper.name, helper)
        for arguments, expected in (((self.target,), 0), (("--check",), 0), (("1.2.3-rc.1",), 1)):
            with self.subTest(arguments=arguments):
                result = subprocess.run(
                    ["cmd.exe", "/d", "/c", str(helper), *arguments], cwd=self.temp.name,
                    capture_output=True, encoding="utf-8", timeout=30,
                )
                self.assertEqual(result.returncode, expected, result.stdout + result.stderr)
                self.assertEqual(release_version.repository_version(self.root), self.target)


if __name__ == "__main__":
    unittest.main()
