#!/usr/bin/env python3
"""Verify notice inventory only; does not validate any linked or packaged binary."""
import argparse
import hashlib
import json
from pathlib import Path

REQUIRED = {
    'mnn', 'flatbuffers', 'half', 'rapidjson', 'skia', 'tensorflow', 'jinja',
    'llvm-compiler-rt', 'llvm-libcxx', 'llvm-libcxxabi', 'llvm-libunwind', 'unicode',
}
CONTROL_FILES = {'README.md', 'manifest.json', 'verify.py'}


def verify(root):
    root = root.resolve()
    manifest = json.loads((root / 'manifest.json').read_text(encoding='utf-8'))
    if manifest.get('schema_version') != 1:
        raise ValueError('unsupported notice schema')
    components = manifest['components']
    ids = [component['id'] for component in components]
    if len(ids) != len(set(ids)) or set(ids) != REQUIRED:
        raise ValueError('required component inventory mismatch')
    declared = {}
    for item in manifest['files']:
        relative = item['path']
        path = root / relative
        if (Path(relative).is_absolute() or '..' in Path(relative).parts
                or path.is_symlink() or not path.resolve().is_relative_to(root)):
            raise ValueError(f'unsafe inventory path: {relative}')
        if relative in declared or relative in CONTROL_FILES:
            raise ValueError(f'duplicate or reserved inventory path: {relative}')
        data = path.read_bytes()
        if len(data) != item['bytes'] or hashlib.sha256(data).hexdigest() != item['sha256']:
            raise ValueError(f'notice content/hash mismatch: {relative}')
        if not item.get('source') or not item.get('extraction'):
            raise ValueError(f'missing provenance: {relative}')
        declared[relative] = item
    for component in components:
        if not component['license_files']:
            raise ValueError(f'missing license: {component["id"]}')
        for reference in component['license_files'] + component['attribution_files']:
            if reference not in declared:
                raise ValueError(f'undeclared component file: {reference}')
    actual = {str(path.relative_to(root)) for path in root.rglob('*') if path.is_file()}
    if actual != set(declared) | CONTROL_FILES:
        raise ValueError(f'unexpected/missing files: {sorted(actual ^ (set(declared) | CONTROL_FILES))}')
    if manifest['libatomic']['included_component'] is not False:
        raise ValueError('libatomic placeholder incorrectly classified')
    return len(components), len(declared)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parent)
    args = parser.parse_args()
    try:
        components, files = verify(args.root)
    except (ValueError, KeyError, TypeError, OSError) as error:
        parser.exit(1, f'FAIL: {error}\n')
    print(f'PASS: {components} required components, {files} hashed notice/evidence files')
    print('Scope: source inventory only; final APK/.so linkage and license UI are NOT verified')


if __name__ == '__main__':
    main()
