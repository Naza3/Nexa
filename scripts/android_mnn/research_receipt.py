#!/usr/bin/env python3
"""Same-run test evidence, not authentication or product/model admission.

Only reviewed helper execution plus independent CI review supplies trust. A local
operator who controls tests can forge this format; it is not a signed attestation.
"""
import ctypes
import errno
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import stat
import subprocess
import time

ROOT = Path(__file__).resolve().parents[2]
PREREQUISITES = ('tools', 'inputs', 'patch', 'baseline', 'linux_native', 'native_real', 'rust_linux')
MAX_FILE = 65536
MAX_AGE = 2700
MODEL_DIGEST = '1ec59d439451738b4992f2ea5b06438788d752da81d55f11e1e8866d03fa7a57'
SCOPES = ('native/mnn-shim', 'native/mnn-patches', 'native/mnn-probe', 'scripts/android_mnn',
          'mobile/runtime', 'crates/runtime-core', 'crates/runtime-types', 'Cargo.toml', 'Cargo.lock',
          '.github/workflows/android-mnn-native.yml')
CONTEXT_FIELDS = {'mode', 'source_commit', 'source_tree', 'source_clean', 'source_snapshot_sha256',
                  'run_id', 'run_attempt', 'job', 'context_id'}
SUBJECT_FIELDS = {'artifact_manifest_sha256', 'upstream_commit', 'patch_set_sha256', 'policy_sha256',
                  'header_sha256', 'target', 'compiler', 'compiler_sha256', 'profile',
                  'input_lock_sha256', 'candidate_identity_sha256', 'template_sha256'}
RECEIPT_FIELDS = {'schema_version', 'purpose', 'research_only', 'production_admitted', 'android_run',
                  'context', 'issued_at', 'expires_at', 'subject', 'prerequisites'}
SHA = re.compile(r'[0-9a-f]{64}')


class ReceiptError(ValueError):
    pass


def require(value):
    if not value:
        raise ReceiptError('research_receipt_rejected')


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result)
        result[key] = value
    return result


def parse_bytes(data):
    require(len(data) <= MAX_FILE)
    try:
        return json.loads(data.decode('utf-8'), object_pairs_hook=unique_object,
                          parse_constant=lambda _: (_ for _ in ()).throw(ReceiptError('research_receipt_rejected')))
    except (ValueError, UnicodeError, RecursionError):
        raise ReceiptError('research_receipt_rejected') from None


def file_bytes(path, readonly=False):
    require(path.is_absolute() and path.resolve() == path and not path.is_symlink())
    before = path.lstat()
    require(stat.S_ISREG(before.st_mode) and before.st_nlink == 1 and before.st_size <= MAX_FILE)
    require(not readonly or not before.st_mode & 0o222)
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(fd, 'rb') as file:
        metadata = os.fstat(file.fileno())
        require((metadata.st_dev, metadata.st_ino) == (before.st_dev, before.st_ino))
        data = file.read(MAX_FILE + 1)
        after = os.fstat(file.fileno())
    current = path.lstat()
    require((before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns) ==
            (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns))
    require((after.st_dev, after.st_ino) == (current.st_dev, current.st_ino) and len(data) == before.st_size)
    require(len(data) <= MAX_FILE)
    return data


def read(path):
    return parse_bytes(file_bytes(path))


def sha(data):
    return hashlib.sha256(data).hexdigest()


def digest(path):
    with path.open('rb') as file:
        return hashlib.file_digest(file, 'sha256').hexdigest()


def encode(value):
    return (json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=True) + '\n').encode()


def source_state(mode):
    def git(*args):
        return subprocess.run(['git', '-C', str(ROOT), *args], check=True, capture_output=True,
                              encoding='utf-8', timeout=20).stdout
    commit, tree = git('rev-parse', 'HEAD').strip(), git('rev-parse', 'HEAD^{tree}').strip()
    clean = not bool(git('status', '--porcelain', '--untracked-files=all'))
    paths = sorted(set(git('ls-files', '-z', '--cached', '--others', '--exclude-standard', '--', *SCOPES).split('\0')) - {''})
    h = hashlib.sha256()
    for relative in paths:
        path = ROOT / relative
        require(path.is_file() and not path.is_symlink() and path.stat().st_size <= 8 * 1024 * 1024)
        h.update(relative.encode() + b'\0' + digest(path).encode() + b'\0')
    if mode == 'github-ci':
        require(clean and os.environ.get('GITHUB_ACTIONS') == 'true' and os.environ.get('GITHUB_SHA') == commit)
    return {'source_commit': commit, 'source_tree': tree, 'source_clean': clean, 'source_snapshot_sha256': h.hexdigest()}


