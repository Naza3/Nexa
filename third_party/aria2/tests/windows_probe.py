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
import time

CERT_ERRORS = {"80090325", "80090322", "80090328", "800b0109", "800b010f", "800b0101"}
PRIVATE_URLS = {
    "private_literal": "https://127.0.0.1/",
    "private_mapped": "https://[::ffff:127.0.0.1]/",
}
NUMERIC_ALIAS_URLS = {
    "private_integer": "https://2130706433/",
    "private_hex": "https://0x7f000001/",
    "private_octal": "https://0177.0.0.1/",
}
INVALID_URI_URLS = {
    "initial_http": "http://127.0.0.1/",
    "initial_port": "https://127.0.0.1:444/",
    "initial_userinfo": "https://u@127.0.0.1/",
    "initial_fragment": "https://127.0.0.1/#x",
}
CERTIFICATE_CASES = {
    "wrong_hostname": ("https://wrong.host.badssl.com/", {"80090322", "800b010f"}),
    "untrusted_certificate": ("https://self-signed.badssl.com/", {"80090325", "800b0109"}),
    "expired_certificate": ("https://expired.badssl.com/", {"80090328", "800b0101"}),
}
PUBLIC_HTTPS_URL = "https://example.com/"
RETRY_DELAYS = (2, 5)


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


def download_command(binary: Path, directory: Path, url: str, expectation: str, extra: tuple = ()) -> list[str]:
    # Request::parseUri failures leave no Request. Their exception is emitted by
    # AbstractCommand at DEBUG, unlike errors for an already-created request.
    level = "debug" if expectation == "invalid_uri" else "warn"
    return [str(binary), "--no-conf", "--no-netrc=true", "--check-certificate=true",
            "--enable-rpc=false", "--max-tries=1", "--connect-timeout=15", "--timeout=20",
            "--split=1", "--max-connection-per-server=1", "--file-allocation=none",
            "--auto-file-renaming=false", "--allow-overwrite=false", "--summary-interval=0",
            "--download-result=hide", "--console-log-level=" + level, "--max-download-limit=1M",
            "--dir=" + str(directory), "--out=payload", *extra, url]


def classify_download(name: str, url: str, expectation: str, code: int, text: str, size: int) -> dict:
    """Classify one fixed fixture, never turn an arbitrary failure into a pass.

    Resolver rejection is only evidence that this exact alias stopped before
    socket creation in this run. It is NOT evidence of the destination gate or
    a promise that other Windows resolver configurations reject the alias.
    """
    text = re.sub(r"\x1b\[[0-9;]*m", "", text)
    result = {"passed": False, "evidence_class": "unexpected_result",
              "socket_gate_rejection_observed": False, "matched_evidence": []}
    if expectation == "public_https":
        if name == "public_https" and url == "https://example.com/" and code == 0 and 0 < size <= 1024 * 1024:
            result.update(passed=True, evidence_class="public_https_downloaded")
        return result
    if code == 0 or size != 0:
        return result

    # Bind exception evidence to the one URI actually supplied, including its
    # terminator (a matching prefix of a different URI is insufficient).
    uri_logged = bool(re.search(
        r"^Exception: \[[^\r\n\]]*[/\\]AbstractCommand\.cc:\d+\] errorCode=1 URI="
        + re.escape(url) + r"\r?$", text, re.M))
    if expectation in {"private_socket", "numeric_alias"}:
        fixtures = PRIVATE_URLS if expectation == "private_socket" else NUMERIC_ALIAS_URLS
        if fixtures.get(name) != url or not uri_logged:
            return result
        if re.search(r"^  -> \[[^\r\n\]]*[/\\]SocketCore\.cc:\d+\] errorCode=1 Nexa policy: destination rejected\r?$", text, re.M):
            result.update(passed=True, evidence_class="destination_policy_rejected", socket_gate_rejection_observed=True)
        elif expectation == "numeric_alias":
            host = url.removeprefix("https://").removesuffix("/")
            # This pinned Winsock diagnostic was observed in the original CI
            # report. Do not accept DNS timeouts, other hosts, generic errors,
            # certificate failures, or the same words from another source.
            marker = (r"^  -> \[[^\r\n\]]*[/\\]SocketCore\.cc:\d+\] errorCode=1 "
                      r"Failed to resolve the hostname " + re.escape(host) + r", cause: Unknown error\r?$")
            errors = re.findall(r"^.*errorCode=.*$", text, re.M)
            if len(errors) == 2 and re.search(marker, text, re.M) and not re.search(
                    r"timeout|timed out|SSL/TLS|certificate|connection refused", text, re.I):
                result.update(passed=True, evidence_class="resolver_rejected_before_socket")
    elif expectation == "invalid_limit":
        if "Nexa policy: invalid payload byte limit" in text:
            result.update(passed=True, evidence_class="payload_limit_rejected")
    elif expectation == "unsupported_option":
        if "Nexa policy: unsupported credential, proxy, RPC or TLS option" in text:
            result.update(passed=True, evidence_class="unsupported_option_rejected")
    elif expectation == "invalid_uri" and INVALID_URI_URLS.get(name) == url:
        selected = re.findall(r"\[DEBUG\] FeedbackURISelector selected ([^\r\n]*)", text)
        marker = (r"^Exception: \[[^\r\n\]]*[/\\]CreateRequestCommand\.cc:\d+\] "
                  r"errorCode=-1 No URI available\.\r?$")
        errors = re.findall(r"^.*errorCode=.*$", text, re.M)
        # No-URI can also follow exhausted network attempts. Require the fixed
        # parser path with no competing error or network-attempt diagnostics.
        network = (r"\[ERROR\]|SocketCore\.cc:|Connecting to |Resolving hostname |"
                   r"Name resolution |DNS cache hit:|Sending request:|SSL/TLS|"
                   r"Failed to resolve|timeout|timed out|connection refused|certificate error")
        if (selected == [url] and re.search(marker, text, re.M)
                and len(errors) == 1 and not re.search(network, text, re.I)):
            result.update(passed=True, evidence_class="request_uri_rejected_before_socket")
    elif expectation == "certificate" and name in CERTIFICATE_CASES:
        expected_url, expected_errors = CERTIFICATE_CASES[name]
        if (url == expected_url and uri_logged and re.search(
                r"^  -> \[[^\r\n\]]*[/\\]SocketCore\.cc:\d+\] errorCode=1 SSL/TLS handshake failure:", text, re.M)
                and certificate_rejected(code, text)
                and any(re.search(r"\(" + e + r"\)", text.lower()) for e in expected_errors)):
            result.update(passed=True, evidence_class="schannel_certificate_rejected")
    if result["passed"]:
        # Preserve classifier inputs independently of the bounded diagnostic
        # tail, so verbose DEBUG output cannot hide the asserted evidence.
        result["matched_evidence"] = [line for line in text.splitlines()
                                      if "errorCode=" in line or "FeedbackURISelector selected " in line
                                      or any("(" + e + ")" in line.lower() for e in CERT_ERRORS)]
    return result


