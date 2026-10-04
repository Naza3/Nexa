#!/usr/bin/env python3
"""Run only the pinned, patched build on Windows; no TLS/CA weakening.

Public fixtures test real Schannel. Availability/TLS failures are failures, never
silently relabeled as a successful negative case. Linux LD_PRELOAD evidence is
not imported into this report.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import socket
import subprocess
import tempfile
import threading

CERT_ERRORS = {"80090325", "80090322", "80090328", "800b0109", "800b010f", "800b0101"}


def sha(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def clean_env(payload_limit: str | None = None) -> dict:
    env = {k: v for k, v in os.environ.items() if "proxy" not in k.lower() and k.upper() not in
           {"LD_PRELOAD", "LD_LIBRARY_PATH", "DYLD_INSERT_LIBRARIES", "ARIA2_CONF_PATH", "NEXA_PAYLOAD_MAX_BYTES"}}
    if payload_limit is not None:
        env["NEXA_PAYLOAD_MAX_BYTES"] = payload_limit
    return env


def certificate_rejected(code: int, text: str) -> bool:
    return code != 0 and any(re.search(r"\b" + e + r"\b", text.lower()) for e in CERT_ERRORS)


def probe(artifacts: Path, output: Path) -> bool:
    report = {"schema_version": 1, "platform": platform.platform(), "windows_runtime_tested": False,
              "target_win10_device_tested": False, "cases": [], "passed": False,
              "limitations": ["No Windows DNS rebind/public-to-private redirect socket instrumentation in this slice",
                              "Native Request tests cover redirect parsing; they do not replace live redirect traces",
                              "OS DNS and Schannel AIA/CRL/OCSP are outside the aria2 download-socket gate",
                              "No model/CDN, resume/cancel/Job or product publication transaction is tested here"]}
    cases = report["cases"]
    try:
        if os.name != "nt":
            raise RuntimeError("This probe requires a real Windows runner")
        manifest = json.loads((artifacts / "build-manifest.json").read_text(encoding="utf-8"))
        lock = json.loads((Path(__file__).resolve().parents[1] / "source-lock.json").read_text(encoding="utf-8"))
        if manifest["source_lock"] != lock:
            raise RuntimeError("Build input lock mismatch")
        report["build_manifest_sha256"] = sha(artifacts / "build-manifest.json")
        report["source_commit"] = manifest["source_commit"]
        if os.environ.get("GITHUB_SHA") and manifest["source_commit"] != os.environ["GITHUB_SHA"]:
            raise RuntimeError("Build and test checkout commits differ")
        for name in ("nexa-aria2.exe", "policy_unit.exe", "engine_unit.exe", "payload_unit.exe", "config.h", "pe.txt"):
            if sha(artifacts / name) != manifest["files"][name]["sha256"]:
                raise RuntimeError("Executable hash mismatch: " + name)
        config = (artifacts / "config.h").read_text(encoding="utf-8")
        pe_text = (artifacts / "pe.txt").read_text(encoding="utf-8")
        if (not re.search(r"^#define SECURITY_WIN32 1$", config, re.M) or
                not re.search(r"^#define ENABLE_SSL 1$", config, re.M) or
                "secur32.dll" not in {s.lower() for s in manifest["pe"]["imports"]} or
                not re.search(r"Name:\s+secur32\.dll", pe_text, re.I)):
            raise RuntimeError("Missing authenticated Schannel configuration/import evidence")
        report["tls_backend_evidence"] = "hashed config SECURITY_WIN32+ENABLE_SSL, hashed PE secur32 import; real certificate cases below"
        binary = artifacts / "nexa-aria2.exe"
        report["binary_sha256"] = sha(binary)
        for name, expected in (("policy_unit.exe", "68 policy unit cases passed"),
                               ("engine_unit.exe", "26 Request parser/redirect cases passed")):
            r = subprocess.run([str(artifacts / name)], capture_output=True, timeout=30, env=clean_env())
            text = (r.stdout + r.stderr).decode("utf-8", "replace")
            good = r.returncode == 0 and expected in text
            if name == "engine_unit.exe":
                good = good and "4 actual SocketCore private destination cases passed" in text
            cases.append({"name": name, "passed": good, "exit": r.returncode, "output": text[:2048]})
            if not good:
                raise RuntimeError("Native policy/engine test failed")
        r = subprocess.run([str(binary), "--version"], capture_output=True, timeout=15, env=clean_env())
        version = (r.stdout + r.stderr).decode("utf-8", "replace")
        report["version_output"] = version[:8192]
        good = r.returncode == 0 and "aria2 version 1.37.0" in version and "HTTPS" in version
        good = good and not any(s in version for s in ("BitTorrent", "Metalink", "Async DNS", "SFTP", "OpenSSL", "GnuTLS"))
        cases.append({"name": "version_features", "passed": good, "exit": r.returncode})
        if not good:
            raise RuntimeError("Unexpected runtime version/features")
        report["windows_runtime_tested"] = True
        with tempfile.TemporaryDirectory(prefix="nexa-policy-") as tmp:
            tmp = Path(tmp)
            r = subprocess.run([str(artifacts / "payload_unit.exe"), str(tmp / "unit-payload")],
                               capture_output=True, timeout=30, env=clean_env())
            text = (r.stdout + r.stderr).decode("utf-8", "replace")
            good = r.returncode == 0 and "53 payload/IOFile cases passed" in text
            cases.append({"name": "payload_unit.exe", "passed": good, "exit": r.returncode, "output": text[:2048]})
            if not good:
                raise RuntimeError("Native payload/IOFile test failed")
            def run(name: str, url: str, expectation: str, extra: tuple = (), payload_limit: str | None = "1048576") -> None:
                directory = tmp / name
                directory.mkdir()
                argv = [str(binary), "--no-conf", "--no-netrc=true", "--check-certificate=true",
                        "--enable-rpc=false", "--max-tries=1", "--connect-timeout=15", "--timeout=20",
                        "--split=1", "--max-connection-per-server=1", "--file-allocation=none",
                        "--auto-file-renaming=false", "--allow-overwrite=false", "--summary-interval=0",
                        "--download-result=hide", "--console-log-level=warn", "--max-download-limit=1M",
                        "--dir=" + str(directory), "--out=payload", *extra, url]
                r = subprocess.run(argv, capture_output=True, timeout=60, env=clean_env(payload_limit))
                text = (r.stdout + r.stderr).decode("utf-8", "replace")
                payload = directory / "payload"
                size = payload.stat().st_size if payload.exists() else 0
                if expectation == "public_https":
                    good = r.returncode == 0 and 0 < size <= 1024 * 1024
                elif expectation == "private_socket":
                    good = r.returncode != 0 and "Nexa policy: destination rejected" in text and size == 0
                elif expectation == "invalid_limit":
                    good = r.returncode != 0 and "Nexa policy: invalid payload byte limit" in text and size == 0
                elif expectation == "unsupported_option":
                    good = r.returncode != 0 and "Nexa policy: unsupported" in text and size == 0
                elif expectation == "invalid_uri":
                    good = r.returncode != 0 and "Unrecognized URI or unsupported protocol" in text and size == 0
                else:
                    good = certificate_rejected(r.returncode, text) and size == 0
                cases.append({"name": name, "passed": good, "exit": r.returncode, "bytes": size,
                              "expectation": expectation, "diagnostic": text[-4096:]})
            # A real loopback listener detects regressions that actually connect.
            # No firewall rule, privileged port reservation or trusted CA changes.
            accepted = []
            listener = socket.socket()
            listener.bind(("127.0.0.1", 443))
            listener.listen()
            listener.settimeout(0.2)
            stopped = threading.Event()
            def listen() -> None:
                while not stopped.is_set():
                    try:
                        connection, _ = listener.accept()
                        accepted.append(True)
                        connection.close()
                    except socket.timeout:
                        continue
            thread = threading.Thread(target=listen)
            thread.start()
            try:
                for i, limit in enumerate((None, "", "0", "3", "04", "+4", "4 ", "17179869185", "18446744073709551616")):
                    run("invalid_limit_" + str(i), "https://127.0.0.1/", "invalid_limit", payload_limit=limit)
                for name, url in (("private_literal", "https://127.0.0.1/"),
                                  ("private_integer", "https://2130706433/"),
                                  ("private_hex", "https://0x7f000001/"),
                                  ("private_octal", "https://0177.0.0.1/"),
                                  ("private_mapped", "https://[::ffff:127.0.0.1]/")):
                    run(name, url, "private_socket")
            finally:
                stopped.set()
                thread.join(timeout=2)
                listener.close()
            cases.append({"name": "no_private_connection", "passed": not accepted, "accepted": len(accepted)})
            for name, url in (("initial_http", "http://127.0.0.1/"),
                              ("initial_port", "https://127.0.0.1:444/"),
                              ("initial_userinfo", "https://u@127.0.0.1/"),
                              ("initial_fragment", "https://127.0.0.1/#x")):
                run(name, url, "invalid_uri")
            for name, option in (("proxy", "--https-proxy=https://127.0.0.1:443"),
                                 ("rpc", "--enable-rpc=true"), ("tls_off", "--check-certificate=false"),
                                 ("netrc", "--no-netrc=false"), ("header", "--header=Authorization: no")):
                run(name, "https://example.com/", "unsupported_option", (option,))
            run("public_https", "https://example.com/", "public_https")
            run("wrong_hostname", "https://wrong.host.badssl.com/", "certificate")
            run("untrusted_certificate", "https://self-signed.badssl.com/", "certificate")
            run("expired_certificate", "https://expired.badssl.com/", "certificate")
        report["passed"] = bool(cases) and all(c["passed"] for c in cases)
    except Exception as exc:
        report["failure"] = str(exc)
    finally:
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    return report["passed"]


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--artifacts", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    raise SystemExit(0 if probe(args.artifacts.resolve(), args.output.resolve()) else 1)