def validate_context(context):
    require(isinstance(context, dict) and set(context) == CONTEXT_FIELDS)
    require(context['mode'] in ('github-ci', 'local-verification'))
    require(all(isinstance(context[k], str) and re.fullmatch(r'[0-9a-f]{40}', context[k]) for k in ('source_commit', 'source_tree')))
    require(all(isinstance(context[k], str) and SHA.fullmatch(context[k]) for k in ('source_snapshot_sha256', 'context_id')))
    require(type(context['source_clean']) is bool and context['job'] == 'native-adapter')
    require(all(isinstance(context[k], str) and re.fullmatch(r'[1-9][0-9]{0,19}', context[k]) for k in ('run_id', 'run_attempt')))
    if context['mode'] == 'github-ci':
        require(context['source_clean'])


def check_current(context):
    validate_context(context)
    current = source_state(context['mode'])
    require({k: context[k] for k in current} == current)
    if context['mode'] == 'github-ci':
        require(os.environ.get('GITHUB_RUN_ID') == context['run_id'] and os.environ.get('GITHUB_RUN_ATTEMPT') == context['run_attempt']
                and os.environ.get('GITHUB_JOB') == context['job'])
    else:
        require(os.environ.get('GITHUB_ACTIONS') != 'true')


def initialize(evidence, mode):
    require(mode in ('github-ci', 'local-verification'))
    if mode == 'local-verification':
        require(os.environ.get('GITHUB_ACTIONS') != 'true')
    context = dict(source_state(mode), mode=mode, context_id=secrets.token_hex(32), job='native-adapter',
                   run_id=os.environ.get('GITHUB_RUN_ID', str(time.time_ns())) if mode == 'github-ci' else str(time.time_ns()),
                   run_attempt=os.environ.get('GITHUB_RUN_ATTEMPT', '1') if mode == 'github-ci' else '1')
    validate_context(context)
    evidence.mkdir(parents=True, exist_ok=True)
    path = evidence.resolve() / 'context.private.json'
    with path.open('xb') as file:
        file.write(encode(context))
    path.chmod(0o600)
    return context


def load_context(evidence):
    context = read(evidence.resolve() / 'context.private.json')
    check_current(context)
    return context


def outcomes(text):
    pairs = [item.split(':') for item in text.split(',')]
    require(len(pairs) == len(PREREQUISITES) and all(len(pair) == 2 for pair in pairs))
    result = dict(pairs)
    require(set(result) == set(PREREQUISITES) and all(value == 'success' for value in result.values()))
    return result


def success_envelope(report, context, name):
    require(isinstance(report, dict) and type(report.get('schema')) is int and report.get('schema') == 2 and report.get('context') == context and report.get('stage') == name)
    require(report.get('status') == 'ok' and report.get('failure_case') == 'none' and report.get('android_run') is False)
    codes, commands = report.get('exit_codes'), report.get('commands')
    require(isinstance(codes, list) and codes and all(type(code) is int and code == 0 for code in codes))
    require(isinstance(commands, list) and len(codes) == len(commands))
    for i, command in enumerate(commands):
        require(command.get('index') == i and command.get('exit_code') == 0 and command.get('cleanup_confirmed') is True
                and command.get('timed_out') is False and command.get('log_limit_exceeded') is False
                and command.get('failure_case') == 'none')


def subject_for(artifact):
    manifest_path = artifact.resolve() / 'artifact.json'
    manifest = read(manifest_path)
    lock = read(ROOT / 'native/mnn-patches/lock.json')
    candidate_path = ROOT / 'scripts/android_mnn/candidate-model.json'
    candidate = read(candidate_path)
    require(set(manifest) == {'schema_version', 'abi_version', 'target', 'upstream_commit', 'patch_set_sha256',
            'policy_sha256', 'header_sha256', 'compiler', 'build_type', 'static_libraries', 'system_libraries', 'silent_logs'})
    require(manifest['system_libraries'] == ['m', 'dl', 'pthread', 'stdc++'])
    require(manifest['target'] == 'x86_64-unknown-linux-gnu' and type(manifest['abi_version']) is int and manifest['abi_version'] == 1
            and type(manifest['schema_version']) is int and manifest['schema_version'] == 1 and manifest['build_type'] == 'Release' and manifest['silent_logs'] is True)
    require([entry['name'] for entry in manifest['static_libraries']] == ['nexa-mnn-shim', 'MNN'])
    for entry in manifest['static_libraries']:
        require(set(entry) == {'name', 'path', 'sha256'} and isinstance(entry['sha256'], str) and SHA.fullmatch(entry['sha256']))
        require(entry['path'] == 'lib/lib' + entry['name'] + '.a')
        path = artifact.resolve() / entry['path']
        require(path.is_file() and path.resolve() == path and digest(path) == entry['sha256'])
    for key in ('upstream_commit', 'patch_set_sha256', 'policy_sha256'):
        require(manifest[key] == lock[key])
    require(manifest['header_sha256'] == digest(ROOT / 'native/mnn-shim/include/nexa_mnn.h'))
    return dict({key: manifest[key] for key in ('upstream_commit', 'patch_set_sha256', 'policy_sha256', 'header_sha256', 'target', 'compiler')},
                artifact_manifest_sha256=digest(manifest_path), compiler_sha256=sha(manifest['compiler'].encode()),
                input_lock_sha256=digest(candidate_path), candidate_identity_sha256=MODEL_DIGEST,
                template_sha256=candidate['template_sha256'])


