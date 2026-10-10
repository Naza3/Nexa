#!/usr/bin/env python3
"""Opt-in real Nexa + pinned PI serializer two-turn in-memory tool validation.

Requires explicit built CLI, external pinned client, exact GGUF and expected
SHA256. Creates only its own temporary runtime; never touches user credentials
or an existing service. No model download, arbitrary tool, retry, push or GUI.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
from model_identity import embedded_template_hash


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def command(args, timeout=180):
    result = subprocess.run(args, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                            stderr=subprocess.DEVNULL, check=False, timeout=timeout)
    if result.returncode:
        raise RuntimeError("cli_command_failed")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("cli", "client", "model", "out"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--expected-sha256", required=True)
    parser.add_argument("--node", default="node")
    args = parser.parse_args()
    if len(args.expected_sha256) != 64 or any(c not in "0123456789abcdef" for c in args.expected_sha256):
        parser.error("expected SHA256 must be exact lowercase hexadecimal")
    cli, client, model = (path.resolve() for path in (args.cli, args.client, args.model))
    if not cli.is_file() or not model.is_file() or not client.is_dir():
        parser.error("explicit input is missing")
    report = {"schema_version": 1, "status": "failed", "model_sha256": digest(model),
              "model_size_bytes": model.stat().st_size,
              "chat_template_sha256": embedded_template_hash(model),
              "configuration": {"context_size": 2048, "threads": 2, "batch_size": 128},
              "scope": "Linux/Windows host test, not PI Desktop GUI or user-device acceptance",
              "forced_cleanup": False, "steps": {}}
    if report["model_sha256"] != args.expected_sha256:
        parser.error("model SHA256 differs from the explicit expected input")
    child = None
    stage = "init"
    started = time.monotonic()
    try:
        with tempfile.TemporaryDirectory(prefix="nexa-pi-tools-") as temporary:
            root = Path(temporary) / "private-data"
            base = [str(cli), "--data-dir", str(root)]
            try:
                command(base + ["init"], 30)
                config = root / "config.toml"
                text = config.read_text(encoding="utf-8")
                if "127.0.0.1:18080" not in text:
                    raise RuntimeError("default_listener_not_found")
                config.write_text(text.replace("127.0.0.1:18080", "127.0.0.1:0"), encoding="utf-8")
                stage = "serve"
                child = subprocess.Popen(base + ["serve"], stdin=subprocess.DEVNULL,
                                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                discovery = root / "runtime/instance.json"
                deadline = time.monotonic() + 30
                while not discovery.is_file():
                    if child.poll() is not None or time.monotonic() >= deadline:
                        raise RuntimeError("discovery_failed")
                    time.sleep(0.02)
                address = json.loads(discovery.read_text(encoding="utf-8"))["listen"]
                if not address.startswith("127.0.0.1:"):
                    raise RuntimeError("non_loopback_discovery")
                stage = "import"
                command(base + ["models", "import", "--id", "qa-tools", "--file", str(model)])
                stage = "load"
                command(base + ["load", "qa-tools", "--context", "2048", "--threads", "2", "--batch", "128"])
                stage = "real_tool_roundtrip"
                # Only the new test runtime's private token is read, sent over a
                # private child stdin pipe, used on its loopback endpoint, and
                # destroyed with this test. Never logged or persisted to output.
                token = (root / "secrets/api-token").read_text(encoding="utf-8")
                connection = {"kind": "nexa-owned-temporary-tools-test", "baseUrl": "http://" + address + "/v1", "token": token}
                child_result = subprocess.run([args.node, str(Path(__file__).with_name("real-client.mjs")), str(client)],
                                              input=json.dumps(connection), text=True, encoding="utf-8",
                                              stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=420, check=False)
                token = ""
                connection.clear()
                report["client"] = json.loads(child_result.stdout)
                if child_result.returncode or report["client"]["status"] != "pass":
                    raise RuntimeError("roundtrip_failed")
                report["status"] = "pass"
            finally:
                if child is not None and child.poll() is None:
                    try:
                        command(base + ["stop"], 30)
                        if child.wait(timeout=10):
                            report["status"] = "failed"
                    except (OSError, RuntimeError, subprocess.TimeoutExpired):
                        report["forced_cleanup"] = True
                        report["status"] = "failed"
                        child.kill()
                        child.wait(timeout=10)
    except (OSError, ValueError, KeyError, RuntimeError, subprocess.TimeoutExpired) as error:
        report["status"] = "failed"
        report["failed_stage"] = stage
        report["error_kind"] = type(error).__name__
    report["elapsed_ms"] = round((time.monotonic() - started) * 1000)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print("real PI tool roundtrip: " + report["status"])
    return 0 if report["status"] == "pass" else 1


if __name__ == "__main__":
    raise SystemExit(main())
