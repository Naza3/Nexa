#!/usr/bin/env python3
"""Independent, bounded Windows sidecar observation; never a product downloader.

Only the official pinned archive is fetched externally. All transfer cases use an
8 MiB loopback HTTP fixture. No claim about production HTTPS/IP policy or MS/HF.
"""
from __future__ import annotations

import argparse
import ctypes
import hashlib
import http.server
import json
import os
from pathlib import Path
import re
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request
import zipfile

VERSION = '1.37.0'
ASSET = 'aria2-1.37.0-win-64bit-build1.zip'
RELEASE_API = 'https://api.github.com/repos/aria2/aria2/releases/tags/release-1.37.0'
ARCHIVE_URL = f'https://github.com/aria2/aria2/releases/download/release-1.37.0/{ASSET}'
# Independently observed bytes from the official release, NOT a publisher signature.
ARCHIVE_SHA256 = '67d015301eef0b612191212d564c5bb0a14b5b9c4796b76454276a4d28d9b288'
DATA = bytes(range(256)) * 32768
DATA_SHA256 = hashlib.sha256(DATA).hexdigest()
MAX_CAPTURE = 65536
BASE = [
    '--no-conf=true', '--no-netrc=true', '--enable-rpc=false',
    '--follow-torrent=false', '--follow-metalink=false',
    '--enable-dht=false', '--enable-dht6=false', '--enable-peer-exchange=false',
    '--bt-enable-lpd=false', '--all-proxy=', '--http-proxy=', '--https-proxy=',
    '--ftp-proxy=', '--split=1', '--max-connection-per-server=1',
    '--max-concurrent-downloads=1', '--file-allocation=none',
    '--auto-file-renaming=false', '--allow-overwrite=false',
    '--always-resume=true', '--continue=true', '--auto-save-interval=1',
    '--max-tries=3', '--retry-wait=1', '--connect-timeout=3', '--timeout=5',
    '--human-readable=false', '--enable-color=false',
    '--truncate-console-readout=false', '--summary-interval=1',
    '--console-log-level=warn', '--check-certificate=true',
]


def child_environment(root: Path) -> dict[str, str]:
    env = {key: value for key, value in os.environ.items()
           if key.upper() in {'SYSTEMROOT', 'WINDIR', 'PATH', 'COMSPEC'}}
    env.update(HOME=str(root), USERPROFILE=str(root), TEMP=str(root), TMP=str(root))
    return env


def hidden_kwargs() -> dict:
    return {'creationflags': subprocess.CREATE_NO_WINDOW} if os.name == 'nt' else {}


def console_summary(raw: bytes) -> dict:
    # Allowlisted numeric fields only: do not publish raw output, paths, URLs or argv.
    progress = re.findall(rb'\[#([0-9a-fA-F]+)\s+(\d+)B/(\d+)B\((\d+)%\)', raw)
    return {'captured_bytes': len(raw), 'cr': raw.count(b'\r'), 'lf': raw.count(b'\n'),
            'crlf': raw.count(b'\r\n'),
            'progress': [{'done': int(d), 'total': int(t), 'percent': int(p)}
                         for _, d, t, p in progress[:32]]}


class Capture:
    def __init__(self, pipe):
        self.raw = bytearray()
        self.total = 0
        self.thread = threading.Thread(target=self.drain, args=(pipe,), daemon=True)
        self.thread.start()

    def drain(self, pipe):
        try:
            while chunk := pipe.read(4096):
                self.total += len(chunk)
                self.raw.extend(chunk[:max(0, MAX_CAPTURE - len(self.raw))])
        finally:
            pipe.close()

    def finish(self):
        self.thread.join(5)
        if self.thread.is_alive():
            raise RuntimeError('pipe_drain_timeout')
        return {**console_summary(bytes(self.raw)), 'total_bytes': self.total,
                'truncated': self.total > MAX_CAPTURE}


