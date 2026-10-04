"""Offline classifier/orchestration regressions, not Windows/Winsock evidence."""
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location(
    "nexa_windows_policy_probe", ROOT / "third_party/aria2/tests/windows_probe.py")
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)


def exception_log(url, detail):
    return (f"[ERROR] CUID#7 - Download aborted. URI={url}\n"
            f"Exception: [/build/src/AbstractCommand.cc:351] errorCode=1 URI={url}\n"
            f"  -> [/build/src/SocketCore.cc:438] errorCode=1 {detail}\n")


def resolver_log(url):
    host = url.removeprefix("https://").removesuffix("/")
    return exception_log(url, f"Failed to resolve the hostname {host}, cause: Unknown error")


def parser_log(url):
    return (f"[DEBUG] FeedbackURISelector selected {url}\n"
            "[DEBUG] Exception caught\n"
            "Exception: [/build/src/CreateRequestCommand.cc:107] errorCode=-1 No URI available.\n")


def certificate_log(url, code):
    return exception_log(url, f"SSL/TLS handshake failure: Error: certificate rejected.\n({code})")


class ClassificationTests(unittest.TestCase):
    def check(self, name, url, expectation, text, code=1, size=0):
        return probe.classify_download(name, url, expectation, code, text, size)

    def test_each_numeric_alias_resolver_is_separate_from_socket_gate(self):
        for name, url in probe.NUMERIC_ALIAS_URLS.items():
            with self.subTest(name=name):
                result = self.check(name, url, "numeric_alias", resolver_log(url))
                self.assertTrue(result["passed"])
                self.assertEqual(result["evidence_class"], "resolver_rejected_before_socket")
                self.assertFalse(result["socket_gate_rejection_observed"])

    def test_each_numeric_alias_can_instead_reach_policy_gate(self):
        for name, url in probe.NUMERIC_ALIAS_URLS.items():
            with self.subTest(name=name):
                result = self.check(name, url, "numeric_alias", exception_log(url, "Nexa policy: destination rejected"))
                self.assertTrue(result["passed"])
                self.assertEqual(result["evidence_class"], "destination_policy_rejected")
                self.assertTrue(result["socket_gate_rejection_observed"])

    def test_canonical_and_mapped_require_actual_policy_marker(self):
        for name, url in probe.PRIVATE_URLS.items():
            with self.subTest(name=name):
                self.assertFalse(self.check(name, url, "private_socket", resolver_log(url))["passed"])
                self.assertTrue(self.check(name, url, "private_socket", exception_log(url, "Nexa policy: destination rejected"))["passed"])

    def test_alias_exception_is_bound_to_fixed_name_url_host_and_source(self):
        name, url = "private_integer", probe.NUMERIC_ALIAS_URLS["private_integer"]
        original = resolver_log(url)
        variations = [
            original.replace("hostname 2130706433,", "hostname example.com,"),
            original.replace("hostname 2130706433,", "hostname 21307064330,"),
            original.replace("hostname 2130706433,", "hostname 2130706433.evil,"),
            original.replace("SocketCore.cc:", "OtherSource.cc:"),
            original.replace("AbstractCommand.cc:", "OtherSource.cc:"),
            original.replace("URI=" + url, "URI=https://example.com/"),
            original.replace("URI=" + url, "URI=https://example.com/") + "URI=" + url + "\n",
            original.replace("URI=" + url, "URI=" + url + "suffix"),
            original.replace("cause: Unknown error", "cause: DNS timeout"),
            original.replace("cause: Unknown error", "cause: Unknown error extra"),
            "Failed to resolve the hostname 2130706433, cause: Unknown error",
        ]
        for text in variations:
            with self.subTest(text=text):
                self.assertFalse(self.check(name, url, "numeric_alias", text)["passed"])
        for bad_name, bad_url in (("unlisted", url), (name, "https://example.com/"),
                                  (name, "https://21307064330/"), (name, "https://2130706433/path")):
            with self.subTest(name=bad_name, url=bad_url):
                self.assertFalse(self.check(bad_name, bad_url, "numeric_alias", resolver_log(bad_url))["passed"])

    def test_alias_generic_network_tls_errors_and_mixed_failures_fail(self):
        name, url = "private_hex", probe.NUMERIC_ALIAS_URLS["private_hex"]
        for detail in ("timeout", "connection refused", "SSL/TLS handshake failure (80090325)",
                       "Failed to resolve the hostname example.com, cause: Unknown error", "", "Unknown error"):
            with self.subTest(detail=detail):
                self.assertFalse(self.check(name, url, "numeric_alias", exception_log(url, detail))["passed"])
                self.assertFalse(self.check(name, url, "numeric_alias", exception_log(url, detail) + resolver_log(url))["passed"])
        for suffix in ("timeout", "connection refused", "SSL/TLS (80090325)"):
            self.assertFalse(self.check(name, url, "numeric_alias", resolver_log(url) + suffix)["passed"])

    def test_alias_nonzero_and_zero_payload_are_both_mandatory(self):
        name, url = "private_octal", probe.NUMERIC_ALIAS_URLS["private_octal"]
        for code, size in ((0, 0), (1, 1), (0, 64)):
            self.assertFalse(self.check(name, url, "numeric_alias", resolver_log(url), code, size)["passed"])

    def test_uri_requires_fixed_fixture_and_parser_evidence(self):
        for name, url in probe.INVALID_URI_URLS.items():
            with self.subTest(name=name):
                result = self.check(name, url, "invalid_uri", parser_log(url))
                self.assertTrue(result["passed"])
                self.assertEqual(result["evidence_class"], "request_uri_rejected_before_socket")
                self.assertFalse(result["socket_gate_rejection_observed"])
                self.assertFalse(self.check(name, url, "invalid_uri", "aria2 will resume download if the transfer is restarted.")["passed"])
                self.assertFalse(self.check(name, url, "invalid_uri", "Unrecognized URI or unsupported protocol: " + url)["passed"])
                self.assertFalse(self.check(name, url, "invalid_uri", parser_log(url), code=0)["passed"])
                self.assertFalse(self.check(name, url, "invalid_uri", parser_log(url), size=1)["passed"])
        self.assertFalse(self.check("initial_http", "https://example.com/", "invalid_uri", parser_log("https://example.com/"))["passed"])

    def test_uri_wrong_source_url_or_error_code_is_not_parser_evidence(self):
        name, url = "initial_fragment", probe.INVALID_URI_URLS["initial_fragment"]
        original = parser_log(url)
        for text in (original.replace("CreateRequestCommand.cc:", "SocketCore.cc:"),
                     original.replace(url, "https://example.com/"),
                     original.replace(url, url + "suffix"),
                     original.replace("errorCode=-1", "errorCode=1"),
                     original.replace("[DEBUG]", "[INFO]"),
                     original + "[DEBUG] FeedbackURISelector selected https://example.com/\n",
                     "No URI available."):
            with self.subTest(text=text):
                self.assertFalse(self.check(name, url, "invalid_uri", text)["passed"])

    def test_uri_no_uri_after_network_failure_is_not_parser_rejection(self):
        name, url = "initial_userinfo", probe.INVALID_URI_URLS["initial_userinfo"]
        for prefix in ("timeout\n", "connection refused\n", "[ERROR] generic failure\n",
                       "[INFO] CUID#7 - Connecting to 127.0.0.1:443\n",
                       "[INFO] CUID#7 - Resolving hostname 127.0.0.1\n",
                       "[INFO] CUID#7 - Sending request:\n", resolver_log(url),
                       certificate_log(url, "80090325")):
            with self.subTest(prefix=prefix):
                self.assertFalse(self.check(name, url, "invalid_uri", prefix + parser_log(url))["passed"])

    def test_uri_log_contract_uses_debug_without_changing_security_options(self):
        for name, url in probe.INVALID_URI_URLS.items():
            argv = probe.download_command(Path("aria2.exe"), Path("fixture"), url, "invalid_uri")
            self.assertIn("--console-log-level=debug", argv)
            self.assertEqual(argv[-1], url)
            for required in ("--no-conf", "--no-netrc=true", "--check-certificate=true", "--enable-rpc=false", "--max-tries=1"):
                self.assertIn(required, argv)
        self.assertIn("--console-log-level=warn", probe.download_command(Path("aria2.exe"), Path("fixture"), "https://example.com/", "public_https"))

    def test_ansi_crlf_and_long_debug_tail_retain_matched_evidence(self):
        name, url = "initial_port", probe.INVALID_URI_URLS["initial_port"]
        text = parser_log(url).replace("[DEBUG]", "[\x1b[1;37mDEBUG\x1b[0m]").replace("\n", "\r\n") + "x" * 10000
        result = self.check(name, url, "invalid_uri", text)
        self.assertTrue(result["passed"])
        self.assertIn("FeedbackURISelector selected " + url, "\n".join(result["matched_evidence"]))
        self.assertIn("CreateRequestCommand.cc:107", "\n".join(result["matched_evidence"]))

    def test_certificate_each_fixture_needs_its_own_schannel_error(self):
        for name, (url, errors) in probe.CERTIFICATE_CASES.items():
            for error in errors:
                with self.subTest(name=name, error=error):
                    self.assertTrue(self.check(name, url, "certificate", certificate_log(url, error))["passed"])
                    self.assertFalse(self.check(name, url, "certificate", certificate_log(url, error), code=0)["passed"])
                    self.assertFalse(self.check(name, url, "certificate", certificate_log(url, error), size=1)["passed"])
            for error in probe.CERT_ERRORS - errors:
                self.assertFalse(self.check(name, url, "certificate", certificate_log(url, error))["passed"])

    def test_certificate_network_failure_wrong_host_and_fake_markers_fail(self):
        name, (url, _) = next(iter(probe.CERTIFICATE_CASES.items()))
        for text in ("80090322", "SSL/TLS handshake failure (80090322)", resolver_log(url),
                     exception_log(url, "connection timeout"), certificate_log(url, "80092013"),
                     certificate_log("https://example.com/", "80090322"),
                     certificate_log(url, "80090322").replace("SocketCore.cc:", "OtherSource.cc:")):
            with self.subTest(text=text):
                self.assertFalse(self.check(name, url, "certificate", text)["passed"])

    def test_public_control_requires_success_and_nonempty_bounded_payload(self):
        for code, size in ((1, 0), (1, 577), (0, 0), (0, 1048577)):
            for text in ("", "Nexa policy: destination rejected", "(80090325)", resolver_log("https://example.com/")):
                self.assertFalse(self.check("public_https", "https://example.com/", "public_https", text, code, size)["passed"])
        self.assertTrue(self.check("public_https", "https://example.com/", "public_https", "", 0, 577)["passed"])

    def test_unknown_expectation_never_falls_through_to_certificate_success(self):
        self.assertFalse(self.check("wrong_hostname", "https://wrong.host.badssl.com/", "typo", certificate_log("https://wrong.host.badssl.com/", "80090322"))["passed"])


