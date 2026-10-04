#!/usr/bin/env python3
"""Run real T04 CLI/API smoke using temporary credentials and one owned service.

The caller verifies the GGUF against the fixed model matrix first. This wrapper
uses the actual CLI for init, online import and stop; it never reads or records a
Bearer token. Tests use an ephemeral loopback port and no persistent user data.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time


def execute(command, stdout, stderr, timeout):
    with stdout.open("wb") as out, stderr.open("wb") as err:
        result = subprocess.run(command, stdin=subprocess.DEVNULL, stdout=out,
                                stderr=err, timeout=timeout, check=False)
    return result.returncode


def file_hash(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("cli", "xtask", "model", "out-dir"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--timeout-seconds", type=int, default=600)
    parser.add_argument("--disconnect-cycles", type=int, default=5)
    args = parser.parse_args()
    if not 30 <= args.timeout_seconds <= 1200:
        parser.error("timeout must be 30..1200 seconds")
    if not 1 <= args.disconnect_cycles <= 50:
        parser.error("disconnect cycles must be 1..50")
    cli, xtask, model = (path.resolve() for path in (args.cli, args.xtask, args.model))
    for path in (cli, xtask, model):
        if not path.is_file():
            parser.error("an explicit executable or model file is missing")
    output = args.out_dir.resolve()
    output.mkdir(parents=True, exist_ok=True)
    started = time.monotonic()
    report = {"schema_version": 1, "kind": "real-http-cli-lifecycle",
              "project_commit": os.environ.get("GITHUB_SHA"),
              "model_sha256": file_hash(model),
              "configuration": {"backend": "cpu", "context_size": 2048,
                                "threads": 2, "batch_size": 128, "gpu_layers": 0},
              "disconnect_cycles": args.disconnect_cycles,
              "status": "failed", "steps": {}, "forced_cleanup": False}
    child = None
    stage = "initialize"
    try:
        with tempfile.TemporaryDirectory(prefix="nexa-t04-") as temporary:
            root = Path(temporary) / "private-data"  # init creates its private ACL
            base = [str(cli), "--data-dir", str(root)]
            try:
                code = execute(base + ["init"], output / "cli-init.json",
                               output / "cli-init.stderr", 30)
                report["steps"][stage] = code
                if code:
                    raise RuntimeError("init_failed")
                config = root / "config.toml"
                config.write_text(config.read_text(encoding="utf-8").replace(
                    "127.0.0.1:18080", "127.0.0.1:0"), encoding="utf-8")
                stage = "serve"
                with (output / "serve.stdout").open("wb") as out, (output / "serve.stderr").open("wb") as err:
                    child = subprocess.Popen(base + ["serve"], stdin=subprocess.DEVNULL,
                                             stdout=out, stderr=err)
                    deadline = time.monotonic() + 30
                    discovery = root / "runtime/instance.json"
                    while not discovery.is_file():
                        if child.poll() is not None:
                            raise RuntimeError("serve_exited_before_discovery")
                        if time.monotonic() >= deadline:
                            raise RuntimeError("discovery_timeout")
                        time.sleep(0.02)
                    record = json.loads(discovery.read_text(encoding="utf-8"))
                    address = record["listen"]
                    stage = "online_import"
                    code = execute(base + ["models", "import", "--id", "qa-small", "--file", str(model)],
                                   output / "cli-import.json", output / "cli-import.stderr", 180)
                    report["steps"][stage] = code
                    if code:
                        raise RuntimeError("online_import_failed")
                    stage = "api_smoke"
                    code = execute([str(xtask), "api-smoke", "--base-url", "http://" + address,
                                    "--data-dir", str(root), "--model", "qa-small", "--out",
                                    str(output / "api-smoke.json"), "--disconnect-cycles", str(args.disconnect_cycles)], output / "api-smoke.stdout",
                                   output / "api-smoke.stderr", args.timeout_seconds)
                    report["steps"][stage] = code
                    if code:
                        raise RuntimeError("api_smoke_failed")
                    stage = "service_exit"
                    code = child.wait(timeout=30)
                    report["steps"][stage] = code
                    if code or discovery.exists():
                        raise RuntimeError("service_exit_not_confirmed")
                    report["status"] = "pass"
            finally:
                if child is not None and child.poll() is None:
                    try:
                        report["steps"]["cleanup_stop"] = execute(base + ["stop"],
                            output / "cleanup-stop.json", output / "cleanup-stop.stderr", 30)
                        child.wait(timeout=10)
                    except (OSError, subprocess.TimeoutExpired):
                        report["forced_cleanup"] = True
                        child.kill()  # Only this wrapper's own known child.
                        child.wait(timeout=10)
                    if report["forced_cleanup"] or child.returncode != 0:
                        report["status"] = "failed"
    except (OSError, RuntimeError, ValueError, KeyError, subprocess.TimeoutExpired) as error:
        report["status"] = "failed"
        report["failed_stage"] = stage
        # No raw error text: it may include private paths or request data.
        report["error_kind"] = type(error).__name__
    report["elapsed_ms"] = round((time.monotonic() - started) * 1000)
    (output / "lifecycle.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print("real HTTP/CLI smoke: " + report["status"])
    return 0 if report["status"] == "pass" else 1


if __name__ == "__main__":
    raise SystemExit(main())