def validate_subject(subject, context):
    require(isinstance(subject, dict) and set(subject) == SUBJECT_FIELDS)
    require(subject['target'] == 'x86_64-unknown-linux-gnu' and subject['candidate_identity_sha256'] == MODEL_DIGEST)
    require(all(isinstance(subject[key], str) and SHA.fullmatch(subject[key]) for key in SUBJECT_FIELDS if key.endswith('sha256')))
    require(isinstance(subject['upstream_commit'], str) and re.fullmatch(r'[0-9a-f]{40}', subject['upstream_commit']))
    require(isinstance(subject['compiler'], str) and subject['compiler_sha256'] == sha(subject['compiler'].encode()))
    if context['mode'] == 'github-ci':
        require(subject['profile'] == 'ubuntu24.04-gcc13.3-cpu-v1'
                and subject['compiler'] == 'g++-13 (Ubuntu 13.3.0-6ubuntu2~24.04.1) 13.3.0')
    else:
        require(subject['profile'] == 'debian-gcc14.2-local-cpu-v1' and subject['compiler'] == 'c++ (Debian 14.2.0-19) 14.2.0')


def crosscheck(reports, subject):
    native = reports['linux_native']['checks']['artifact']
    require(native['manifest_sha256'] == subject['artifact_manifest_sha256'])
    require(all(native[k] == subject[k] for k in ('upstream_commit', 'patch_set_sha256', 'policy_sha256', 'header_sha256', 'target', 'compiler_sha256')))
    require(reports['rust_linux']['checks']['linked_manifest_sha256'] == subject['artifact_manifest_sha256'])
    require(reports['inputs']['checks']['model_lock_sha256'] == subject['input_lock_sha256'])
    require(reports['tools']['checks']['source'] == {k: reports['tools']['context'][k]
            for k in ('source_commit', 'source_tree', 'source_clean')})


def verify_live_subject(artifact, subject, mode, native_report=None):
    actual = subject_for(artifact)
    actual['profile'] = 'ubuntu24.04-gcc13.3-cpu-v1' if mode == 'github-ci' else 'debian-gcc14.2-local-cpu-v1'
    require(actual == subject)
    if native_report is not None:
        manifest = read(artifact.resolve() / 'artifact.json')
        require(native_report['checks']['artifact']['archives'] ==
                {entry['name']: entry['sha256'] for entry in manifest['static_libraries']})


def publish_directory(source, destination):
    # Linux CI/local verification: RENAME_NOREPLACE avoids overwriting even an
    # empty destination created between validation and publication.
    libc = ctypes.CDLL(None, use_errno=True)
    rename = libc.renameat2
    rename.argtypes = (ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p, ctypes.c_uint)
    rename.restype = ctypes.c_int
    if rename(-100, os.fsencode(source), -100, os.fsencode(destination), 1) != 0:
        code = ctypes.get_errno()
        if code == errno.EEXIST:
            raise ReceiptError('research_receipt_rejected')
        raise OSError(code, 'receipt publication failed')


