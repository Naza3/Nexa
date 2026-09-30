use crate::{Baseline, Options, Result, check, hash_file, json, root};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const OUTPUT_LIMIT: usize = 64 * 1024;

pub fn run(options: &Options, baseline: &Baseline) -> Result<Vec<Value>> {
    let binary = options.binary.as_ref().ok_or("missing smoke binary")?;
    let binary = binary
        .canonicalize()
        .map_err(|_| "cannot open smoke binary")?;
    let binary_hash = hash_file(&binary)?;
    let model = options
        .model
        .canonicalize()
        .map_err(|_| "cannot open model file")?;
    let cases = [
        ("english_reload", "english.txt", "generate", 64, 2),
        ("chinese", "chinese.txt", "generate", 64, 1),
        ("long_summary", "summary-long.txt", "generate", 128, 1),
        ("token_budget", "english.txt", "budget", 1, 1),
        (
            "cancel_before_load",
            "english.txt",
            "cancel-before-load",
            64,
            1,
        ),
        (
            "cancel_before_prepare",
            "english.txt",
            "cancel-before-prepare",
            64,
            1,
        ),
        (
            "cancel_before_generate",
            "english.txt",
            "cancel-before-generate",
            64,
            1,
        ),
        ("cancel_during", "english.txt", "cancel-during", 64, 1),
        ("consumer_stop", "english.txt", "consumer-stop", 64, 1),
    ];
    let mut results = Vec::new();
    for (index, (name, fixture, mode, max_tokens, repeat)) in cases.into_iter().enumerate() {
        let arguments = [
            "--mode".into(),
            mode.to_owned(),
            "--max-tokens".into(),
            max_tokens.to_string(),
            "--context-size".into(),
            baseline.model.context_size.to_string(),
            "--repeat".into(),
            repeat.to_string(),
        ];
        let mut command = Command::new(&binary);
        command
            .args(&arguments)
            .arg("--model")
            .arg(&model)
            .arg("--prompt-file")
            .arg(root().join("tests/fixtures").join(fixture))
            .current_dir(root())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let begin = Instant::now();
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(_) => {
                results.push(check(
                    name,
                    false,
                    json!({"error":"failed to start smoke binary"}),
                ));
                for (pending, ..) in cases.iter().skip(index + 1) {
                    results.push(json!({"name":pending,"status":"skipped",
                        "details":{"reason":"smoke binary could not be started"}}));
                }
                break;
            }
        };
        let stdout = child.stdout.take().ok_or("missing child stdout")?;
        let stderr = child.stderr.take().ok_or("missing child stderr")?;
        let out_reader = thread::spawn(move || collect(stdout));
        let err_reader = thread::spawn(move || collect(stderr));
        let (status, timed_out) = loop {
            if let Some(status) = child.try_wait()? {
                break (status, false);
            }
            if begin.elapsed() >= Duration::from_secs(options.timeout_seconds) {
                // The process may exit between try_wait and kill. Reap that race
                // without losing the failure report or waiting on a live process.
                if let Err(error) = child.kill() {
                    if let Some(status) = child.try_wait()? {
                        break (status, true);
                    }
                    return Err(error.into());
                }
                break (child.wait()?, true);
            }
            thread::sleep(Duration::from_millis(25));
        };
        let stdout = out_reader.join().map_err(|_| "stdout reader panicked")??;
        let stderr = err_reader.join().map_err(|_| "stderr reader panicked")??;
        let parsed = validate_output(
            &stdout.bytes,
            &baseline.llama_commit,
            &baseline.model.chat_template_sha256,
            mode,
            repeat,
            max_tokens,
            baseline.model.context_size,
        );
        let binary_unchanged = hash_file(&binary).ok().as_ref() == Some(&binary_hash);
        let valid = status.success()
            && !timed_out
            && !stdout.truncated
            && parsed.is_ok()
            && binary_unchanged;
        let (summary, error) = match parsed {
            Ok(summary) => (summary, Value::Null),
            Err(error) => (Value::Null, json!(error.to_string())),
        };
        eprintln!(
            "native case {name}: {}",
            if valid { "pass" } else { "fail" }
        );
        results.push(check(name, valid, json!({
            "command":{"executable":"native-smoke","binary_sha256":binary_hash,"args":arguments,
                "model":"baseline.model (local path omitted)","fixture":format!("tests/fixtures/{fixture}")},
            "exit_code":status.code(),"timed_out":timed_out,"elapsed_ms":begin.elapsed().as_millis(),
            "stdout_sha256":stdout.sha256,"stderr_sha256":stderr.sha256,
            "stdout_truncated":stdout.truncated,"stderr_truncated":stderr.truncated,
            "observations":summary["observations"],"load_options":summary["load_options"],
            "generation_options":summary["generation_options"],"binary_unchanged":binary_unchanged,"error":error
        })));
    }
    Ok(results)
}

