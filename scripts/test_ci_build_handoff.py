import copy
import json
import os
from pathlib import Path
import shutil
import stat
import tempfile
import unittest
from unittest import mock
import zipfile

import ci_build_handoff as handoff
from test_license_bundle import compact_fixture


class HandoffTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.home = Path(temporary.name)
        self.root = self.home / "repo"
        self.root.mkdir()
        (self.root / "Cargo.lock").write_text("locked fixture\n", encoding="utf-8")
        self.artifact = self.home / "artifact"
        self.source = {"commit": "a" * 40, "dirty": False, "worktree_files_sha256": "b" * 64}
        for patch in (
            mock.patch.object(handoff, "ROOT", self.root),
            mock.patch.object(handoff.desktop, "source_identity", return_value=self.source),
            mock.patch.dict(os.environ, {"GITHUB_SHA": "a" * 40, "GITHUB_RUN_ID": "1234",
                                       "GITHUB_RUN_ATTEMPT": "1", "NEXA_WINDOWS_HANDOFF_KEY": "e" * 64}),
        ):
            patch.start()
            self.addCleanup(patch.stop)

    def files(self, kind):
        for name in handoff.FILES[kind]:
            file = self.root / name
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_bytes(b"MZ synthetic test input, never executed")

    def remove_outputs(self, kind):
        for name in handoff.FILES[kind]:
            (self.root / name).unlink()

    def ready(self, kind="desktop"):
        if kind == "runtime":
            self.runtime()
        else:
            self.files(kind)
        handoff.stage(kind, self.artifact)
        self.remove_outputs(kind)

    def manifest(self):
        return json.loads((self.artifact / handoff.MANIFEST).read_text(encoding="utf-8"))

    def save_manifest(self, value):
        handoff.base.write_json(self.artifact / handoff.MANIFEST, value)

    def reseal(self):
        manifest = self.manifest()
        manifest["files"] = [handoff.file_record(self.artifact, name) for name in sorted(handoff.FILES["runtime"])]
        self.save_manifest(manifest)

    def runtime(self):
        self.files("runtime")
        for name, product in handoff.PACKAGES.items():
            stage = self.home / name
            stage.mkdir()
            compact_fixture(stage, html=False)
            required = ("ai-runtime.exe", "ai-runtime-worker.exe", "config.example.toml", "README.md") if name == "windows-x64-cpu" else ("nexa-acceptance.exe",)
            for filename in (*required, "THIRD_PARTY_NOTICES.md"):
                (stage / filename).write_bytes(b"synthetic complete package fixture")
            manifest = {"product": product, "project_commit": self.source["commit"], "project_dirty": False,
                        "source": {**self.source, "tree": "c" * 40}, "target": handoff.base.TARGET,
                        "configuration": "Release", "cargo_lock_sha256": handoff.base.digest(self.root / "Cargo.lock"),
                        "dependencies": {}, "files": handoff.base.entries(stage)}
            handoff.base.write_json(stage / "manifest.json", manifest)
            (stage / "SHA256SUMS").write_text("".join(f"{r['sha256']}  {r['path']}\n" for r in handoff.base.entries(stage)), encoding="utf-8")
            archive = self.root / "dist" / (name + ".zip")
            with zipfile.ZipFile(archive, "w") as zipped:
                for record in handoff.base.entries(stage):
                    zipped.write(stage / record["path"], name + "/" + record["path"])
            archive.with_suffix(".zip.sha256").write_text(f"{handoff.base.digest(archive)}  {archive.name}\n", encoding="utf-8")

    def alter_zip(self, transform):
        archive = self.artifact / "dist/windows-x64-cpu.zip"
        with zipfile.ZipFile(archive) as zipped:
            members = [(info, zipped.read(info)) for info in zipped.infolist()]
        with zipfile.ZipFile(archive, "w") as zipped:
            for name, raw in transform(members):
                zipped.writestr(name, raw)
        archive.with_suffix(".zip.sha256").write_text(f"{handoff.base.digest(archive)}  {archive.name}\n", encoding="utf-8")
        self.reseal()

    def test_desktop_round_trip_and_failed_job_rerun_share_only_same_run(self):
        self.ready()
        with mock.patch.dict(os.environ, {"GITHUB_RUN_ATTEMPT": "2", "NEXA_WINDOWS_CACHE_KEY": "different-runner-local-cache"}):
            handoff.restore("desktop", self.artifact)
        for name in handoff.FILES["desktop"]:
            self.assertEqual((self.root / name).read_bytes(), (self.artifact / name).read_bytes())
        self.assertNotIn("run_attempt", self.manifest())

    def test_runtime_round_trip_runs_real_package_and_license_verifiers(self):
        self.ready("runtime")
        with mock.patch.object(handoff.base, "verify_package", wraps=handoff.base.verify_package) as verify:
            handoff.restore("runtime", self.artifact)
        self.assertEqual(verify.call_count, 2)
        for name in handoff.FILES["runtime"]:
            self.assertEqual((self.root / name).read_bytes(), (self.artifact / name).read_bytes())
        for name in handoff.PACKAGES:
            self.assertTrue((self.root / "dist" / name / "manifest.json").is_file())

    def test_run_toolchain_source_and_role_mismatches_fail_before_copy(self):
        self.ready()
        original = self.manifest()
        variants = []
        for field, value in (("run_id", "9999"), ("windows_toolchain_key", "different-msvc"), ("kind", "runtime"), ("schema_version", True)):
            variants.append({**original, field: value})
        for field, value in (("commit", "c" * 40), ("dirty", 0), ("worktree_files_sha256", "d" * 64)):
            variants.append({**original, "source": {**original["source"], field: value}})
        for value in variants:
            with self.subTest(value=value):
                self.save_manifest(value)
                with self.assertRaisesRegex(ValueError, "identity mismatch"):
                    handoff.restore("desktop", self.artifact)
                self.assertFalse((self.root / handoff.FILES["desktop"][0]).exists())

    def test_closed_inventory_rejects_extra_missing_changed_and_empty_directories(self):
        self.ready()
        name = handoff.FILES["desktop"][0]
        executable = self.artifact / name
        original = executable.read_bytes()
        for mode in ("extra", "missing", "changed", "empty-directory"):
            with self.subTest(mode=mode):
                extra = self.artifact / "unexpected"
                if mode == "extra":
                    extra.write_bytes(b"must not be transferred")
                elif mode == "missing":
                    executable.unlink()
                elif mode == "changed":
                    executable.write_bytes(original + b"tampered")
                else:
                    extra.mkdir()
                with self.assertRaisesRegex(ValueError, "file set/hash/size"):
                    handoff.restore("desktop", self.artifact)
                if extra.is_dir():
                    extra.rmdir()
                else:
                    extra.unlink(missing_ok=True)
                executable.write_bytes(original)

    def test_invalid_records_and_duplicate_json_keys_are_rejected(self):
        self.ready()
        original = self.manifest()
        for field, value in (("path", "../escape.exe"), ("size_bytes", True), ("sha256", "not-a-digest")):
            with self.subTest(field=field):
                changed = copy.deepcopy(original)
                changed["files"][0][field] = value
                self.save_manifest(changed)
                with self.assertRaisesRegex(ValueError, "record is invalid"):
                    handoff.restore("desktop", self.artifact)
        (self.artifact / handoff.MANIFEST).write_text('{"schema_version":1,"schema_version":1}', encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "duplicate JSON key"):
            handoff.restore("desktop", self.artifact)

    def test_existing_output_or_staging_directory_is_never_overwritten(self):
        self.files("desktop")
        handoff.stage("desktop", self.artifact)
        with self.assertRaisesRegex(ValueError, "overwrite"):
            handoff.stage("desktop", self.artifact)
        target = self.root / handoff.FILES["desktop"][0]
        target.write_bytes(b"existing local output")
        with self.assertRaisesRegex(ValueError, "overwrite"):
            handoff.restore("desktop", self.artifact)
        self.assertEqual(target.read_bytes(), b"existing local output")

    def test_symlink_file_and_destination_ancestor_are_rejected(self):
        self.ready()
        executable = self.artifact / handoff.FILES["desktop"][0]
        original = executable.read_bytes()
        target = self.home / "external.exe"
        target.write_bytes(original)
        executable.unlink()
        try:
            executable.symlink_to(target)
        except OSError as error:
            self.skipTest("creating symlinks is unavailable: " + str(error.errno))
        with self.assertRaisesRegex(ValueError, "symlink/reparse"):
            handoff.restore("desktop", self.artifact)
        executable.unlink()
        executable.write_bytes(original)
        shutil.rmtree(self.root / "build")
        (self.root / "build").symlink_to(self.artifact / "build", target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "symlink/reparse"):
            handoff.restore("desktop", self.artifact)
        self.assertEqual(target.read_bytes(), original)

    def test_zip_paths_and_link_members_fail_before_promotion(self):
        self.ready("runtime")
        archive = self.artifact / "dist/windows-x64-cpu.zip"
        original = archive.read_bytes()
        paths = ("../escaped", "/absolute", "windows-x64-cpu/../escaped", "windows-x64-cpu/C:stream",
                 "windows-x64-cpu/evil\\path", "windows-x64-cpu/NUL.txt", "windows-x64-cpu/README.md.",
                 "windows-x64-cpu/readme.md", "other-package/file")
        for path in (*paths, "symlink"):
            with self.subTest(path=path):
                archive.write_bytes(original)
                member = path
                if path == "symlink":
                    member = zipfile.ZipInfo("windows-x64-cpu/linked")
                    member.create_system = 3
                    member.external_attr = (stat.S_IFLNK | 0o777) << 16
                self.alter_zip(lambda values: [*values, (member, b"unsafe")])
                with self.assertRaises(ValueError):
                    handoff.restore("runtime", self.artifact)
                self.assertFalse((self.root / "dist/windows-x64-cpu").exists())
                self.assertFalse((self.root / "dist/windows-x64-cpu.zip").exists())
                self.assertFalse((self.root / "escaped").exists())

    def test_zip_checksum_and_inner_package_integrity_are_independent_gates(self):
        self.ready("runtime")
        checksum = self.artifact / "dist/windows-x64-cpu.zip.sha256"
        checksum.write_text("0" * 64 + "  windows-x64-cpu.zip\n", encoding="utf-8")
        self.reseal()
        with self.assertRaisesRegex(ValueError, "ZIP checksum mismatch"):
            handoff.restore("runtime", self.artifact)
        self.alter_zip(lambda values: [(name, b"corrupt payload" if name.filename.endswith("ai-runtime.exe") else raw) for name, raw in values])
        with self.assertRaisesRegex(ValueError, "manifest file set/hash/size"):
            handoff.restore("runtime", self.artifact)
        self.assertFalse((self.root / "dist/windows-x64-cpu.zip").exists())

    def test_package_from_other_commit_cannot_be_relabelled_by_handoff(self):
        self.ready("runtime")
        def wrong_source(values):
            for member, raw in values:
                if member.filename.endswith("manifest.json"):
                    value = json.loads(raw)
                    value["project_commit"] = "d" * 40
                    raw = json.dumps(value).encode("utf-8")
                yield member, raw
        self.alter_zip(wrong_source)
        with self.assertRaisesRegex(ValueError, "source/lock/Release"):
            handoff.restore("runtime", self.artifact)

    def test_missing_ci_identity_dirty_checkout_and_mid_copy_source_drift_fail(self):
        self.files("desktop")
        for variable in ("GITHUB_SHA", "GITHUB_RUN_ID", "NEXA_WINDOWS_HANDOFF_KEY"):
            with self.subTest(variable=variable), mock.patch.dict(os.environ, {variable: ""}):
                with self.assertRaisesRegex(ValueError, "requires exact CI"):
                    handoff.stage("desktop", self.artifact)
        with mock.patch.object(handoff.desktop, "source_identity", return_value={**self.source, "dirty": True}):
            with self.assertRaisesRegex(ValueError, "clean exact CI source"):
                handoff.stage("desktop", self.artifact)
        with mock.patch.object(handoff.desktop, "source_identity", side_effect=[self.source, {**self.source, "worktree_files_sha256": "f" * 64}]):
            with self.assertRaisesRegex(ValueError, "changed while staging"):
                handoff.stage("desktop", self.artifact)
        self.assertFalse(self.artifact.exists())

    def test_restore_source_drift_keeps_all_destinations_unpublished(self):
        self.ready("runtime")
        with mock.patch.object(handoff.desktop, "source_identity", side_effect=[self.source, {**self.source, "worktree_files_sha256": "f" * 64}]):
            with self.assertRaisesRegex(ValueError, "changed while restoring"):
                handoff.restore("runtime", self.artifact)
        for name in handoff.FILES["runtime"]:
            self.assertFalse((self.root / name).exists())
        for name in handoff.PACKAGES:
            self.assertFalse((self.root / "dist" / name).exists())

    def test_failed_promotion_rolls_back_only_newly_owned_outputs(self):
        self.ready("runtime")
        sentinel = self.root / "dist/unrelated-local-file"
        sentinel.write_bytes(b"preserve this")
        rename = Path.rename
        def fail_second_move(source, destination):
            if destination == self.root / "dist/acceptance-tools.zip":
                raise OSError("simulated destination I/O error")
            return rename(source, destination)
        with mock.patch.object(Path, "rename", fail_second_move):
            with self.assertRaisesRegex(OSError, "simulated destination"):
                handoff.restore("runtime", self.artifact)
        for name in handoff.FILES["runtime"]:
            self.assertFalse((self.root / name).exists())
        self.assertEqual(sentinel.read_bytes(), b"preserve this")
        self.assertTrue((self.artifact / handoff.MANIFEST).is_file())

    def test_zip_expanded_size_limit_is_checked_before_promotion(self):
        self.ready("runtime")
        with mock.patch.object(handoff, "MAX_ZIP_BYTES", 1):
            with self.assertRaisesRegex(ValueError, "ZIP exceeds its bounds"):
                handoff.restore("runtime", self.artifact)
        self.assertFalse((self.root / "dist/windows-x64-cpu").exists())


if __name__ == "__main__":
    unittest.main()