class Fixture(http.server.ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self):
        super().__init__(('127.0.0.1', 0), Handler)
        self.requests = []
        self.sent = 0
        self.drop = False
        self.dropped = False
        self.delay = .025
        self.lock = threading.Lock()


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'

    def log_message(self, *_):
        pass

    def do_GET(self):
        match = re.fullmatch(r'bytes=(\d+)-(\d*)', self.headers.get('Range', ''))
        start = int(match[1]) if match else 0
        end = int(match[2]) if match and match[2] else len(DATA) - 1
        if start > end or end >= len(DATA):
            self.send_error(416)
            return
        status = 206 if match else 200
        with self.server.lock:
            self.server.requests.append({'status': status, 'start': start, 'end': end})
            drop = self.server.drop and not self.server.dropped
            if drop:
                self.server.dropped = True
        self.send_response(status)
        self.send_header('Content-Type', 'application/octet-stream')
        self.send_header('Content-Length', str(end - start + 1))
        self.send_header('Accept-Ranges', 'bytes')
        if match:
            self.send_header('Content-Range', f'bytes {start}-{end}/{len(DATA)}')
        self.end_headers()
        limit = min(end + 1, start + 2 * 1024 * 1024) if drop else end + 1
        try:
            for offset in range(start, limit, 65536):
                chunk = DATA[offset:min(offset + 65536, limit)]
                self.wfile.write(chunk)
                self.wfile.flush()
                with self.server.lock:
                    self.server.sent += len(chunk)
                time.sleep(self.server.delay)
            if drop:
                self.connection.shutdown(socket.SHUT_RDWR)
                self.connection.close()
                self.close_connection = True
        except (OSError, ConnectionError):
            pass


def command(exe, root, port, parent=None):
    args = [str(exe), *BASE, f'--dir={root}', '--out=payload.part',
            f'--checksum=sha-256={DATA_SHA256}']
    if parent is not None:
        args.append(f'--stop-with-process={parent}')
    return [*args, f'http://127.0.0.1:{port}/fixture']


def verify_file(root):
    path = root / 'payload.part'
    return path.is_file() and path.stat().st_size == len(DATA) and hashlib.sha256(path.read_bytes()).hexdigest() == DATA_SHA256


def cleanup(root):
    for name in ('payload.part', 'payload.part.aria2'):
        (root / name).unlink(missing_ok=True)
    return not any(root.iterdir())


def visible_windows(pid):
    found = []
    callback_type = ctypes.WINFUNCTYPE(ctypes.c_bool, ctypes.c_void_p, ctypes.c_void_p)
    user32 = ctypes.WinDLL('user32', use_last_error=True)
    @callback_type
    def callback(hwnd, _):
        process_id = ctypes.c_ulong()
        user32.GetWindowThreadProcessId(ctypes.c_void_p(hwnd), ctypes.byref(process_id))
        if process_id.value == pid and user32.IsWindowVisible(ctypes.c_void_p(hwnd)):
            found.append(1)
        return True
    if not user32.EnumWindows(callback, 0):
        raise RuntimeError('window_enumeration_failed')
    return len(found)


def wait_started(server, process):
    deadline = time.monotonic() + 12
    while server.sent < 1024 * 1024:
        if process.poll() is not None or time.monotonic() > deadline:
            raise RuntimeError('fixture_start_timeout')
        time.sleep(.05)


