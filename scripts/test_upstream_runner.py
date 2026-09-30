"""Harness tests use trivial subprocesses; they do not claim model validation."""
import json
from pathlib import Path
import sys
import tempfile
import unittest

from run_upstream_baseline import run_process, validate_benchmark, validate_completion


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
        rows = [{"n_prompt": 128, "n_gen": 0, "samples_ns": [10] * 5},
                {"n_prompt": 0, "n_gen": 32, "samples_ns": [20] * 5}]
        self.out.write_text(json.dumps(rows), encoding="utf-8")
        self.assertTrue(validate_benchmark(self.out))
        rows[0]["samples_ns"] = []
        self.out.write_text(json.dumps(rows), encoding="utf-8")
        self.assertFalse(validate_benchmark(self.out))
        self.out.write_text("<think>中文</think>", encoding="utf-8")
        self.assertFalse(validate_completion(self.out))


if __name__ == "__main__":
    unittest.main()
