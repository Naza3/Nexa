//! Developer verification only. Never claims Windows/Android acceptance on a Linux host.
mod api_smoke;
mod gguf;
mod smoke;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{SystemTime, UNIX_EPOCH};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
const HELP: &str = "Nexa verification tools (run from any directory)\n\
  cargo run --locked -p xtask -- baseline-verify --model PATH --out REPORT.json\n\
  cargo run --locked -p xtask -- native-smoke --model PATH --bin PATH --out REPORT.json\n\
Options: --manifest PATH (default tests/fixtures/baseline.json), --device LABEL\n\
Native smoke only: --timeout-seconds N (per case, default 180), --threads N (1..=256)\n\
Threads: --threads > NEXA_TEST_THREADS > min(4, available_parallelism)\n\
Exit codes: 0 verified requested scope; 1 verification failed; 2 invalid command/input.\n\
Baseline verification checks metadata and hashes, not inference or platform acceptance.\n\
api-smoke --base-url URL --data-dir PATH --model ID --out REPORT [--disconnect-cycles 1..50] (T04; ends by shutting down the test instance).\n\
check, test --suite contract and build are not implemented yet.";

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Baseline {
    schema_version: u32,
    llama_commit: String,
    rust_toolchain: String,
    model: Model,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Model {
    id: String,
    source: String,
    revision: String,
    filename: String,
    sha256: String,
    architecture: String,
    quantization: String,
    gguf_file_type: u32,
    license: String,
    license_url: String,
    chat_template_sha256: String,
    context_size: u32,
}

struct Options {
    command: String,
    manifest: PathBuf,
    model: PathBuf,
    output: PathBuf,
    binary: Option<PathBuf>,
    device: String,
    timeout_seconds: u64,
    threads: Option<ThreadSelection>,
}

#[derive(Debug, Serialize)]
struct ThreadSelection {
    threads: u32,
    available_parallelism: usize,
    oversubscribed: bool,
    source: &'static str,
}

fn select_threads(
    cli: Option<&OsStr>,
    environment: Option<&OsStr>,
    available: usize,
) -> Result<ThreadSelection> {
    let (raw, source) = if let Some(value) = cli {
        (Some(value), "cli")
    } else if let Some(value) = environment {
        (Some(value), "NEXA_TEST_THREADS")
    } else {
        (None, "available_parallelism")
    };
    let threads = match raw {
        Some(value) => value
            .to_str()
            .ok_or("test threads must be UTF-8")?
            .parse::<u32>()
            .map_err(|_| "test threads must be an integer in 1..=256")?,
        None => available.clamp(1, 4) as u32,
    };
    if !(1..=256).contains(&threads) {
        return Err("test threads must be in 1..=256".into());
    }
    Ok(ThreadSelection {
        threads,
        available_parallelism: available,
        oversubscribed: threads as usize > available,
        source,
    })
}

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.is_empty() || args == ["--help"] || args == ["-h"] || args == ["help"] {
        println!("{HELP}");
        return ExitCode::SUCCESS;
    }
    if args.first().is_some_and(|value| value == "api-smoke") {
        return api_smoke::main(&args[1..]);
    }
    let options = match parse(args) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("{error}\n\n{HELP}");
            return ExitCode::from(2);
        }
    };
    match execute(&options) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("verification could not run: {error}");
            ExitCode::from(2)
        }
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask is a workspace member")
        .to_path_buf()
}

