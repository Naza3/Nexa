"""Bounded, read-only direct HTTPS observations; never a model-download verdict."""

import argparse
import http.client
import json
from pathlib import Path
import re
import socket
import ssl
from urllib.parse import urljoin, urlsplit


IDS = ("qwen3-0.6b-q8-0", "qwen3-4b-q4-k-m")
BODY_LIMIT = 4096
TIMEOUT = 8
MAX_REDIRECTS = 5
CATALOG = Path(__file__).resolve().parents[1] / "crates/desktop-bridge/src/model-catalog.json"


def target_metadata(url):
    """Only bounded, parsed routing metadata may leave this function."""
    safe = {"host": None, "scheme": "invalid", "port": None,
            "has_userinfo": False, "has_fragment": False, "has_query": False}
    if not isinstance(url, str) or len(url) > 16384 or any(ord(c) < 32 or ord(c) == 127 for c in url):
        return safe, "invalid_location"
    try:
        parsed = urlsplit(url)
        safe.update(scheme=parsed.scheme if parsed.scheme in ("http", "https") else "other",
                    has_userinfo="@" in parsed.netloc,
                    has_fragment="#" in url, has_query="?" in url)
        host = (parsed.hostname or "").encode("idna").decode("ascii").lower()
        if not host or len(host) > 253 or not re.fullmatch(r"[a-z0-9.-]+", host):
            return safe, "invalid_host"
        safe["host"] = host
        safe["port"] = parsed.port
    except (ValueError, UnicodeError):
        return safe, "invalid_location"
    for condition, reason in ((safe["scheme"] != "https", "non_https"),
                              (safe["port"] not in (None, 443), "non_standard_port"),
                              (safe["has_userinfo"], "userinfo"),
                              (safe["has_fragment"], "fragment"),
                              (safe["host"] != "modelscope.cn", "unapproved_host")):
        if condition:
            return safe, reason
    return safe, None


def error_category(error):
    if isinstance(error, ssl.SSLCertVerificationError):
        return "tls_certificate_failed"
    if isinstance(error, ssl.SSLError):
        return "tls_failed"
    if isinstance(error, socket.gaierror):
        return "dns_failed"
    if isinstance(error, (TimeoutError, socket.timeout)):
        return "timeout"
    if isinstance(error, http.client.HTTPException):
        return "http_protocol_failed"
    if isinstance(error, OSError):
        return "connection_failed"
    return "probe_internal_failed"


def numeric_header(value):
    return int(value) if isinstance(value, str) and re.fullmatch(r"[0-9]{1,20}", value) else None


def probe(catalog_id, url, connection_factory=http.client.HTTPSConnection):
    result = {"catalog_id": catalog_id, "observational_only": True,
              "full_download": False, "body_prefix_limit": BODY_LIMIT,
              "tls_verify": True, "proxy_used": False, "hops": []}
    context = ssl.create_default_context()
    for hop in range(MAX_REDIRECTS + 1):
        routing, rejection = target_metadata(url)
        record = {"hop": hop, "request_target": routing}
        result["hops"].append(record)
        if rejection:
            record["reason"] = rejection
            result["outcome"] = "target_rejected"
            return result
        connection = None
        response = None
        try:
            parsed = urlsplit(url)
            connection = connection_factory("modelscope.cn", port=443, timeout=TIMEOUT, context=context)
            connection.connect()
            connection.sock.settimeout(TIMEOUT)
            # http.client does not consult proxy environment variables or add a User-Agent.
            path = parsed.path or "/"
            if parsed.query:
                path += "?" + parsed.query
            connection.request("GET", path, headers={"Accept-Encoding": "identity"})
            response = connection.getresponse()
            record["status"] = response.status
            if 300 <= response.status < 400:
                location = response.getheader("Location")
                if location is None:
                    record["reason"] = "missing_location"
                elif len(location) > 16384 or any(ord(c) < 32 or ord(c) == 127 for c in location):
                    record["reason"] = "invalid_location"
                else:
                    try:
                        target = urljoin(url, location)
                        metadata, reason = target_metadata(target)
                        record["redirect_target"] = metadata
                        if reason:
                            record["reason"] = reason
                        elif hop == MAX_REDIRECTS:
                            record["reason"] = "redirect_limit"
                        else:
                            url = target
                            continue
                    except (ValueError, UnicodeError):
                        record["reason"] = "invalid_location"
                result["outcome"] = "redirect_blocked"
                return result
            if response.status != 200:
                result["outcome"] = "http_non_200"
                return result
            record["content_length"] = numeric_header(response.getheader("Content-Length"))
            encoding = response.getheader("Content-Encoding")
            record["content_encoding"] = "absent" if encoding is None else "identity" if encoding == "identity" else "other"
            prefix = response.read(BODY_LIMIT)
            record["body_prefix_bytes"] = len(prefix)
            record["prefix_gguf"] = prefix[:4] == b"GGUF"
            result["outcome"] = "prefix_observed"
            return result
        except Exception as error:
            record["reason"] = error_category(error)
            result["outcome"] = "request_failed"
            return result
        finally:
            if response is not None:
                response.close()
            if connection is not None:
                connection.close()
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=Path("artifacts/modelscope-route-probe.json"))
    args = parser.parse_args()
    report = {"schema_version": 1, "observational_only": True, "full_download": False,
              "tls_verify": True, "body_prefix_limit": BODY_LIMIT, "results": []}
    try:
        entries = json.loads(CATALOG.read_text(encoding="utf-8"))["entries"]
        for catalog_id in IDS:
            entry = next(item for item in entries if item["catalog_id"] == catalog_id)
            source = next(item for item in entry["sources"] if item["source"] == "modelscope")
            # Pinning is read only from the checked-in catalog; no arbitrary URL argument.
            report["results"].append(probe(catalog_id, source["url"]))
    except Exception:
        report["setup_error"] = "catalog_or_probe_setup_failed"
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, ensure_ascii=True, indent=2) + "\n", encoding="utf-8")
    return 1 if "setup_error" in report else 0


if __name__ == "__main__":
    raise SystemExit(main())
