"""Offline CI plumbing checks; never claim native Windows execution."""
import contextlib
import io
import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import build_windows_ci as ci


class WindowsCiTests(unittest.TestCase):
    def test_actual_rust_release_and_host_are_checked_not_the_alias(self):
        for rust, good in (("rustc 1.98.1\nrelease: 1.98.1\nhost: x86_64-pc-windows-msvc", True),
                           ("release: 1.98.0\nhost: x86_64-pc-windows-msvc", False),
                           ("release: 1.98.1\nhost: x86_64-unknown-linux-gnu", False),
                           ("release: 1.98.1-nightly\nhost: x86_64-pc-windows-msvc", False)):
            with self.subTest(rust=rust), mock.patch.object(ci.base, "command", return_value=rust):
                if good:
                    self.assertEqual(ci.checked_rust({}), rust)
                else:
                    with self.assertRaisesRegex(ValueError, "actual Rust"):
                        ci.checked_rust({})

    def test_floating_rust_pin_is_not_allowed(self):
        with mock.patch.object(ci.tomllib, "loads", return_value={"toolchain": {"channel": "stable"}}), mock.patch.object(ci.base, "command") as run, self.assertRaisesRegex(ValueError, "floating"):
            ci.checked_rust({})
        run.assert_not_called()

    def test_environment_export_is_utf8_append_only_and_single_line(self):
        with tempfile.TemporaryDirectory() as folder:
            file = Path(folder) / "env"
            file.write_text("EXISTING=1\n", encoding="utf-8")
            ci.export_environment(file, {"AIR_NATIVE_DIR": "C:/构建 路径/native-one"})
            self.assertEqual(file.read_text(encoding="utf-8"), "EXISTING=1\nAIR_NATIVE_DIR=C:/构建 路径/native-one\n")
            for key, value in (("INVALID=NAME", "value"), ("VALID", "x\ny"), ("VALID", "x\ry"), ("VALID", "x\0y")):
                with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                    ci.export_environment(file, {key: value})
            self.assertEqual(file.read_text(encoding="utf-8"), "EXISTING=1\nAIR_NATIVE_DIR=C:/构建 路径/native-one\n")

    def fixture(self, folder):
        native = Path(folder) / "native-selected"
        selected = {"installationVersion": "17.14.37710.0"}
        env = {"PATH": "selected compiler", "VCTOOLSVERSION": "14.44.35207", "CMAKE_BUILD_PARALLEL_LEVEL": "4", "NEXA_CMAKE_BIN": "selected CMake bin",
               "GITHUB_TOKEN": "never exported", "RUNNER_CONTROL": "never exported"}
        configure = ["cmake", "-B", native, "-G", "Visual Studio 17 2022"]
        return native, selected, env, configure

    def invoke(self, action, folder, source_outputs=None, native_override=None):
        native, selected, env, configure = self.fixture(folder)
        with contextlib.ExitStack() as stack:
            stack.enter_context(mock.patch.object(ci.sys, "platform", "win32"))
            stack.enter_context(mock.patch.object(ci.sys, "maxsize", 2**63 - 1))
            stack.enter_context(mock.patch.object(ci.base, "selected_visual_studio", return_value=(selected, Path(folder), env, env, {})))
            settings = stack.enter_context(mock.patch.object(ci.base, "native_build_settings", return_value=(native, configure)))
            stack.enter_context(mock.patch.object(ci, "checked_rust", return_value="checked pinned Rust"))
            stack.enter_context(mock.patch.object(ci.base, "checked_cmake", return_value="checked CMake"))
            stack.enter_context(mock.patch.object(ci.base, "command", side_effect=source_outputs))
            stack.enter_context(mock.patch.dict(os.environ, {"GITHUB_SHA": "a" * 40, "GITHUB_ENV": str(Path(folder) / "env"),
                                                            "AIR_NATIVE_DIR": str(native if native_override is None else native_override)}, clear=True))
            logged = stack.enter_context(mock.patch.object(ci, "run_logged"))
            stack.enter_context(contextlib.redirect_stdout(io.StringIO()))
            ci.run(action)
            return logged, settings, native, configure

    def test_prepare_exports_only_selected_build_environment_and_keyed_native_tree(self):
        with tempfile.TemporaryDirectory() as folder:
            logged, _, native, _ = self.invoke("prepare", folder, ["a" * 40, "", ci.base.LLAMA_COMMIT, ""])
            logged.assert_not_called()
            data = (Path(folder) / "env").read_text(encoding="utf-8")
            self.assertIn(f"AIR_NATIVE_DIR={native}\n", data)
            self.assertIn("PATH=selected compiler\n", data)
            self.assertIn("NEXA_CMAKE_BIN=selected CMake bin\n", data)
            self.assertNotIn("GITHUB_TOKEN", data)
            self.assertNotIn("RUNNER_CONTROL", data)

    def test_prepare_rejects_wrong_or_dirty_project_and_vendor(self):
        for outputs in (["b" * 40], ["a" * 40, " M source"], ["a" * 40, "", "b" * 40],
                        ["a" * 40, "", ci.base.LLAMA_COMMIT, " M vendor-source"]):
            with self.subTest(outputs=outputs), tempfile.TemporaryDirectory() as folder, self.assertRaises(ValueError):
                self.invoke("prepare", folder, outputs)

    def test_build_keeps_all_native_targets_and_ctest_on_the_packager_tree(self):
        with tempfile.TemporaryDirectory() as folder:
            logged, settings, native, configure = self.invoke("build", folder)
            self.assertEqual(settings.call_count, 2)
            self.assertEqual(logged.call_args_list[0].args[0], configure)
            self.assertEqual(logged.call_args_list[1].args[0], ["cmake", "--build", native, "--config", "Release", "--target", *ci.NATIVE_TARGETS, "--parallel", "4"])
            self.assertEqual(logged.call_args_list[2].args[0], ["ctest", "--test-dir", native, "-C", "Release", "--output-on-failure"])

    def test_build_rejects_toolchain_selection_drift(self):
        with tempfile.TemporaryDirectory() as folder, self.assertRaisesRegex(ValueError, "selection changed"):
            self.invoke("build", folder, native_override=Path(folder) / "other-instance")

    def test_non_windows_or_32_bit_host_is_not_native_evidence(self):
        for platform, maxsize in (("linux", 2**63 - 1), ("win32", 2**31 - 1)):
            with self.subTest(platform=platform), mock.patch.object(ci.sys, "platform", platform), mock.patch.object(ci.sys, "maxsize", maxsize), self.assertRaisesRegex(ValueError, "native Windows x64"):
                ci.run("prepare")

    def test_workflow_retains_full_pipeline_with_public_standard_runners(self):
        workflow = (ci.base.ROOT / ".github/workflows/native-windows.yml").read_text(encoding="utf-8")
        self.assertIn("branches: ['codex/nexa-add-model']", workflow)
        self.assertEqual(workflow.count("if: ${{ !github.event.repository.private }}"), 2)
        self.assertEqual(workflow.count("runs-on:"), 2)
        for runner in ("ubuntu-24.04", "windows-2022"):
            self.assertIn("runs-on: " + runner, workflow)
        self.assertIn("cancel-in-progress: true", workflow)
        self.assertNotIn("pull_request:", workflow)
        self.assertNotIn("build/native-release", workflow)
        self.assertNotIn("cmake==", workflow)
        self.assertIn('"cmake>=4.2"', workflow)
        self.assertIn("$env:NEXA_CMAKE_BIN = (python -c", workflow)
        self.assertIn("sysconfig.get_path('scripts')", workflow)
        self.assertIn("RUSTUP_TOOLCHAIN: '1.98.1'", workflow)
        self.assertIn("build/windows-x64-cpu/cargo-*/x86_64-pc-windows-msvc/release/*.pdb", workflow)
        for gate in ("windows_probe.py", "npm run test", "cargo test --locked --workspace", "cargo clippy", "real_model", "real_runtime", "real_credit", "run_api_smoke.py", "run_desktop_smoke.py", "package_windows", "package_acceptance", "desktop_acceptance", "--disconnect-cycles 50"):
            # package_windows is dispatched by xtask, not directly by this YAML.
            if gate != "package_windows":
                self.assertIn(gate, workflow)
        self.assertIn("cargo run --locked -p xtask -- build --platform windows-x64 --backend cpu", workflow)
        self.assertNotIn("retention-days: 14", workflow)


if __name__ == "__main__":
    unittest.main()