fn parse(args: Vec<OsString>) -> Result<Options> {
    let command = args[0].to_str().ok_or("command must be UTF-8")?;
    if !matches!(command, "baseline-verify" | "native-smoke") {
        return Err(
            format!("unsupported command: {command}; no verification was performed").into(),
        );
    }
    let mut pairs = BTreeMap::new();
    for pair in args[1..].chunks(2) {
        if pair.len() != 2 {
            return Err("every option requires a value".into());
        }
        let key = pair[0].to_str().ok_or("option must be UTF-8")?;
        if !matches!(
            key,
            "--model"
                | "--manifest"
                | "--out"
                | "--bin"
                | "--device"
                | "--timeout-seconds"
                | "--threads"
        ) {
            return Err(format!("unknown option: {key}").into());
        }
        if pairs.insert(key.to_owned(), pair[1].clone()).is_some() {
            return Err(format!("duplicate option: {key}").into());
        }
    }
    if command == "baseline-verify"
        && (pairs.contains_key("--bin")
            || pairs.contains_key("--timeout-seconds")
            || pairs.contains_key("--threads"))
    {
        return Err("--bin, --threads, and --timeout-seconds require native-smoke".into());
    }
    let model = pairs.remove("--model").ok_or("--model is required")?;
    let output = pairs.remove("--out").ok_or("--out is required")?;
    let binary = pairs.remove("--bin").map(PathBuf::from);
    if command == "native-smoke" && binary.is_none() {
        return Err(
            "native-smoke requires --bin pointing to the built native-smoke example".into(),
        );
    }
    let timeout_seconds = match pairs.remove("--timeout-seconds") {
        Some(value) => value.to_str().ok_or("invalid timeout")?.parse()?,
        None => 180,
    };
    if !(1..=3600).contains(&timeout_seconds) {
        return Err("timeout must be between 1 and 3600 seconds".into());
    }
    let threads = if command == "native-smoke" {
        let cli_threads = pairs.remove("--threads");
        let environment = std::env::var_os("NEXA_TEST_THREADS");
        let available = std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1);
        Some(select_threads(
            cli_threads.as_deref(),
            environment.as_deref(),
            available,
        )?)
    } else {
        None
    };
    Ok(Options {
        command: command.to_owned(),
        model: model.into(),
        output: output.into(),
        binary,
        manifest: pairs
            .remove("--manifest")
            .map(PathBuf::from)
            .unwrap_or_else(|| root().join("tests/fixtures/baseline.json")),
        device: pairs
            .remove("--device")
            .map(|s| s.into_string().map_err(|_| "device label must be UTF-8"))
            .transpose()?
            .unwrap_or_else(|| "unavailable".into()),
        timeout_seconds,
        threads,
    })
}

fn execute(options: &Options) -> Result<bool> {
    if let Ok(output) = options.output.canonicalize() {
        for input in [&options.model, &options.manifest]
            .into_iter()
            .chain(options.binary.iter())
        {
            if input.canonicalize().ok().as_ref() == Some(&output) {
                return Err("report output must differ from all input files".into());
            }
        }
    }
    let baseline: Baseline = serde_json::from_reader(
        File::open(&options.manifest).map_err(|_| "cannot read baseline manifest")?,
    )?;
    validate_manifest(&baseline)?;
    let mut checks = Vec::new();
    let vendor_commit = command_text("git", &["-C", "vendor/llama.cpp", "rev-parse", "HEAD"]);
    checks.push(compare(
        "llama_commit",
        &vendor_commit,
        &baseline.llama_commit,
    ));
    let vendor_dirty = command_text(
        "git",
        &[
            "-C",
            "vendor/llama.cpp",
            "status",
            "--porcelain",
            "--untracked-files=no",
        ],
    );
    checks.push(compare("llama_tracked_tree_clean", &vendor_dirty, ""));
    let rustc = command_text("rustc", &["--version"]);
    checks.push(check(
        "rust_toolchain",
        rustc.split_whitespace().nth(1) == Some(&baseline.rust_toolchain),
        json!({"expected":baseline.rust_toolchain,"actual":rustc}),
    ));
    let toolchain_file = fs::read_to_string(root().join("rust-toolchain.toml"))?;
    checks.push(check(
        "rust_toolchain_file",
        toolchain_file
            .lines()
            .any(|line| line.trim() == format!("channel = \"{}\"", baseline.rust_toolchain)),
        Value::Null,
    ));
    let model_hash = hash_file(&options.model).map_err(|_| "cannot read model file")?;
    checks.push(compare("model_sha256", &model_hash, &baseline.model.sha256));
    match gguf::read(File::open(&options.model).map_err(|_| "cannot open model file")?) {
        Ok(metadata) => {
            checks.push(compare(
                "gguf_architecture",
                &metadata.architecture,
                &baseline.model.architecture,
            ));
            checks.push(check(
                "gguf_file_type",
                metadata.file_type == baseline.model.gguf_file_type,
                json!({"actual":metadata.file_type,"expected":baseline.model.gguf_file_type}),
            ));
            checks.push(compare(
                "chat_template_sha256",
                &hash(metadata.template.as_bytes()),
                &baseline.model.chat_template_sha256,
            ));
            checks.push(check(
                "context_size",
                baseline.model.context_size <= metadata.context_length,
                json!({"requested":baseline.model.context_size,"trained":metadata.context_length}),
            ));
        }
        Err(error) => checks.push(check(
            "gguf_metadata",
            false,
            json!({"error":error.to_string()}),
        )),
    }
    let prerequisites_passed = passed(&checks);
    let mut fixtures = BTreeMap::new();
    for name in [
        "english.txt",
        "chinese.txt",
        "summary-long.txt",
        "upstream-prompt-zh.txt",
    ] {
        fixtures.insert(name, hash_file(&root().join("tests/fixtures").join(name))?);
    }
    let smoke = if options.command == "native-smoke" && prerequisites_passed {
        smoke::run(options, &baseline)?
    } else {
        Vec::new()
    };
    let successful = prerequisites_passed && (options.command != "native-smoke" || passed(&smoke));
    let project_commit = command_text("git", &["rev-parse", "HEAD"]);
    let working_tree = command_text("git", &["status", "--porcelain"]);
    let report = json!({
        "schema_version":1,
        "command":{"program":"cargo run --locked -p xtask --","subcommand":options.command,
            "model":"baseline.model (local path omitted)","manifest_sha256":hash_file(&options.manifest)?,
            "output":"local report path omitted", "threads":options.threads.as_ref().map(|s| s.threads)},
        "exit_code":if successful {0} else {1},
        "scope":options.command,
        "result":if successful {"pass"} else {"fail"},
        "generated_at_unix_seconds":SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        "project":{"commit":project_commit,"working_tree_dirty":!working_tree.is_empty()},
        "llama_commit":vendor_commit,
        "toolchain":{"rustc":rustc,"cargo":command_text("cargo", &["--version"]),
            "cargo_lock_sha256":hash_file(&root().join("Cargo.lock"))?},
        "host":{"os":std::env::consts::OS,"arch":std::env::consts::ARCH,"device":options.device,
            "os_version":host_os_version(),"gpu_driver":"unavailable","backend":"cpu"},
        "baseline":baseline,
        "model_sha256":model_hash,
        "manifest_sha256":hash_file(&options.manifest)?,
        "fixture_sha256":fixtures,
        "checks":checks,
        "native_smoke":smoke,
        "thread_options":options.threads,
        "inference":if options.command == "native-smoke" && prerequisites_passed {"attempted"} else {"skipped"},
        "performance":{"peak_memory_bytes":"unavailable","release_memory_bytes":"unavailable",
            "ttft_ms":"unavailable","decode_tokens_per_second":"unavailable"},
        "acceptance":{"windows_x64_cpu":"skipped","android_arm64_cpu":"skipped",
            "T00":"not_evaluated","T01":"not_evaluated"},
        "limitations":["Host-only developer verification; target-device acceptance remains required.",
            "Metadata/hash verification does not prove inference quality or GGUF loader safety.",
            "No HTTP, worker isolation, sustained memory, or performance acceptance is performed.",
            "Mid-prefill cancellation, 100-request endurance, 20-load cycles, and memory release metrics are not tested by this command.",
            "Report omits local model paths, prompt text, generated text, and child stderr content."]
    });
    write_report(&options.output, &report)?;
    println!(
        "{}: {} (report written)",
        options.command,
        if successful { "pass" } else { "fail" }
    );
    Ok(successful)
}