def mint(evidence, bundle, artifact, actual_outcomes, sanitizer, *, work, now=None):
    context = load_context(evidence)
    states = outcomes(actual_outcomes)
    require(bundle.is_absolute() and bundle.parent.resolve() == bundle.parent and not bundle.exists() and not bundle.is_symlink())
    require(not (work / 'cleanup-unconfirmed').exists())
    reports, raw = {}, {}
    for name in PREREQUISITES:
        data = file_bytes(evidence.resolve() / (name + '.json'))
        report = parse_bytes(data)
        sanitizer(name, report)
        success_envelope(report, context, name)
        reports[name], raw[name] = report, data
    subject = subject_for(artifact)
    subject['profile'] = 'ubuntu24.04-gcc13.3-cpu-v1' if context['mode'] == 'github-ci' else 'debian-gcc14.2-local-cpu-v1'
    validate_subject(subject, context)
    crosscheck(reports, subject)
    verify_live_subject(artifact, subject, context['mode'], reports['linux_native'])
    timestamp = int(time.time()) if now is None else now
    receipt = {'schema_version': 1, 'purpose': 'ci-linux-b2-research', 'research_only': True,
        'production_admitted': False, 'android_run': False, 'context': context,
        'issued_at': timestamp, 'expires_at': timestamp + MAX_AGE, 'subject': subject,
        'prerequisites': [{'stage': name, 'sha256': sha(raw[name]), 'outcome': states[name]} for name in PREREQUISITES]}
    check_current(context)
    temp = bundle.with_name(bundle.name + '.pending-' + secrets.token_hex(8))
    temp.mkdir(mode=0o700, parents=False)
    try:
        for name, data in dict({name + '.json': raw[name] for name in PREREQUISITES}, **{'receipt.json': encode(receipt)}).items():
            with (temp / name).open('xb') as file:
                file.write(data)
            (temp / name).chmod(0o444)
        require(not bundle.exists())
        temp.chmod(0o555)
        # Check the complete sealed pending tree before it becomes consumable.
        verify_bundle(temp / 'receipt.json', context, evidence, sanitizer, now=timestamp)
        require(not bundle.exists() and not bundle.is_symlink())
        publish_directory(temp, bundle)
    except Exception:
        if temp.exists():
            temp.chmod(0o700)
        for path in temp.iterdir() if temp.exists() else []:
            path.unlink()
        if temp.exists():
            temp.rmdir()
        raise
    return result_for(receipt, encode(receipt))


def verify_bundle(path, context, evidence, sanitizer, *, now=None, live=True, artifact=None):
    require(path.name == 'receipt.json' and path.parent.resolve() == path.parent)
    require(stat.S_ISDIR(path.parent.lstat().st_mode) and not path.parent.stat().st_mode & 0o222)
    data = file_bytes(path, readonly=True)
    receipt = parse_bytes(data)
    require(isinstance(receipt, dict) and set(receipt) == RECEIPT_FIELDS and type(receipt['schema_version']) is int and receipt['schema_version'] == 1)
    require(receipt['purpose'] == 'ci-linux-b2-research' and receipt['research_only'] is True
            and receipt['production_admitted'] is False and receipt['android_run'] is False)
    validate_context(context)
    require(receipt['context'] == context)
    validate_subject(receipt['subject'], context)
    current = int(time.time()) if now is None else now
    require(type(receipt['issued_at']) is int and type(receipt['expires_at']) is int
            and 0 <= receipt['issued_at'] <= current < receipt['expires_at']
            and 0 < receipt['expires_at'] - receipt['issued_at'] <= MAX_AGE)
    if live:
        check_current(context)
    proofs = receipt['prerequisites']
    require(isinstance(proofs, list) and len(proofs) == len(PREREQUISITES))
    require([proof.get('stage') for proof in proofs] == list(PREREQUISITES))
    require({p.name for p in path.parent.iterdir()} == {'receipt.json'} | {name + '.json' for name in PREREQUISITES})
    reports = {}
    for proof in proofs:
        require(set(proof) == {'stage', 'sha256', 'outcome'} and proof['outcome'] == 'success' and SHA.fullmatch(proof['sha256']))
        content = file_bytes(path.parent / (proof['stage'] + '.json'), readonly=True)
        require(sha(content) == proof['sha256'])
        report = parse_bytes(content)
        success_envelope(report, context, proof['stage'])
        reports[proof['stage']] = report
        sanitizer(proof['stage'], report)
        require(file_bytes(evidence.resolve() / (proof['stage'] + '.json')) == content)
    crosscheck(reports, receipt['subject'])
    if artifact is not None:
        verify_live_subject(artifact, receipt['subject'], context['mode'], reports['linux_native'])
    return result_for(receipt, data)


def result_for(receipt, data):
    context = receipt['context']
    return {'receipt_sha256': sha(data), 'context_id': context['context_id'],
            'manifest_sha256': receipt['subject']['artifact_manifest_sha256'], 'research_only': True,
            'production_admitted': False, 'mode': context['mode']}
