"""Format and orchestration negatives; fixtures are not real execution evidence."""
import copy
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import b1_ci as ci
import research_receipt as r
from test_b1_ci import CONTEXT, report


class ReceiptTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.evidence = self.root / 'evidence'
        self.evidence.mkdir()
        self.work = self.root / 'work'
        self.work.mkdir()
        self.bundle = self.work / 'bundle'
        self.context = copy.deepcopy(CONTEXT)
        self.subject = dict(artifact_manifest_sha256='c' * 64,
            upstream_commit=ci.read(ci.ROOT / 'native/mnn-patches/lock.json')['upstream_commit'],
            patch_set_sha256=ci.read(ci.ROOT / 'native/mnn-patches/lock.json')['patch_set_sha256'],
            policy_sha256=ci.read(ci.ROOT / 'native/mnn-patches/lock.json')['policy_sha256'],
            header_sha256=ci.digest(ci.ROOT / 'native/mnn-shim/include/nexa_mnn.h'),
            target='x86_64-unknown-linux-gnu', compiler='g++-13 (Ubuntu 13.3.0-6ubuntu2~24.04.1) 13.3.0',
            profile='ubuntu24.04-gcc13.3-cpu-v1', input_lock_sha256=ci.digest(ci.run_probe.LOCK),
            candidate_identity_sha256=r.MODEL_DIGEST, template_sha256=ci.read(ci.run_probe.LOCK)['template_sha256'])
        self.subject['compiler_sha256'] = r.sha(self.subject['compiler'].encode())
        self.artifact = self.work / 'artifact'
        (self.artifact / 'lib').mkdir(parents=True)
        libraries = []
        for name in ('nexa-mnn-shim', 'MNN'):
            archive = self.artifact / ('lib/lib' + name + '.a')
            archive.write_bytes(b'format-only archive fixture ' + name.encode())
            libraries.append(dict(name=name, path='lib/lib' + name + '.a', sha256=r.digest(archive)))
        manifest = {k:self.subject[k] for k in ('upstream_commit','patch_set_sha256','policy_sha256','header_sha256','target','compiler')}
        manifest.update(schema_version=1, abi_version=1, build_type='Release', silent_logs=True,
                        static_libraries=libraries, system_libraries=['m','dl','pthread','stdc++'])
        (self.artifact / 'artifact.json').write_bytes(r.encode(manifest))
        self.subject['artifact_manifest_sha256'] = r.digest(self.artifact / 'artifact.json')
        (self.evidence / 'context.private.json').write_bytes(r.encode(self.context))
        for name in r.PREREQUISITES:
            value = report(name)
            if name == 'linux_native':
                value['checks']['artifact']['compiler_sha256'] = self.subject['compiler_sha256']
                value['checks']['artifact']['manifest_sha256'] = self.subject['artifact_manifest_sha256']
                value['checks']['artifact']['archives'] = {entry['name']:entry['sha256'] for entry in libraries}
            if name == 'rust_linux':
                value['checks']['linked_manifest_sha256'] = self.subject['artifact_manifest_sha256']
            (self.evidence / (name + '.json')).write_bytes(r.encode(value))
        self.outcomes = ','.join(n + ':success' for n in r.PREREQUISITES)
        self.current = patch.object(r, 'check_current')
        self.current.start()
        self.addCleanup(self.current.stop)
        self.addCleanup(self.unlock)

    def unlock(self):
        for p in self.work.iterdir():
            if p.is_dir():
                p.chmod(0o700)

    def mint(self):
        return r.mint(self.evidence, self.bundle, self.work / 'artifact', self.outcomes,
                      ci.sanitize_report, work=self.work, now=1000)

    def verify(self, now=1001):
        return r.verify_bundle(self.bundle / 'receipt.json', self.context, self.evidence,
                               ci.sanitize_report, now=now)

    def change_receipt(self, mutate):
        path = self.bundle / 'receipt.json'
        value = r.read(path)
        mutate(value)
        path.chmod(0o600)
        path.write_bytes(r.encode(value))
        path.chmod(0o444)

    def test_complete_atomic_readonly_bundle_and_exact_original_proofs(self):
        result = self.mint()
        self.assertEqual(self.verify(), result)
        self.assertEqual({p.name for p in self.bundle.iterdir()}, {'receipt.json'} | {s + '.json' for s in r.PREREQUISITES})
        self.assertEqual(self.bundle.stat().st_mode & 0o777, 0o555)
        self.assertTrue(all(p.stat().st_mode & 0o777 == 0o444 for p in self.bundle.iterdir()))
        self.assertFalse(list(self.work.glob('*.pending-*')))
        with self.assertRaises(ValueError):
            self.mint()

    def test_each_prerequisite_failure_or_missing_cannot_publish(self):
        for name in r.PREREQUISITES:
            path = self.evidence / (name + '.json')
            original = path.read_bytes()
            for mutate in (lambda v: v.update(status='failed', failure_case='timeout'),
                           lambda v: v['exit_codes'].__setitem__(0, 1),
                           lambda v: v['commands'][0].update(cleanup_confirmed=False),
                           lambda v: v['commands'][0].update(timed_out=True),
                           lambda v: v['commands'][0].update(log_limit_exceeded=True),
                           lambda v: v['context'].update(context_id='5' * 64),
                           lambda v: v.update(checks={}),
                           lambda v: v.update(schema=1)):
                value = r.parse_bytes(original)
                mutate(value)
                path.write_bytes(r.encode(value))
                with self.subTest(name=name), self.assertRaises(ValueError):
                    self.mint()
                self.assertFalse(self.bundle.exists())
            path.unlink()
            with self.assertRaises(FileNotFoundError):
                self.mint()
            path.write_bytes(original)

    def test_outcomes_no_skipped_duplicate_cycle_or_other_names(self):
        original = self.outcomes
        for bad in (original.replace('tools:success', 'tools:skipped'), original + ',tools:success',
                    original.replace('tools:', 'b2_linux:'), original.replace('tools:', 'research_receipt:'),
                    original.replace('success', 'failure'), ''):
            self.outcomes = bad
            with self.assertRaises(ValueError):
                self.mint()
            self.assertFalse(self.bundle.exists())

    def test_live_manifest_archive_header_and_template_mismatch(self):
        archive=self.artifact / 'lib/libMNN.a'
        original=archive.read_bytes()
        archive.write_bytes(original+b'changed')
        with self.assertRaises(ValueError):
            self.mint()
        self.assertFalse(self.bundle.exists())
        archive.write_bytes(original)
        manifest=self.artifact / 'artifact.json'
        original=manifest.read_bytes()
        for key,value in [('header_sha256','a'*64),('target','aarch64-linux-android'),('compiler','wrong'),('silent_logs',False),('extra',True)]:
            content=r.parse_bytes(original);content[key]=value;manifest.write_bytes(r.encode(content))
            with self.assertRaises(ValueError):
                self.mint()
            self.assertFalse(self.bundle.exists())
        manifest.write_bytes(original)
        self.mint()
        archive.write_bytes(b'tampered-after-mint')
        with self.assertRaises(ValueError):
            r.verify_bundle(self.bundle / 'receipt.json', self.context, self.evidence, ci.sanitize_report,
                            now=1001, artifact=self.artifact)

    def test_cleanup_marker_blocks_mint(self):
        (self.work / 'cleanup-unconfirmed').touch()
        with self.assertRaises(ValueError):
            self.mint()
        self.assertFalse(self.bundle.exists())

    def test_failure_during_sealing_publishes_nothing(self):
        with patch.object(r, 'verify_bundle', side_effect=r.ReceiptError('rejected')):
            with self.assertRaises(ValueError):
                self.mint()
        self.assertFalse(self.bundle.exists())
        self.assertFalse(list(self.work.glob('*.pending-*')))

    def test_publication_race_never_replaces_existing_directory(self):
        original = r.publish_directory
        def competing(source, destination):
            destination.mkdir()
            original(source, destination)
        with patch.object(r, 'publish_directory', side_effect=competing):
            with self.assertRaises(ValueError):
                self.mint()
        self.assertEqual(list(self.bundle.iterdir()), [])
        self.assertFalse(list(self.work.glob('*.pending-*')))

    def test_receipt_negative_matrix(self):
        self.mint()
        path = self.bundle / 'receipt.json'
        original = path.read_bytes()
        for mutate in (lambda v: v.update(extra='untrusted'), lambda v: v.update(schema_version=True),
                       lambda v: v.update(production_admitted=True), lambda v: v.update(android_run=True),
                       lambda v: v.update(research_only=False), lambda v: v.update(issued_at=1002),
                       lambda v: v.update(expires_at=1001), lambda v: v.update(expires_at=3701),
                       lambda v: v['context'].update(run_id='2'), lambda v: v['context'].update(source_tree='7' * 40),
                       lambda v: v['subject'].update(target='aarch64-linux-android'),
                       lambda v: v['subject'].update(compiler='unreviewed compiler'),
                       lambda v: v['subject'].update(extra=True),
                       lambda v: v['subject'].update(artifact_manifest_sha256='e' * 64),
                       lambda v: v['subject'].update(input_lock_sha256='e' * 64),
                       lambda v: v['subject'].update(candidate_identity_sha256='e' * 64),
                       lambda v: v['prerequisites'].__setitem__(0, v['prerequisites'][1]),
                       lambda v: v['prerequisites'][0].update(stage='../tools'),
                       lambda v: v['prerequisites'][0].update(outcome='skipped'),
                       lambda v: v['prerequisites'][0].update(sha256='e' * 64),
                       lambda v: v['prerequisites'][0].update(extra=True)):
            path.chmod(0o600)
            path.write_bytes(original)
            path.chmod(0o444)
            self.change_receipt(mutate)
            with self.assertRaises((ValueError, KeyError, TypeError)):
                self.verify()

    def test_proof_or_original_replacement_and_extra_files_rejected(self):
        self.mint()
        for path in (self.bundle / 'tools.json', self.evidence / 'tools.json'):
            original = path.read_bytes()
            path.chmod(0o600)
            path.write_bytes(original + b' ')
            path.chmod(0o444)
            with self.assertRaises(ValueError):
                self.verify()
            path.chmod(0o600)
            path.write_bytes(original)
            path.chmod(0o444)
        self.bundle.chmod(0o700)
        (self.bundle / 'extra').touch()
        self.bundle.chmod(0o555)
        with self.assertRaises(ValueError):
            self.verify()

    def test_file_bounds_links_permissions_and_json(self):
        self.mint()
        path = self.bundle / 'tools.json'
        path.chmod(0o644)
        with self.assertRaises(ValueError):
            self.verify()
        path.chmod(0o444)
        linked = self.work / 'hardlink'
        os.link(path, linked)
        with self.assertRaises(ValueError):
            self.verify()
        linked.unlink()
        linked.symlink_to(path)
        with self.assertRaises(ValueError):
            r.file_bytes(linked)
        for payload in (b'{"x":1,"x":2}', b'{"x":{"y":1,"y":2}}', b'{"x":NaN}', b' ' * (r.MAX_FILE + 1)):
            with self.assertRaises(ValueError):
                r.parse_bytes(payload)
        with self.assertRaises(ValueError):
            self.verify(now=3700)

    def test_same_run_source_environment_is_checked(self):
        self.current.stop()
        source = {k: self.context[k] for k in ('source_commit', 'source_tree', 'source_clean', 'source_snapshot_sha256')}
        env = dict(GITHUB_ACTIONS='true', GITHUB_RUN_ID='1', GITHUB_RUN_ATTEMPT='1', GITHUB_JOB='native-adapter')
        with patch.object(r, 'source_state', return_value=source), patch.dict(os.environ, env):
            r.check_current(self.context)
            for key in ('GITHUB_RUN_ID', 'GITHUB_RUN_ATTEMPT', 'GITHUB_JOB'):
                with patch.dict(os.environ, {key: 'bad'}), self.assertRaises(ValueError):
                    r.check_current(self.context)
            source['source_snapshot_sha256'] = '9' * 64
            with self.assertRaises(ValueError):
                r.check_current(self.context)


if __name__ == '__main__':
    unittest.main()