fn validate_manifest(baseline: &Baseline) -> Result<()> {
    if baseline.schema_version != 1 || !hex(&baseline.llama_commit, 40) {
        return Err("invalid baseline schema or llama commit".into());
    }
    let parts: Vec<_> = baseline.rust_toolchain.split('.').collect();
    if parts.len() != 3
        || parts
            .iter()
            .any(|s| s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err("rust toolchain must be an exact version".into());
    }
    let model = &baseline.model;
    // This first baseline supports only the actually tested quantization mapping.
    if model.quantization != "Q8_0" || model.gguf_file_type != 7 {
        return Err(
            "baseline quantization mapping is not supported (expected Q8_0 / type 7)".into(),
        );
    }
    if !hex(&model.revision, 40) || !hex(&model.sha256, 64) || !hex(&model.chat_template_sha256, 64)
    {
        return Err("model revision or SHA-256 is invalid".into());
    }
    if !model.source.starts_with("https://")
        || !model.license_url.starts_with("https://")
        || !model.license_url.contains(&model.revision)
        || model.context_size == 0
        || [
            &model.id,
            &model.filename,
            &model.architecture,
            &model.quantization,
            &model.license,
        ]
        .iter()
        .any(|s| s.trim().is_empty())
    {
        return Err("model metadata is incomplete or license URL is not revision-pinned".into());
    }
    Ok(())
}

fn hex(text: &str, length: usize) -> bool {
    text.len() == length
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn check(name: &str, pass: bool, details: Value) -> Value {
    json!({"name":name,"status":if pass {"pass"} else {"fail"},"details":details})
}

fn compare(name: &str, actual: &str, expected: &str) -> Value {
    check(
        name,
        actual == expected,
        json!({"actual":actual,"expected":expected}),
    )
}

fn passed(checks: &[Value]) -> bool {
    !checks.is_empty() && checks.iter().all(|item| item["status"] == "pass")
}

fn command_text(program: &str, args: &[&str]) -> String {
    Command::new(program)
        .args(args)
        .current_dir(root())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|text| text.trim().to_owned())
        .unwrap_or_else(|| "unavailable".into())
}

