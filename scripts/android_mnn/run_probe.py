#!/usr/bin/env python3
"""Offline Linux T07-A synthetic model harness. Not an Android runtime or importer."""
import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import tempfile
import time

MNN_COMMIT = "d407447ed56c4121a11ccbd266dc184ca1ead0c2"
LOCK = Path(__file__).with_name("candidate-model.json")
CONFIG_KEYS = {"llm_model", "llm_weight", "backend_type", "thread_num", "precision", "memory",
               "sampler_type", "mixed_samplers", "penalty", "temperature", "topP", "topK", "min_p"}
MODEL_KEYS = {"hidden_size", "layer_nums", "attention_mask", "key_value_shape", "bos",
              "system_prompt_template", "user_prompt_template", "assistant_prompt_template",
              "is_visual", "jinja", "tie_embeddings"}
CASES = {
    "short-en": [{"role": "system", "content": "Answer briefly."},
                 {"role": "user", "content": "What is 2 + 3?"}],
    "short-zh": [{"role": "system", "content": "请简短回答。"},
                 {"role": "user", "content": "用一句话描述晴天 ☀️。"}],
    "multiturn": [{"role": "system", "content": "Answer briefly."},
                  {"role": "user", "content": "Remember the word blue."},
                  {"role": "assistant", "content": "Blue."},
                  {"role": "user", "content": "Which word did I give you?"}],
    "empty": [{"role": "user", "content": ""}],
}


def digest(path):
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def text_digest(value):
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def load_json(path):
    def unique(pairs):
        result = {}
        for k, v in pairs:
            if k in result:
                raise ValueError("duplicate_json_key")
            result[k] = v
        return result
    if path.stat().st_size > 1024 * 1024:
        raise ValueError("metadata_too_large")
    return json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=unique,
                      parse_constant=lambda _: (_ for _ in ()).throw(ValueError("nonfinite_json")))


def validate_metadata(config, metadata):
    if not isinstance(config, dict) or set(config) != CONFIG_KEYS:
        raise ValueError("unsupported_config_keys")
    if not isinstance(metadata, dict) or set(metadata) != MODEL_KEYS:
        raise ValueError("unsupported_model_keys")
    if config["llm_model"] != "llm.mnn" or config["llm_weight"] != "llm.mnn.weight":
        raise ValueError("unsupported_model_reference")
    if metadata["is_visual"] is not False:
        raise ValueError("text_only_required")
    jinja = metadata["jinja"]
    if not isinstance(jinja, dict) or set(jinja) != {"chat_template", "eos"}:
        raise ValueError("unsupported_template_metadata")
    if not isinstance(jinja["chat_template"], str) or not jinja["chat_template"]:
        raise ValueError("template_required")
    if metadata["tie_embeddings"] != [275780066, 431362530, 19447808, 8, 64]:
        raise ValueError("unsupported_embedding_layout")


def validate_model(model, lock):
    if model.is_symlink() or not model.is_dir():
        raise ValueError("plain_model_directory_required")
    # No optional context, external graph, symlink or cache is allowed in this input.
    allowed = set(lock["files"]) | {"README.md", ".gitattributes", "LICENSE"}
    if {p.name for p in model.iterdir()} - allowed:
        raise ValueError("unexpected_model_file")
    for name, item in lock["files"].items():
        if Path(name).name != name or name in {".", ".."}:
            raise ValueError("invalid_locked_path")
        path = model / name
        if path.is_symlink() or not path.is_file():
            raise ValueError("plain_model_file_required")
        if path.stat().st_size != item["size"] or digest(path) != item["sha256"]:
            raise ValueError("model_identity_mismatch:" + name)
    config = load_json(model / "config.json")
    metadata = load_json(model / "llm_config.json")
    validate_metadata(config, metadata)
    if text_digest(metadata["jinja"]["chat_template"]) != lock["template_sha256"]:
        raise ValueError("template_identity_mismatch")
    offset, alpha, size, bits, block = metadata["tie_embeddings"]
    weight_size = lock["files"]["llm.mnn.weight"]["size"]
    if not (0 < offset < alpha < alpha + size <= weight_size and bits == 8 and block == 64):
        raise ValueError("embedding_bounds_invalid")
    return config, metadata


