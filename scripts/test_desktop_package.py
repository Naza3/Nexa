import json
from pathlib import Path
import tempfile
import tomllib
import unittest
from types import SimpleNamespace
from unittest import mock
import package_desktop_windows as pack


class DesktopPackageTests(unittest.TestCase):
    def download_fixture(self, root):
        component = root / "download"
        component.mkdir()
        for name in pack.DOWNLOAD_FILES - {"build-manifest.json"}:
            path = component / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b"fixture")
        pe = bytearray(128)
        pe[:2] = b"MZ"
        pe[0x3c:0x40] = (64).to_bytes(4, "little")
        pe[64:70] = b"PE\0\0\x64\x86"
        (component / "nexa-aria2.exe").write_bytes(pe)
        files = {item["path"]: {"sha256": item["sha256"], "bytes": item["size_bytes"]} for item in pack.base.entries(component)}
        build = {"schema_version": 1, "source_commit": "a" * 40, "source_lock": json.loads((pack.ROOT / "third_party/aria2/source-lock.json").read_text(encoding="utf-8")),
                 "binary_name": "nexa-aria2.exe", "product_relative_path": "download/nexa-aria2.exe", "target": "x86_64-w64-mingw32", "tls_backend": "Schannel", "features": {"SECURITY_WIN32": True, "ENABLE_SSL": True, **dict.fromkeys(pack.aria2_build.FORBIDDEN, False)}, "files": files}
        pack.base.write_json(component / "build-manifest.json", build)
        self.seal_download(component)

    def seal_download(self, component):
        for name in ("manifest.json", "SHA256SUMS"):
            (component / name).unlink(missing_ok=True)
        pack.base.write_json(component / "manifest.json", {"product": "nexa-download", "project_commit": "a" * 40, "project_dirty": False, "files": pack.base.entries(component)})
        (component / "SHA256SUMS").write_text("".join(f"{item['sha256']}  {item['path']}\n" for item in pack.base.entries(component)), encoding="utf-8")

    def test_source_identity_does_not_depend_on_package_stage(self):
        with mock.patch.object(pack.base, "command", side_effect=["", "a" * 40, ""]):
            identity = pack.source_identity({})
        self.assertEqual(identity["commit"], "a" * 40)
        self.assertFalse(identity["dirty"])

    def test_build_runs_source_identity_before_runtime_admission(self):
        # Exercise the production entry path without pretending to inspect PE or
        # run Visual Studio on Linux. The deliberate next-stage failure proves
        # source identity does not reference a not-yet-created package stage.
        with mock.patch.object(pack, "os", SimpleNamespace(name="nt", environ={})), \
             mock.patch.object(pack.base, "selected_visual_studio", return_value=({}, Path("vs"), {}, None, {})), \
             mock.patch.object(pack.base, "command", side_effect=["", "a" * 40, ""]), \
             mock.patch.object(pack.base, "regular", side_effect=ValueError("runtime-admission-reached")):
            with self.assertRaisesRegex(ValueError, "runtime-admission-reached"):
                pack.build(Path("desktop.exe"), Path("download"))

    def test_nested_download_is_independently_reverified(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest = self.fixture(root)
            (root / "download/licenses/aria2-COPYING").write_bytes(b"changed")
            self.seal_download(root / "download")
            manifest["files"] = pack.base.entries(root)
            with self.assertRaises(ValueError):
                pack.verify(root, manifest)

    def test_download_closed_roles_and_provenance(self):
        for mutation in ("extra", "missing-license", "source", "lock", "payload", "empty-dir", "test-exe"):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                self.download_fixture(root)
                component = root / "download"
                pack.verify_download(component, "a" * 40, False)
                if mutation in ("source", "lock"):
                    path = component / "build-manifest.json"
                    value = json.loads(path.read_text(encoding="utf-8"))
                    value["source_commit" if mutation == "source" else "source_lock"] = "wrong"
                    pack.base.write_json(path, value)
                elif mutation == "missing-license":
                    (component / "licenses/aria2-COPYING").unlink()
                elif mutation == "empty-dir":
                    (component / "unknown").mkdir()
                elif mutation == "payload":
                    (component / pack.DOWNLOAD_SOURCE).write_bytes(b"changed")
                else:
                    (component / ("extra.txt" if mutation == "extra" else "policy_unit.exe")).write_bytes(b"unexpected")
                self.seal_download(component)
                with self.assertRaises(ValueError):
                    pack.verify_download(component, "a" * 40, False)

    def fixture(self, root):
        self.download_fixture(root)
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
        commands.update({"models_pick", "models_add", "models_selection_discard", "model_directory_configure", "models_reconcile", "model_test", "model_directory_discover", "model_catalog", "model_download_start", "model_download_next", "model_download_cancel"})
        self.assertEqual(set(capability["permissions"]), {"allow-" + name.replace("_", "-") for name in commands})
        native = (pack.SHELL / "src/windows.rs").read_text(encoding="utf-8")
        import re
        self.assertEqual(set(re.findall(r"#\[tauri::command\]\s*async fn ([a-z_]+)", native)), commands)
        manifest_builder = (pack.SHELL / "build.rs").read_text(encoding="utf-8")
        self.assertEqual(set(re.findall(r'"([a-z_]+)"', manifest_builder)) - {"windows"}, commands)
        directory_request = re.search(r"struct DirectoryRequest \{(.*?)\}", native, re.DOTALL)[1]
        self.assertEqual(re.findall(r"([a-z_]+):", directory_request), ["selection_id"])
        download_request = re.search(r"struct DownloadRequest \{(.*?)\}", native, re.DOTALL)[1]
        self.assertEqual(re.findall(r"([a-z_]+):", download_request), ["catalog_id", "auto_test"])
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
