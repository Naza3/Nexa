#!/usr/bin/env python3
"""Run only the real fixed research candidate gate, never grant production admission."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--model-dir', type=Path, required=True)
    parser.add_argument('--config', type=Path, required=True)
    args = parser.parse_args()
    workspace = Path(__file__).resolve().parents[1]
    repo = workspace.parents[1]
    lock = json.loads((repo / 'scripts/android_mnn/candidate-model.json').read_text())
    model = args.model_dir.resolve(strict=True)
    for name, expected in lock['files'].items():
        path = model / name
        if path.is_symlink() or not path.is_file() or path.stat().st_size != expected['size']:
            raise SystemExit('Research asset size/type mismatch: ' + name)
        with path.open('rb') as file:
            actual = hashlib.file_digest(file, 'sha256').hexdigest()
        if actual != expected['sha256']:
            raise SystemExit('Research asset hash mismatch: ' + name)
    config = args.config.resolve(strict=True)
    data = json.loads(config.read_text())
    if Path(data['base_dir']).resolve() != model:
        raise SystemExit('Controlled config base_dir does not match verified model')
    # The runtime config remains trusted research input, not a production importer.
    # In particular the caller must keep this directory/config immutable for the run.
    identity = hashlib.sha256(json.dumps(lock, sort_keys=True, separators=(',', ':')).encode()).hexdigest()
    env = dict(os.environ, NEXA_MNN_TEST_CONFIG=str(config), NEXA_MNN_TEST_ARTIFACT_SHA256=identity)
    print('Verified fixed research input SHA256:', identity, flush=True)
    result = subprocess.run(['cargo', 'test', '--manifest-path', str(workspace / 'Cargo.toml'), '--locked', '--offline', '--test', 'real_model', '--', '--ignored', '--test-threads=1'], env=env)
    raise SystemExit(result.returncode)


if __name__ == '__main__':
    main()
