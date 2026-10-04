#!/usr/bin/env python3
"""Linux-only deterministic HTTPS cap fixture. Never a Windows evidence claim.

The separate LD_PRELOAD fixture maps accepted public addresses to our local TLS
server and observes actual post-write sizes. It does NOT implement the cap.
"""
import argparse
import hashlib
import http.server
import json
import os
from pathlib import Path
import ssl
import subprocess
import threading

DATA = b"0123456789abcdef" * 4


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    requests = []

    def log_message(self, *_args):
        pass

    def do_GET(self):
        self.requests.append(self.path)
        path = self.path
        if path in ("/chunked", "/chunked_over"):
            self.send_response(200)
            self.send_header("Transfer-Encoding", "chunked")
            self.send_header("Connection", "close")
            self.end_headers()
            chunks = [DATA[:32], DATA[32:]] if path == "/chunked" else [DATA[:32], DATA * 2]
            try:
                for chunk in chunks:
                    self.wfile.write(("%x\r\n" % len(chunk)).encode() + chunk + b"\r\n")
                    self.wfile.flush()
                self.wfile.write(b"0\r\n\r\n")
            except (OSError, ssl.SSLError):
                pass
            return
        data = DATA * 2 if path in ("/length_over", "/range_over") else DATA
        start = int(self.headers.get("Range", "bytes=0-").split("=")[1].split("-")[0])
        if self.headers.get("Range"):
            self.send_response(206)
            self.send_header("Content-Range", f"bytes {start}-{len(data)-1}/{len(data)}")
        else:
            self.send_response(200)
        self.send_header("Content-Length", str(len(data) - start))
        self.send_header("Connection", "close")
        self.end_headers()
        try:
            self.wfile.write(data[start:])
        except (OSError, ssl.SSLError):
            pass


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--hook", type=Path, required=True)
    parser.add_argument("--work", type=Path, required=True)
    args = parser.parse_args()
    work = args.work.resolve()
    work.mkdir(parents=True, exist_ok=True)
    cert, key = work / "test.crt", work / "test.key"
    subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", str(key),
                    "-out", str(cert), "-days", "1", "-subj", "/CN=public.test", "-addext",
                    "subjectAltName=DNS:public.test"], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(cert, key)
    server.socket = context.wrap_socket(server.socket, server_side=True)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    results = []
    def run(name, path, success=False, seed=b"", allocation="none", limit="64"):
        directory = work / name
        directory.mkdir()
        payload = directory / "payload"
        payload.write_bytes(seed)
        trace = directory / "io.log"
        trace.write_text("", encoding="utf-8")
        env = {k: v for k, v in os.environ.items() if "proxy" not in k.lower() and k != "NEXA_PAYLOAD_MAX_BYTES"}
        env.update(LD_PRELOAD=str(args.hook.resolve()), NEXA_TEST_LOG=str(trace), NEXA_TEST_PORT=str(server.server_port))
        if limit is not None:
            env["NEXA_PAYLOAD_MAX_BYTES"] = limit
        argv = [str(args.binary.resolve()), "--no-conf", "--no-netrc=true", "--check-certificate=true",
                "--enable-rpc=false", "--max-tries=1", "--connect-timeout=3", "--timeout=3",
                "--split=1", "--max-connection-per-server=1", "--file-allocation=" + allocation,
                "--allow-overwrite=true", "--auto-file-renaming=false", "--continue=true", "--disk-cache=0",
                "--enable-mmap=false", "--dir=" + str(directory), "--out=payload",
                "--ca-certificate=" + str(cert), "https://public.test" + path]
        before = len(Handler.requests)
        result = subprocess.run(argv, env=env, capture_output=True, timeout=15)
        text = (result.stdout + result.stderr).decode("utf-8", "replace")
        events = trace.read_text(encoding="utf-8").splitlines()
        sizes = [int(e.split()[1]) for e in events if e.startswith("PAYLOAD_SIZE ")]
        sizes += [len(seed), payload.stat().st_size]
        passed = (result.returncode == 0) == success and max(sizes) <= 64
        if success:
            passed = passed and payload.read_bytes() == DATA and bool([e for e in events if e.startswith("PAYLOAD_SIZE ")])
        elif limit not in (None, "3", "04", "18446744073709551616"):
            passed = passed and "Nexa policy: payload byte limit exceeded" in text
        else:
            passed = passed and "Nexa policy: invalid payload byte limit" in text and not events and len(Handler.requests) == before
        results.append({"case": name, "passed": passed, "exit": result.returncode,
                        "max_observed_payload_bytes": max(sizes), "final_payload_bytes": payload.stat().st_size,
                        "events": events, "requests": len(Handler.requests) - before,
                        "diagnostic": text[-3000:].replace(str(work), "<fixture>")})
    try:
        run("boundary", "/data", True)
        run("chunked_boundary", "/chunked", True)
        run("range_boundary", "/data", True, seed=DATA[:32])
        run("length_over", "/length_over")
        run("chunked_over", "/chunked_over")
        run("range_over", "/range_over", seed=DATA[:32])
        run("preallocate_trunc_over", "/length_over", allocation="trunc")
        run("preallocate_falloc_over", "/length_over", allocation="falloc")
        for i, limit in enumerate((None, "3", "04", "18446744073709551616")):
            run("invalid_limit_" + str(i), "/data", limit=limit)
    finally:
        server.shutdown()
    (work / "results.json").write_text(json.dumps({"platform": "linux-fixture-only", "cases": results,
                                                  "passed": all(r["passed"] for r in results)}, indent=2) + "\n", encoding="utf-8")
    for r in results:
        print(r["case"], r["passed"], r["exit"], r["max_observed_payload_bytes"])
    return 0 if all(r["passed"] for r in results) else 1


if __name__ == "__main__":
    raise SystemExit(main())
