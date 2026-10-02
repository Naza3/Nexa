#!/usr/bin/env python3
"""T07-B1 CI orchestration. Raw output stays private; uploads are reconstructed JSON.

No implicit SDK/model acquisition: only the inputs stage calls T07-A explicit setup.
No Android execution. All commands have bounded timeouts and two-job build limits.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import time

import ci_verify
import run_probe
import research_receipt as receipt

ROOT = Path(__file__).resolve().parents[2]
STAGES = ('tools', 'inputs', 'patch', 'baseline', 'linux_native', 'native_real',
          'rust_linux', 'research_receipt', 'b2_linux', 'android_native', 'rust_android', 'elf')
TARGET = 'aarch64-linux-android'
SHA = re.compile(r'[0-9a-f]{64}')

# Fixed B2 public unit-test targets. These are cfg(test) research checks, not a
# production admission switch. Every ignored test must actually execute once.
B2_REAL_CASES = {
    'negative_tests::real_store_lease_reopen_tamper_and_interruption': ('mnn-model-store', 'b2_store_safe_copy'),
    'tests::real_store_core_lifecycle': ('mnn-executor', 'b2_core_lifecycle'),
    'tests::real_owner_backpressure_cancel_disconnect_shutdown': ('mnn-executor', 'b2_backpressure_cleanup'),
    'tests::real_owner_fault_reload_load_timeout_and_idle': ('mnn-executor', 'b2_fault_reload_deadlines'),
}
ANDROID_TARGETS = {
    'mnn_adapter': ('mnn-adapter', 'lib', 'lib', True, 'src/lib.rs'),
    'real_model': ('mnn-adapter', 'test', 'bin', True, 'tests/real_model.rs'),
    'build_identity': ('mnn-adapter', 'example', 'bin', False, 'examples/build_identity.rs'),
    'mnn_model_store': ('mnn-model-store', 'lib', 'lib', True, 'src/lib.rs'),
    'mnn_executor': ('mnn-executor', 'lib', 'lib', True, 'src/lib.rs'),
}

ARCHIVE_NAMES = ('nexa-mnn-shim', 'MNN', 'c++_static', 'c++abi', 'unwind', 'clang_rt_builtins')


class ArchiveInputError(ValueError):
    def __init__(self, failure_case, name=None):
        self.failure_case = failure_case
        self.name = name if name in ARCHIVE_NAMES else None
        super().__init__(failure_case)


def archive_objects(path, name, machine, boundary):
    """Stream fixed-size ar/ELF headers; never read an archive body into memory."""
    if not path.is_file():
        raise ArchiveInputError('export_archive_missing', name)
    if not path.resolve().is_relative_to(boundary.resolve()):
        raise ArchiveInputError('export_archive_path_mismatch', name)
    count = 0
    try:
        with path.open('rb') as archive:
            length = os.fstat(archive.fileno()).st_size
            if archive.read(8) != b'!<arch>\n':
                raise ValueError('archive')
            position = 8
            while position < length:
                header = archive.read(60)
                if len(header) != 60 or header[58:] != b'`\n':
                    raise ValueError('header')
                size = int(header[48:58].decode('ascii').strip())
                if size < 0 or position + 60 + size > length:
                    raise ValueError('size')
                prefix = archive.read(min(size, 20))
                if prefix.startswith(b'\x7fELF'):
                    if len(prefix) < 20 or prefix[4:6] != bytes([2, 1]) or int.from_bytes(prefix[18:20], 'little') != machine:
                        raise ArchiveInputError('export_archive_architecture', name)
                    count += 1
                elif header[:16].decode('ascii').strip() not in ('/', '//', '/SYM64/'):
                    raise ValueError('non-ELF member')
                position += 60 + size + size % 2
                if position > length:
                    raise ValueError('padding')
                archive.seek(position)
            if count == 0:
                raise ValueError('no objects')
    except ArchiveInputError:
        raise
    except (ValueError, OSError):
        raise ArchiveInputError('export_invalid_archive', name) from None
    return count


def android_runtime_archives(toolchain, resource_output):
    """NDK r30 Clang21 resource tree only; never glob host/musl alternatives."""
    resource = Path(resource_output.strip())
    if (len(resource_output.splitlines()) != 1 or not resource.is_absolute()
            or not resource.is_dir() or resource.resolve() != (toolchain / 'lib/clang/21').resolve()
            or not resource.resolve().is_relative_to(toolchain.resolve())):
        raise ArchiveInputError('export_resource_dir_mismatch')
    lib = toolchain / 'sysroot/usr/lib/aarch64-linux-android'
    archives = {'c++_static': lib / 'libc++_static.a', 'c++abi': lib / 'libc++abi.a',
        'unwind': resource / 'lib/linux/aarch64/libunwind.a',
        'clang_rt_builtins': resource / 'lib/linux/libclang_rt.builtins-aarch64-android.a'}
    for name, path in archives.items():
        if path.resolve() != toolchain.resolve() / path.relative_to(toolchain):
            raise ArchiveInputError('export_archive_path_mismatch', name)
    counts = {name: archive_objects(path, name, 183, toolchain) for name, path in archives.items()}
    return archives, counts


def missing_archive_name(output):
    names = {'lib' + name + '.a': name for name in ARCHIVE_NAMES}
    names['libclang_rt.builtins-aarch64-android.a'] = 'clang_rt_builtins'
    for line in output.splitlines():
        if line.startswith('missing archive '):
            basename = Path(line.removeprefix('missing archive ')).name
            if basename in names:
                return names[basename]
    return None


def require(value):
    if not value:
        raise ValueError('required_evidence_failed')


def read(path):
    require(path.is_file() and not path.is_symlink())
    return json.loads(path.read_text(encoding='utf-8'))


def digest(path):
    with path.open('rb') as file:
        return hashlib.file_digest(file, 'sha256').hexdigest()


ELF_FAILURE_CASES = {'elf_identity', 'elf_interpreter', 'elf_dependencies', 'elf_load_alignment',
                     'elf_writable_executable', 'elf_relro', 'elf_stack', 'elf_malformed_headers'}


class ElfAuditError(ValueError):
    def __init__(self, failure_case):
        self.failure_case = failure_case
        super().__init__(failure_case)


def elf_require(condition, failure_case):
    if not condition:
        raise ElfAuditError(failure_case)


def valid_android_dependencies(needed):
    # Unused optional libraries may be removed by the real linker's --as-needed.
    return (isinstance(needed, list) and all(isinstance(name, str) for name in needed)
            and 'libc.so' in needed and needed == sorted(set(needed))
            and set(needed) <= {'libc.so', 'libdl.so', 'libm.so'})


def parse_elf(text):
    try:
        return parse_elf_headers(text)
    except ElfAuditError:
        raise
    except (ValueError, IndexError):
        raise ElfAuditError('elf_malformed_headers') from None


def parse_elf_headers(text):
    elf_require(re.search(r'Class:\s+ELF64', text) and re.search(r'Machine:\s+AArch64', text)
                and re.search(r'Type:\s+DYN', text), 'elf_identity')
    elf_require('[Requesting program interpreter: /system/bin/linker64]' in text, 'elf_interpreter')
    needed = sorted(re.findall(r'\(NEEDED\).*?\[([^]]+)\]', text))
    elf_require(valid_android_dependencies(needed), 'elf_dependencies')
    loads = []
    for line in text.splitlines():
        fields = line.split()
        if fields and fields[0] == 'LOAD':
            offset, address, alignment = (int(fields[i], 16) for i in (1, 2, -1))
            elf_require(alignment >= 16384 and not alignment & (alignment - 1)
                        and offset % 16384 == address % 16384, 'elf_load_alignment')
            flags = ''.join(fields[6:-1])
            elf_require(not ('W' in flags and 'E' in flags), 'elf_writable_executable')
            loads.append({'offset': offset, 'virtual_address': address, 'alignment': alignment})
    relro = [line.split() for line in text.splitlines() if line.lstrip().startswith('GNU_RELRO ')]
    elf_require(loads and len(relro) == 1, 'elf_relro')
    start, size = int(relro[0][2], 16), int(relro[0][5], 16)
    elf_require(size > 0 and (start + size) % 16384 == 0, 'elf_relro')
    stack = [line.split() for line in text.splitlines() if line.lstrip().startswith('GNU_STACK ')]
    elf_require(len(stack) == 1 and 'E' not in ''.join(stack[0][6:-1]), 'elf_stack')
    return {'needed': needed, 'load_segments': loads, 'relro_start': start,
            'relro_size': size, 'relro_end': start + size, 'android_run': False}


MAX_PRIVATE_LOG = 16 * 1024 * 1024
MAX_OUTPUT_READ = 256 * 1024
MAX_DIAGNOSTIC_BYTES = 4096
BUILD_CATEGORIES = {'configure', 'compile_link'}
CATEGORIES = BUILD_CATEGORIES | {'tool', 'native_request', 'privacy', 'comparison', 'verification', 'rust_runtime', 'source_identity', 'candidate_validation', 'artifact_export', 'b2_real'}
PRIVACY_CASES = {'corrupt_config', 'corrupt_tokenizer', 'corrupt_graph', 'bad_template', 'missing_tokenizer',
                 'missing_graph', 'unsafe_backend', 'unsafe_option', 'success_en_zh_multi_phase_cancel_callback_recovery'}
FAILURE_CASES = {'none', 'timeout', 'log_limit', 'cleanup_unconfirmed', 'nonzero_exit', 'spawn_failed',
                 'native_assertion_failed', 'native_request_failed', 'privacy_failed', 'comparison_failed'} | {
                     'privacy_' + case for case in PRIVACY_CASES} | {
                     'comparison_' + key for key in ('rendered_prompt', 'prompt_tokens', 'output_tokens')}

FIXED_ERRORS = {
    'source_identity': {'patch target must be a regular private file': 'source_nonregular_target',
        'source identity mismatch:': 'source_identity_mismatch', 'patch identity mismatch': 'patch_identity_mismatch',
        'upstream mismatch': 'upstream_commit_mismatch', 'unexpected private source modifications': 'source_unexpected_modifications',
        'unexpected private source files': 'source_unexpected_files', 'upstream must be clean': 'source_dirty',
        'destination must be new private directory': 'source_destination_exists'},
    'candidate_validation': {'candidate size/type mismatch:': 'candidate_size_type_mismatch',
        'candidate hash mismatch:': 'candidate_hash_mismatch', 'candidate template mismatch': 'candidate_template_mismatch'},
    'artifact_export': {'Release build required': 'export_not_release', 'compiler identity mismatch': 'export_compiler_mismatch',
        'missing archive ': 'export_archive_missing', 'expected actual archive:': 'export_invalid_archive',
        'Android toolchain mismatch': 'export_android_toolchain_mismatch'},
}
FAILURE_CASES |= {'export_archive_architecture', 'export_archive_path_mismatch', 'export_resource_dir_mismatch'}
FAILURE_CASES |= {value for mapping in FIXED_ERRORS.values() for value in mapping.values()}
FAILURE_CASES |= {case_id for _, case_id in B2_REAL_CASES.values()}
STAGE_FAILURE_CASES = FAILURE_CASES | ELF_FAILURE_CASES | {'stage_validation_failed', 'previous_cleanup_unconfirmed', 'b2_test_inventory_mismatch', 'research_receipt_rejected', 'source_context_mismatch'}


def command_category(command):
    args = [str(x) for x in command]
    exe = Path(args[0]).name
    if exe == 'cmake':
        return 'compile_link' if '--build' in args else 'configure'
    if exe in ('gcc-13', 'g++-13', 'aarch64-linux-android28-clang') and '-o' in args:
        return 'compile_link'
    if exe == 'cargo':
        if '--ignored' in args and '--exact' in args and any(name in args for name in B2_REAL_CASES):
            return 'b2_real'
        return 'compile_link' if args[1] == 'clippy' or '--no-run' in args else 'rust_runtime'
    for helper, category in (('mnn-patches/identity.py', 'source_identity'), ('prepare_research_config.py', 'candidate_validation'), ('export_artifact.py', 'artifact_export')):
        if any(arg.endswith(helper) for arg in args):
            return category
    if exe == 'mnn-request-test':
        return 'native_request'
    if any(x.endswith('/privacy_canaries.py') for x in args):
        return 'privacy'
    if any(x.endswith('/compare_upstream.py') for x in args):
        return 'comparison'
    return 'verification'


def safe_diagnostics(output):
    result, total = [], 0
    for line in output.splitlines():
        if not re.search(r'(?i)(fatal error:|error(?:\[[A-Z][0-9]+\])?:|undefined (?:reference|symbol)|CMake Error|ninja: build stopped)', line):
            continue
        line = re.sub(r'\x1b\[[0-9;]*[A-Za-z]', '', line)
        # No absolute workspace, home, SDK or source paths survive.
        line = re.sub(r'(?<![A-Za-z0-9])(?:[A-Za-z]:[\\/]|/)[^\s\"\'<>(),;:]*', '<path>', line)
        line = ''.join(c if ' ' <= c <= '~' else '?' for c in line).strip()[:400]
        if line and total + len(line) <= MAX_DIAGNOSTIC_BYTES and len(result) < 16:
            result.append(line)
            total += len(line)
    return result


def group_exists(pid):
    try:
        os.killpg(pid, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True


def terminate_and_reap(process, timeout):
    """Bounded owned-group cleanup. ESRCH is a race, not proof of reaping."""
    deadline = time.monotonic() + timeout
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    except PermissionError:
        return False
    try:
        process.wait(timeout=max(0.001, deadline - time.monotonic()))
    except subprocess.TimeoutExpired:
        return False
    while group_exists(process.pid):
        if time.monotonic() >= deadline:
            return False
        time.sleep(min(0.02, max(0, deadline - time.monotonic())))
    return True


def bounded_command(command, log, env, timeout, log_limit=MAX_PRIVATE_LOG, reap_timeout=5):
    """Disk limit is soft (50ms polling); final fast-exit check is mandatory."""
    info = {'category': command_category(command), 'timeout_seconds': timeout, 'timed_out': False,
            'log_limit_exceeded': False, 'cleanup_confirmed': True, 'failure_case': 'none',
            'exit_code': 127, 'log_bytes': 0, 'output_truncated': False, 'diagnostics': []}
    process = None
    with log.open('wb') as output:
        try:
            process = subprocess.Popen([str(x) for x in command], cwd=ROOT, env=env,
                stdout=output, stderr=subprocess.STDOUT, start_new_session=True)
        except OSError:
            info['failure_case'] = 'spawn_failed'
        if process is not None:
            deadline = time.monotonic() + timeout
            while process.poll() is None:
                if log.stat().st_size > log_limit:
                    info['log_limit_exceeded'] = True
                    break
                if time.monotonic() >= deadline:
                    info['timed_out'] = True
                    break
                time.sleep(min(0.05, max(0, deadline - time.monotonic())))
            # Includes processes that overflow and exit before the first poll.
            info['log_limit_exceeded'] |= log.stat().st_size > log_limit
            if process.poll() is None or group_exists(process.pid):
                info['cleanup_confirmed'] = terminate_and_reap(process, reap_timeout)
            info['exit_code'] = process.returncode if process.returncode is not None else 125
            info['failure_case'] = ('cleanup_unconfirmed' if not info['cleanup_confirmed'] else
                'timeout' if info['timed_out'] else 'log_limit' if info['log_limit_exceeded'] else
                'nonzero_exit' if info['exit_code'] != 0 else 'none')
    info['log_bytes'] = log.stat().st_size
    info['log_limit_exceeded'] |= info['log_bytes'] > log_limit
    if info['failure_case'] == 'none' and info['log_limit_exceeded']:
        info['failure_case'] = 'log_limit'
    info['output_truncated'] = info['log_bytes'] > MAX_OUTPUT_READ
    with log.open('rb') as source:
        source.seek(max(0, info['log_bytes'] - MAX_OUTPUT_READ))
        text = source.read(MAX_OUTPUT_READ).decode('utf-8', errors='replace')
    # Reclaim soft overshoot once the writer is known stopped. Unconfirmed cleanup
    # remains explicit and blocks later stages; do not pretend that writer stopped.
    if info['cleanup_confirmed'] and info['log_bytes'] > log_limit:
        with log.open('r+b') as source:
            source.truncate(log_limit)
    if info['failure_case'] == 'nonzero_exit':
        category = info['category']
        if category in BUILD_CATEGORIES:
            info['diagnostics'] = safe_diagnostics(text)
        elif category == 'native_request':
            info['failure_case'] = 'native_assertion_failed' if 'Assertion' in text else 'native_request_failed'
        elif category == 'comparison':
            match = re.search(r'AssertionError: (rendered_prompt|prompt_tokens|output_tokens)\s*$', text)
            info['failure_case'] = 'comparison_' + match[1] if match else 'comparison_failed'
        elif category == 'privacy':
            info['failure_case'] = 'privacy_failed'
            for line in text.splitlines():
                try:
                    data, _ = json.JSONDecoder().raw_decode(line)
                except ValueError:
                    continue
                if isinstance(data, list):
                    for case in data:
                        if isinstance(case, dict) and case.get('case') in PRIVACY_CASES and case.get('passed') is False:
                            info['failure_case'] = 'privacy_' + case['case']
                            break
        elif category == 'b2_real':
            for name, (_, case_id) in B2_REAL_CASES.items():
                if name in command:
                    info['failure_case'] = case_id
                    break
        elif category in FIXED_ERRORS:
            for line in text.splitlines():
                for prefix, label in FIXED_ERRORS[category].items():
                    if line.startswith(prefix):
                        info['failure_case'] = label
    return info, text


class Runner:
    def __init__(self, work, evidence, stage):
        self.work, self.evidence, self.stage = work.resolve(), evidence.resolve(), stage
        self.work.mkdir(parents=True, exist_ok=True)
        self.evidence.mkdir(parents=True, exist_ok=True)
        self.context_mode = 'github-ci'
        self.prerequisite_outcomes = None
        self.context = None
        self.result = {'schema': 2, 'context': None, 'stage': stage, 'status': 'failed', 'android_run': False,
                       'exit_codes': [], 'commands': [], 'failure_case': 'stage_validation_failed', 'missing_archive': None, 'checks': {}}
        self.env = dict(os.environ, RUSTUP_TOOLCHAIN='1.98.1', CARGO_BUILD_JOBS='2',
                        CARGO_INCREMENTAL='0', CMAKE_BUILD_PARALLEL_LEVEL='2')
        self.source = self.work / 'mnn'
        self.private = self.work / 'private-mnn'
        self.inputs = self.work / 'inputs'
        self.model = self.inputs / 'model'
        self.ndk = self.inputs / 'android-ndk-r30'
        self.toolchain = self.ndk / 'toolchains/llvm/prebuilt/linux-x86_64'
        self.config = self.work / 'runtime.json'

    def bind_context(self):
        if self.stage == 'tools':
            self.context = receipt.initialize(self.evidence, self.context_mode)
        else:
            self.context = receipt.load_context(self.evidence)
        self.result['context'] = self.context

    def linux_compilers(self):
        return ('cc', 'c++') if self.context and self.context['mode'] == 'local-verification' else ('/usr/bin/gcc-13', '/usr/bin/g++-13')

    def run(self, command, timeout=600, env=None):
        index = len(self.result['exit_codes'])
        # Model/test output is private; only bounded build diagnostics are eligible.
        log = self.work / f'{self.stage}-{index}.private.log'
        info, output = bounded_command(command, log, env or self.env, timeout)
        info['index'] = index
        self.result['commands'].append(info)
        self.result['exit_codes'].append(info['exit_code'])
        if not info['cleanup_confirmed']:
            (self.work / 'cleanup-unconfirmed').write_text('blocked\n', encoding='utf-8')
        ci_verify.write(self.evidence / f'{self.stage}.json', self.result)
        if info['failure_case'] != 'none':
            self.result['failure_case'] = info['failure_case']
            if info['failure_case'] == 'export_archive_missing':
                self.result['missing_archive'] = missing_archive_name(output)
        require(info['failure_case'] == 'none')
        return output

    def python(self, script, *args, **kw):
        return self.run([sys.executable, script, *args], **kw)

    def build(self, directory, source, patched=True, android=False):
        args = ['cmake', '-G', 'Ninja', '-S', source, '-B', directory,
                '-DCMAKE_BUILD_TYPE=Release', f'-DNEXA_MNN_SOURCE={self.private if patched else self.source}']
        if android:
            args += [f'-DCMAKE_TOOLCHAIN_FILE={self.ndk}/build/cmake/android.toolchain.cmake',
                     '-DANDROID_ABI=arm64-v8a', '-DANDROID_PLATFORM=android-28']
        else:
            cc, cxx = self.linux_compilers()
            args += ['-DCMAKE_C_COMPILER=' + cc, '-DCMAKE_CXX_COMPILER=' + cxx]
        self.run(args)
        self.run(['cmake', '--build', directory, '-j2'], timeout=900)

    def export(self, directory, android=False):
        compiler = self.toolchain / 'bin/clang++' if android else self.linux_compilers()[1]
        identity = self.run([compiler, '--version']).splitlines()[0]
        args = [directory, '--target', TARGET if android else 'x86_64-unknown-linux-gnu', '--compiler', identity]
        native_archives = {'nexa-mnn-shim': directory / 'libnexa-mnn-shim.a', 'MNN': directory / 'mnn/libMNN.a'}
        counts = {name: archive_objects(path, name, 183 if android else 62, directory)
                  for name, path in native_archives.items()}
        if android:
            resource = self.run([compiler, '--print-resource-dir'])
            runtimes, runtime_counts = android_runtime_archives(self.toolchain, resource)
            counts.update(runtime_counts)
            args += ['--cxx-static', runtimes['c++_static'], '--cxxabi-static', runtimes['c++abi'],
                     '--unwind-static', runtimes['unwind'], '--builtins-static', runtimes['clang_rt_builtins']]
        self.python('native/mnn-shim/export_artifact.py', *args)
        # Full manifest remains local for the Rust gate. Upload only verified identity fields.
        manifest = read(directory / 'artifact/artifact.json')
        libraries = manifest['static_libraries']
        expected = ['nexa-mnn-shim', 'MNN'] + (['c++_static', 'c++abi', 'unwind', 'clang_rt_builtins'] if android else [])
        require([item['name'] for item in libraries] == expected)
        for item in libraries:
            require(item['path'] == 'lib/lib' + item['name'] + '.a')
            require(digest(directory / 'artifact' / item['path']) == item['sha256'])
        lock = read(ROOT / 'native/mnn-patches/lock.json')
        for key in ('upstream_commit', 'patch_set_sha256', 'policy_sha256'):
            require(manifest[key] == lock[key])
        require(manifest['header_sha256'] == digest(ROOT / 'native/mnn-shim/include/nexa_mnn.h'))
        require(manifest['silent_logs'] is True and manifest['build_type'] == 'Release' and manifest['abi_version'] == 1)
        self.result['checks']['artifact'] = {key: manifest[key] for key in
            ('upstream_commit', 'patch_set_sha256', 'policy_sha256', 'header_sha256', 'target')}
        self.result['checks']['artifact']['manifest_sha256'] = digest(directory / 'artifact/artifact.json')
        self.result['checks']['artifact']['compiler_sha256'] = hashlib.sha256(identity.encode()).hexdigest()
        self.result['checks']['artifact']['archives'] = {item['name']: item['sha256'] for item in libraries}
        self.result['checks']['artifact']['archive_object_counts'] = counts
        audit = read(directory / 'logging-audit.json')
        require(audit['failures'] == [] and audit['compiled_source_count'] > 0 and audit['header_count'] > 0)
        self.result['checks']['logging_audit'] = {key: audit[key] for key in ('compiled_source_count', 'header_count')}

    def cargo(self, action, *args, android=False):
        env = dict(self.env, NEXA_MNN_ARTIFACT_DIR=str(self.work / ('android' if android else 'linux') / 'artifact'),
                   CARGO_TARGET_DIR=str(self.work / ('rust-android' if android else 'rust-linux')))
        if android:
            env['CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER'] = str(self.toolchain / 'bin/aarch64-linux-android28-clang')
            # Both page settings are needed: max alone leaves RELRO rounded to 4KiB.
            env['CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS'] = (
                '-C link-arg=-Wl,-z,max-page-size=16384 '
                '-C link-arg=-Wl,-z,common-page-size=16384')
        return env, ['cargo', action, '--manifest-path', 'mobile/runtime/Cargo.toml', '--locked', '--offline',
                     *(['--target', TARGET] if android else []), *args]

    def tools(self):
        self.python('scripts/android_mnn/ci_verify.py', 'tools', '--report', self.work / 'tools-input.json')
        cc, cxx = self.linux_compilers()
        gcc = '14.2.0' if self.context['mode'] == 'local-verification' else '13.3.0'
        require(self.run([cxx, '-dumpfullversion']).strip() == gcc)
        require(self.run([cc, '-dumpfullversion']).strip() == gcc)
        require(self.run(['rustc', '--version']).startswith('rustc 1.98.1 '))
        require(self.run(['cargo', '--version']).startswith('cargo 1.98.1 '))
        require(TARGET in self.run(['rustup', 'target', 'list', '--installed']).splitlines())
        identity = ci_verify.source_identity()
        require(identity['source_clean'] or self.context['mode'] == 'local-verification')
        require(self.run(['git', '-C', self.source, 'rev-parse', 'HEAD']).strip() == run_probe.MNN_COMMIT)
        require(not self.run(['git', '-C', self.source, 'status', '--porcelain', '--untracked-files=all']).strip())
        self.python('-m', 'unittest', 'discover', '-s', 'scripts/android_mnn', '-p', 'test_*.py')
        self.python('native/mnn-shim/notices/verify.py')
        self.result['checks'] = {'source': identity, 'gcc': gcc, 'rust': '1.98.1', 'cmake': '4.4.3', 'ninja': '1.13.2',
                                 'notice_inventory': notice_identity()}

    def inputs_stage(self):
        self.python('scripts/android_mnn/ci_verify.py', 'setup', '--destination', self.inputs,
                    '--report', self.work / 'setup-input.json', timeout=900)
        # Cargo registry network access is explicit and separate from offline builds/tests.
        self.run(['cargo', 'fetch', '--manifest-path', 'mobile/runtime/Cargo.toml', '--locked'], timeout=300)
        setup = read(self.work / 'setup-input.json')
        lock = read(run_probe.LOCK)
        require(setup['status'] == 'ok' and setup['ndk'] == ci_verify.NDK and setup['model_files'] == lock['files'])
        self.result['checks'] = {'ndk_sha256': ci_verify.NDK['sha256'], 'ndk_revision': ci_verify.NDK['revision'],
                                 'model_lock_sha256': digest(run_probe.LOCK)}

    def patch(self):
        self.python('native/mnn-patches/identity.py', '--source', self.source, '--destination', self.private)
        self.python('native/mnn-patches/identity.py', '--source', self.private, '--verify-post')
        require(not self.run(['git', '-C', self.source, 'status', '--porcelain', '--untracked-files=all']).strip())
        self.result['checks']['pristine_source_unchanged'] = True

    def baseline(self):
        directory = self.work / 'baseline'
        self.build(directory, 'native/mnn-probe', patched=False)
        self.run(['ctest', '--test-dir', directory, '--output-on-failure'])
        require(not self.run(['git', '-C', self.source, 'status', '--porcelain', '--untracked-files=all']).strip())
        self.result['checks']['unpatched_probe_ctest'] = True

    def linux_native(self):
        directory = self.work / 'linux'
        self.build(directory, 'native/mnn-shim')
        self.run(['ctest', '--test-dir', directory, '--output-on-failure'])
        self.export(directory)
        sanitizer = self.work / 'stream-asan'
        self.run([self.linux_compilers()[1], '-std=c++17', '-UNDEBUG', '-fsanitize=address,undefined', '-fno-omit-frame-pointer',
                  '-I', 'native/mnn-shim/src', 'native/mnn-shim/tests/stream_buffer_test.cpp', '-o', sanitizer])
        self.run([sanitizer])
        self.result['checks']['ctest_and_stream_sanitizers'] = True

    def native_real(self):
        self.python('native/mnn-shim/prepare_research_config.py', '--model-root', self.model, '--output', self.config)
        native_report, upstream_report = self.work / 'native.private.json', self.work / 'upstream.private.json'
        output = self.run([self.work / 'linux/mnn-request-test', self.config, native_report], timeout=900)
        metrics = parse_cancel_metrics(output)
        request = self.work / 'request.private.json'
        ci_verify.write(request, {'max_new_tokens': 12, 'logical_context': 2048,
                                 'messages': [{'role': 'user', 'content': 'Reply with one short sentence about the sky.'}]})
        self.run([self.work / 'baseline/nexa-mnn-probe', self.config, request, upstream_report], timeout=180)
        comparison = json.loads(self.python('native/mnn-shim/tests/compare_upstream.py', native_report, upstream_report))
        require(comparison['exact_unpatched_upstream_comparison'] == 'passed')
        canaries = json.loads(self.python('native/mnn-shim/tests/privacy_canaries.py',
                                        self.work / 'linux/mnn-request-test', self.config, timeout=900))
        expected = {'corrupt_config', 'corrupt_tokenizer', 'corrupt_graph', 'bad_template', 'missing_tokenizer',
                    'missing_graph', 'unsafe_backend', 'unsafe_option', 'success_en_zh_multi_phase_cancel_callback_recovery'}
        require({entry['case'] for entry in canaries['privacy_canaries']} == expected)
        require(len(canaries['privacy_canaries']) == len(expected))
        require(all(entry['passed'] is True and entry['exit_code'] == 0 for entry in canaries['privacy_canaries']))
        self.result['checks'] = {'upstream_exact': True, 'privacy_canaries': len(expected),
            'prompt_tokens': comparison['prompt_tokens'], 'completion_tokens': comparison['completion_tokens'],
            'cancel_safe_return_ms': metrics}

    def rust_stage(self, android=False):
        env, command = self.cargo('clippy', '--all-targets', '--', '-D', 'warnings', android=android)
        self.run(command, env=env)
        if not android:
            compile_env, compile_command = self.cargo('test', '--no-run')
            self.run(compile_command, env=compile_env)
            self.run(['cargo', 'fmt', '--manifest-path', 'mobile/runtime/Cargo.toml', '--all', '--', '--check'])
        env, command = self.cargo('test', *(['--no-run', '--message-format=json'] if android else []), android=android)
        output = self.run(command, env=env)
        if android:
            binaries = parse_android_artifacts(output, self.work / 'rust-android')
            ci_verify.write(self.work / 'android-binaries.private.json', binaries)
        self.python('mobile/runtime/scripts/test_artifact_gate.py', env=env)
        compiler = self.toolchain / 'bin/aarch64-linux-android28-clang' if android else self.linux_compilers()[0]
        abi = self.work / ('abi-android.o' if android else 'abi-linux')
        self.run([compiler, '-std=c11', '-Wall', '-Wextra', '-Werror', '-I', 'native/mnn-shim/include',
                  *(['-c'] if android else []), 'mobile/runtime/crates/mnn-adapter/tests/abi_layout.c', '-o', abi])
        if not android:
            self.run([abi])
            self.python('mobile/runtime/scripts/test_real_model.py', '--model-dir', self.model, '--config', self.config,
                        env=env, timeout=900)
        self.result['checks'] = {'clippy': True, 'abi_layout': True, 'artifact_negative': True,
            'final_elf_link' if android else 'unit_compile_fail_and_real_model': True}
        if not android:
            env, command = self.cargo('run', '--quiet', '--example', 'build_identity')
            output = self.run(command, env=env)
            manifest_path = self.work / 'linux/artifact/artifact.json'
            verify_rust_identity(output, read(manifest_path), digest(manifest_path))
            self.result['checks']['linked_manifest_sha256'] = digest(manifest_path)

    def research_receipt(self):
        # A real linked identity check also gives this pure validation stage a
        # bounded command outcome; no synthetic success command is invented.
        env, command = self.cargo('run', '--quiet', '--example', 'build_identity')
        output = self.run(command, env=env)
        artifact = self.work / 'linux/artifact'
        verify_rust_identity(output, read(artifact / 'artifact.json'), digest(artifact / 'artifact.json'))
        self.python('native/mnn-shim/prepare_research_config.py', '--model-root', self.model,
                    '--output', self.work / 'receipt-verified-runtime.json')
        result = receipt.mint(self.evidence, self.work / 'research-bundle', artifact,
                              self.prerequisite_outcomes, sanitize_report, work=self.work)
        self.result['checks'] = result

    def b2_linux(self):
        result = receipt.verify_bundle(self.work / 'research-bundle/receipt.json', self.context, self.evidence, sanitize_report, artifact=self.work / 'linux/artifact')
        self.b2_real_tests()
        self.result['checks']['receipt'] = result
        receipt.verify_bundle(self.work / 'research-bundle/receipt.json', self.context, self.evidence, sanitize_report, artifact=self.work / 'linux/artifact')

    def b2_real_tests(self):
        # This only supplies the already hash-locked fixed candidate. Research
        # admission stays in reviewed cfg(test) code, never an environment grant.
        self.python('native/mnn-shim/prepare_research_config.py', '--model-root', self.model,
                    '--output', self.work / 'b2-verified-runtime.json')
        for package in sorted({package for package, _ in B2_REAL_CASES.values()}):
            env, command = self.cargo('test', '-p', package, '--lib', '--', '--ignored', '--list')
            output = self.run(command, env=env)
            self.result['failure_case'] = 'b2_test_inventory_mismatch'
            verify_b2_inventory(output, package)
        passed = {}
        for name, (package, case_id) in B2_REAL_CASES.items():
            env, command = self.cargo('test', '-p', package, '--lib', name,
                                      '--', '--ignored', '--exact', '--test-threads=1')
            env['NEXA_MNN_TEST_MODEL'] = str(self.model)
            env['NEXA_MNN_B2_RESEARCH_RECEIPT'] = str(self.work / 'research-bundle/receipt.json')
            env['NEXA_MNN_B2_CONTEXT'] = json.dumps(self.context, sort_keys=True, separators=(',', ':'))
            self.result['failure_case'] = case_id
            output = self.run(command, env=env, timeout=900)
            verify_b2_execution(output, name)
            passed[name] = 'pass'
        # Tampering scenarios must touch only private copies. Verify original
        # candidate identity again after the complete real B2 matrix.
        self.python('native/mnn-shim/prepare_research_config.py', '--model-root', self.model,
                    '--output', self.work / 'b2-verified-runtime.json')
        self.result['checks']['b2_real_matrix'] = {'cases': passed, 'research_only': True,
            'production_admitted': False, 'model_lock_sha256': digest(run_probe.LOCK)}

    def android_native(self):
        directory = self.work / 'android'
        self.build(directory, 'native/mnn-shim', android=True)
        self.export(directory, android=True)

    def elf(self):
        reports = {}
        for binary in read(self.work / 'android-binaries.private.json'):
            require(binary['name'] in ANDROID_TARGETS and binary['name'] not in reports)
            path = Path(binary['path']).resolve(strict=True)
            require(path.is_relative_to(self.work / 'rust-android' / TARGET / 'debug'))
            output = self.run([self.toolchain / 'bin/llvm-readelf', '-h', '-l', '-d', '-W', path])
            reports[binary['name']] = dict(parse_elf(output), sha256=digest(path), size=path.stat().st_size)
        require(set(reports) == set(ANDROID_TARGETS))
        self.result['checks'] = reports

    def execute(self):
        try:
            if (self.work / 'cleanup-unconfirmed').exists():
                self.result['failure_case'] = 'previous_cleanup_unconfirmed'
                raise ValueError('previous_cleanup_unconfirmed')
            self.bind_context()
            if self.stage == 'inputs':
                self.inputs_stage()
            elif self.stage in ('rust_linux', 'rust_android'):
                self.rust_stage(self.stage == 'rust_android')
            else:
                getattr(self, self.stage)()
            receipt.check_current(self.context)
            self.result['status'] = 'ok'
            self.result['failure_case'] = 'none'
        except receipt.ReceiptError:
            self.result['failure_case'] = 'research_receipt_rejected'
            raise
        except ElfAuditError as error:
            self.result['failure_case'] = error.failure_case
            raise
        except ArchiveInputError as error:
            self.result['failure_case'] = error.failure_case
            if error.failure_case == 'export_archive_missing':
                self.result['missing_archive'] = error.name
            raise
        finally:
            ci_verify.write(self.evidence / f'{self.stage}.json', self.result)
        print(json.dumps({'stage': self.stage, 'status': self.result['status']}))


def notice_identity():
    path = ROOT / 'native/mnn-shim/notices/manifest.json'
    manifest = read(path)
    return {'manifest_sha256': digest(path), 'components': len(manifest['components']),
            'files': len(manifest['files']), 'final_apk_verified': False}


def parse_android_artifacts(output, target_dir):
    binaries = {}
    for line in output.splitlines():
        if not line.startswith('{'):
            continue
        data = json.loads(line)
        if data.get('reason') != 'compiler-artifact' or not data.get('executable'):
            continue
        target = data['target']
        name = target['name']
        require(name in ANDROID_TARGETS and name not in binaries)
        package, kind, crate_type, is_test, source = ANDROID_TARGETS[name]
        package_root = ROOT / 'mobile/runtime/crates' / package
        require(data['package_id'] == 'path+' + package_root.as_uri() + '#0.1.0')
        require(target['kind'] == [kind] and target['crate_types'] == [crate_type])
        require(data['profile']['test'] is is_test and target['test'] is is_test)
        require(target['src_path'] == str(package_root / source))
        executable = Path(data['executable'])
        require(executable.is_absolute() and executable.resolve().is_relative_to(target_dir.resolve() / TARGET / 'debug'))
        require(str(executable) in data['filenames'])
        binaries[name] = {'name': name, 'path': str(executable)}
    require(set(binaries) == set(ANDROID_TARGETS))
    require(len({entry['path'] for entry in binaries.values()}) == len(ANDROID_TARGETS))
    return [binaries[name] for name in sorted(binaries)]


def verify_b2_inventory(output, package):
    expected = {name for name, (owner, _) in B2_REAL_CASES.items() if owner == package}
    require(expected)
    listed = re.findall(r'^(\S+): test$', output, re.M)
    require(len(listed) == len(expected) and set(listed) == expected)


def verify_b2_execution(output, name):
    require(name in B2_REAL_CASES)
    require(re.findall(r'^running ([0-9]+) tests?$', output, re.M) == ['1'])
    require(len(re.findall(r'^test result:', output, re.M)) == 1)
    require(len(re.findall(r'^test ' + re.escape(name) + r' \.\.\. ok$', output, re.M)) == 1)
    require(len(re.findall(r'^test result: ok\. 1 passed; 0 failed; 0 ignored; 0 measured; [0-9]+ filtered out; finished in [0-9.]+s$', output, re.M)) == 1)


def verify_rust_identity(output, manifest, manifest_sha256):
    lines = output.splitlines()
    require(len(lines) == 7 and all('=' in line for line in lines))
    values = dict(line.split('=', 1) for line in lines)
    require(values == {
        'identity_schema': 'nexa-mnn-native-build-v1', 'artifact_manifest_sha256': manifest_sha256,
        'upstream_commit': manifest['upstream_commit'], 'patch_sha256': manifest['patch_set_sha256'],
        'policy_sha256': manifest['policy_sha256'], 'target': manifest['target'], 'compiler': manifest['compiler'],
    })


def parse_cancel_metrics(output):
    lines = output.splitlines()
    require(lines[0] == 'abi_sampler_pass' and lines[-1] == 'real_request_pass')
    metrics = {}
    for line in lines[1:-1]:
        match = re.fullmatch(r'(cancel_load_checkpoint|cancel_prepare_phase|cancel_phase) ([0-6]) safe_return_ms ([0-9.]+)', line)
        require(match)
        key = match[1] + '_' + match[2]
        value = float(match[3])
        require(key not in metrics and math.isfinite(value) and value >= 0)
        metrics[key] = value
    require(set(metrics) == CANCEL_KEYS)
    return metrics


CANCEL_KEYS = ({'cancel_load_checkpoint_' + str(x) for x in (0, 2, 3, 4, 5, 6)} |
               {'cancel_prepare_phase_2', 'cancel_prepare_phase_3', 'cancel_phase_4', 'cancel_phase_5'})
CHECK_KEYS = {
    'tools': {'source', 'gcc', 'rust', 'cmake', 'ninja', 'notice_inventory'},
    'inputs': {'ndk_sha256', 'ndk_revision', 'model_lock_sha256'},
    'patch': {'pristine_source_unchanged'}, 'baseline': {'unpatched_probe_ctest'},
    'linux_native': {'artifact', 'logging_audit', 'ctest_and_stream_sanitizers'},
    'native_real': {'upstream_exact', 'privacy_canaries', 'prompt_tokens', 'completion_tokens', 'cancel_safe_return_ms'},
    'rust_linux': {'clippy', 'abi_layout', 'artifact_negative', 'unit_compile_fail_and_real_model', 'linked_manifest_sha256'},
    'research_receipt': {'receipt_sha256', 'context_id', 'manifest_sha256', 'research_only', 'production_admitted', 'mode'},
    'b2_linux': {'b2_real_matrix', 'receipt'},
    'android_native': {'artifact', 'logging_audit'},
    'rust_android': {'clippy', 'abi_layout', 'artifact_negative', 'final_elf_link'},
    'elf': set(ANDROID_TARGETS),
}


def validate_identity(value):
    require(set(value) == {'source_commit', 'source_tree', 'source_clean'})
    require(all(re.fullmatch(r'[0-9a-f]{40}', value[key]) for key in ('source_commit', 'source_tree')))
    require(type(value['source_clean']) is bool)


def validate_checks(stage, checks, context=None):
    require(set(checks) == CHECK_KEYS[stage])
    lock = read(ROOT / 'native/mnn-patches/lock.json')
    if stage == 'research_receipt':
        validate_receipt_result(checks, context)
        return
    for key, value in checks.items():
        if key == 'source':
            validate_identity(value)
            require(value['source_clean'] or (context and context['mode'] == 'local-verification'))
        elif key in ('gcc', 'rust', 'cmake', 'ninja', 'ndk_revision', 'ndk_sha256', 'model_lock_sha256'):
            require(value == {'gcc': '14.2.0' if context and context['mode'] == 'local-verification' else '13.3.0', 'rust': '1.98.1', 'cmake': '4.4.3', 'ninja': '1.13.2',
                'ndk_revision': ci_verify.NDK['revision'], 'ndk_sha256': ci_verify.NDK['sha256'],
                'model_lock_sha256': digest(run_probe.LOCK)}[key])
        elif key == 'notice_inventory':
            require(value == notice_identity() and value['components'] == 12 and value['final_apk_verified'] is False)
        elif key == 'receipt':
            validate_receipt_result(value, context)
        elif key == 'b2_real_matrix':
            require(value == {'cases': {name: 'pass' for name in B2_REAL_CASES}, 'research_only': True,
                              'production_admitted': False, 'model_lock_sha256': digest(run_probe.LOCK)})
            require(value['research_only'] is True and value['production_admitted'] is False)
        elif key == 'artifact':
            require(set(value) == {'upstream_commit', 'patch_set_sha256', 'policy_sha256', 'header_sha256',
                                    'target', 'compiler_sha256', 'manifest_sha256', 'archives', 'archive_object_counts'})
            require(all(value[k] == lock[k] for k in ('upstream_commit', 'patch_set_sha256', 'policy_sha256')))
            require(value['header_sha256'] == digest(ROOT / 'native/mnn-shim/include/nexa_mnn.h'))
            require(value['target'] == (TARGET if stage == 'android_native' else 'x86_64-unknown-linux-gnu'))
            require(all(isinstance(value[k], str) and SHA.fullmatch(value[k]) for k in ('compiler_sha256', 'manifest_sha256')))
            names = {'nexa-mnn-shim', 'MNN'} | ({'c++_static', 'c++abi', 'unwind', 'clang_rt_builtins'} if stage == 'android_native' else set())
            require(set(value['archives']) == names and all(isinstance(v, str) and SHA.fullmatch(v) for v in value['archives'].values()))
            require(set(value['archive_object_counts']) == names and all(type(v) is int and v > 0 for v in value['archive_object_counts'].values()))
        elif key == 'linked_manifest_sha256':
            require(isinstance(value, str) and SHA.fullmatch(value))
        elif key == 'logging_audit':
            require(set(value) == {'compiled_source_count', 'header_count'})
            require(all(type(v) is int and v > 0 for v in value.values()))
        elif key == 'cancel_safe_return_ms':
            require(set(value) == CANCEL_KEYS)
            require(all(type(v) in (int, float) and math.isfinite(v) and v >= 0 for v in value.values()))
        elif key in ('prompt_tokens', 'completion_tokens', 'privacy_canaries'):
            require(type(value) is int and 0 < value <= 2048)
            if key == 'privacy_canaries':
                require(value == 9)
            if key == 'completion_tokens':
                require(value <= 12)
        elif stage == 'elf':
            require(set(value) == {'needed', 'load_segments', 'relro_start', 'relro_size', 'relro_end', 'android_run', 'sha256', 'size'})
            require(valid_android_dependencies(value['needed']) and value['android_run'] is False)
            require(isinstance(value['sha256'], str) and SHA.fullmatch(value['sha256']))
            require(all(type(value[k]) is int and value[k] >= 0 for k in ('size', 'relro_start', 'relro_size', 'relro_end')))
            require(value['size'] > 0 and value['relro_size'] > 0 and value['relro_end'] == value['relro_start'] + value['relro_size'] and value['relro_end'] % 16384 == 0)
            require(isinstance(value['load_segments'], list) and 0 < len(value['load_segments']) <= 16)
            for segment in value['load_segments']:
                require(set(segment) == {'offset', 'virtual_address', 'alignment'})
                require(all(type(v) is int and v >= 0 for v in segment.values()))
                alignment = segment['alignment']
                require(alignment >= 16384 and not alignment & (alignment - 1) and segment['offset'] % 16384 == segment['virtual_address'] % 16384)
        else:
            require(value is True)


def validate_receipt_result(value, context):
    require(isinstance(value, dict) and set(value) == CHECK_KEYS['research_receipt'])
    require(all(isinstance(value[k], str) and SHA.fullmatch(value[k]) for k in ('receipt_sha256', 'context_id', 'manifest_sha256')))
    require(value['research_only'] is True and value['production_admitted'] is False)
    require(context and value['context_id'] == context['context_id'] and value['mode'] == context['mode'])


def validate_commands(commands, exit_codes):
    require(isinstance(commands, list) and len(commands) == len(exit_codes))
    for index, command in enumerate(commands):
        require(set(command) == {'index', 'category', 'timeout_seconds', 'timed_out', 'log_limit_exceeded',
            'cleanup_confirmed', 'failure_case', 'exit_code', 'log_bytes', 'output_truncated', 'diagnostics'})
        require(command['index'] == index and command['exit_code'] == exit_codes[index])
        require(command['category'] in CATEGORIES and command['failure_case'] in FAILURE_CASES)
        require(type(command['timeout_seconds']) in (int, float) and 0 < command['timeout_seconds'] <= 900)
        require(type(command['log_bytes']) is int and command['log_bytes'] >= 0)
        require(all(type(command[k]) is bool for k in ('timed_out', 'log_limit_exceeded', 'cleanup_confirmed', 'output_truncated')))
        if not command['cleanup_confirmed']:
            require(command['failure_case'] == 'cleanup_unconfirmed')
        elif command['timed_out']:
            require(command['failure_case'] == 'timeout')
        elif command['log_limit_exceeded']:
            require(command['failure_case'] == 'log_limit')
        elif command['failure_case'] != 'none':
            require(command['exit_code'] != 0)
        if command['failure_case'] == 'none':
            require(command['exit_code'] == 0 and command['cleanup_confirmed'] and not command['timed_out'] and not command['log_limit_exceeded'])
        diagnostics = command['diagnostics']
        require(isinstance(diagnostics, list) and all(isinstance(line, str) for line in diagnostics))
        require(not diagnostics or (command['category'] in BUILD_CATEGORIES and command['failure_case'] != 'none'))
        require(len(diagnostics) <= 16 and sum(len(line) for line in diagnostics) <= MAX_DIAGNOSTIC_BYTES)
        require(all(len(line) <= 400 for line in diagnostics) and safe_diagnostics('\n'.join(diagnostics)) == diagnostics)


def sanitize_report(stage, report):
    require(set(report) == {'schema', 'context', 'stage', 'status', 'android_run', 'exit_codes', 'commands', 'failure_case', 'missing_archive', 'checks'})
    require(type(report['schema']) is int and report['schema'] == 2 and report['stage'] == stage and report['status'] in ('ok', 'failed') and report['android_run'] is False)
    if report['context'] is not None:
        receipt.validate_context(report['context'])
    require(isinstance(report['exit_codes'], list) and len(report['exit_codes']) <= 32)
    require(all(type(code) is int and -128 <= code <= 255 for code in report['exit_codes']))
    require(report['failure_case'] in STAGE_FAILURE_CASES)
    require(report['missing_archive'] is None or report['missing_archive'] in ARCHIVE_NAMES)
    require(report['missing_archive'] is None or report['failure_case'] == 'export_archive_missing')
    validate_commands(report['commands'], report['exit_codes'])
    if stage not in ('baseline', 'linux_native', 'rust_linux', 'research_receipt', 'b2_linux', 'android_native', 'rust_android'):
        require(all(not command['diagnostics'] for command in report['commands']))
    if report['status'] == 'ok':
        require(report['failure_case'] == 'none')
        require(all(command['failure_case'] == 'none' for command in report['commands']))
        require(report['exit_codes'] and all(code == 0 for code in report['exit_codes']))
        require(report['context'] is not None)
        validate_checks(stage, report['checks'], report['context'])
    else:
        require(report['failure_case'] != 'none')
    # Failure reports retain exit codes but never partial/unvalidated nested content.
    return dict(report, checks=report['checks'] if report['status'] == 'ok' else {})


def stage_reports(evidence, destination, outcomes, receipt_bundle):
    destination.mkdir(parents=True, exist_ok=False)
    status = {'schema': 2, 'android_run': False, 'all_required_steps_succeeded': False,
              'evidence_verified': False, 'invalid_reports': [], 'missing_reports': []}
    try:
        pairs = [item.split(':', 1) for item in outcomes.split(',')]
        require(len(pairs) == len(STAGES) and all(len(p) == 2 for p in pairs))
        states = dict(pairs)
        require(set(states) == set(STAGES) and all(v in ('success', 'failure', 'cancelled', 'skipped') for v in states.values()))
        status['steps'] = states
        status.update(ci_verify.source_identity())
        context = None
        try:
            context = receipt.load_context(evidence)
        except (ValueError, KeyError, TypeError, OSError):
            status['invalid_reports'].append('context')
        status['context'] = context
        reports = {}
        for name in STAGES:
            path = evidence / (name + '.json')
            if not path.exists():
                status['missing_reports'].append(name)
                continue
            try:
                raw = receipt.file_bytes(path.absolute())
                reports[name] = sanitize_report(name, receipt.parse_bytes(raw))
                if reports[name]['status'] == 'ok':
                    # Preserve exact verified bytes so uploaded proof hashes remain reproducible.
                    (destination / path.name).write_bytes(raw)
                else:
                    ci_verify.write(destination / path.name, reports[name])
                require(context is not None and reports[name]['context'] == context)
            except (ValueError, KeyError, TypeError, OSError):
                status['invalid_reports'].append(name)
        reports_valid = (not status['missing_reports'] and not status['invalid_reports']
            and all(r['status'] == 'ok' for r in reports.values()))
        if reports_valid:
            try:
                bundle_path = receipt_bundle / 'receipt.json'
                bundle_result = receipt.verify_bundle(bundle_path, context, evidence, sanitize_report, artifact=receipt_bundle.parent / 'linux/artifact')
                require(reports['research_receipt']['checks'] == bundle_result and reports['b2_linux']['checks']['receipt'] == bundle_result)
                receipt_bytes = receipt.file_bytes(bundle_path, readonly=True)
                require(receipt.sha(receipt_bytes) == bundle_result['receipt_sha256'])
                require(reports['rust_linux']['checks']['linked_manifest_sha256'] == reports['linux_native']['checks']['artifact']['manifest_sha256'])
                require(reports['tools']['checks']['source'] == {key: status[key] for key in ('source_commit', 'source_tree', 'source_clean')})
                (destination / 'research-receipt.json').write_bytes(receipt_bytes)
                status['evidence_verified'] = True
            except (ValueError, KeyError, TypeError, OSError):
                status['invalid_reports'].append('research_bundle')
                raise
        status['all_required_steps_succeeded'] = (status['evidence_verified'] and status['source_clean']
                                                 and all(value == 'success' for value in states.values()))
    finally:
        ci_verify.write(destination / 'ci-status.json', status)
    require(status['all_required_steps_succeeded'])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    run = sub.add_parser('run')
    run.add_argument('stage', choices=STAGES)
    run.add_argument('--work', type=Path, required=True)
    run.add_argument('--evidence', type=Path, required=True)
    run.add_argument('--context-mode', choices=('github-ci', 'local-verification'), default='github-ci')
    run.add_argument('--prerequisite-outcomes')
    stage = sub.add_parser('stage')
    stage.add_argument('--evidence', type=Path, required=True)
    stage.add_argument('--destination', type=Path, required=True)
    stage.add_argument('--outcomes', required=True)
    stage.add_argument('--receipt-bundle', type=Path, required=True)
    args = parser.parse_args()
    try:
        if args.command == 'run':
            runner = Runner(args.work, args.evidence, args.stage)
            runner.context_mode = args.context_mode
            runner.prerequisite_outcomes = args.prerequisite_outcomes
            require(args.stage == 'research_receipt' or args.prerequisite_outcomes is None)
            runner.execute()
        else:
            stage_reports(args.evidence, args.destination, args.outcomes, args.receipt_bundle)
    except Exception:
        # Deliberately fixed failure marker: paths, prompts, compiler logs and canaries remain private.
        print('T07-B1 required stage failed; inspect sanitized stage outcomes', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
