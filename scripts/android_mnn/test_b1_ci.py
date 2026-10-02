import fnmatch
import copy
import json
import os
import subprocess
import time
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import b1_ci as ci

ELF = '''Class: ELF64
Type: DYN (Position-Independent Executable file)
Machine: AArch64
[Requesting program interpreter: /system/bin/linker64]
LOAD 0x000000 0x000000 0x000000 0x001000 0x001000 R E 0x4000
LOAD 0x004000 0x004000 0x004000 0x001000 0x001000 RW 0x4000
GNU_RELRO 0x004000 0x004000 0x004000 0x001000 0x004000 R 0x1
GNU_STACK 0x000000 0x000000 0x000000 0x000000 0x000000 RW 0x10
(NEEDED) Shared library: [libc.so]
(NEEDED) Shared library: [libdl.so]
(NEEDED) Shared library: [libm.so]
'''
IDENTITY = {'source_commit': '1' * 40, 'source_tree': '2' * 40, 'source_clean': True}


def checks(stage):
    result = {key: True for key in ci.CHECK_KEYS[stage]}
    if stage == 'tools':
        return dict(source=IDENTITY.copy(), gcc='13.3.0', rust='1.98.1', cmake='4.4.3', ninja='1.13.2', notice_inventory=ci.notice_identity())
    if stage == 'inputs':
        return dict(ndk_sha256=ci.ci_verify.NDK['sha256'], ndk_revision=ci.ci_verify.NDK['revision'], model_lock_sha256=ci.digest(ci.run_probe.LOCK))
    if stage in ('linux_native', 'android_native'):
        lock = ci.read(ci.ROOT / 'native/mnn-patches/lock.json')
        result['artifact'] = {key: lock[key] for key in ('upstream_commit', 'patch_set_sha256', 'policy_sha256')}
        result['artifact'].update(header_sha256=ci.digest(ci.ROOT / 'native/mnn-shim/include/nexa_mnn.h'),
            target=ci.TARGET if stage == 'android_native' else 'x86_64-unknown-linux-gnu', compiler_sha256='a' * 64, manifest_sha256='c' * 64,
            archives={key: 'b' * 64 for key in ['nexa-mnn-shim', 'MNN'] + (['c++_static', 'c++abi', 'unwind', 'clang_rt_builtins'] if stage == 'android_native' else [])})
        result['artifact']['archive_object_counts'] = {name: 1 for name in result['artifact']['archives']}
        result['logging_audit'] = dict(compiled_source_count=20, header_count=30)
    if stage == 'rust_linux':
        result['linked_manifest_sha256'] = 'c' * 64
        result['b2_real_matrix'] = {'cases': {name: 'pass' for name in ci.B2_REAL_CASES}, 'research_only': True,
                                    'production_admitted': False, 'model_lock_sha256': ci.digest(ci.run_probe.LOCK)}
    if stage == 'native_real':
        return dict(upstream_exact=True, privacy_canaries=9, prompt_tokens=36, completion_tokens=12,
                    cancel_safe_return_ms={key: 0.1 for key in ci.CANCEL_KEYS})
    if stage == 'elf':
        return {key: dict(ci.parse_elf(ELF), sha256='a' * 64, size=10000) for key in ci.ANDROID_TARGETS}
    return result


def report(stage):
    return dict(schema=1, stage=stage, status='ok', failure_case='none', missing_archive=None, android_run=False, exit_codes=[0], commands=[dict(
        index=0, category='verification', timeout_seconds=600, timed_out=False, log_limit_exceeded=False,
        cleanup_confirmed=True, failure_case='none', exit_code=0, log_bytes=0, output_truncated=False, diagnostics=[])], checks=checks(stage))


