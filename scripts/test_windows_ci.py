"""Offline CI plumbing checks; never claim native Windows execution."""
import contextlib
import io
import json
import os
from pathlib import Path
import re
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
        native = Path(folder) / "build/windows-x64-cpu/native-0123456789abcdef"
        selected = {"installationVersion": "17.14.37710.0", "instanceId": "selected-instance", "productId": "Microsoft.VisualStudio.Product.Enterprise"}
        env = {"PATH": "selected compiler", "VCTOOLSVERSION": "14.44.35207", "CMAKE_BUILD_PARALLEL_LEVEL": "4", "NEXA_CMAKE_BIN": "selected CMake bin",
               "VCTOOLSINSTALLDIR": "C:/VS/VC/Tools/MSVC/14.44.35207", "WINDOWSSDKDIR": "C:/Windows Kits/10", "WINDOWSSDKVERSION": "10.0.26100.0/",
               "UNIVERSALCRTSDKDIR": "C:/Windows Kits/10", "UCRTVERSION": "10.0.26100.0", "IMAGEOS": "win22", "IMAGEVERSION": "20260927.320.1",
               "GITHUB_TOKEN": "never exported", "RUNNER_CONTROL": "never exported"}
        configure = ["cmake", "-B", native, "-G", "Visual Studio 17 2022"]
        return native, selected, env, configure

    def invoke(self, action, folder, source_outputs=None, native_override=None):
        native, selected, env, configure = self.fixture(folder)
        redist = self.crt_fixture(folder)
        with contextlib.ExitStack() as stack:
            stack.enter_context(mock.patch.object(ci.sys, "platform", "win32"))
            stack.enter_context(mock.patch.object(ci.sys, "maxsize", 2**63 - 1))
            stack.enter_context(mock.patch.object(ci.base, "selected_visual_studio", return_value=(selected, Path(folder), env, env, redist)))
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
            target = Path(folder) / "build/windows-x64-cpu/cargo-0123456789abcdef"
            self.assertIn(f"NEXA_RUNTIME_CARGO_TARGET_DIR={target.resolve()}\n", data)
            self.assertRegex(data, r"(?m)^NEXA_WINDOWS_CACHE_KEY=[a-f0-9]{64}$")
            self.assertRegex(data, r"(?m)^NEXA_WINDOWS_HANDOFF_KEY=[a-f0-9]{64}$")
            self.assertIn("PATH=selected compiler\n", data)
            self.assertIn("NEXA_CMAKE_BIN=selected CMake bin\n", data)
            self.assertNotIn("GITHUB_TOKEN", data)
            self.assertNotIn("RUNNER_CONTROL", data)

    def test_cache_partition_changes_when_effective_build_identity_changes(self):
        with tempfile.TemporaryDirectory() as folder:
            _, selected, env, configure = self.fixture(folder)
            vs = Path(folder) / "VS"
            generator = configure[configure.index("-G") + 1]
            rust, cmake = "rustc 1.98.1\nhost: x86_64-pc-windows-msvc", "cmake version 4.4.3"
            original = ci.windows_cache_key(selected, vs, env, generator, rust, cmake)
            for key in selected:
                with self.subTest(visual_studio=key):
                    changed = {**selected, key: selected[key] + "-changed"}
                    self.assertNotEqual(original, ci.windows_cache_key(changed, vs, env, generator, rust, cmake))
            for key in ci.CACHE_IDENTITY_ENVIRONMENT:
                with self.subTest(environment=key):
                    changed = {**env, key: env[key] + "-changed"}
                    self.assertNotEqual(original, ci.windows_cache_key(selected, vs, changed, generator, rust, cmake))
            for identity in ((vs / "other", generator, rust, cmake),
                             (vs, "Visual Studio 18 2026", rust, cmake),
                             (vs, generator, rust + "\nLLVM version: changed", cmake),
                             (vs, generator, rust, "cmake version 4.4.4")):
                with self.subTest(identity=identity):
                    self.assertNotEqual(original, ci.windows_cache_key(selected, identity[0], env, *identity[1:]))
            with mock.patch.object(ci.base, "ROOT", Path(folder) / "different-checkout"):
                self.assertNotEqual(original, ci.windows_cache_key(selected, vs, env, generator, rust, cmake))

    def test_cache_partition_ignores_credentials_commit_and_environment_order(self):
        with tempfile.TemporaryDirectory() as folder:
            _, selected, env, _ = self.fixture(folder)
            vs = Path(folder)
            arguments = ("Visual Studio 17 2022", "checked Rust", "checked CMake")
            original = ci.windows_cache_key(selected, vs, env, *arguments)
            changed = dict(reversed(list(env.items())))
            changed.update(GITHUB_TOKEN="different credential", GITHUB_SHA="b" * 40,
                           RUNNER_CONTROL="different control", GITHUB_RUN_ID="456")
            self.assertEqual(original, ci.windows_cache_key(dict(reversed(list(selected.items()))), vs, changed, *arguments))

    def crt_fixture(self, folder):
        directory = Path(folder) / "redist"
        directory.mkdir(parents=True)
        result = {}
        for name in ("vcruntime140.dll", "msvcp140.dll"):
            file = directory / name
            file.write_bytes(("test CRT bytes for " + name).encode("ascii"))
            result[name] = file
        return result

    def test_handoff_allows_different_runner_instances_with_identical_tools_and_crt(self):
        with tempfile.TemporaryDirectory() as folder:
            _, selected, env, _ = self.fixture(folder)
            redist = self.crt_fixture(Path(folder) / "runner-one")
            other_redist = self.crt_fixture(Path(folder) / "runner-two")
            rust, cmake = "checked Rust", "checked CMake"
            original = ci.windows_handoff_key(env, rust, cmake, redist)
            changed = {**env, "IMAGEVERSION": "different-image", "VCTOOLSINSTALLDIR": "D:/VS/Tools",
                       "WINDOWSSDKDIR": "D:/SDK", "UNIVERSALCRTSDKDIR": "D:/SDK",
                       "NEXA_CMAKE_BIN": "D:/CMake/bin"}
            other_selected = {**selected, "instanceId": "different-instance"}
            self.assertNotEqual(ci.windows_cache_key(selected, Path(folder), env, "VS", rust, cmake),
                                ci.windows_cache_key(other_selected, Path(folder) / "other-VS", changed, "VS", rust, cmake))
            self.assertEqual(original, ci.windows_handoff_key(changed, rust, cmake, other_redist))

    def test_handoff_rejects_different_tool_versions_or_crt_bytes(self):
        with tempfile.TemporaryDirectory() as folder:
            _, _, env, _ = self.fixture(folder)
            redist = self.crt_fixture(folder)
            rust, cmake = "checked Rust", "checked CMake"
            original = ci.windows_handoff_key(env, rust, cmake, redist)
            for key in ("VCTOOLSVERSION", "WINDOWSSDKVERSION", "UCRTVERSION"):
                with self.subTest(version=key):
                    self.assertNotEqual(original, ci.windows_handoff_key({**env, key: "different"}, rust, cmake, redist))
            self.assertNotEqual(original, ci.windows_handoff_key(env, rust + " changed", cmake, redist))
            self.assertNotEqual(original, ci.windows_handoff_key(env, rust, cmake + " changed", redist))
            with mock.patch.object(ci.base, "TARGET", "aarch64-pc-windows-msvc"):
                self.assertNotEqual(original, ci.windows_handoff_key(env, rust, cmake, redist))
            self.assertNotEqual(original, ci.windows_handoff_key(env, rust, cmake, {"vcruntime140.dll": redist["vcruntime140.dll"]}))
            redist["vcruntime140.dll"].write_bytes(b"different CRT bytes")
            self.assertNotEqual(original, ci.windows_handoff_key(env, rust, cmake, redist))
            with self.assertRaisesRegex(ValueError, "Release CRT"):
                ci.windows_handoff_key(env, rust, cmake, {})

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

    def test_build_includes_every_registered_ctest_executable(self):
        cmake = (ci.base.ROOT / "native/llama-shim/CMakeLists.txt").read_text(encoding="utf-8")
        registered_tests = re.findall(r"add_test\(\s*NAME\s+(\S+)\s+COMMAND\s+([^\s)]+)", cmake)
        self.assertTrue(registered_tests, "expected native CTest registrations")
        self.assertEqual(len(registered_tests), len(re.findall(r"\badd_test\s*\(", cmake)),
                         "update this check for the new CMake test registration syntax")
        with tempfile.TemporaryDirectory() as folder:
            logged, _, _, _ = self.invoke("build", folder)
            build = logged.call_args_list[1].args[0]
            targets = build[build.index("--target") + 1:build.index("--parallel")]
            for name, executable in registered_tests:
                with self.subTest(test=name, executable=executable):
                    self.assertIn(executable, targets,
                                  "CTest executable must be built before running the suite")

    def test_build_rejects_toolchain_selection_drift(self):
        with tempfile.TemporaryDirectory() as folder, self.assertRaisesRegex(ValueError, "selection changed"):
            self.invoke("build", folder, native_override=Path(folder) / "other-instance")

    def test_non_windows_or_32_bit_host_is_not_native_evidence(self):
        for platform, maxsize in (("linux", 2**63 - 1), ("win32", 2**31 - 1)):
            with self.subTest(platform=platform), mock.patch.object(ci.sys, "platform", platform), mock.patch.object(ci.sys, "maxsize", maxsize), self.assertRaisesRegex(ValueError, "native Windows x64"):
                ci.run("prepare")

    def test_workflow_retains_full_pipeline_with_public_standard_runners(self):
        workflow = (ci.base.ROOT / ".github/workflows/native-windows.yml").read_text(encoding="utf-8")
        self.assertIn("branches: ['codex/dev']", workflow)
        # Each independent job must keep its own public-repository guard;
        # release also retains the stricter tag-push condition below.
        job_section = workflow.split("jobs:\n", 1)[1]
        matches = list(re.finditer(r"^  ([a-z][a-z0-9-]*):\s*$", job_section, re.M))
        jobs = {match[1]: job_section[match.end():matches[index + 1].start() if index + 1 < len(matches) else len(job_section)]
                for index, match in enumerate(matches)}
        public_builds = {"release-identity", "download-component", "desktop-build", "runtime-build", "native"}
        self.assertEqual(set(jobs), public_builds | {"release"})
        for job in public_builds:
            with self.subTest(public_job=job):
                self.assertIn("if: ${{ !github.event.repository.private }}", jobs[job])
                self.assertRegex(jobs[job], r"runs-on: (?:ubuntu-24\.04|windows-2022)\n")
        def dependencies(name):
            line = re.search(r"^    needs: (.+)$", jobs[name], re.M)
            self.assertIsNotNone(line, f"{name} needs explicit successful prerequisites")
            return {item.strip() for item in line[1].strip("[]").split(",")}
        # Preserve the useful parallel edge and require both actual build
        # results before any final packaging/installer acceptance can run.
        self.assertEqual(dependencies("desktop-build"), {"release-identity"})
        self.assertNotIn("desktop-build", dependencies("runtime-build"))
        self.assertTrue({"desktop-build", "runtime-build"} <= dependencies("native"))
        for kind in ("desktop", "runtime"):
            with self.subTest(current_run_handoff=kind):
                self.assertIn(f"scripts/ci_build_handoff.py stage --kind {kind}", jobs[kind + "-build"])
                self.assertIn(f"scripts/ci_build_handoff.py restore --kind {kind}", jobs["native"])
        # Tauri's production hook must remain the single frontend build.
        configuration = json.loads((ci.base.ROOT / "apps/desktop/src-tauri/tauri.conf.json").read_text(encoding="utf-8"))
        self.assertEqual(configuration["build"]["beforeBuildCommand"], "npm run build")
        self.assertNotRegex(workflow, re.compile(r"^\s+npm run build\s*$", re.M))
        # A host-target omission or a new target directory would silently
        # compile the expensive Release dependency graph a second time.
        harness = re.search(r"^\s+cargo build --locked --release ([^\n]+) --bin nexa-desktop-harness\s*$",
                            jobs["runtime-build"], re.M)
        self.assertIsNotNone(harness)
        self.assertIn("--target x86_64-pc-windows-msvc", harness[1])
        self.assertLess(jobs["runtime-build"].index("$env:CARGO_TARGET_DIR = $env:NEXA_RUNTIME_CARGO_TARGET_DIR"), harness.start())
        self.assertIn("Join-Path $env:CARGO_TARGET_DIR 'x86_64-pc-windows-msvc/release/nexa-desktop-harness.exe'",
                      jobs["runtime-build"])
        for runner in ("ubuntu-24.04", "windows-2022"):
            self.assertIn("runs-on: " + runner, workflow)
        self.assertIn("cancel-in-progress: ${{ github.ref_type != 'tag' }}", workflow)
        self.assertIn("github.event_name == 'push' && github.ref_type == 'tag' && !github.event.repository.private", workflow)
        self.assertEqual(workflow.count("contents: write"), 1)
        self.assertIn("needs: [release-identity, download-component, native]", workflow)
        for job in ("desktop-build", "runtime-build"):
            with self.subTest(early_installer_compile=job):
                compile_or_check = re.search(r"\bcargo (?:build|test|clippy|tree|fmt)\b", jobs[job])
                self.assertIsNotNone(compile_or_check)
                self.assertLess(jobs[job].index("--check-toolchain"), compile_or_check.start())
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
