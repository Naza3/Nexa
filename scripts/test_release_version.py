import json
from pathlib import Path
import shutil
import tempfile
import unittest

import release_version as version


class ReleaseVersionTests(unittest.TestCase):
    def test_canonical_stable_msi_versions(self):
        for value in ("0.1.0", "1.0.0", "255.255.65535"):
            with self.subTest(value=value):
                self.assertEqual(version.validate_version(value), value)
                self.assertEqual(version.tag_version("v" + value), value)

    def test_rejects_aliases_prereleases_injection_and_msi_overflow(self):
        for value in ("0.0.0", "01.2.3", "1.02.3", "1.2.03", "1.2", "1.2.3.4", "1.2.3-rc.1", "1.2.3+build", "256.0.0", "1.256.0", "1.0.65536", "1.2.3\n", " 1.2.3", "１.2.3", "1.2.3;echo", "-1.2.3", None, True):
            with self.subTest(value=value), self.assertRaises(ValueError):
                version.validate_version(value)
        for value in ("1.2.3", "V1.2.3", "refs/tags/v1.2.3", "v1.2.3/next"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                version.tag_version(value)

    def fixture(self, root):
        source = Path(__file__).resolve().parents[1]
        paths = ["Cargo.toml", "Cargo.lock", "apps/desktop/package.json", "apps/desktop/package-lock.json", "apps/desktop/src-tauri/Cargo.toml", "apps/desktop/src-tauri/Cargo.lock", "apps/desktop/src-tauri/tauri.conf.json"]
        paths += [str(path.relative_to(source)) for path in (source / "crates").glob("*/Cargo.toml")]
        paths += ["xtask/Cargo.toml"]
        for path in paths:
            target = root / path
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source / path, target)
        return version.repository_version(root)

    def test_repository_versions_match(self):
        with tempfile.TemporaryDirectory() as temp:
            self.assertEqual(self.fixture(Path(temp)), version.repository_version(Path(__file__).resolve().parents[1]))

    def test_each_declared_version_and_local_lock_is_checked(self):
        for path in ("Cargo.toml", "Cargo.lock", "apps/desktop/package.json", "apps/desktop/package-lock.json", "apps/desktop/src-tauri/Cargo.toml", "apps/desktop/src-tauri/Cargo.lock", "apps/desktop/src-tauri/tauri.conf.json"):
            with self.subTest(path=path), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                current = self.fixture(root)
                target = root / path
                changed = "1.2.3" if current != "1.2.3" else "1.2.4"
                target.write_text(target.read_text(encoding="utf-8").replace(f'"{current}"', f'"{changed}"', 1), encoding="utf-8")
                with self.assertRaises(ValueError):
                    version.repository_version(root)

    def test_local_crate_cannot_override_version(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            current = self.fixture(root)
            changed = "1.2.3" if current != "1.2.3" else "1.2.4"
            path = root / "crates/runtime-core/Cargo.toml"
            path.write_text(path.read_text(encoding="utf-8").replace("version.workspace = true", f'version = "{changed}"'), encoding="utf-8")
            with self.assertRaises(ValueError):
                version.repository_version(root)

    def test_npm_lock_root_version_is_not_ignored(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            current = self.fixture(root)
            path = root / "apps/desktop/package-lock.json"
            value = json.loads(path.read_text(encoding="utf-8"))
            value["packages"][""]["version"] = "1.2.3" if current != "1.2.3" else "1.2.4"
            path.write_text(json.dumps(value), encoding="utf-8")
            with self.assertRaises(ValueError):
                version.repository_version(root)