def runtime_config(model, metadata, threads, context, max_tokens):
    # Deliberately reconstruct rather than copy arbitrary model execution options.
    jinja = copy.deepcopy(metadata["jinja"])
    jinja["context"] = {"enable_thinking": False}
    return {"base_dir": str(model.resolve()) + "/", "llm_model": "llm.mnn",
            "llm_weight": "llm.mnn.weight", "llm_config": "llm_config.json",
            "tokenizer_file": "tokenizer.txt", "context_file": "nexa-probe-disabled-context.json",
            "backend_type": "cpu", "thread_num": threads, "precision": "high", "memory": "low",
            "sampler_type": "greedy", "max_all_tokens": context, "max_new_tokens": max_tokens,
            "reuse_kv": False, "prompt_cache": False, "speculative_type": "", "async": False,
            "use_mmap": False, "kvcache_mmap": False, "use_cached_mmap": False,
            "is_visual": False, "is_audio": False, "has_talker": False, "jinja": jinja}


def expected_render(messages):
    # Independent golden rendering for these limited, plain-text Qwen3 fixtures.
    # This is validation, never the prompt sent to native inference.
    return "".join("<|im_start|>" + m["role"] + "\n" + m["content"] + "<|im_end|>\n"
                   for m in messages) + "<|im_start|>assistant\n<think>\n\n</think>\n\n"


def run_bounded(command, cwd, log, timeout):
    # Kill applies ONLY to this isolated Linux development process, never an App thread.
    with log.open("wb") as stream:
        proc = subprocess.Popen(command, cwd=cwd, stdout=stream, stderr=subprocess.STDOUT)
        deadline = time.monotonic() + timeout
        reason = None
        try:
            while proc.poll() is None:
                if time.monotonic() >= deadline:
                    reason = "development_process_timeout"
                elif log.stat().st_size > 1024 * 1024:
                    reason = "development_log_limit"
                if reason:
                    proc.kill()
                    break
                time.sleep(0.05)
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                reason = "development_cleanup_unconfirmed"
        finally:
            if proc.poll() is None:
                proc.kill()
                try:
                    proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    reason = "development_cleanup_unconfirmed"
        # A short-lived process can exit between polls after exceeding the threshold.
        # This is a polling-triggered soft limit, not a hard bounded pipe sink.
        if reason is None and log.stat().st_size > 1024 * 1024:
            reason = "development_log_limit"
        return proc.returncode, reason