struct Captured {
    bytes: Vec<u8>,
    sha256: String,
    truncated: bool,
}

// Continue draining after the limit so verbose native diagnostics cannot deadlock the child.
fn collect(mut reader: impl Read) -> std::io::Result<Captured> {
    let mut bytes = Vec::new();
    let mut digest = Sha256::new();
    let mut buffer = [0; 8192];
    let mut truncated = false;
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
        let keep = count.min(OUTPUT_LIMIT - bytes.len());
        bytes.extend_from_slice(&buffer[..keep]);
        truncated |= keep != count;
    }
    Ok(Captured {
        bytes,
        sha256: format!("{:x}", digest.finalize()),
        truncated,
    })
}

fn validate_output(
    bytes: &[u8],
    commit: &str,
    template_sha256: &str,
    mode: &str,
    repeat: u32,
    max_tokens: u32,
    context_size: u32,
) -> Result<Value> {
    let text = std::str::from_utf8(bytes).map_err(|_| "smoke stdout is not UTF-8")?;
    let mut records: Vec<Value> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(serde_json::from_str)
        .collect::<std::result::Result<_, _>>()
        .map_err(|_| "smoke stdout contains invalid JSON")?;
    let summary = records.pop().ok_or("smoke emitted no final result")?;
    if summary["result"] != "pass" || summary["runs"].as_u64() != Some(repeat.into()) {
        return Err("smoke final result is missing or failed".into());
    }
    if summary["build_info"]["llama_commit"] != commit
        || summary["build_info"]["backend"] != "cpu"
        || summary["build_info"]["shim_version"] != 1
    {
        return Err("smoke binary build identity differs from baseline".into());
    }
    if mode != "cancel-before-load" && summary["template_sha256"] != template_sha256 {
        return Err("smoke loaded template differs from baseline".into());
    }
    if records.len() != repeat as usize {
        return Err("smoke iteration count differs from requested repetitions".into());
    }
    let mut safe_records = Vec::new();
    for (index, record) in records.iter().enumerate() {
        let prompt = record["prompt_tokens"]
            .as_u64()
            .ok_or("missing prompt token count")?;
        let completion = record["completion_tokens"]
            .as_u64()
            .ok_or("missing completion token count")?;
        let text_bytes = record["text_bytes"]
            .as_u64()
            .ok_or("missing text byte count")?;
        let callbacks = record["callbacks"]
            .as_u64()
            .ok_or("missing callback count")?;
        if record["iteration"].as_u64() != Some(index as u64 + 1)
            || record["mode"] != mode
            || completion > max_tokens.into()
            || prompt > context_size.into()
            || prompt + completion > context_size.into()
        {
            return Err("smoke iteration, mode, or token budget is invalid".into());
        }
        let reason = record["finish_reason"]
            .as_str()
            .ok_or("missing finish reason")?;
        let valid = match mode {
            "generate" => {
                prompt > 0
                    && text_bytes > 0
                    && callbacks > 0
                    && matches!(reason, "stop" | "length")
                    && prompt + u64::from(max_tokens) <= context_size.into()
            }
            "budget" => completion == 1 && reason == "length",
            "cancel-before-load" | "cancel-before-prepare" | "cancel-before-generate" => {
                reason == "cancelled" && completion == 0 && text_bytes == 0 && callbacks == 0
            }
            "cancel-during" => reason == "cancelled" && callbacks > 0,
            "consumer-stop" => reason == "consumer_stopped" && callbacks == 1,
            _ => false,
        };
        if !valid {
            return Err("smoke observation does not satisfy its case".into());
        }
        // Select known scalar fields: never copy arbitrary child content into reports.
        safe_records.push(json!({"iteration":index + 1,"mode":mode,"prompt_tokens":prompt,
            "completion_tokens":completion,"text_bytes":text_bytes,"callbacks":callbacks,"finish_reason":reason}));
    }
    let load = &summary["load_options"];
    let generation = &summary["generation_options"];
    if load["context_size"] != context_size
        || load["threads"] != 4
        || load["batch_size"] != 512_u32.min(context_size)
        || generation["max_tokens"] != max_tokens
        || generation["seed"] != 42
        || generation["stop_count"] != 0
        || generation["temperature"].as_f64() != Some(0.0)
        || generation["top_p"]
            .as_f64()
            .is_none_or(|n| (n - 0.9).abs() > 0.000001)
    {
        return Err("smoke load or sampling parameters differ from fixed test settings".into());
    }
    Ok(json!({"observations":safe_records,
        "load_options":{"context_size":context_size,"threads":4,"batch_size":512_u32.min(context_size)},
        "generation_options":{"max_tokens":max_tokens,"seed":42,"stop_count":0,
            "temperature":generation["temperature"].as_f64(),"top_p":generation["top_p"].as_f64()}
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn report(reason: &str) -> Vec<u8> {
        let observation = json!({"iteration":1,"mode":"generate","prompt_tokens":20,
            "completion_tokens":5,"text_bytes":10,"callbacks":3,"finish_reason":reason});
        let summary = json!({"result":"pass","runs":1,
            "build_info":{"llama_commit":"abc","backend":"cpu","shim_version":1},
            "template_sha256":"template","load_options":{"context_size":64,"threads":4,"batch_size":64},
            "generation_options":{"max_tokens":8,"seed":42,"stop_count":0,"temperature":0.0,"top_p":0.9}});
        format!("{observation}\n{summary}\n").into_bytes()
    }
    #[test]
    fn checks_observations_instead_of_trusting_exit_or_pass_label() {
        assert!(validate_output(&report("stop"), "abc", "template", "generate", 1, 8, 64).is_ok());
        assert!(
            validate_output(
                &report("cancelled"),
                "abc",
                "template",
                "generate",
                1,
                8,
                64
            )
            .is_err()
        );
        assert!(
            validate_output(
                &report("stop"),
                "different",
                "template",
                "generate",
                1,
                8,
                64
            )
            .is_err()
        );
        assert!(validate_output(&report("stop"), "abc", "template", "generate", 1, 4, 64).is_err());
        assert!(validate_output(&report("stop"), "abc", "template", "generate", 2, 8, 64).is_err());
        assert!(validate_output(b"", "abc", "template", "generate", 1, 8, 64).is_err());
    }
    #[test]
    fn rejects_unbounded_counts_without_overflow() {
        let text = String::from_utf8(report("stop")).unwrap().replace(
            "\"prompt_tokens\":20",
            "\"prompt_tokens\":18446744073709551615",
        );
        assert!(validate_output(text.as_bytes(), "abc", "template", "generate", 1, 8, 64).is_err());
    }
    #[test]
    fn report_whitelists_fields_instead_of_copying_child_text() {
        let text = String::from_utf8(report("stop")).unwrap().replace(
            "\"callbacks\":3",
            "\"prompt\":\"private input sentinel\",\"callbacks\":3",
        );
        let safe =
            validate_output(text.as_bytes(), "abc", "template", "generate", 1, 8, 64).unwrap();
        assert!(!safe.to_string().contains("private input sentinel"));
    }
    #[test]
    fn bounded_reader_drains_and_hashes_entire_stream() {
        let bytes = vec![b'a'; OUTPUT_LIMIT + 100];
        let captured = collect(bytes.as_slice()).unwrap();
        assert_eq!(captured.bytes.len(), OUTPUT_LIMIT);
        assert!(captured.truncated);
        assert_eq!(captured.sha256, crate::hash(&bytes));
    }
}