fn host_os_version() -> String {
    #[cfg(unix)]
    {
        command_text("uname", &["-sr"])
    }
    #[cfg(windows)]
    {
        command_text("cmd", &["/C", "ver"])
    }
    #[cfg(not(any(unix, windows)))]
    {
        "unavailable".into()
    }
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn hash_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut bytes = [0; 128 * 1024];
    loop {
        let count = file.read(&mut bytes)?;
        if count == 0 {
            break;
        }
        digest.update(&bytes[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn write_report(path: &Path, report: &Value) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!("json.{}.tmp", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let result = (|| -> Result<()> {
        serde_json::to_writer_pretty(&mut file, report)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }
    #[test]
    fn refuses_unimplemented_and_ambiguous_commands() {
        assert!(parse(args(&["api-smoke", "--model", "x", "--out", "y"])).is_err());
        assert!(parse(args(&["baseline-verify", "--out", "y"])).is_err());
        assert!(
            parse(args(&[
                "baseline-verify",
                "--model",
                "x",
                "--out",
                "y",
                "--out",
                "z"
            ]))
            .is_err()
        );
        assert!(parse(args(&["native-smoke", "--model", "x", "--out", "y"])).is_err());
        assert!(
            parse(args(&[
                "baseline-verify",
                "--model",
                "x",
                "--out",
                "y",
                "--bin",
                "z"
            ]))
            .is_err()
        );
    }
    #[test]
    fn thread_defaults_follow_available_cpu_limit() {
        for (available, expected) in [(1, 1), (2, 2), (4, 4), (16, 4)] {
            let selection = select_threads(None, None, available).unwrap();
            assert_eq!(selection.threads, expected);
            assert_eq!(selection.available_parallelism, available);
            assert!(!selection.oversubscribed);
            assert_eq!(selection.source, "available_parallelism");
        }
    }
    #[test]
    fn thread_precedence_preserves_explicit_oversubscription() {
        let cli = select_threads(Some(OsStr::new("4")), Some(OsStr::new("invalid")), 2).unwrap();
        assert_eq!(cli.threads, 4);
        assert!(cli.oversubscribed);
        assert_eq!(cli.source, "cli");
        let environment = select_threads(None, Some(OsStr::new("2")), 4).unwrap();
        assert_eq!(environment.threads, 2);
        assert_eq!(environment.source, "NEXA_TEST_THREADS");
        assert!(!environment.oversubscribed);
        assert_eq!(
            select_threads(Some(OsStr::new("256")), None, 2)
                .unwrap()
                .threads,
            256
        );
    }
    #[test]
    fn thread_options_reject_out_of_range_and_non_integer_values() {
        for value in ["0", "257", "-1", "1.5", "", "no", "4294967296"] {
            assert!(
                select_threads(Some(OsStr::new(value)), None, 2).is_err(),
                "{value}"
            );
            assert!(
                select_threads(None, Some(OsStr::new(value)), 2).is_err(),
                "{value}"
            );
        }
        assert!(
            parse(args(&[
                "baseline-verify",
                "--model",
                "x",
                "--out",
                "y",
                "--threads",
                "2"
            ]))
            .is_err()
        );
        assert!(
            parse(args(&[
                "native-smoke",
                "--model",
                "x",
                "--out",
                "y",
                "--bin",
                "z",
                "--threads",
                "0"
            ]))
            .is_err()
        );
        let options = parse(args(&[
            "native-smoke",
            "--model",
            "x",
            "--out",
            "y",
            "--bin",
            "z",
            "--threads",
            "2",
        ]))
        .unwrap();
        assert_eq!(options.threads.unwrap().threads, 2);
    }
    #[test]
    fn strict_manifest_and_hash_validation() {
        let mut baseline: Baseline =
            serde_json::from_str(include_str!("../../tests/fixtures/baseline.json")).unwrap();
        validate_manifest(&baseline).unwrap();
        baseline.model.sha256 = "unavailable".into();
        assert!(validate_manifest(&baseline).is_err());
        assert_eq!(
            hash(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
    #[test]
    fn an_empty_or_skipped_suite_never_passes() {
        assert!(!passed(&[]));
        assert!(!passed(&[json!({"status":"skipped"})]));
        assert!(passed(&[check("real", true, Value::Null)]));
    }
}