def validate_result(result, request):
    if result.get("mnn_commit") != MNN_COMMIT or result.get("status") != "ok":
        raise ValueError("native_identity_or_status_mismatch")
    required = {"prototype": "T07-A", "probe_report_version": 1, "requested_backend": "cpu", "compiled_backend": "cpu", "runtime_backend": "cpu",
                "sampler": "load-time-greedy", "per_request_sampling": False,
                "thread_safe_cancel": False, "production_executor": False}
    if any(type(result.get(k)) is not type(v) or result[k] != v for k, v in required.items()):
        raise ValueError("native_capability_mismatch")
    if not isinstance(result.get("output_text"), str) or not result["output_text"]:
        raise ValueError("native_text_missing")
    try:
        result["output_text"].encode("utf-8", errors="strict")
    except UnicodeEncodeError as error:
        raise ValueError("native_text_invalid_utf8") from error
    if result.get("logical_context") != request["logical_context"] or result.get("requested_max_new_tokens") != request["max_new_tokens"]:
        raise ValueError("native_request_mismatch")
    if result.get("rendered_prompt") != expected_render(request["messages"]):
        raise ValueError("native_template_golden_mismatch")
    prompt, output = result.get("prompt_tokens"), result.get("output_tokens")
    for vector in (prompt, output):
        if not isinstance(vector, list) or not vector or any(type(x) is not int or x < 0 for x in vector):
            raise ValueError("invalid_native_tokens")
    if len(prompt) + request["max_new_tokens"] > request["logical_context"]:
        raise ValueError("native_budget_mismatch")
    if len(output) > request["max_new_tokens"] or result.get("native_prompt_len") != len(prompt):
        raise ValueError("native_usage_mismatch")
    if result.get("finish_reason") not in {"native_stop", "native_max_tokens"}:
        raise ValueError("native_finish_mismatch")
    if result["finish_reason"] == "native_max_tokens" and len(output) != request["max_new_tokens"]:
        raise ValueError("native_max_tokens_mismatch")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--probe", required=True, type=Path)
    parser.add_argument("--model-dir", required=True, type=Path)
    parser.add_argument("--out-dir", required=True, type=Path)
    parser.add_argument("--case", choices=CASES, default="short-en")
    parser.add_argument("--threads", type=int, choices=range(1, 5), default=2)
    parser.add_argument("--max-new-tokens", type=int, choices=range(1, 65), default=16)
    parser.add_argument("--logical-context", type=int, default=512)
    parser.add_argument("--timeout-seconds", type=int, default=180)
    args = parser.parse_args(argv)
    if platform.system() != "Linux":
        parser.error("This harness is Linux development-only; it does not operate an Android device")
    if not 1 <= args.logical_context <= 2048 or not 1 <= args.timeout_seconds <= 600:
        parser.error("Context must be 1..2048 and timeout 1..600 seconds")
    args.out_dir.mkdir(parents=True, exist_ok=False)
    os.chmod(args.out_dir, 0o700)
    report = {"prototype": "T07-A", "status": "failed", "case": args.case,
              "android_validated": False, "exporter_commit_verified": False,
              "thread_safe_cancel": False, "per_request_sampling": False}
    code = 1
    try:
        lock = load_json(LOCK)
        _, metadata = validate_model(args.model_dir, lock)
        probe = args.probe.resolve(strict=True)
        report.update({"model_repository": lock["repository"], "model_revision": lock["revision"],
                       "model_files": lock["files"], "template_sha256": lock["template_sha256"],
                       "probe_sha256": digest(probe), "threads": args.threads,
                       "precision": "high", "sampler": "load-time-greedy"})
        request = {"messages": CASES[args.case], "max_new_tokens": args.max_new_tokens,
                   "logical_context": args.logical_context}
        # Runtime JSON may contain absolute paths. Keep it private and transient.
        with tempfile.TemporaryDirectory(prefix="nexa-mnn-probe-") as work:
            work = Path(work)
            (work / "config.json").write_text(json.dumps(runtime_config(args.model_dir, metadata,
                args.threads, args.logical_context, args.max_new_tokens)), encoding="utf-8")
            (work / "request.json").write_text(json.dumps(request, ensure_ascii=False), encoding="utf-8")
            raw = args.out_dir.resolve() / "synthetic-result.json"
            exit_code, reason = run_bounded([str(probe), str(work / "config.json"),
                str(work / "request.json"), str(raw)], work,
                args.out_dir.resolve() / "private-native.log", args.timeout_seconds)
            report["process_exit_code"] = exit_code
            if reason:
                raise ValueError(reason)
            if exit_code != 0:
                raise ValueError("native_process_failed")
            result = load_json(raw)
            validate_result(result, request)
            for key in ("mnn_commit", "probe_report_version", "target_system", "target_processor", "compiler", "runtime_backend", "finish_reason", "load_wall_us", "prepare_wall_us",
                        "generate_wall_us", "native_prefill_us", "native_decode_us"):
                report[key] = result[key]
            report.update({"status": "ok", "prompt_token_count": len(result["prompt_tokens"]),
                           "output_token_count_including_native_stop": len(result["output_tokens"]),
                           "rendered_prompt_sha256": text_digest(result["rendered_prompt"]),
                           "output_text_sha256": text_digest(result["output_text"]),
                           "prompt_tokens_sha256": text_digest(json.dumps(result["prompt_tokens"]))})
            code = 0
    except (ValueError, OSError, KeyError, TypeError) as error:
        # Full OS messages can contain user paths. Stable class/labels only in shareable report.
        report["error"] = str(error) if isinstance(error, ValueError) else type(error).__name__
    (args.out_dir / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"status": report["status"], "error": report.get("error")}, ensure_ascii=False))
    return code


if __name__ == "__main__":
    raise SystemExit(main())