def run_case(exe, root, kind):
    root.mkdir()
    server = Fixture()
    server.drop = kind == 'resume'
    server.delay = .25 if kind == 'parent_exit' else (.1 if kind == 'cancel' else .025)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    process = None
    capture = None
    child_handle = None
    result = {'case': kind}
    try:
        if kind == 'parent_exit':
            # Real supervisor owns aria2. Outer probe retains an OS process handle,
            # so PID reuse cannot masquerade as successful child reclamation.
            process = subprocess.Popen([sys.executable, str(Path(__file__).resolve()),
                '--supervise', str(exe), str(root), str(server.server_port)],
                stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                env=child_environment(root), **hidden_kwargs())
            # Supervisor emits only PID before sleeping; bounded via reader thread.
            pid_lines = []
            reader = threading.Thread(target=lambda: pid_lines.append(process.stdout.readline()), daemon=True)
            reader.start()
            reader.join(10)
            if reader.is_alive() or not pid_lines or not pid_lines[0].strip().isdigit():
                raise RuntimeError('supervisor_start_timeout')
            pid = int(pid_lines[0])
            kernel = ctypes.WinDLL('kernel32', use_last_error=True)
            kernel.OpenProcess.restype = ctypes.c_void_p
            child_handle = kernel.OpenProcess(0x00100000 | 0x0001, False, pid)
            if not child_handle:
                raise RuntimeError('child_handle_unavailable')
            wait_started(server, process)
            result['visible_windows_at_sample'] = visible_windows(pid)
            process.kill()
            process.wait(timeout=5)
            waited = kernel.WaitForSingleObject(ctypes.c_void_p(child_handle), 15000)
            result.update(parent_killed_and_waited=True, child_exited_without_wrapper_kill=waited == 0)
            result['transfer_incomplete_at_child_exit'] = not verify_file(root)
            result['passed'] = (waited == 0 and result['visible_windows_at_sample'] == 0
                                and result['transfer_incomplete_at_child_exit'])
        else:
            process = subprocess.Popen(command(exe, root, server.server_port),
                stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                env=child_environment(root), **hidden_kwargs())
            capture = Capture(process.stdout)
            wait_started(server, process)
            result['visible_windows_at_sample'] = visible_windows(process.pid)
            if kind == 'cancel':
                process.kill()
                result['killed_and_waited'] = True
            code = process.wait(timeout=45)
            result['exit_code'] = code
            result['console'] = capture.finish()
            result['payload_present_before_cleanup'] = (root / 'payload.part').exists()
            result['control_present_before_cleanup'] = (root / 'payload.part.aria2').exists()
            if kind == 'cancel':
                result['passed'] = code != 0 and result['payload_present_before_cleanup']
            else:
                result['full_size_and_sha256_match'] = verify_file(root)
                result['passed'] = code == 0 and result['full_size_and_sha256_match']
                if kind == 'complete':
                    result['passed'] &= bool(result['console']['progress'])
                if kind == 'resume':
                    result['passed'] &= server.dropped and any(r['status'] == 206 and r['start'] > 0 for r in server.requests)
            result['passed'] &= result['visible_windows_at_sample'] == 0
    finally:
        if process is not None:
            if process.poll() is None:
                process.kill()
            process.wait(timeout=5)
            if capture:
                capture.finish()
            elif process.stdout:
                process.stdout.close()
        if child_handle:
            if kernel.WaitForSingleObject(ctypes.c_void_p(child_handle), 0) != 0:
                kernel.TerminateProcess(ctypes.c_void_p(child_handle), 99)
                if kernel.WaitForSingleObject(ctypes.c_void_p(child_handle), 5000) != 0:
                    raise RuntimeError('child_cleanup_timeout')
            kernel.CloseHandle(ctypes.c_void_p(child_handle))
        server.shutdown()
        server.server_close()
        thread.join(5)
        result['requests'] = list(server.requests)
        result['wrapper_cleanup_complete'] = cleanup(root)
    result['passed'] &= result['wrapper_cleanup_complete']
    return result


def fetch(url, limit):
    request = urllib.request.Request(url, headers={'User-Agent': 'Nexa-aria2-observation', 'Accept': 'application/vnd.github+json' if url == RELEASE_API else 'application/octet-stream'})
    with urllib.request.urlopen(request, timeout=30) as response:
        body = response.read(limit + 1)
    if len(body) > limit:
        raise RuntimeError('download_size_limit')
    return body