def transient_transport_failure(name: str, url: str, expectation: str,
                                code: int, text: str, size: int) -> str | None:
    """Recognize only bounded, observed transport failures of fixed public fixtures.

    This authorizes another attempt, never a passing certificate result. An
    unexpected certificate error, ambiguous error chain or received payload
    must remain a hard failure even if a later request might succeed.
    """
    fixed_public = name == "public_https" and url == PUBLIC_HTTPS_URL and expectation == "public_https"
    fixed_certificate = (expectation == "certificate" and name in CERTIFICATE_CASES
                         and url == CERTIFICATE_CASES[name][0])
    if not (fixed_public or fixed_certificate) or code not in (1, 2) or size != 0:
        return None
    text = re.sub(r"\x1b\[[0-9;]*m", "", text)
    # A Schannel/HRESULT diagnostic takes precedence, including unexpected
    # certificate/revocation errors outside this fixture's accepted codes.
    if certificate_rejected(code, text) or re.search(
            r"\([0-9a-f]{8}\)|Nexa policy:|certificate|revocation", text, re.I):
        return None
    aborted = re.findall(r"\[ERROR\] CUID#\d+ - Download aborted\. URI=([^\r\n]+)", text)
    if aborted != [url] or text.count("[ERROR]") != 1:
        return None
    if any(found != url for found in re.findall(r"\bURI=([^\r\n]+)", text)):
        return None
    errors = re.findall(r"^.*errorCode=.*$", text, re.M)
    abort_line = r"\[ERROR\] CUID#\d+ - Download aborted\. URI=" + re.escape(url) + r"\r?\n"
    if code == 2 and len(errors) == 1 and re.fullmatch(
            r"Exception: \[[^\r\n\]]*[/\\]AbstractCommand\.cc:\d+\] errorCode=2 Timeout\.\r?", errors[0]):
        if re.search(abort_line + re.escape(errors[0].rstrip("\r")) + r"\r?$", text, re.M):
            return "network_timeout"
        return None
    if code != 1 or len(errors) != 2 or not re.fullmatch(
            r"Exception: \[[^\r\n\]]*[/\\]AbstractCommand\.cc:\d+\] errorCode=1 URI="
            + re.escape(url) + r"\r?", errors[0]):
        return None
    socket_error = r"  -> \[[^\r\n\]]*[/\\]SocketCore\.cc:\d+\] errorCode=1 SSL/TLS handshake failure:[^\r\n]*"
    if not re.fullmatch(socket_error + r"\r?", errors[1]):
        return None
    numeric_errors = re.findall(r"(?mi)^\(([0-9a-f]+)\)\r?$", text)
    if len(numeric_errors) != 1 or not re.search(
            abort_line + re.escape(errors[0].rstrip("\r")) + r"\r?\n"
            + socket_error + r"\r?\n\(" + re.escape(numeric_errors[0]) + r"\)\r?$", text, re.M):
        return None
    return {"2746": "connection_reset", "2745": "connection_aborted",
            "274c": "network_timeout"}.get(numeric_errors[0].lower())


