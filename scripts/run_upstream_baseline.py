#!/usr/bin/env python3
"""Run the pinned upstream tools with EOF stdin and a bounded process lifetime.

This is development verification, not Nexa's production process host. The caller
must verify model identity first with xtask baseline-verify. Reports never mark a
skipped or timed-out command successful.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import time


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run_process(command, output, error, timeout):
    """Redirect before launch; subprocess.run kills and reaps on TimeoutExpired."""
    started = time.monotonic()
    result = {"status": "failed", "exit_code": None, "timed_out": False}
    with output.open("wb") as stdout, error.open("wb") as stderr:
        try:
            completed = subprocess.run(
                command,
                stdin=subprocess.DEVNULL,
                stdout=stdout,
                stderr=stderr,
                timeout=timeout,
                check=False,
            )
            result["exit_code"] = completed.returncode
            result["status"] = "pass" if completed.returncode == 0 else "failed"
        except subprocess.TimeoutExpired:
            result["timed_out"] = True
            result["error"] = "process_timeout"
        except OSError:
            result["error"] = "process_start_failed"
    result.update(
        elapsed_ms=round((time.monotonic() - started) * 1000),
        stdout_file=output.name,
        stderr_file=error.name,
        stdout_sha256=digest(output),
        stderr_sha256=digest(error),
    )
    return result


def validate_completion(path):
    text = path.read_bytes().decode("utf-8-sig", errors="strict")
    return bool(re.search(r"[\u4e00-\u9fff]", text)) and not any(
        marker in text for marker in ("<think>", "</think>")
    )


def validate_benchmark(path):
    data = json.loads(path.read_text(encoding="utf-8-sig"))
    if not isinstance(data, list) or len(data) != 2:
        return False
    actual = set()
    for row in data:
        if not isinstance(row, dict):
            return False
        samples = row.get("samples_ns")
        if not isinstance(samples, list) or len(samples) != 5:
            return False
        if not all(isinstance(value, (int, float)) and value > 0 for value in samples):
            return False
        actual.add((row.get("n_prompt"), row.get("n_gen")))
    return actual == {(128, 0), (0, 32)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("completion", "bench", "model", "prompt-file", "out-dir"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--timeout-seconds", type=int, default=300)
    args = parser.parse_args()
    if not 1 <= args.timeout_seconds <= 600:
        parser.error("timeout must be 1..600 seconds per process")
    args.out_dir.mkdir(parents=True, exist_ok=True)
    report = {
        "schema_version": 1,
        "result": "failed",
        "timeout_seconds_per_process": args.timeout_seconds,
        "stdin": "closed",
        "completion": {"status": "skipped"},
        "benchmark": {"status": "skipped"},
    }
    report_path = args.out_dir / "upstream-processes.json"
    try:
        print("Starting bounded upstream completion", flush=True)
        out = args.out_dir / "upstream-zh.txt"
        err = args.out_dir / "upstream-zh.log"
        report["completion"] = run_process(
            [str(args.completion.resolve()), "-m", str(args.model.resolve()),
             "-c", "2048", "-b", "128", "-t", "4", "-n", "64",
             "--temp", "0", "--seed", "42", "--simple-io",
             "--no-conversation", "--no-display-prompt", "-f", str(args.prompt_file.resolve())],
            out, err, args.timeout_seconds,
        )
        report_path.write_text(json.dumps(report, indent=2), encoding="utf-8")
        print("Completion:", report["completion"]["status"], flush=True)
        if report["completion"]["status"] != "pass":
            return 1
        if not validate_completion(out):
            report["completion"].update(status="failed", error="invalid_chat_output")
            return 1
        print("Starting bounded upstream benchmark (5 repeats)", flush=True)
        out = args.out_dir / "upstream-bench.json"
        err = args.out_dir / "upstream-bench.log"
        report["benchmark"] = run_process(
            [str(args.bench.resolve()), "-m", str(args.model.resolve()),
             "-p", "128", "-n", "32", "-b", "128", "-ub", "128",
             "-t", "4", "-ngl", "0", "-r", "5", "-o", "json"],
            out, err, args.timeout_seconds,
        )
        print("Benchmark:", report["benchmark"]["status"], flush=True)
        if report["benchmark"]["status"] != "pass":
            return 1
        if not validate_benchmark(out):
            report["benchmark"].update(status="failed", error="invalid_benchmark_samples")
            return 1
        report["result"] = "pass"
        return 0
    except (OSError, UnicodeError, ValueError):
        report["error"] = "invalid_or_unreadable_verification_output"
        return 1
    finally:
        report_path.write_text(json.dumps(report, indent=2), encoding="utf-8")


if __name__ == "__main__":
    raise SystemExit(main())
