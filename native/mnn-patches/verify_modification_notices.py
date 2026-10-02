#!/usr/bin/env python3
"""Replay the exact changed files only and verify notice-only postimage changes."""
import argparse, hashlib, json, pathlib, shutil, subprocess, tempfile
root = pathlib.Path(__file__).resolve().parent
parser = argparse.ArgumentParser()
parser.add_argument('--source', type=pathlib.Path, required=True,
                    help='Clean exact upstream checkout; never modified')
args = parser.parse_args()
lock = json.loads((root / 'lock.json').read_text())
report = json.loads((root / 'modification-notice-verification.json').read_text())
def digest(data):
    return hashlib.sha256(data).hexdigest()
def require(condition, message):
    if not condition:
        raise SystemExit(message)
require(report['patch_set_sha256'] == lock['patch_set_sha256'], 'patch identity mismatch')
require(report['upstream_commit'] == lock['upstream_commit'], 'upstream changed')
require(report['policy_sha256'] == lock['policy_sha256'], 'policy changed')
require(report['header_sha256'] == digest((root / '../mnn-shim/include/nexa_mnn.h').read_bytes()), 'ABI header changed')
require(subprocess.check_output(['git', '-C', str(args.source), 'rev-parse', 'HEAD'], text=True).strip() == lock['upstream_commit'], 'wrong upstream')
require(not subprocess.check_output(['git', '-C', str(args.source), 'status', '--porcelain', '--untracked-files=all'], text=True).strip(), 'upstream is not clean')
files = {item['path']: item for item in lock['files']}
require(set(files) == {item['path'] for item in report['files']}, 'notice inventory mismatch')
# Only the 13 patch targets are copied, not another source checkout/build tree.
with tempfile.TemporaryDirectory(prefix='nexa-notice-replay-') as directory:
    replay = pathlib.Path(directory)
    for path, expected in files.items():
        original = args.source / path
        require(not original.is_symlink() and original.is_file(), 'non-regular patch target')
        require(digest(original.read_bytes()) == expected['before'], 'preimage mismatch')
        destination = replay / path
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(original, destination)
    for patch in lock['patches']:
        file = root / patch['file']
        require(digest(file.read_bytes()) == patch['sha256'], 'patch bytes mismatch')
        for action in (['--check'], ['--whitespace=error']):
            subprocess.run(['git', 'apply', *action, str(file)], cwd=replay, check=True)
    for item in report['files']:
        content = (replay / item['path']).read_bytes()
        notice = (item['added_notice'] + '\n').encode()
        require(content.startswith(notice), 'prominent notice missing')
        require(digest(content) == files[item['path']]['after'] == item['new_postimage_sha256'], 'postimage mismatch')
        stripped_hash = digest(content[len(notice):])
        require(stripped_hash == item['prior_postimage_sha256'] == item['stripped_new_postimage_sha256'], 'non-notice bytes changed')
print(json.dumps({'files': len(files), 'notice_only_replay': 'passed', 'patch_set_sha256': lock['patch_set_sha256']}))