def run_download_case(binary: Path, root: Path, name: str, url: str, expectation: str,
                      extra: tuple = (), payload_limit: str | None = "1048576") -> dict:
    attempts = []
    failure_kind = None
    for number in range(1, len(RETRY_DELAYS) + 2):
        directory = root / f"attempt-{number}" / name
        directory.mkdir(parents=True)
        argv = download_command(binary, directory, url, expectation, extra)
        process_timeout = False
        try:
            completed = subprocess.run(argv, capture_output=True, timeout=60, env=clean_env(payload_limit))
            code = completed.returncode
            raw = completed.stdout + completed.stderr
        except subprocess.TimeoutExpired as error:
            # subprocess.run has killed and reaped the timed-out child. Preserve
            # this attempt and previous evidence, but do not retry a hung child.
            process_timeout = True
            code = None
            raw = (error.output or b"") + (error.stderr or b"")
        text = raw.decode("utf-8", "replace")
        payload = directory / "payload"
        size = payload.stat().st_size if payload.exists() else 0
        result = (classify_download(name, url, expectation, code, text, size) if not process_timeout else
                  {"passed": False, "evidence_class": "unexpected_result",
                   "socket_gate_rejection_observed": False, "matched_evidence": []})
        reason = None if result["passed"] or process_timeout else transient_transport_failure(
            name, url, expectation, code, text, size)
        attempt = {"attempt": number, **result, "exit": code, "bytes": size,
                   "diagnostic": text[-8192:], "transport_failure": reason}
        attempts.append(attempt)
        if result["passed"]:
            break
        failure_kind = "process_timeout" if process_timeout else "unexpected_result"
        if reason is None:
            break
        failure_kind = "network_retries_exhausted"
        if number > len(RETRY_DELAYS):
            break
        delay = RETRY_DELAYS[number - 1]
        print(f"Policy fixture {name}: attempt {number}/3 failed ({reason}); retrying in {delay}s", flush=True)
        time.sleep(delay)
    last = attempts[-1]
    report = {"name": name, **{key: value for key, value in last.items() if key != "attempt"},
              "url": url, "expectation": expectation, "attempt_count": len(attempts),
              "recovered_after_retry": last["passed"] and len(attempts) > 1, "attempts": attempts}
    if not last["passed"]:
        report["failure_kind"] = failure_kind
        print(f"Policy fixture {name}: FAILED ({failure_kind}; attempts={len(attempts)})", flush=True)
    elif len(attempts) > 1:
        print(f"Policy fixture {name}: passed on attempt {len(attempts)}; earlier failures retained", flush=True)
    return report


def probe(artifacts: Path, output: Path) -> bool:
    report = {"schema_version": 1, "platform": platform.platform(), "windows_runtime_tested": False,
              "target_win10_device_tested": False, "cases": [], "passed": False,
              "limitations": ["No Windows DNS rebind/public-to-private redirect socket instrumentation in this slice",
                              "Native Request tests cover redirect parsing; they do not replace live redirect traces",
                              "Numeric alias resolver failures are reported separately and do not exercise the socket gate",
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
                cases.append(run_download_case(binary, tmp, name, url, expectation, extra, payload_limit))
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
                for name, url in PRIVATE_URLS.items():
                    run(name, url, "private_socket")
                for name, url in NUMERIC_ALIAS_URLS.items():
                    run(name, url, "numeric_alias")
            finally:
                stopped.set()
                thread.join(timeout=2)
                listener.close()
            cases.append({"name": "no_private_connection", "passed": not accepted, "accepted": len(accepted)})
            for name, url in INVALID_URI_URLS.items():
                run(name, url, "invalid_uri")
            for name, option in (("proxy", "--https-proxy=https://127.0.0.1:443"),
                                 ("rpc", "--enable-rpc=true"), ("tls_off", "--check-certificate=false"),
                                 ("netrc", "--no-netrc=false"), ("header", "--header=Authorization: no")):
                run(name, "https://example.com/", "unsupported_option", (option,))
            run("public_https", "https://example.com/", "public_https")
            for name, (url, _) in CERTIFICATE_CASES.items():
                run(name, url, "certificate")
        report["passed"] = bool(cases) and all(c["passed"] for c in cases)
    except Exception as exc:
        report["failure"] = str(exc)
    finally:
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        print(f"Windows component policy: {sum(case['passed'] for case in cases)}/{len(cases)} cases passed; "
              f"overall={'pass' if report['passed'] else 'FAIL'}", flush=True)
    return report["passed"]


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--artifacts", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    raise SystemExit(0 if probe(args.artifacts.resolve(), args.output.resolve()) else 1)
