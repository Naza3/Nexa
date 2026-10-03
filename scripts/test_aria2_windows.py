"""Offline fixture/wrapper tests; passing these is not Windows sidecar evidence."""
import hashlib
import io
import json
import os
from pathlib import Path
import tempfile
import threading
import unittest
from unittest import mock
import urllib.request
import zipfile

import probe_aria2_windows as probe


class ProbeTests(unittest.TestCase):
    def test_fixture_identity(self):
        self.assertEqual(len(probe.DATA), 8388608)
        self.assertEqual(hashlib.sha256(probe.DATA).hexdigest(), probe.DATA_SHA256)

    def test_child_environment_excludes_credentials_and_proxy(self):
        with mock.patch.dict(os.environ, {'SECRET': 'secret', 'HTTP_PROXY': 'secret', 'https_proxy': 'secret', 'GITHUB_TOKEN': 'secret'}, clear=True):
            env = probe.child_environment(Path('/fixture'))
        self.assertEqual(set(env), {'HOME', 'USERPROFILE', 'TEMP', 'TMP'})

    def test_command_disables_unwanted_inputs(self):
        args = probe.command('aria2c.exe', Path('/fixture'), 1234, 42)
        for option in ('--no-conf=true', '--no-netrc=true', '--enable-rpc=false', '--follow-torrent=false', '--follow-metalink=false', '--stop-with-process=42', '--check-certificate=true'):
            self.assertIn(option, args)
        self.assertEqual(args[-1], 'http://127.0.0.1:1234/fixture')

    def test_console_allowlist_crlf(self):
        raw = b'private C:\\user\\secret https://host/token\r\n[#abcdef 12B/100B(12%) CN:1]\r[#abcdef 25B/100B(25%) CN:1]\n'
        result = probe.console_summary(raw)
        self.assertEqual((result['cr'], result['lf'], result['crlf']), (2, 2, 1))
        self.assertEqual(result['progress'], [{'done': 12, 'total': 100, 'percent': 12}, {'done': 25, 'total': 100, 'percent': 25}])
        self.assertNotIn('secret', json.dumps(result))
        self.assertNotIn('http', json.dumps(result))

    def test_capture_bounded_but_drains(self):
        data = b'x' * (probe.MAX_CAPTURE * 4)
        capture = probe.Capture(io.BytesIO(data))
        result = capture.finish()
        self.assertEqual(result['captured_bytes'], probe.MAX_CAPTURE)
        self.assertEqual(result['total_bytes'], len(data))
        self.assertTrue(result['truncated'])

    def test_complete_size_alone_is_not_verification(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / 'payload.part').write_bytes(b'x' * len(probe.DATA))
            self.assertFalse(probe.verify_file(root))
            (root / 'payload.part').write_bytes(probe.DATA)
            self.assertTrue(probe.verify_file(root))
            (root / 'payload.part.aria2').write_bytes(b'fixture')
            self.assertTrue(probe.cleanup(root))

    def test_fixture_range_response(self):
        server = probe.Fixture()
        server.delay = 0
        thread = threading.Thread(target=server.serve_forever)
        thread.start()
        try:
            request = urllib.request.Request(f'http://127.0.0.1:{server.server_port}/fixture', headers={'Range': 'bytes=1048576-1114111'})
            with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(request, timeout=5) as response:
                self.assertEqual(response.status, 206)
                self.assertEqual(response.read(), probe.DATA[1048576:1114112])
            self.assertEqual(server.requests, [{'status': 206, 'start': 1048576, 'end': 1114111}])
        finally:
            server.shutdown()
            server.server_close()
            thread.join()

    def fake_archive(self):
        data = io.BytesIO()
        with zipfile.ZipFile(data, 'w') as archive:
            archive.writestr('aria2/aria2c.exe', b'offline mock executable never run')
        return data.getvalue()

    def test_acquire_missing_publisher_digest_labels_observation(self):
        archive = self.fake_archive()
        metadata = {'assets': [{'name': probe.ASSET, 'browser_download_url': probe.ARCHIVE_URL, 'digest': None}]}
        with tempfile.TemporaryDirectory() as folder, mock.patch.object(probe, 'fetch', side_effect=[json.dumps(metadata).encode(), archive]), mock.patch.object(probe, 'ARCHIVE_SHA256', hashlib.sha256(archive).hexdigest()):
            _, identity = probe.acquire(Path(folder))
        self.assertEqual(identity['checksum_basis'], 'observed_official_download_pin_not_publisher_signature')

    def test_acquire_rejects_pin_mismatch(self):
        metadata = {'assets': [{'name': probe.ASSET, 'browser_download_url': probe.ARCHIVE_URL}]}
        with tempfile.TemporaryDirectory() as folder, mock.patch.object(probe, 'fetch', side_effect=[json.dumps(metadata).encode(), self.fake_archive()]):
            with self.assertRaisesRegex(RuntimeError, 'archive_pin_mismatch'):
                probe.acquire(Path(folder))

    def test_acquire_rejects_publisher_digest_mismatch(self):
        archive = self.fake_archive()
        metadata = {'assets': [{'name': probe.ASSET, 'browser_download_url': probe.ARCHIVE_URL, 'digest': 'sha256:' + '0' * 64}]}
        with tempfile.TemporaryDirectory() as folder, mock.patch.object(probe, 'fetch', side_effect=[json.dumps(metadata).encode(), archive]), mock.patch.object(probe, 'ARCHIVE_SHA256', hashlib.sha256(archive).hexdigest()):
            with self.assertRaisesRegex(RuntimeError, 'publisher_digest_mismatch'):
                probe.acquire(Path(folder))


if __name__ == '__main__':
    unittest.main()
