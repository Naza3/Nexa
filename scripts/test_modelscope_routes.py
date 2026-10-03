import json
import socket
import ssl
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import probe_modelscope_routes as routes


URL = "https://modelscope.cn/api/v1/models/Qwen/test/repo?Revision=pinned&FilePath=f.gguf"


class Response:
    def __init__(self, status=200, headers=None, body=b"GGUF" + b"x" * 10000):
        self.status = status
        self.headers = headers or {}
        self.body = body
        self.reads = []
        self.closed = False

    def getheader(self, key):
        return self.headers.get(key)

    def read(self, size):
        self.reads.append(size)
        return self.body[:size]

    def close(self):
        self.closed = True


class Factory:
    def __init__(self, responses, error=None):
        self.responses = list(responses)
        self.connections = []
        self.error = error

    def __call__(self, host, **kwargs):
        factory = self

        class Connection:
            def __init__(self):
                self.host = host
                self.options = kwargs
                self.sock = self
                self.closed = False
                self.requests = []

            def connect(self):
                if factory.error:
                    raise factory.error

            def settimeout(self, value):
                self.read_timeout = value

            def request(self, *args, **kwargs):
                self.requests.append((args, kwargs))

            def getresponse(self):
                return factory.responses.pop(0)

            def close(self):
                self.closed = True

        connection = Connection()
        self.connections.append(connection)
        return connection


