import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import zipfile

import ci_verify as ci

ELF = '''Class: ELF64
Type: DYN (Shared object file)
Machine: AArch64
[Requesting program interpreter: /system/bin/linker64]
LOAD 0x000000 0x000000 0x000000 0x123 0x123 R E 0x4000
LOAD 0x010123 0x014123 0x014123 0x100 0x200 RW 0x4000
GNU_RELRO 0x010123 0x014123 0x014123 0x100 0x3edd R 0x1
GNU_STACK 0x000000 0x000000 0x000000 0x000 0x000 RW 0x0
(NEEDED) Shared library: [libandroid.so]
(NEEDED) Shared library: [libc.so]
(NEEDED) Shared library: [libdl.so]
(NEEDED) Shared library: [liblog.so]
(NEEDED) Shared library: [libm.so]
'''


class CiTests(unittest.TestCase):
    def test_elf_happy_path(self):
        result = ci.parse_elf(ELF)
        self.assertEqual(result['machine'], 'AArch64')
        self.assertFalse(result['android_run'])
        self.assertEqual(len(result['load_segments']), 2)

    def test_elf_rejects_identity_dependency_alignment_and_relro(self):
        mutations = [('AArch64', 'X86-64'), ('ELF64', 'ELF32'), ('DYN', 'EXEC'),
                     ('/system/bin/linker64', '/other/linker'), ('libm.so', 'libMNN.so'),
                     ('0x4000', '0x1000'), ('GNU_RELRO', 'IGNORED'), ('0x3edd', '0x3edc'),
                     ('0x014123', '0x014124'), ('0x000 RW 0x0', '0x000 RWE 0x0')]
        for before, after in mutations:
            with self.subTest(before=before), self.assertRaises(ValueError):
                ci.parse_elf(ELF.replace(before, after))

    def test_archive_paths_rejected(self):
        for name in ('/tmp/a', 'android-ndk-r30/../a', '../a', 'other/a', 'android-ndk-r30\\a'):
            with self.subTest(name=name), self.assertRaises(ValueError):
                ci.safe_member(name)

    def test_archive_modes_and_contained_symlink(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive = root / 'ndk.zip'
            with zipfile.ZipFile(archive, 'w') as bundle:
                bundle.writestr('android-ndk-r30/source.properties', 'Pkg.Revision = ' + ci.NDK['revision'] + '\n')
                executable = zipfile.ZipInfo('android-ndk-r30/bin/clang')
                executable.external_attr = 0o100755 << 16
                bundle.writestr(executable, 'test data, not executed')
                link = zipfile.ZipInfo('android-ndk-r30/bin/clang++')
                link.external_attr = 0o120777 << 16
                bundle.writestr(link, 'clang')
            ndk = ci.extract_ndk(archive, root)
            self.assertEqual((ndk/'bin/clang').stat().st_mode & 0o777, 0o755)
            self.assertTrue((ndk/'bin/clang++').is_symlink())
            self.assertEqual((ndk/'bin/clang++').read_text(), 'test data, not executed')

    def test_archive_external_symlink_rejected_before_extract(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive = root / 'ndk.zip'
            with zipfile.ZipFile(archive, 'w') as bundle:
                link = zipfile.ZipInfo('android-ndk-r30/bin/bad')
                link.external_attr = 0o120777 << 16
                bundle.writestr(link, '../../outside')
            with self.assertRaisesRegex(ValueError, 'unsafe_ndk_archive_symlink'):
                ci.extract_ndk(archive, root)
            self.assertFalse((root/'android-ndk-r30').exists())

    def test_archive_symlink_ancestor_attack_rejected_before_write(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive = root / 'ndk.zip'
            with zipfile.ZipFile(archive, 'w') as bundle:
                for name, target in (("a", "x/../outside"), ("x", ".")):
                    link = zipfile.ZipInfo('android-ndk-r30/' + name)
                    link.external_attr = 0o120777 << 16
                    bundle.writestr(link, target)
                bundle.writestr('android-ndk-r30/a/owned', 'must not escape')
            with self.assertRaisesRegex(ValueError, 'ndk_archive_symlink_ancestor'):
                ci.extract_ndk(archive, root)
            self.assertFalse((root/'outside/owned').exists())
            self.assertFalse((root/'android-ndk-r30').exists())

    def test_download_validates_before_rename(self):
        import hashlib
        body = b'controlled fixture'
        with tempfile.TemporaryDirectory() as directory, patch.object(ci.urllib.request, 'urlopen', return_value=io.BytesIO(body)):
            path = Path(directory)/'fixture'
            ci.download('https://example.invalid', path, len(body), hashlib.sha256(body).hexdigest(), hashlib.sha1(body).hexdigest())
            self.assertEqual(path.read_bytes(), body)

    def test_download_rejects_hash_and_size(self):
        for size, sha in ((1, 'wrong'), (3, 'wrong')):
            with tempfile.TemporaryDirectory() as directory, patch.object(ci.urllib.request, 'urlopen', return_value=io.BytesIO(b'abc')):
                path = Path(directory)/'fixture'
                with self.assertRaisesRegex(ValueError, 'download_'):
                    ci.download('https://example.invalid', path, size, sha)
                self.assertFalse(path.exists())

    def test_sanitizer_rejects_nested_extra_data_and_paths(self):
        for item in ({'model_files': {'secret': 'private'}}, {'compiler': '/private/path'},
                     {'cases': {'private': 'pass'}}, {'ndk': dict(ci.NDK, private='secret')}):
            with self.subTest(item=item), self.assertRaises(ValueError):
                ci.sanitize(item, set(item))

    def test_stage_failure_evidence_is_allowlisted(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            evidence = root/'evidence'
            ci.write(evidence/'suite/short-en/report.json', {'status': 'failed', 'error': '/private/path', 'output_text': 'secret'})
            ci.write(evidence/'extra.json', {'secret': 'secret'})
            (evidence/'suite/short-en/private-native.log').write_text('secret')
            outcomes = ','.join(key + ':failure' for key in ci.STEPS)
            ci.stage(evidence, root/'linux', root/'android', root/'upload', outcomes)
            files = sorted(p.name for p in (root/'upload').iterdir())
            self.assertEqual(files, ['ci-status.json', 'short-en.json'])
            self.assertEqual(json.loads((root/'upload/short-en.json').read_text()), {'status': 'failed'})
            self.assertFalse(json.loads((root/'upload/ci-status.json').read_text())['all_required_steps_succeeded'])

    @staticmethod
    def complete_reports(root):
        lock = ci.probe.load_json(ci.probe.LOCK)
        ci.write(root/'tools.json', {'status': 'ok', 'cmake_package': '4.4.3', 'ninja_package': '1.13.2'})
        ci.write(root/'setup.json', {'status': 'ok', 'ndk': ci.NDK, 'model_files': lock['files'], 'model_revision': lock['revision']})
        ci.write(root/'suite.json', {'status': 'ok', 'android_run': False, 'repeat_identical': True, 'cases': {case: 'pass' for case in ci.CASES}})
        ci.write(root/'android-elf.json', ci.parse_elf(ELF))
        for case in ci.CASES:
            ci.write(root/(case+'.json'), {'status': 'failed' if case == 'boundary-over' else 'ok',
                'process_exit_code': 1 if case == 'boundary-over' else 0, 'mnn_commit': ci.probe.MNN_COMMIT})
        common = {'prototype': 'T07-A', 'mnn_commit': ci.probe.MNN_COMMIT, 'patches': 'none',
            'build_type': 'Release', 'backend': 'cpu', 'http': 'off', 'omni': 'off', 'sampler': 'load-time-greedy',
            'cancel': 'unsupported', 'cmake': '4.4.3', 'compiler': 'fixture-only'}
        ci.write(root/'linux-build.json', dict(common, system='Linux', processor='x86_64'))
        ci.write(root/'android-build.json', dict(common, system='Android', processor='aarch64', android_abi='arm64-v8a', android_api='android-28'))

    def test_complete_report_set_passes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.complete_reports(root)
            ci.verify_staged_reports(root)

    def test_report_status_build_and_expected_negative_gates(self):
        for name, changes in (('boundary-over.json', {'status': 'ok'}),
                              ('short-en.json', {'process_exit_code': 1}),
                              ('suite.json', {'cases': {'short-en': 'pass'}}),
                              ('setup.json', {'status': 'failed'}),
                              ('android-build.json', {'android_api': 'android-29'})):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                self.complete_reports(root)
                report = ci.probe.load_json(root/name)
                report.update(changes)
                ci.write(root/name, report)
                with self.assertRaises(ValueError):
                    ci.verify_staged_reports(root)

    def test_stage_success_missing_evidence_fails_and_preserves_status(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            outcomes = ','.join(key + ':success' for key in ci.STEPS)
            with self.assertRaisesRegex(ValueError, 'required_evidence_missing'):
                ci.stage(root/'missing', root/'linux', root/'android', root/'upload', outcomes)
            status = json.loads((root/'upload/ci-status.json').read_text())
            self.assertTrue(status['steps_succeeded'])
            self.assertFalse(status['all_required_steps_succeeded'])
            self.assertFalse(status['evidence_verified'])

    def test_stage_dirty_ci_fails_after_preserving_evidence(self):
        identity = {'source_commit': 'a'*40, 'source_tree': 'b'*40, 'source_clean': False}
        with tempfile.TemporaryDirectory() as directory, patch.object(ci, 'source_identity', return_value=identity), patch.dict(ci.os.environ, {'GITHUB_SHA': 'a'*40}):
            root = Path(directory)
            outcomes = ','.join(key + ':failure' for key in ci.STEPS)
            ci.write(root/'evidence/tools.json', {'status': 'failed'})
            with self.assertRaisesRegex(ValueError, 'ci_source_not_clean'):
                ci.stage(root/'evidence', root/'linux', root/'android', root/'upload', outcomes)
            self.assertTrue((root/'upload/tools.json').is_file())
            status = json.loads((root/'upload/ci-status.json').read_text())
            self.assertFalse(status['source_clean'])
            self.assertFalse(status['all_required_steps_succeeded'])

    def test_workflow_pins_permissions_and_failure_staging(self):
        workflow = Path(__file__).resolve().parents[2]/'.github/workflows/android-mnn-probe.yml'
        content = workflow.read_text(encoding='utf-8')
        self.assertIn('actions/checkout@11d5960a326750d5838078e36cf38b85af677262', content)
        self.assertIn('actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02', content)
        self.assertIn('contents: read', content)
        self.assertIn('timeout-minutes: 40', content)
        self.assertIn('cmake==4.4.3 ninja==1.13.2', content)
        self.assertIn('ref: ' + ci.probe.MNN_COMMIT, content)
        self.assertNotIn("sdkmanager", content)


if __name__ == '__main__':
    unittest.main()