class OrchestrationTests(unittest.TestCase):
    """Mock all native execution/listening; check gates, not OS behavior."""

    def run_fixture(self, failed_case=None, listener_connected=False):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            files = {name: b"mock executable, never run" for name in
                     ("nexa-aria2.exe", "policy_unit.exe", "engine_unit.exe", "payload_unit.exe")}
            files.update({"config.h": b"#define SECURITY_WIN32 1\n#define ENABLE_SSL 1\n",
                          "pe.txt": b"Name: secur32.dll\n"})
            for name, data in files.items():
                (root / name).write_bytes(data)
            manifest = {"source_lock": json.loads((ROOT / "third_party/aria2/source-lock.json").read_text(encoding="utf-8")),
                        "source_commit": "a" * 40, "pe": {"imports": ["secur32.dll"]},
                        "files": {name: {"sha256": probe.sha(root / name)} for name in files}}
            (root / "build-manifest.json").write_text(json.dumps(manifest), encoding="utf-8")

            def execute(argv, **kwargs):
                binary = Path(argv[0]).name
                unit = {"policy_unit.exe": "68 policy unit cases passed",
                        "engine_unit.exe": "26 Request parser/redirect cases passed\n4 actual SocketCore private destination cases passed",
                        "payload_unit.exe": "53 payload/IOFile cases passed"}
                if binary in unit:
                    return subprocess.CompletedProcess(argv, 0, unit[binary].encode(), b"")
                if "--version" in argv:
                    return subprocess.CompletedProcess(argv, 0, b"aria2 version 1.37.0 HTTPS", b"")
                directory = Path(next(x.removeprefix("--dir=") for x in argv if x.startswith("--dir=")))
                name, url = directory.name, argv[-1]
                code = 1
                if name == failed_case:
                    text = "timeout"
                elif name.startswith("invalid_limit_"):
                    text = "Nexa policy: invalid payload byte limit"
                elif name in probe.PRIVATE_URLS:
                    text = exception_log(url, "Nexa policy: destination rejected")
                elif name in probe.NUMERIC_ALIAS_URLS:
                    text = resolver_log(url)
                elif name in probe.INVALID_URI_URLS:
                    self.assertIn("--console-log-level=debug", argv)
                    text = parser_log(url)
                elif name in probe.CERTIFICATE_CASES:
                    text = certificate_log(url, sorted(probe.CERTIFICATE_CASES[name][1])[0])
                elif name == "public_https":
                    code, text = 0, ""
                    (directory / "payload").write_bytes(b"synthetic public fixture")
                else:
                    text = "Nexa policy: unsupported credential, proxy, RPC or TLS option"
                return subprocess.CompletedProcess(argv, code, text.encode(), b"")

            event = mock.Mock()
            event.is_set.side_effect = [False, True]
            listener = mock.Mock()
            listener.accept.return_value = (mock.Mock(), ("127.0.0.1", 1))

            def thread_factory(*, target):
                thread = mock.Mock()
                if listener_connected:
                    thread.start.side_effect = target
                return thread

            with mock.patch.object(probe, "os", SimpleNamespace(name="nt", environ={})), \
                    mock.patch.object(probe.platform, "platform", return_value="synthetic offline fixture"), \
                    mock.patch.object(probe.subprocess, "run", side_effect=execute), \
                    mock.patch.object(probe.socket, "socket", return_value=listener), \
                    mock.patch.object(probe.threading, "Event", return_value=event), \
                    mock.patch.object(probe.threading, "Thread", side_effect=thread_factory):
                passed = probe.probe(root, root / "report.json")
            report = json.loads((root / "report.json").read_text(encoding="utf-8"))
            self.assertEqual(passed, report["passed"])
            return report

    def test_report_preserves_all_32_cases_and_separate_alias_evidence(self):
        report = self.run_fixture()
        self.assertTrue(report["passed"])
        self.assertEqual(len(report["cases"]), 32)
        aliases = [case for case in report["cases"] if case["name"] in probe.NUMERIC_ALIAS_URLS]
        self.assertEqual(len(aliases), 3)
        self.assertTrue(all(c["evidence_class"] == "resolver_rejected_before_socket" and not c["socket_gate_rejection_observed"] for c in aliases))

    def test_public_and_each_certificate_remain_report_hard_gates(self):
        for name in ("public_https", *probe.CERTIFICATE_CASES):
            with self.subTest(name=name):
                report = self.run_fixture(failed_case=name)
                self.assertFalse(report["passed"])
                self.assertEqual([c["name"] for c in report["cases"] if not c["passed"]], [name])

    def test_private_listener_connection_remains_report_hard_gate(self):
        report = self.run_fixture(listener_connected=True)
        self.assertFalse(report["passed"])
        listener = next(c for c in report["cases"] if c["name"] == "no_private_connection")
        self.assertFalse(listener["passed"])
        self.assertEqual(listener["accepted"], 1)


if __name__ == "__main__":
    unittest.main()
