"""Harness tests use trivial subprocesses; they do not claim model validation."""
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

from run_upstream_baseline import main, run_process, validate_benchmark, validate_completion


class RunnerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.out = self.root / "stdout"
        self.err = self.root / "stderr"

    def test_stdin_eof_and_utf8_files(self):
        result = run_process(
            [sys.executable, "-c", "import sys; assert sys.stdin.read() == ''; sys.stdout.buffer.write('中文'.encode()); print('log', file=sys.stderr)"],
            self.out, self.err, 5,
        )
        self.assertEqual(result["status"], "pass")
        self.assertTrue(validate_completion(self.out))
        self.assertEqual(self.err.read_text(encoding="utf-8").strip(), "log")

    def test_timeout_is_failure_and_preserves_partial_output(self):
        result = run_process(
            [sys.executable, "-c", "import time; print('started', flush=True); time.sleep(30)"],
            self.out, self.err, 2,
        )
        self.assertTrue(result["timed_out"])
        self.assertEqual(result["status"], "failed")
        self.assertIsNone(result["exit_code"])
        self.assertEqual(self.out.read_text(encoding="utf-8").strip(), "started")
        self.assertLess(result["elapsed_ms"], 10000)

    def test_nonzero_and_spawn_failure(self):
        result = run_process([sys.executable, "-c", "raise SystemExit(3)"], self.out, self.err, 5)
        self.assertEqual(result["exit_code"], 3)
        self.assertEqual(result["status"], "failed")
        result = run_process([str(self.root / "missing")], self.out, self.err, 5)
        self.assertEqual(result["error"], "process_start_failed")

    def test_benchmark_requires_actual_five_samples(self):
        rows = [{"n_prompt": 128, "n_gen": 0, "n_threads": 2, "samples_ns": [10] * 5},
                {"n_prompt": 0, "n_gen": 32, "n_threads": 2, "samples_ns": [20] * 5}]
        self.out.write_text(json.dumps(rows), encoding="utf-8")
        self.assertTrue(validate_benchmark(self.out))
        self.assertTrue(validate_benchmark(self.out, 2))
        self.assertFalse(validate_benchmark(self.out, 4))
        rows[0]["samples_ns"] = []
        self.out.write_text(json.dumps(rows), encoding="utf-8")
        self.assertFalse(validate_benchmark(self.out))
        self.out.write_text("<think>中文</think>", encoding="utf-8")
        self.assertFalse(validate_completion(self.out))

    def test_diagnostic_failure_stays_visible_and_does_not_skip_selected_baseline(self):
        def fake(command, output, error, timeout):
            error.write_bytes(b"")
            if output.name == "upstream-bench.json":
                rows = [{"n_prompt": 128, "n_gen": 0, "n_threads": 2, "samples_ns": [10] * 5},
                        {"n_prompt": 0, "n_gen": 32, "n_threads": 2, "samples_ns": [20] * 5}]
                output.write_text(json.dumps(rows), encoding="utf-8")
            else:
                output.write_text("中文", encoding="utf-8")
            return {"status": "failed" if "threads-4" in output.name else "pass"}
        arguments = ["runner", "--completion", "completion", "--bench", "bench",
                     "--model", "model", "--prompt-file", "prompt", "--out-dir", str(self.root),
                     "--threads", "2", "--diagnostic-threads", "1", "4"]
        with patch.object(sys, "argv", arguments), patch("run_upstream_baseline.run_process", fake):
            self.assertEqual(main(), 0)
        report = json.loads((self.root / "upstream-processes.json").read_text(encoding="utf-8"))
        self.assertEqual(report["result"], "pass")
        self.assertEqual(report["baseline_result"], "pass")
        self.assertEqual(report["diagnostic_result"], "failed")
        self.assertEqual([r["status"] for r in report["diagnostics"]], ["pass", "failed"])

    def test_selected_baseline_failure_still_fails_and_collects_benchmark(self):
        def fake(command, output, error, timeout):
            error.write_bytes(b"")
            if output.name == "upstream-bench.json":
                rows = [{"n_prompt": 128, "n_gen": 0, "n_threads": 2, "samples_ns": [10] * 5},
                        {"n_prompt": 0, "n_gen": 32, "n_threads": 2, "samples_ns": [20] * 5}]
                output.write_text(json.dumps(rows), encoding="utf-8")
                return {"status": "pass"}
            output.write_bytes(b"")
            return {"status": "failed", "timed_out": True}
        arguments = ["runner", "--completion", "completion", "--bench", "bench",
                     "--model", "model", "--prompt-file", "prompt", "--out-dir", str(self.root),
                     "--threads", "2"]
        with patch.object(sys, "argv", arguments), patch("run_upstream_baseline.run_process", fake):
            self.assertEqual(main(), 1)
        report = json.loads((self.root / "upstream-processes.json").read_text(encoding="utf-8"))
        self.assertEqual(report["result"], "failed")
        self.assertEqual(report["baseline_result"], "failed")
        self.assertEqual(report["benchmark"]["status"], "pass")


if __name__ == "__main__":
    unittest.main()
