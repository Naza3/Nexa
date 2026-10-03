import json
from pathlib import Path
import tempfile
import tomllib
import unittest
from unittest import mock
import package_desktop_windows as pack


class DesktopPackageTests(unittest.TestCase):
    def fixture(self, root):
        runtime = root / "runtime"
        runtime.mkdir()
        for name in ("ai-runtime.exe", "ai-runtime-worker.exe", "config.example.toml", "README.md", "THIRD_PARTY_NOTICES.md", "licenses/index.json"):
            path = runtime / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("fixture", encoding="utf-8")
        runtime_manifest = {"product": "nexa-runtime", "project_commit": "a" * 40, "project_dirty": False, "files": pack.base.entries(runtime), "dependencies": {"ai-runtime.exe": {"imports": [{"name": "kernel32.dll", "kind": "os"}]}, "ai-runtime-worker.exe": {"imports": [{"name": "kernel32.dll", "kind": "os"}]}}}
        pack.base.write_json(runtime / "manifest.json", runtime_manifest)
        (runtime / "SHA256SUMS").write_text("".join(f"{item['sha256']}  {item['path']}\n" for item in pack.base.entries(runtime)), encoding="utf-8")
        for name in ("nexa-desktop.exe", "README.md", "THIRD_PARTY_NOTICES.md", "licenses/index.json", "licenses/npm-index.json"):
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("fixture", encoding="utf-8")
        return {"product": "nexa-desktop", "project_commit": "a" * 40, "project_dirty": False, "runtime_manifest_sha256": pack.base.digest(runtime / "manifest.json"), "dependencies": {"nexa-desktop.exe": {"imports": [{"name": "kernel32.dll", "kind": "os"}]}}, "files": pack.base.entries(root)}

    def test_nested_runtime_and_exact_files(self):
        for change in ("valid", "extra", "tamper", "mixed-source", "mixed-manifest", "pollution", "missing", "closure", "hashlist"):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                manifest = self.fixture(root)
                pack.verify(root, manifest)
                if change == "valid":
                    continue
                if change == "extra":
                    (root / "unexpected.log").write_text("secret", encoding="utf-8")
                elif change == "tamper":
                    (root / "runtime/ai-runtime.exe").write_text("modified", encoding="utf-8")
                elif change == "mixed-source":
                    manifest["project_commit"] = "b" * 40
                elif change == "mixed-manifest":
                    manifest["runtime_manifest_sha256"] = "b" * 64
                elif change == "pollution":
                    (root / "licenses/model.gguf").write_text("GGUF", encoding="utf-8")
                    manifest["files"] = pack.base.entries(root)
                elif change == "missing":
                    (root / "nexa-desktop.exe").unlink()
                elif change == "closure":
                    manifest["dependencies"]["nexa-desktop.exe"]["imports"].append({"name": "vcruntime140.dll", "kind": "app-local"})
                else:
                    (root / "SHA256SUMS").write_text("wrong", encoding="utf-8")
                with self.assertRaises(ValueError):
                    pack.verify(root, manifest)

    def test_desktop_os_and_crt_closure(self):
        for name in ("comctl32.dll", "dwmapi.dll", "uiautomationcore.dll", "api-ms-win-core-libraryloader-l1-2-0.dll"):
            self.assertEqual(pack.dependency_kind(name, {}), "os")
        for name in ("unknown.dll", "webview2loader.dll", "ucrtbased.dll"):
            with self.assertRaises(ValueError):
                pack.dependency_kind(name, {})
        with tempfile.TemporaryDirectory() as temporary:
            deps = {"nexa-desktop.exe": ["comctl32.dll", "vcruntime140.dll"], "vcruntime140.dll": ["kernel32.dll"]}
            copied = []
            result = pack.collect_dependencies(Path(temporary), {"vcruntime140.dll": "source"}, lambda path: deps[path.name], lambda source, dest: copied.append(dest.name))
            self.assertEqual(set(result), set(deps))
            self.assertEqual(copied, ["vcruntime140.dll"])

    def test_failed_promotion_preserves_previous_package(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            dist = root / "dist"
            dist.mkdir()
            (dist / "desktop-windows").mkdir()
            (dist / "desktop-windows/old").write_text("prior", encoding="utf-8")
            stage = root / "desktop-windows"
            stage.mkdir()
            with self.assertRaises(FileNotFoundError):
                pack.promote(stage, root / "desktop-windows.zip", root / "desktop-windows.zip.sha256", dist)
            self.assertEqual((dist / "desktop-windows/old").read_text(encoding="utf-8"), "prior")

    def test_capabilities_are_local_exact_and_no_plugin_escape_hatches(self):
        config = json.loads((pack.SHELL / "tauri.conf.json").read_text(encoding="utf-8"))
        capability = json.loads((pack.SHELL / "capabilities/main-window.json").read_text(encoding="utf-8"))
        self.assertEqual(capability["windows"], ["main"])
        self.assertEqual(capability["webviews"], ["main"])
        self.assertTrue(capability["local"])
        self.assertNotIn("remote", capability)
        commands = {"desktop_snapshot", "runtime_start", "model_pick", "model_import", "model_directory_pick", "model_directory_apply", "models_scan", "model_library_next", "model_library_cancel", "models_page", "model_load", "model_unload", "chat_start", "chat_next", "chat_cancel", "settings_save", "runtime_idle_save", "token_copy", "runtime_stop", "desktop_close"}
        commands.update({"model_directory_discover", "model_catalog", "model_download_start", "model_download_next", "model_download_cancel"})
        self.assertEqual(set(capability["permissions"]), {"allow-" + name.replace("_", "-") for name in commands})
        native = (pack.SHELL / "src/windows.rs").read_text(encoding="utf-8")
        import re
        self.assertEqual(set(re.findall(r"#\[tauri::command\]\s*async fn ([a-z_]+)", native)), commands)
        manifest_builder = (pack.SHELL / "build.rs").read_text(encoding="utf-8")
        self.assertEqual(set(re.findall(r'"([a-z_]+)"', manifest_builder)) - {"windows"}, commands)
        directory_request = re.search(r"struct DirectoryRequest \{(.*?)\}", native, re.DOTALL)[1]
        self.assertEqual(re.findall(r"([a-z_]+):", directory_request), ["selection_id"])
        download_request = re.search(r"struct DownloadRequest \{(.*?)\}", native, re.DOTALL)[1]
        self.assertEqual(re.findall(r"([a-z_]+):", download_request), ["catalog_id"])
        self.assertEqual(config["app"]["security"]["capabilities"], ["main-window"])
        self.assertFalse(config["app"]["withGlobalTauri"])
        self.assertFalse(config["app"]["windows"][0]["devtools"])
        self.assertFalse(config["app"]["security"]["assetProtocol"]["enable"])
        self.assertEqual(config["bundle"]["windows"]["webviewInstallMode"]["type"], "skip")
        self.assertNotIn("http://127.0.0.1", config["app"]["security"]["csp"])
        self.assertNotIn("unsafe-eval", config["app"]["security"]["csp"])

    def test_missing_registry_license_requires_exact_upstream_revision(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "crate"
            source.mkdir()
            (source / ".cargo_vcs_info.json").write_text(json.dumps({"git": {"sha1": "wrong"}}), encoding="utf-8")
            metadata = {"packages": [{"id": "root", "name": "nexa-desktop", "source": None}, {"id": "selectors", "name": "selectors", "version": "0.38.0", "source": "registry+test", "manifest_path": str(source / "Cargo.toml")}], "resolve": {"nodes": [{"id": "root", "deps": [{"pkg": "selectors", "dep_kinds": [{"kind": None}]}]}, {"id": "selectors", "deps": []}]}}
            with self.assertRaisesRegex(ValueError, "revision mismatch"):
                pack.copy_rust_licenses(root / "stage", metadata, {})

    def test_reviewed_supplemental_license_hashes(self):
        root = pack.ROOT / "packaging/desktop-windows/third-party"
        sources = json.loads((root / "sources.json").read_text(encoding="utf-8"))
        self.assertEqual({item["package"] for item in sources["files"]}, {"defmt-parser-1.0.0", "selectors-0.38.0", "alloc-stdlib-0.3.0", "webview2-com-0.39.1", "webview2-com-sys-0.39.1", "webview2-com-macros-0.8.1", "clipboard-win-5.4.1"})
        native = json.loads((root / "native-components.json").read_text(encoding="utf-8"))
        for item in sources["files"] + native["components"][0]["license_files"]:
            self.assertEqual(pack.base.digest(root / item["path"]), item["sha256"])
        self.assertEqual(native["components"][0]["crate_file"], "x64/WebView2LoaderStatic.lib")

    def test_tauri_cli_managed_feature_arrays_are_explicit(self):
        # Tauri CLI 2.12.1 persists these arrays before invoking cargo, even with
        # --locked. Omitting them changes source and must not be ignored away.
        manifest = tomllib.loads((pack.SHELL / "Cargo.toml").read_text(encoding="utf-8"))
        tauri = manifest["target"]["cfg(windows)"]["dependencies"]["tauri"]
        build = manifest["build-dependencies"]["tauri-build"]
        self.assertEqual(tauri["version"], "=2.12.1")
        self.assertEqual(build["version"], "=2.7.1")
        self.assertEqual(tauri["features"], [])
        self.assertEqual(build["features"], [])
