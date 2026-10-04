"""Pure harness tests; never a substitute for MNN model/device evidence."""
from pathlib import Path
import tempfile
import sys
import unittest
from unittest.mock import patch
import run_probe as probe


class ProbeTests(unittest.TestCase):
    def test_lock_has_exact_identity(self):
        lock = probe.load_json(probe.LOCK)
        self.assertEqual(len(lock["files"]), 5)
        self.assertFalse(lock["admitted_production_package"])
        self.assertIsNone(lock["exporter_commit"])
        self.assertEqual(sum(x["size"] for x in lock["files"].values()), 454470710)

    def test_duplicate_json_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "bad.json"
            path.write_text('{"a":1,"a":2}')
            with self.assertRaisesRegex(ValueError, "duplicate"):
                probe.load_json(path)

    def test_nonfinite_json_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "bad.json"
            path.write_text('{"a":NaN}')
            with self.assertRaisesRegex(ValueError, "nonfinite"):
                probe.load_json(path)

    def test_missing_model_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(ValueError, "plain_model_file"):
                probe.validate_model(Path(directory), probe.load_json(probe.LOCK))

    def test_extra_file_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            (Path(directory) / "context.json").write_text('{}')
            with self.assertRaisesRegex(ValueError, "unexpected"):
                probe.validate_model(Path(directory), probe.load_json(probe.LOCK))

    def test_symlink_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            (path / "config.json").symlink_to("missing.json")
            with self.assertRaisesRegex(ValueError, "plain_model_file"):
                probe.validate_model(path, probe.load_json(probe.LOCK))

    def test_tampered_file_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            (path / "config.json").write_text('{}')
            with self.assertRaisesRegex(ValueError, "identity_mismatch"):
                probe.validate_model(path, probe.load_json(probe.LOCK))

    @staticmethod
    def metadata():
        config = dict.fromkeys(probe.CONFIG_KEYS)
        config.update({"llm_model": "llm.mnn", "llm_weight": "llm.mnn.weight"})
        model = dict.fromkeys(probe.MODEL_KEYS)
        model.update({"is_visual": False, "jinja": {"chat_template": "fixture", "eos": "end"},
                      "tie_embeddings": [275780066, 431362530, 19447808, 8, 64]})
        return config, model

    def test_metadata_rejects_hidden_path_keys(self):
        for key in ("context_file", "draft_model", "embedding_file", "npu_model_dir", "base_dir"):
            config, model = self.metadata()
            model[key] = "outside"
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, "unsupported_model_keys"):
                probe.validate_metadata(config, model)

    def test_metadata_rejects_reference_escape(self):
        config, model = self.metadata()
        config["llm_model"] = "../llm.mnn"
        with self.assertRaisesRegex(ValueError, "unsupported_model_reference"):
            probe.validate_metadata(config, model)

    def test_metadata_rejects_empty_template(self):
        config, model = self.metadata()
        model["jinja"]["chat_template"] = ""
        with self.assertRaisesRegex(ValueError, "template_required"):
            probe.validate_metadata(config, model)

    def test_runtime_options_fixed_and_template_copied(self):
        metadata = {"jinja": {"chat_template": "fixture", "eos": "end"}}
        runtime = probe.runtime_config(Path("model"), metadata, 2, 512, 8)
        self.assertEqual(runtime["sampler_type"], "greedy")
        self.assertEqual(runtime["backend_type"], "cpu")
        self.assertFalse(runtime["jinja"]["context"]["enable_thinking"])
        self.assertNotIn("context", metadata["jinja"])
        self.assertFalse(runtime["async"])
        self.assertFalse(runtime["reuse_kv"])
        self.assertFalse(runtime["prompt_cache"])
        self.assertEqual(runtime["context_file"], "nexa-probe-disabled-context.json")

    def test_golden_fixture_preserves_system_turns_and_unicode(self):
        rendered = probe.expected_render(probe.CASES["short-zh"])
        self.assertIn("☀️", rendered)
        self.assertTrue(rendered.startswith("<|im_start|>system\n"))
        self.assertTrue(rendered.endswith("<think>\n\n</think>\n\n"))
        self.assertEqual(probe.expected_render(probe.CASES["multiturn"]).count("<|im_start|>"), 5)

    @staticmethod
    def result():
        request = {"messages": probe.CASES["short-en"], "logical_context": 512, "max_new_tokens": 2}
        result = {"mnn_commit": probe.MNN_COMMIT, "status": "ok", "prototype": "T07-A",
                  "probe_report_version": 1, "runtime_backend": "cpu", "requested_backend": "cpu", "compiled_backend": "cpu", "sampler": "load-time-greedy",
                  "per_request_sampling": False, "thread_safe_cancel": False, "production_executor": False,
                  "rendered_prompt": probe.expected_render(request["messages"]), "prompt_tokens": [1, 2, 3],
                  "output_tokens": [4, 5], "output_text": "hello", "native_prompt_len": 3,
                  "finish_reason": "native_max_tokens", "logical_context": 512, "requested_max_new_tokens": 2}
        return result, request

    def test_result_validation_accepts_expected(self):
        probe.validate_result(*self.result())

    def test_result_validation_rejects_capability_mismatch(self):
        for key, value in (("compiled_backend", "opencl"), ("per_request_sampling", True),
                           ("thread_safe_cancel", 0), ("production_executor", True), ("mnn_commit", "other")):
            result, request = self.result()
            result[key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                probe.validate_result(result, request)

    def test_result_validation_rejects_bad_text(self):
        for text in ("", None, "\ud800"):
            result, request = self.result()
            result["output_text"] = text
            with self.subTest(text=repr(text)), self.assertRaises(ValueError):
                probe.validate_result(result, request)

    def test_result_validation_rejects_budget_or_template_drift(self):
        for key, value in (("native_prompt_len", 4), ("output_tokens", [4, 5, 6]),
                           ("prompt_tokens", [True]), ("rendered_prompt", "wrong"),
                           ("finish_reason", "cancelled"), ("logical_context", 513)):
            result, request = self.result()
            result[key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                probe.validate_result(result, request)

    def test_fast_process_log_limit_after_exit(self):
        with tempfile.TemporaryDirectory() as directory:
            code, reason = probe.run_bounded([sys.executable, "-c",
                "import os; os.write(1, b'x' * (2 * 1024 * 1024))"],
                directory, Path(directory) / "log", 2)
            self.assertEqual(reason, "development_log_limit")

    def test_reap_has_timeout(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(probe.subprocess, "Popen") as factory:
            proc = factory.return_value
            proc.poll.return_value = 0
            proc.returncode = 0
            code, reason = probe.run_bounded(["not-executed"], directory, Path(directory) / "log", 1)
            self.assertEqual(code, 0)
            self.assertIsNone(reason)
            proc.wait.assert_called_once_with(timeout=5)

    def test_unconfirmed_cleanup_reported(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(probe.subprocess, "Popen") as factory, patch.object(probe.time, "monotonic", side_effect=[0, 2]):
            proc = factory.return_value
            proc.poll.return_value = None
            proc.returncode = None
            proc.wait.side_effect = probe.subprocess.TimeoutExpired("not-executed", 5)
            code, reason = probe.run_bounded(["not-executed"], directory, Path(directory) / "log", 1)
            self.assertEqual(reason, "development_cleanup_unconfirmed")
            self.assertIsNone(code)
            self.assertEqual(proc.wait.call_count, 2)
            self.assertTrue(all(call.kwargs == {"timeout": 5} for call in proc.wait.call_args_list))


if __name__ == "__main__":
    unittest.main()