class EvidenceTests(unittest.TestCase):
    def test_all_stage_schemas(self):
        for stage in ci.STAGES:
            self.assertEqual(ci.sanitize_report(stage, report(stage)), report(stage))

    def test_no_raw_or_unknown_fields(self):
        for stage in ci.STAGES:
            value = report(stage)
            value['prompt'] = 'PRIVATE'
            with self.assertRaises(ValueError):
                ci.sanitize_report(stage, value)
            value = report(stage)
            value['checks']['raw_log'] = 'PRIVATE'
            with self.assertRaises(ValueError):
                ci.sanitize_report(stage, value)

    def test_no_success_with_failed_or_empty_commands(self):
        for codes in ([], [1], [0, 124], [True]):
            value = report('patch')
            value['exit_codes'] = codes
            with self.assertRaises(ValueError):
                ci.sanitize_report('patch', value)

    def test_failed_partial_checks_are_removed(self):
        value = report('native_real')
        value.update(status='failed', failure_case='stage_validation_failed', checks={'raw_log': 'PRIVATE'})
        self.assertEqual(ci.sanitize_report('native_real', value)['checks'], {})

    def test_nested_string_and_false_claims_rejected(self):
        for mutate in (
            lambda v: v['artifact']['archives'].update(MNN='PRIVATE'),
            lambda v: v['artifact'].update(target='wrong'),
            lambda v: v['artifact'].update(compiler_sha256='PRIVATE'),
            lambda v: v['logging_audit'].update(header_count=0),
            lambda v: v.update(ctest_and_stream_sanitizers=False),
        ):
            value = checks('linux_native')
            mutate(value)
            with self.assertRaises(ValueError):
                ci.validate_checks('linux_native', value)

    def test_linked_rust_identity_binds_actual_manifest(self):
        manifest = dict(upstream_commit='a', patch_set_sha256='b', policy_sha256='c', target='x86_64-unknown-linux-gnu', compiler='fixed compiler')
        output = 'identity_schema=nexa-mnn-native-build-v1\nartifact_manifest_sha256=d\nupstream_commit=a\npatch_sha256=b\npolicy_sha256=c\ntarget=x86_64-unknown-linux-gnu\ncompiler=fixed compiler\n'
        ci.verify_rust_identity(output, manifest, 'd')
        for bad in (output.replace('sha256=d', 'sha256=wrong'), output + 'raw=PRIVATE\n', output.replace('target=x86_64', 'target=aarch64')):
            with self.assertRaises(ValueError):
                ci.verify_rust_identity(bad, manifest, 'd')

    def test_metrics_require_all_phases_finite_and_no_logs(self):
        lines = ['abi_sampler_pass']
        for key in sorted(ci.CANCEL_KEYS):
            phase, index = key.rsplit('_', 1)
            lines.append(f'{phase} {index} safe_return_ms 0.25')
        lines.append('real_request_pass')
        self.assertEqual(set(ci.parse_cancel_metrics('\n'.join(lines))), ci.CANCEL_KEYS)
        for bad in (lines[:-2] + lines[-1:], lines[:1] + ['PRIVATE'] + lines[1:], lines[:1] + lines[1:2] + lines[1:]):
            with self.assertRaises(ValueError):
                ci.parse_cancel_metrics('\n'.join(bad))
        value = checks('native_real')
        value['cancel_safe_return_ms']['cancel_phase_4'] = float('nan')
        with self.assertRaises(ValueError):
            ci.validate_checks('native_real', value)

    def test_final_elf_hardening(self):
        ci.parse_elf(ELF)
        for bad in (ELF.replace('0x4000', '0x1000'), ELF.replace('RW 0x10', 'RWE 0x10'),
                    ELF.replace('R E 0x4000', 'RWE 0x4000'), ELF.replace('libm.so', 'libc++_shared.so'),
                    ELF.replace('0x004000 R 0x1', '0x003000 R 0x1'), ELF.replace('Machine: AArch64', 'Machine: x86-64'),
                    ELF.replace('GNU_RELRO', 'NO_RELRO'), ELF.replace('/system/bin/linker64', '/wrong')):
            with self.assertRaises(ValueError):
                ci.parse_elf(bad)

    def test_android_dependency_allowlist_optional_unused_libraries(self):
        for removed in ((), ('libm.so',), ('libdl.so',), ('libdl.so', 'libm.so')):
            lines = [line for line in ELF.splitlines() if not any('[' + name + ']' in line for name in removed)]
            result = ci.parse_elf('\n'.join(lines))
            self.assertTrue(ci.valid_android_dependencies(result['needed']))
            value = checks('elf')
            for target in value.values():
                target['needed'] = result['needed']
            ci.validate_checks('elf', value)
        for bad in (ELF.replace('[libc.so]', '[libm.so]'),
                    ELF + '(NEEDED) Shared library: [libc.so]\n',
                    ELF.replace('[libm.so]', '[libMNN.so]'),
                    ELF.replace('[libm.so]', '[libc++_shared.so]'),
                    ELF.replace('[libm.so]', '[unknown.so]')):
            with self.assertRaises(ci.ElfAuditError) as error:
                ci.parse_elf(bad)
            self.assertEqual(error.exception.failure_case, 'elf_dependencies')
        self.assertFalse(ci.valid_android_dependencies(['libdl.so', 'libm.so']))
        self.assertFalse(ci.valid_android_dependencies(['libc.so', 'libdl.so', 'libdl.so']))

    def test_elf_failures_have_fixed_safe_categories(self):
        cases = [(ELF.replace('Machine: AArch64', 'Machine: x86-64'), 'elf_identity'),
                 (ELF.replace('/system/bin/linker64', '/PRIVATE/linker'), 'elf_interpreter'),
                 (ELF.replace('0x4000', '0x1000'), 'elf_load_alignment'),
                 (ELF.replace('R E 0x4000', 'RWE 0x4000'), 'elf_writable_executable'),
                 (ELF.replace('GNU_RELRO', 'NO_RELRO'), 'elf_relro'),
                 (ELF.replace('RW 0x10', 'RWE 0x10'), 'elf_stack'),
                 (ELF.replace('LOAD 0x000000', 'LOAD PRIVATE'), 'elf_malformed_headers')]
        for content, expected in cases:
            with self.assertRaises(ci.ElfAuditError) as error:
                ci.parse_elf(content)
            self.assertEqual(error.exception.failure_case, expected)
            self.assertNotIn('PRIVATE', str(error.exception))
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            runner = ci.Runner(root / 'work', root / 'evidence', 'elf')
            with patch.object(runner, 'elf', side_effect=ci.ElfAuditError('elf_dependencies')), self.assertRaises(ci.ElfAuditError):
                runner.execute()
            value = ci.read(runner.evidence / 'elf.json')
            self.assertEqual(value['failure_case'], 'elf_dependencies')
            ci.sanitize_report('elf', value)

    def prepare(self, directory):
        evidence = Path(directory) / 'evidence'
        evidence.mkdir()
        for stage in ci.STAGES:
            ci.ci_verify.write(evidence / (stage + '.json'), report(stage))
        return evidence, Path(directory) / 'upload', ','.join(stage + ':success' for stage in ci.STAGES)

    def test_complete_gate(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(ci.ci_verify, 'source_identity', return_value=IDENTITY):
            evidence, destination, outcomes = self.prepare(directory)
            ci.stage_reports(evidence, destination, outcomes)
            self.assertTrue(ci.read(destination / 'ci-status.json')['all_required_steps_succeeded'])
            self.assertEqual(len(list(destination.iterdir())), len(ci.STAGES) + 1)

    def test_missing_or_failed_or_invalid_preserves_failure_evidence(self):
        for scenario in ('missing', 'failed', 'invalid', 'outcome', 'symlink'):
            with self.subTest(scenario=scenario), tempfile.TemporaryDirectory() as directory, patch.object(ci.ci_verify, 'source_identity', return_value=IDENTITY):
                evidence, destination, outcomes = self.prepare(directory)
                target = evidence / 'native_real.json'
                if scenario == 'missing':
                    target.unlink()
                elif scenario == 'symlink':
                    target.unlink()
                    target.symlink_to(evidence / 'tools.json')
                elif scenario == 'outcome':
                    outcomes = outcomes.replace('native_real:success', 'native_real:skipped')
                else:
                    value = report('native_real')
                    value.update(status='failed' if scenario == 'failed' else 'ok', failure_case='stage_validation_failed' if scenario == 'failed' else 'none', checks={'raw': 'PRIVATE'})
                    ci.ci_verify.write(target, value)
                with self.assertRaises(ValueError):
                    ci.stage_reports(evidence, destination, outcomes)
                self.assertFalse(ci.read(destination / 'ci-status.json')['all_required_steps_succeeded'])
                self.assertTrue((destination / 'tools.json').is_file())
                self.assertNotIn('PRIVATE', ''.join(p.read_text(encoding='utf-8') for p in destination.iterdir()))

    def test_dirty_source_and_duplicate_outcome_fail_closed(self):
        for scenario in ('dirty', 'duplicate'):
            with tempfile.TemporaryDirectory() as directory:
                evidence, destination, outcomes = self.prepare(directory)
                identity = dict(IDENTITY, source_clean=scenario != 'dirty')
                if scenario == 'duplicate':
                    outcomes += ',tools:success'
                with patch.object(ci.ci_verify, 'source_identity', return_value=identity), self.assertRaises(ValueError):
                    ci.stage_reports(evidence, destination, outcomes)
                self.assertFalse(ci.read(destination / 'ci-status.json')['all_required_steps_succeeded'])


class BoundedCommandTests(unittest.TestCase):
    def execute(self, source, **kwargs):
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / 'private.log'
            info, text = ci.bounded_command([sys.executable, '-c', source], log, dict(os.environ), **kwargs)
            size = log.stat().st_size
        return info, text, size

    def test_fast_overflow_even_after_successful_exit(self):
        info, text, size = self.execute("import os; os.write(1,b'x'*8192)", timeout=2, log_limit=1024)
        self.assertEqual(info['failure_case'], 'log_limit')
        self.assertTrue(info['log_limit_exceeded'])
        self.assertTrue(info['cleanup_confirmed'])
        self.assertEqual(size, 1024)
        self.assertLessEqual(len(text), ci.MAX_OUTPUT_READ)

    def test_read_cap_and_soft_overshoot(self):
        info, text, size = self.execute("import os; os.write(1,b'x'*1048576)", timeout=2, log_limit=1024)
        self.assertTrue(info['output_truncated'])
        self.assertEqual(len(text), ci.MAX_OUTPUT_READ)
        self.assertEqual(size, 1024)

    def test_timeout_is_bounded_and_reaped(self):
        start = time.monotonic()
        info, _, _ = self.execute('import time; time.sleep(10)', timeout=0.05, reap_timeout=1)
        self.assertLess(time.monotonic() - start, 2)
        self.assertEqual(info['failure_case'], 'timeout')
        self.assertTrue(info['timed_out'])
        self.assertTrue(info['cleanup_confirmed'])

    def test_kill_exit_race_still_uses_bounded_wait(self):
        process = Mock(pid=12345)
        with patch.object(ci.os, 'killpg', side_effect=ProcessLookupError), patch.object(ci, 'group_exists', return_value=False):
            self.assertTrue(ci.terminate_and_reap(process, 0.2))
        self.assertGreater(process.wait.call_args.kwargs['timeout'], 0)
        self.assertLessEqual(process.wait.call_args.kwargs['timeout'], 0.2)

    def test_unconfirmed_cleanup_returns_without_unbounded_reap(self):
        process = Mock(pid=12345)
        process.wait.side_effect = subprocess.TimeoutExpired('private', 0.1)
        with patch.object(ci.os, 'killpg'):
            self.assertFalse(ci.terminate_and_reap(process, 0.1))
        process.wait.assert_called_once()
        self.assertIn('timeout', process.wait.call_args.kwargs)

    def test_build_diagnostics_redact_paths_and_bound_size(self):
        output = '/home/runner/work/Nexa/file.cpp:12: error: unknown identifier\n' * 100
        lines = ci.safe_diagnostics(output)
        self.assertLessEqual(len(lines), 16)
        self.assertLessEqual(sum(map(len, lines)), ci.MAX_DIAGNOSTIC_BYTES)
        self.assertNotIn('/home/', ''.join(lines))
        self.assertIn('unknown identifier', lines[0])
        self.assertEqual(ci.safe_diagnostics('\n'.join(lines)), lines)

    def test_runtime_raw_output_is_never_diagnostic(self):
        with patch.object(ci, 'command_category', return_value='native_request'):
            info, _, _ = self.execute("import sys; print('Assertion PRIVATE USER TEXT'); sys.exit(1)", timeout=2)
        self.assertEqual(info['failure_case'], 'native_assertion_failed')
        self.assertEqual(info['diagnostics'], [])
        self.assertNotIn('PRIVATE', str(info))

    def test_failed_build_diagnostics_survive_sanitization(self):
        with patch.object(ci, 'command_category', return_value='compile_link'):
            info, _, _ = self.execute("import sys; print('/workspace/private/a.cpp:12: error: unknown identifier'); sys.exit(1)", timeout=2)
        info['index'] = 0
        value = report('linux_native')
        value.update(status='failed', failure_case=info['failure_case'], exit_codes=[info['exit_code']], commands=[info])
        sanitized = ci.sanitize_report('linux_native', value)
        self.assertEqual(sanitized['commands'][0]['diagnostics'], ['<path>:12: error: unknown identifier'])
        self.assertEqual(sanitized['checks'], {})

    def test_cleanup_unconfirmed_blocks_next_stage(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            runner = ci.Runner(root / 'work', root / 'evidence', 'baseline')
            (runner.work / 'cleanup-unconfirmed').write_text('blocked', encoding='utf-8')
            with patch.object(runner, 'baseline') as action, self.assertRaises(ValueError):
                runner.execute()
            action.assert_not_called()
            value = ci.read(runner.evidence / 'baseline.json')
            self.assertEqual(value['failure_case'], 'previous_cleanup_unconfirmed')
            ci.sanitize_report('baseline', value)

    def test_source_identity_error_has_fixed_label_only(self):
        with patch.object(ci, 'command_category', return_value='source_identity'):
            info, _, _ = self.execute("import sys; print('source identity mismatch: /workspace/PRIVATE/file'); sys.exit(1)", timeout=2)
        self.assertEqual(info['failure_case'], 'source_identity_mismatch')
        self.assertEqual(info['diagnostics'], [])
        self.assertNotIn('PRIVATE', str(info))

    def test_diagnostics_for_runtime_stage_rejected(self):
        value = report('native_real')
        value['status'] = 'failed'
        value['commands'][0].update(category='native_request', failure_case='native_assertion_failed', diagnostics=['error: PRIVATE'])
        with self.assertRaises(ValueError):
            ci.sanitize_report('native_real', value)

    def test_privacy_failure_emits_only_fixed_case(self):
        with patch.object(ci, 'command_category', return_value='privacy'):
            info, _, _ = self.execute("import sys; print('[{\"case\":\"corrupt_tokenizer\",\"passed\":false,\"raw\":\"PRIVATE\"}]'); sys.exit(1)", timeout=2)
        self.assertEqual(info['failure_case'], 'privacy_corrupt_tokenizer')
        self.assertNotIn('PRIVATE', str(info))


class AndroidArchiveTests(unittest.TestCase):
    def archive(self, path, machine=183):
        path.parent.mkdir(parents=True, exist_ok=True)
        member = bytearray(20)
        member[:6] = b'\x7fELF\x02\x01'
        member[18:20] = machine.to_bytes(2, 'little')
        header = ('unit.o/'.ljust(16) + '0'.ljust(12) + '0'.ljust(6) + '0'.ljust(6)
                  + '100644'.ljust(8) + str(len(member)).ljust(10) + '`\n').encode()
        path.write_bytes(b'!<arch>\n' + header + member)

    def layout(self, root):
        resource = root / 'lib/clang/21'
        resource.mkdir(parents=True)
        files = {'c++_static': root / 'sysroot/usr/lib/aarch64-linux-android/libc++_static.a',
            'c++abi': root / 'sysroot/usr/lib/aarch64-linux-android/libc++abi.a',
            'unwind': resource / 'lib/linux/aarch64/libunwind.a',
            'clang_rt_builtins': resource / 'lib/linux/libclang_rt.builtins-aarch64-android.a'}
        for path in files.values():
            self.archive(path)
        return resource, files

    def test_exact_clang_resource_paths_without_glob(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            resource, files = self.layout(root)
            archives, counts = ci.android_runtime_archives(root, str(resource) + '\n')
            self.assertEqual(archives, files)
            self.assertEqual(counts, {name: 1 for name in files})
            self.assertEqual(archives['unwind'], root / 'lib/clang/21/lib/linux/aarch64/libunwind.a')

    def test_missing_archive_reports_fixed_name(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            resource, files = self.layout(root)
            files['unwind'].unlink()
            with self.assertRaises(ci.ArchiveInputError) as error:
                ci.android_runtime_archives(root, str(resource))
            self.assertEqual(error.exception.failure_case, 'export_archive_missing')
            self.assertEqual(error.exception.name, 'unwind')
        self.assertEqual(ci.missing_archive_name('missing archive /PRIVATE/libunwind.a'), 'unwind')
        self.assertEqual(ci.missing_archive_name('missing archive /PRIVATE/libclang_rt.builtins-aarch64-android.a'), 'clang_rt_builtins')
        self.assertIsNone(ci.missing_archive_name('missing archive /PRIVATE/unreviewed.a'))

    def test_wrong_architecture_and_invalid_archive_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            resource, files = self.layout(root)
            self.archive(files['unwind'], machine=62)
            with self.assertRaises(ci.ArchiveInputError) as error:
                ci.android_runtime_archives(root, str(resource))
            self.assertEqual(error.exception.failure_case, 'export_archive_architecture')
            files['unwind'].write_bytes(b'!<arch>\n')
            with self.assertRaises(ci.ArchiveInputError) as error:
                ci.android_runtime_archives(root, str(resource))
            self.assertEqual(error.exception.failure_case, 'export_invalid_archive')

    def test_wrong_resource_or_musl_symlink_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            resource, files = self.layout(root)
            wrong = root / 'lib/clang/22'
            wrong.mkdir()
            for value in (str(wrong), '/outside/clang/21', str(resource) + '\nPRIVATE'):
                with self.assertRaises(ci.ArchiveInputError) as error:
                    ci.android_runtime_archives(root, value)
                self.assertEqual(error.exception.failure_case, 'export_resource_dir_mismatch')
            musl = root / 'lib/aarch64-unknown-linux-musl/libunwind.a'
            self.archive(musl)
            files['unwind'].unlink()
            files['unwind'].symlink_to(musl)
            with self.assertRaises(ci.ArchiveInputError) as error:
                ci.android_runtime_archives(root, str(resource))
            self.assertEqual(error.exception.failure_case, 'export_archive_path_mismatch')

    def test_runner_preserves_missing_archive_name_on_preflight_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            runner = ci.Runner(root / 'work', root / 'evidence', 'android_native')
            with patch.object(runner, 'android_native', side_effect=ci.ArchiveInputError('export_archive_missing', 'unwind')), self.assertRaises(ci.ArchiveInputError):
                runner.execute()
            value = ci.read(runner.evidence / 'android_native.json')
            self.assertEqual(value['missing_archive'], 'unwind')
            self.assertEqual(value['failure_case'], 'export_archive_missing')
            ci.sanitize_report('android_native', value)

    def test_missing_archive_report_does_not_accept_paths(self):
        value = report('android_native')
        value.update(status='failed', failure_case='export_archive_missing', missing_archive='unwind')
        self.assertEqual(ci.sanitize_report('android_native', value)['missing_archive'], 'unwind')
        value['missing_archive'] = '/PRIVATE/libunwind.a'
        with self.assertRaises(ValueError):
            ci.sanitize_report('android_native', value)


class B2GateTests(unittest.TestCase):
    def test_each_real_test_requires_one_actual_pass(self):
        for name in ci.B2_REAL_CASES:
            output = f'running 1 test\ntest {name} ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out; finished in 2.01s\n'
            ci.verify_b2_execution(output, name)
            for bad in (output.replace('1 passed', '0 passed'), output.replace('0 ignored', '1 ignored'),
                        output.replace('running 1 test', 'running 0 tests'), output.replace(name, 'other_test'),
                        output.replace('... ok', '... ignored'), output + output,
                        output + 'running 0 tests\ntest result: ok. 0 passed\n'):
                with self.assertRaises(ValueError):
                    ci.verify_b2_execution(bad, name)

    def test_exact_ignored_inventory_no_new_or_missing_tests(self):
        for package in ('mnn-model-store', 'mnn-executor'):
            names = [name for name, (owner, _) in ci.B2_REAL_CASES.items() if owner == package]
            output = '\n'.join(name + ': test' for name in names)
            ci.verify_b2_inventory(output, package)
            for bad in ('', output + '\nunreviewed::real_test: test', output + '\n' + names[0] + ': test'):
                with self.assertRaises(ValueError):
                    ci.verify_b2_inventory(bad, package)

    def test_complete_b2_report_no_production_claim(self):
        value = checks('rust_linux')
        ci.validate_checks('rust_linux', value)
        for mutate in (
            lambda v: v['b2_real_matrix']['cases'].pop(next(iter(ci.B2_REAL_CASES))),
            lambda v: v['b2_real_matrix'].update(production_admitted=True),
            lambda v: v['b2_real_matrix'].update(production_admitted=0),
            lambda v: v['b2_real_matrix'].update(research_only=False),
            lambda v: v['b2_real_matrix'].update(model_lock_sha256='0' * 64),
        ):
            bad = copy.deepcopy(value)
            mutate(bad)
            with self.assertRaises(ValueError):
                ci.validate_checks('rust_linux', bad)

    def test_notice_inventory_does_not_claim_apk_verification(self):
        value = checks('tools')
        self.assertFalse(value['notice_inventory']['final_apk_verified'])
        value['notice_inventory']['final_apk_verified'] = True
        with self.assertRaises(ValueError):
            ci.validate_checks('tools', value)

    def events(self, directory):
        result = []
        for name, (package, kind, crate_type, is_test, source) in ci.ANDROID_TARGETS.items():
            root = ci.ROOT / 'mobile/runtime/crates' / package
            executable = str(directory / ci.TARGET / 'debug' / name)
            result.append({'reason': 'compiler-artifact', 'package_id': 'path+' + root.as_uri() + '#0.1.0',
                'target': {'name': name, 'kind': [kind], 'crate_types': [crate_type], 'src_path': str(root / source), 'test': is_test},
                'profile': {'test': is_test}, 'executable': executable, 'filenames': [executable]})
        return result

    def test_android_exact_five_package_target_kinds(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            events = self.events(root)
            result = ci.parse_android_artifacts('\n'.join(map(json.dumps, events)), root)
            self.assertEqual({v['name'] for v in result}, set(ci.ANDROID_TARGETS))
            for bad in (events[:-1], events + events[:1], events[:3]):
                with self.assertRaises(ValueError):
                    ci.parse_android_artifacts('\n'.join(map(json.dumps, bad)), root)
            for mutate in (
                lambda v: v.update(package_id='path+file:///wrong#0.1.0'),
                lambda v: v['target'].update(name='unreviewed_target'),
                lambda v: v['target'].update(kind=['example']),
                lambda v: v['target'].update(crate_types=['bin']),
                lambda v: v['target'].update(src_path='/wrong/lib.rs'),
                lambda v: v['profile'].update(test=False),
                lambda v: v.update(executable=str(root / 'linux/debug/host_binary')),
                lambda v: v.update(filenames=[]),
            ):
                bad = copy.deepcopy(events)
                mutate(bad[0])
                with self.assertRaises(ValueError):
                    ci.parse_android_artifacts('\n'.join(map(json.dumps, bad)), root)

    def test_b2_command_failure_is_named_without_raw_test_logs(self):
        name, (_, expected_id) = next(iter(ci.B2_REAL_CASES.items()))
        command = ['cargo', 'test', '--lib', name, '--', '--ignored', '--exact']
        self.assertEqual(ci.command_category(command), 'b2_real')
        self.assertIn(expected_id, ci.FAILURE_CASES)


class WorkflowFilterTests(unittest.TestCase):
    def test_windows_ignore_only_isolated_known_paths(self):
        text = (ci.ROOT / '.github/workflows/native-windows.yml').read_text(encoding='utf-8')
        block = text.split('    paths-ignore:\n', 1)[1].split('  workflow_dispatch:', 1)[0]
        patterns = [line.strip()[3:-1] for line in block.splitlines() if line.strip().startswith("- '")]
        expected = ['native/mnn-probe/**', 'native/mnn-shim/**', 'native/mnn-patches/**', 'scripts/android_mnn/**', 'mobile/**',
                    '.github/workflows/android-mnn-probe.yml', '.github/workflows/android-mnn-native.yml', 'docs/**',
                    'AGENTS.md', 'PROJECT_INDEX.md', 'PROJECT_STATE.md', 'README.md', 'ai-runtime-v0.1-execution-spec.md']
        self.assertEqual(patterns, expected)
        ignored = lambda path: any(fnmatch.fnmatchcase(path, pattern) for pattern in patterns)
        for path in ('packaging/windows/manifest.json', 'crates/core/src/lib.rs', 'native/llama-shim/src/shim.cpp',
                     'Cargo.toml', 'Cargo.lock', 'CMakeLists.txt', '.github/workflows/native-windows.yml', 'new-path/file'):
            self.assertFalse(ignored(path), path)
        self.assertTrue(ignored('docs/new/file.md'))
        self.assertTrue(ignored('mobile/runtime/Cargo.lock'))
        # GitHub paths-ignore skips only when every changed path is ignored.
        self.assertFalse(all(ignored(p) for p in ['docs/file.md', 'native/llama-shim/src/stream_buffer.h']))


if __name__ == '__main__':
    unittest.main()