def acquire(root):
    metadata = json.loads(fetch(RELEASE_API, 2 * 1024 * 1024))
    assets = [a for a in metadata['assets'] if a['name'] == ASSET]
    if len(assets) != 1 or assets[0]['browser_download_url'] != ARCHIVE_URL:
        raise RuntimeError('release_identity_mismatch')
    archive = fetch(ARCHIVE_URL, 16 * 1024 * 1024)
    digest = hashlib.sha256(archive).hexdigest()
    if digest != ARCHIVE_SHA256:
        raise RuntimeError('archive_pin_mismatch')
    upstream = assets[0].get('digest')
    if upstream is not None and upstream != f'sha256:{digest}':
        raise RuntimeError('publisher_digest_mismatch')
    path = root / 'aria2.zip'
    path.write_bytes(archive)
    with zipfile.ZipFile(path) as zf:
        names = [n for n in zf.namelist() if n.endswith('/aria2c.exe')]
        if len(names) != 1 or zf.getinfo(names[0]).file_size > 32 * 1024 * 1024:
            raise RuntimeError('executable_identity_mismatch')
        exe = root / 'aria2c.exe'
        exe.write_bytes(zf.read(names[0]))
    return exe, {'version_pin': VERSION, 'archive_sha256': digest,
                 'exe_sha256': hashlib.sha256(exe.read_bytes()).hexdigest(),
                 'publisher_asset_digest': upstream,
                 'checksum_basis': 'publisher_asset_digest_and_observed_pin' if upstream else 'observed_official_download_pin_not_publisher_signature'}


def version_info(exe, root):
    proc = subprocess.run([str(exe), '--no-conf=true', '--no-netrc=true', '--version'],
        stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=10,
        env=child_environment(root), **hidden_kwargs())
    text = proc.stdout.decode('ascii', errors='replace')
    if proc.returncode or f'aria2 version {VERSION}' not in text:
        raise RuntimeError('version_mismatch')
    fields = {}
    for prefix in ('aria2 version ', 'Enabled Features:', 'Hash Algorithms:', 'Libraries:', 'Compiler:'):
        for line in text.splitlines():
            if line.startswith(prefix):
                fields[prefix.rstrip(': ')] = re.sub(r'[^A-Za-z0-9 .,+:()/_=-]', '?', line[len(prefix):])[:512]
    fields['tls_backend_reported_only'] = True
    return fields


def main():
    if len(sys.argv) > 1 and sys.argv[1] == '--supervise':
        exe, root, port = sys.argv[2:]
        proc = subprocess.Popen(command(exe, root, int(port), os.getpid()),
            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            env=child_environment(Path(root)), **hidden_kwargs())
        print(proc.pid, flush=True)
        time.sleep(90)
        proc.kill()
        proc.wait(timeout=5)
        return 1
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    report = {'schema': 1, 'platform': 'windows' if os.name == 'nt' else 'unsupported',
              'fixture_bytes': len(DATA), 'fixture_sha256': DATA_SHA256,
              'production_network_policy_tested': False, 'public_https_transfer_tested': False,
              'modelscope_or_hf_tested': False, 'native_window_interactive_tested': False,
              'cases': [], 'passed': False}
    commit = os.environ.get('GITHUB_SHA', '')
    if re.fullmatch(r'[0-9a-f]{40}', commit):
        report['source_commit'] = commit
    try:
        if os.name != 'nt':
            raise RuntimeError('windows_required')
        with tempfile.TemporaryDirectory(prefix='nexa-aria2-observe-') as folder:
            root = Path(folder)
            exe, report['identity'] = acquire(root)
            report['build'] = version_info(exe, root)
            for kind in ('complete', 'resume', 'cancel', 'parent_exit'):
                report['cases'].append(run_case(exe, root / kind, kind))
            report['passed'] = all(c['passed'] for c in report['cases'])
    except Exception as exc:
        # Exceptions can contain URLs, command lines and local paths: class only.
        report['error_class'] = type(exc).__name__
        if isinstance(exc, RuntimeError) and re.fullmatch(r'[a-z_]{1,64}', str(exc)):
            report['error_code'] = str(exc)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
    print(json.dumps({'passed': report['passed'], 'cases_completed': len(report['cases'])}))
    return 0 if report['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