class RouteProbeTests(unittest.TestCase):
    def test_bounded_prefix_and_verified_tls_without_ua(self):
        response = Response(headers={"Content-Length": "2497280256"})
        factory = Factory([response])
        result = routes.probe("test", URL, factory)
        self.assertEqual(result["outcome"], "prefix_observed")
        self.assertFalse(result["full_download"])
        self.assertTrue(result["tls_verify"])
        self.assertEqual(response.reads, [4096])
        self.assertEqual(result["hops"][0]["body_prefix_bytes"], 4096)
        self.assertTrue(result["hops"][0]["prefix_gguf"])
        connection = factory.connections[0]
        self.assertEqual(connection.options["context"].verify_mode, ssl.CERT_REQUIRED)
        self.assertTrue(connection.options["context"].check_hostname)
        self.assertEqual(connection.options["timeout"], 8)
        self.assertEqual(connection.read_timeout, 8)
        self.assertEqual(connection.requests[0][1]["headers"], {"Accept-Encoding": "identity"})
        self.assertTrue(connection.closed and response.closed)

    def test_unknown_host_never_connected_and_secrets_not_reported(self):
        response = Response(302, {"Location": "https://cdn-lfs-cn-1.modelscope.cn/private/path?auth_key=SUPERSECRET"})
        factory = Factory([response])
        result = routes.probe("test", URL, factory)
        hop = result["hops"][0]
        self.assertEqual(hop["reason"], "unapproved_host")
        self.assertEqual(hop["redirect_target"]["host"], "cdn-lfs-cn-1.modelscope.cn")
        self.assertTrue(hop["redirect_target"]["has_query"])
        self.assertEqual(len(factory.connections), 1)
        self.assertEqual(response.reads, [])
        for secret in ("SUPERSECRET", "auth_key", "/private/path", "Revision", "FilePath"):
            self.assertNotIn(secret, json.dumps(result))

    def test_only_safe_same_host_redirect_is_followed(self):
        first = Response(302, {"Location": "/other?secret=REDACT"})
        second = Response()
        factory = Factory([first, second])
        result = routes.probe("test", URL, factory)
        self.assertEqual(len(factory.connections), 2)
        self.assertEqual(result["outcome"], "prefix_observed")
        self.assertEqual(first.reads, [])
        self.assertNotIn("REDACT", json.dumps(result))

    def test_unsafe_redirect_variants_never_followed(self):
        cases = {
            "http://modelscope.cn/file": "non_https",
            "https://modelscope.cn:444/file": "non_standard_port",
            "https://alice:password@modelscope.cn/file": "userinfo",
            "https://modelscope.cn/file#secret": "fragment",
            "https://modelscope.cn.evil.test/file": "unapproved_host",
            "https://modelscope.cn:bad/file": "invalid_location",
            "https://[bad/file": "invalid_location",
            "https://modelscope.cn/file\nsecret": "invalid_location",
            "x" * 16385: "invalid_location",
        }
        for target, reason in cases.items():
            with self.subTest(reason=reason):
                factory = Factory([Response(302, {"Location": target})])
                result = routes.probe("test", URL, factory)
                self.assertEqual(result["hops"][0]["reason"], reason)
                self.assertEqual(len(factory.connections), 1)
                self.assertNotIn("password", json.dumps(result))
                self.assertNotIn("alice", json.dumps(result))

    def test_missing_location_and_hop_limit(self):
        result = routes.probe("test", URL, Factory([Response(302)]))
        self.assertEqual(result["hops"][0]["reason"], "missing_location")
        factory = Factory([Response(302, {"Location": URL}) for _ in range(6)])
        result = routes.probe("test", URL, factory)
        self.assertEqual(result["hops"][-1]["reason"], "redirect_limit")
        self.assertEqual(len(factory.connections), 6)

    def test_non200_never_reads_body(self):
        for status in (206, 403, 404, 500):
            response = Response(status)
            result = routes.probe("test", URL, Factory([response]))
            self.assertEqual(result["outcome"], "http_non_200")
            self.assertEqual(response.reads, [])

    def test_errors_are_fixed_categories_not_exception_text(self):
        for error, category in ((socket.gaierror("secret"), "dns_failed"),
                                (TimeoutError("secret"), "timeout"),
                                (ssl.SSLCertVerificationError("secret"), "tls_certificate_failed"),
                                (ssl.SSLError("secret"), "tls_failed"),
                                (OSError("secret"), "connection_failed"),
                                (RuntimeError("secret"), "probe_internal_failed")):
            result = routes.probe("test", URL, Factory([], error))
            self.assertEqual(result["hops"][0]["reason"], category)
            self.assertNotIn("secret", json.dumps(result))

    def test_invalid_initial_target_makes_no_connection(self):
        factory = Factory([])
        result = routes.probe("test", "https://untrusted.test/", factory)
        self.assertEqual(result["outcome"], "target_rejected")
        self.assertEqual(factory.connections, [])

    def test_header_values_are_sanitized(self):
        response = Response(headers={"Content-Length": "secret", "Content-Encoding": "secret"}, body=b"BAD")
        result = routes.probe("test", URL, Factory([response]))
        self.assertIsNone(result["hops"][0]["content_length"])
        self.assertEqual(result["hops"][0]["content_encoding"], "other")
        self.assertFalse(result["hops"][0]["prefix_gguf"])
        self.assertNotIn("secret", json.dumps(result))

    def test_main_selects_only_two_catalog_modelscope_entries(self):
        entries = [{"catalog_id": catalog_id, "sources": [
            {"source": "huggingface", "url": "https://other.test/ignored"},
            {"source": "modelscope", "url": URL}]} for catalog_id in routes.IDS]
        entries.append({"catalog_id": "not_selected", "sources": []})
        with tempfile.TemporaryDirectory() as directory:
            catalog = Path(directory) / "catalog.json"
            output = Path(directory) / "result.json"
            catalog.write_text(json.dumps({"entries": entries}), encoding="utf-8")
            with mock.patch.object(routes, "CATALOG", catalog), \
                    mock.patch("sys.argv", ["probe", "--output", str(output)]), \
                    mock.patch.object(routes, "probe", return_value={"outcome": "redirect_blocked"}) as probe:
                self.assertEqual(routes.main(), 0)
            self.assertEqual(probe.call_args_list, [mock.call(catalog_id, URL) for catalog_id in routes.IDS])
            report = json.loads(output.read_text(encoding="utf-8"))
            self.assertEqual(len(report["results"]), 2)
            self.assertFalse(report["full_download"])

    def test_catalog_error_is_sanitized(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "result.json"
            with mock.patch.object(routes, "CATALOG", Path(directory) / "SECRET_PATH"), \
                    mock.patch("sys.argv", ["probe", "--output", str(output)]):
                self.assertEqual(routes.main(), 1)
            text = output.read_text(encoding="utf-8")
            self.assertNotIn("SECRET_PATH", text)
            self.assertEqual(json.loads(text)["setup_error"], "catalog_or_probe_setup_failed")


if __name__ == "__main__":
    unittest.main()
