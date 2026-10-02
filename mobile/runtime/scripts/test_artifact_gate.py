#!/usr/bin/env python3
"""Negative build identity tests; never executes inference or replaces native symbols."""
import copy
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


def main():
    workspace = Path(__file__).resolve().parents[1]
    artifact = Path(os.environ['NEXA_MNN_ARTIFACT_DIR']).resolve(strict=True)
    manifest = json.loads((artifact / 'artifact.json').read_text())
    command = ['cargo', 'check', '--manifest-path', str(workspace / 'Cargo.toml'), '--locked', '--offline']
    if manifest['target'] == 'aarch64-linux-android':
        command += ['--target', manifest['target']]

    def rejected(env, expected):
        result = subprocess.run(command, env=env, capture_output=True, text=True)
        if result.returncode == 0 or expected not in result.stderr:
            raise AssertionError('Expected rejection: ' + expected + '\n' + result.stderr)

    cases = [
        ('abi_version', 2, 'assertion'),
        ('target', 'wrong-target', 'target mismatch'),
        ('patch_set_sha256', '0' * 64, 'repository lock'),
        ('header_sha256', '0' * 64, 'reviewed Rust bindings'),
        ('silent_logs', False, 'logging must be silent'),
        ('compiler', 'unrecognized compiler', 'compiler identity mismatch' if 'linux-gnu' in manifest['target'] else 'pinned Android Clang21'),
    ]
    if manifest['target'] == 'aarch64-linux-android':
        cases += [('android_ndk_revision', 'wrong-ndk', 'requires NDK r30'), ('android_api', 27, 'requires Android API28')]
    for key, value, expected in cases:
        with tempfile.TemporaryDirectory(prefix='nexa-mnn-rejection-') as directory:
            modified = copy.deepcopy(manifest)
            modified[key] = value
            Path(directory, 'artifact.json').write_text(json.dumps(modified))
            rejected(dict(os.environ, NEXA_MNN_ARTIFACT_DIR=directory), expected)
            print(key, 'rejected')
    with tempfile.TemporaryDirectory(prefix='nexa-mnn-rejection-') as directory:
        shutil.copytree(artifact, directory, dirs_exist_ok=True)
        archive = Path(directory, manifest['static_libraries'][0]['path'])
        with archive.open('ab') as file:
            file.write(b'corrupt-test-input')
        rejected(dict(os.environ, NEXA_MNN_ARTIFACT_DIR=directory), 'archive digest mismatch')
        print('archive corruption rejected')
    env = dict(os.environ)
    del env['NEXA_MNN_ARTIFACT_DIR']
    rejected(env, 'set NEXA_MNN_ARTIFACT_DIR')
    print('missing explicit input rejected')
    # Restore the verified input as the final Cargo build state.
    subprocess.run(command, check=True)


if __name__ == '__main__':
    main()
