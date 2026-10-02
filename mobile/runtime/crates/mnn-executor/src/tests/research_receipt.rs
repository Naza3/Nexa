//! Test-only CI research evidence. Not authentication or product admission.
use mnn_adapter::BuildIdentity;
use serde::Deserialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX: u64 = 65536;
const STAGES: [&str; 7] = [
    "tools",
    "inputs",
    "patch",
    "baseline",
    "linux_native",
    "native_real",
    "rust_linux",
];
const LOCK: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../../native/mnn-patches/lock.json"
));
const INPUT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../../scripts/android_mnn/candidate-model.json"
));
const HEADER: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../../native/mnn-shim/include/nexa_mnn.h"
));
const NOTICES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../../native/mnn-shim/notices/manifest.json"
));
type Result<T> = std::result::Result<T, ()>;
fn need(value: bool) -> Result<()> {
    if value { Ok(()) } else { Err(()) }
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn hex(value: &str, count: usize) -> bool {
    value.len() == count
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

struct Unique(Value);
impl<'de> Deserialize<'de> for Unique {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Unique;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("strict JSON")
            }
            fn visit_bool<E: serde::de::Error>(self, v: bool) -> std::result::Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> std::result::Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> std::result::Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> std::result::Result<Unique, E> {
                serde_json::Number::from_f64(v)
                    .map(|n| Unique(n.into()))
                    .ok_or_else(|| E::custom("invalid number"))
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> std::result::Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_string<E: serde::de::Error>(
                self,
                v: String,
            ) -> std::result::Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_none<E: serde::de::Error>(self) -> std::result::Result<Unique, E> {
                Ok(Unique(Value::Null))
            }
            fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Unique, E> {
                Ok(Unique(Value::Null))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut a: A,
            ) -> std::result::Result<Unique, A::Error> {
                let mut values = Vec::new();
                while let Some(v) = a.next_element::<Unique>()? {
                    values.push(v.0);
                }
                Ok(Unique(Value::Array(values)))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut a: A,
            ) -> std::result::Result<Unique, A::Error> {
                let mut map = Map::new();
                while let Some((k, v)) = a.next_entry::<String, Unique>()? {
                    if map.insert(k, v.0).is_some() {
                        return Err(serde::de::Error::custom("duplicate key"));
                    }
                }
                Ok(Unique(Value::Object(map)))
            }
        }
        d.deserialize_any(Visitor)
    }
}
fn parse<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    need(bytes.len() <= MAX as usize)?;
    let unique: Unique = serde_json::from_slice(bytes).map_err(|_| ())?;
    serde_json::from_value(unique.0).map_err(|_| ())
}
fn bytes(path: &Path) -> Result<Vec<u8>> {
    need(path.is_absolute() && path.canonicalize().map_err(|_| ())? == path)?;
    let before = fs::symlink_metadata(path).map_err(|_| ())?;
    need(
        before.is_file()
            && before.nlink() == 1
            && before.len() <= MAX
            && before.mode() & 0o222 == 0,
    )?;
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| ())?;
    let actual = file.metadata().map_err(|_| ())?;
    need(actual.dev() == before.dev() && actual.ino() == before.ino())?;
    let mut data = Vec::new();
    (&mut file)
        .take(MAX + 1)
        .read_to_end(&mut data)
        .map_err(|_| ())?;
    let after = file.metadata().map_err(|_| ())?;
    let current = fs::symlink_metadata(path).map_err(|_| ())?;
    need(data.len() as u64 == before.len() && data.len() <= MAX as usize)?;
    need(
        after.dev() == before.dev()
            && after.ino() == before.ino()
            && after.len() == before.len()
            && after.mtime_nsec() == before.mtime_nsec()
            && after.mtime() == before.mtime()
            && after.ctime_nsec() == before.ctime_nsec()
            && after.ctime() == before.ctime()
            && current.dev() == after.dev()
            && current.ino() == after.ino(),
    )?;
    Ok(data)
}

#[derive(Deserialize, Clone, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
struct Context {
    mode: String,
    source_commit: String,
    source_tree: String,
    source_clean: bool,
    source_snapshot_sha256: String,
    run_id: String,
    run_attempt: String,
    job: String,
    context_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Subject {
    artifact_manifest_sha256: String,
    upstream_commit: String,
    patch_set_sha256: String,
    policy_sha256: String,
    header_sha256: String,
    target: String,
    compiler: String,
    compiler_sha256: String,
    profile: String,
    input_lock_sha256: String,
    candidate_identity_sha256: String,
    template_sha256: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Proof {
    stage: String,
    sha256: String,
    outcome: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    schema_version: u32,
    purpose: String,
    research_only: bool,
    production_admitted: bool,
    android_run: bool,
    context: Context,
    issued_at: u64,
    expires_at: u64,
    subject: Subject,
    prerequisites: Vec<Proof>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Command {
    index: usize,
    category: String,
    timeout_seconds: f64,
    timed_out: bool,
    log_limit_exceeded: bool,
    cleanup_confirmed: bool,
    failure_case: String,
    exit_code: i32,
    log_bytes: u64,
    output_truncated: bool,
    diagnostics: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Report {
    schema: u32,
    context: Context,
    stage: String,
    status: String,
    android_run: bool,
    exit_codes: Vec<i32>,
    commands: Vec<Command>,
    failure_case: String,
    missing_archive: Option<String>,
    checks: Value,
}
struct Environment {
    ci: bool,
    sha: String,
    run: String,
    attempt: String,
    job: String,
    expected_context: String,
}
impl Environment {
    fn current() -> Self {
        let get = |k| std::env::var(k).unwrap_or_default();
        Self {
            ci: get("GITHUB_ACTIONS") == "true",
            sha: get("GITHUB_SHA"),
            run: get("GITHUB_RUN_ID"),
            attempt: get("GITHUB_RUN_ATTEMPT"),
            job: get("GITHUB_JOB"),
            expected_context: get("NEXA_MNN_B2_CONTEXT"),
        }
    }
}
fn keys(v: &Value, names: &[&str]) -> Result<()> {
    let map = v.as_object().ok_or(())?;
    need(map.len() == names.len() && names.iter().all(|n| map.contains_key(*n)))
}
fn yes(v: &Value, name: &str) -> Result<()> {
    need(v[name].as_bool() == Some(true))
}
fn positive(v: &Value, name: &str) -> Result<()> {
    need(v[name].as_u64().is_some_and(|n| n > 0))
}
fn report_checks(name: &str, v: &Value, r: &Receipt) -> Result<()> {
    let s = &r.subject;
    match name {
        "tools" => {
            keys(
                v,
                &[
                    "source",
                    "gcc",
                    "rust",
                    "cmake",
                    "ninja",
                    "notice_inventory",
                ],
            )?;
            keys(
                &v["source"],
                &["source_commit", "source_tree", "source_clean"],
            )?;
            need(
                v["source"]["source_commit"] == r.context.source_commit
                    && v["source"]["source_tree"] == r.context.source_tree
                    && v["source"]["source_clean"] == r.context.source_clean,
            )?;
            need(
                v["gcc"]
                    == if r.context.mode == "github-ci" {
                        "13.3.0"
                    } else {
                        "14.2.0"
                    }
                    && v["rust"] == "1.98.1"
                    && v["cmake"] == "4.4.3"
                    && v["ninja"] == "1.13.2",
            )?;
            keys(
                &v["notice_inventory"],
                &[
                    "components",
                    "files",
                    "manifest_sha256",
                    "final_apk_verified",
                ],
            )?;
            need(v["notice_inventory"]["manifest_sha256"] == hash(NOTICES))?;
            need(
                v["notice_inventory"]["components"] == 12
                    && v["notice_inventory"]["final_apk_verified"] == false,
            )?;
            positive(&v["notice_inventory"], "files")
        }
        "inputs" => {
            keys(v, &["ndk_sha256", "ndk_revision", "model_lock_sha256"])?;
            need(
                v["model_lock_sha256"] == s.input_lock_sha256
                    && v["ndk_revision"] == "30.0.16248370"
                    && v["ndk_sha256"]
                        == "753611f410d002cfcd3f3dc2ef49aad532089d3180b436c060a90bf0fcb64df2",
            )
        }
        "patch" => {
            keys(v, &["pristine_source_unchanged"])?;
            yes(v, "pristine_source_unchanged")
        }
        "baseline" => {
            keys(v, &["unpatched_probe_ctest"])?;
            yes(v, "unpatched_probe_ctest")
        }
        "linux_native" => {
            keys(
                v,
                &["artifact", "logging_audit", "ctest_and_stream_sanitizers"],
            )?;
            yes(v, "ctest_and_stream_sanitizers")?;
            keys(
                &v["logging_audit"],
                &["compiled_source_count", "header_count"],
            )?;
            positive(&v["logging_audit"], "compiled_source_count")?;
            positive(&v["logging_audit"], "header_count")?;
            let a = &v["artifact"];
            keys(
                a,
                &[
                    "manifest_sha256",
                    "compiler_sha256",
                    "upstream_commit",
                    "patch_set_sha256",
                    "policy_sha256",
                    "header_sha256",
                    "target",
                    "archives",
                    "archive_object_counts",
                ],
            )?;
            need(
                a["manifest_sha256"] == s.artifact_manifest_sha256
                    && a["compiler_sha256"] == s.compiler_sha256
                    && a["upstream_commit"] == s.upstream_commit
                    && a["patch_set_sha256"] == s.patch_set_sha256
                    && a["policy_sha256"] == s.policy_sha256
                    && a["header_sha256"] == s.header_sha256
                    && a["target"] == s.target,
            )?;
            keys(&a["archives"], &["nexa-mnn-shim", "MNN"])?;
            keys(&a["archive_object_counts"], &["nexa-mnn-shim", "MNN"])?;
            for library in ["nexa-mnn-shim", "MNN"] {
                need(a["archives"][library].as_str().is_some_and(|s| hex(s, 64)))?;
                positive(&a["archive_object_counts"], library)?;
            }
            Ok(())
        }
        "native_real" => {
            keys(
                v,
                &[
                    "upstream_exact",
                    "privacy_canaries",
                    "prompt_tokens",
                    "completion_tokens",
                    "cancel_safe_return_ms",
                ],
            )?;
            yes(v, "upstream_exact")?;
            need(v["privacy_canaries"] == 9)?;
            positive(v, "prompt_tokens")?;
            positive(v, "completion_tokens")?;
            need(
                v["prompt_tokens"].as_u64().is_some_and(|n| n <= 2048)
                    && v["completion_tokens"].as_u64().is_some_and(|n| n <= 12),
            )?;
            let times = &v["cancel_safe_return_ms"];
            keys(
                times,
                &[
                    "cancel_load_checkpoint_0",
                    "cancel_load_checkpoint_2",
                    "cancel_load_checkpoint_3",
                    "cancel_load_checkpoint_4",
                    "cancel_load_checkpoint_5",
                    "cancel_load_checkpoint_6",
                    "cancel_prepare_phase_2",
                    "cancel_prepare_phase_3",
                    "cancel_phase_4",
                    "cancel_phase_5",
                ],
            )?;
            need(
                times
                    .as_object()
                    .ok_or(())?
                    .values()
                    .all(|v| v.as_f64().is_some_and(|f| f.is_finite() && f >= 0.)),
            )
        }
        "rust_linux" => {
            keys(
                v,
                &[
                    "clippy",
                    "abi_layout",
                    "artifact_negative",
                    "unit_compile_fail_and_real_model",
                    "linked_manifest_sha256",
                ],
            )?;
            for field in [
                "clippy",
                "abi_layout",
                "artifact_negative",
                "unit_compile_fail_and_real_model",
            ] {
                yes(v, field)?;
            }
            need(v["linked_manifest_sha256"] == s.artifact_manifest_sha256)
        }
        _ => Err(()),
    }
}
fn validate(path: &Path, env: &Environment, now: u64, build: &BuildIdentity) -> Result<String> {
    let data = bytes(path)?;
    let r: Receipt = parse(&data)?;
    need(
        r.schema_version == 1
            && r.purpose == "ci-linux-b2-research"
            && r.research_only
            && !r.production_admitted
            && !r.android_run,
    )?;
    let c = &r.context;
    let expected: Context = parse(env.expected_context.as_bytes())?;
    need(c == &expected)?;
    need(
        hex(&c.source_commit, 40)
            && hex(&c.source_tree, 40)
            && hex(&c.source_snapshot_sha256, 64)
            && hex(&c.context_id, 64)
            && c.job == "native-adapter",
    )?;
    need([&c.run_id, &c.run_attempt].iter().all(|v| {
        !v.is_empty()
            && v.len() <= 20
            && !v.starts_with('0')
            && v.bytes().all(|b| b.is_ascii_digit())
    }))?;
    if env.ci {
        need(
            c.mode == "github-ci"
                && c.source_clean
                && c.source_commit == env.sha
                && c.run_id == env.run
                && c.run_attempt == env.attempt
                && c.job == env.job,
        )?;
    } else {
        need(c.mode == "local-verification")?;
    }
    need(
        r.issued_at <= now
            && now < r.expires_at
            && r.expires_at
                .checked_sub(r.issued_at)
                .is_some_and(|n| n > 0 && n <= 2700),
    )?;
    let s = &r.subject;
    let lock: Value = parse(LOCK.as_bytes())?;
    need(
        s.target == "x86_64-unknown-linux-gnu"
            && s.target == build.target
            && s.compiler == build.compiler
            && s.compiler_sha256 == hash(s.compiler.as_bytes())
            && s.artifact_manifest_sha256 == build.artifact_manifest_sha256
            && hex(&s.artifact_manifest_sha256, 64),
    )?;
    need(
        s.upstream_commit == build.upstream_commit
            && lock["upstream_commit"] == s.upstream_commit
            && s.patch_set_sha256 == build.patch_sha256
            && lock["patch_set_sha256"] == s.patch_set_sha256
            && s.policy_sha256 == build.policy_sha256
            && lock["policy_sha256"] == s.policy_sha256
            && s.header_sha256 == hash(HEADER),
    )?;
    need(
        s.input_lock_sha256 == hash(INPUT)
            && s.candidate_identity_sha256 == mnn_model_store::candidate_digest()
            && s.template_sha256 == mnn_model_store::TEMPLATE_SHA256,
    )?;
    if env.ci {
        need(
            s.profile == "ubuntu24.04-gcc13.3-cpu-v1"
                && s.compiler == "g++-13 (Ubuntu 13.3.0-6ubuntu2~24.04.1) 13.3.0",
        )?;
    } else {
        need(
            s.profile == "debian-gcc14.2-local-cpu-v1"
                && s.compiler == "c++ (Debian 14.2.0-19) 14.2.0",
        )?;
    }
    need(r.prerequisites.len() == STAGES.len())?;
    need(path.file_name().is_some_and(|name| name == "receipt.json"))?;
    let dir = path.parent().ok_or(())?;
    let directory = fs::symlink_metadata(dir).map_err(|_| ())?;
    need(directory.is_dir() && directory.mode() & 0o222 == 0)?;
    let actual: BTreeSet<_> = fs::read_dir(dir)
        .map_err(|_| ())?
        .map(|e| {
            e.map(|e| e.file_name().to_string_lossy().into_owned())
                .map_err(|_| ())
        })
        .collect::<Result<_>>()?;
    let expected: BTreeSet<_> = std::iter::once("receipt.json".to_owned())
        .chain(STAGES.iter().map(|s| format!("{s}.json")))
        .collect();
    need(actual == expected)?;
    for (name, proof) in STAGES.iter().zip(&r.prerequisites) {
        need(proof.stage == *name && proof.outcome == "success" && hex(&proof.sha256, 64))?;
        let bytes = bytes(&dir.join(format!("{name}.json")))?;
        need(hash(&bytes) == proof.sha256)?;
        let report: Report = parse(&bytes)?;
        need(
            report.schema == 2
                && report.context == r.context
                && report.stage == *name
                && report.status == "ok"
                && !report.android_run
                && report.failure_case == "none"
                && report.missing_archive.is_none(),
        )?;
        need(
            !report.exit_codes.is_empty()
                && report.exit_codes.len() == report.commands.len()
                && report.exit_codes.iter().all(|n| *n == 0),
        )?;
        for (i, command) in report.commands.iter().enumerate() {
            need(
                command.index == i
                    && command.exit_code == 0
                    && command.cleanup_confirmed
                    && !command.timed_out
                    && !command.log_limit_exceeded
                    && command.failure_case == "none"
                    && command.diagnostics.is_empty()
                    && command.timeout_seconds > 0.
                    && command.timeout_seconds <= 900.
                    && command.timeout_seconds.is_finite(),
            )?;
            need(
                [
                    "configure",
                    "compile_link",
                    "tool",
                    "native_request",
                    "privacy",
                    "comparison",
                    "verification",
                    "rust_runtime",
                    "source_identity",
                    "candidate_validation",
                    "artifact_export",
                    "b2_real",
                ]
                .contains(&command.category.as_str())
                    && command.log_bytes <= 16 * 1024 * 1024,
            )?;
            let _ = command.output_truncated;
        }
        report_checks(name, &report.checks, &r)?;
    }
    Ok(hash(&data))
}

pub(super) fn verify(build: &BuildIdentity) -> Result<()> {
    let path = PathBuf::from(std::env::var_os("NEXA_MNN_B2_RESEARCH_RECEIPT").ok_or(())?);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ())?
        .as_secs();
    validate(&path, &Environment::current(), now, build).map(|_| ())
}

#[test]
fn strict_json_rejects_duplicates_unknown_and_oversize() {
    assert!(parse::<Value>(br#"{"a":1,"a":2}"#).is_err());
    assert!(parse::<Value>(br#"{"a":{"x":1,"x":2}}"#).is_err());
    assert!(parse::<Value>(&vec![b' '; MAX as usize + 1]).is_err());
    assert!(
        parse::<Proof>(br#"{"stage":"tools","sha256":"x","outcome":"success","extra":true}"#)
            .is_err()
    );
}

#[cfg(test)]
mod negative_tests {
    use super::*;
    use serde_json::json;
    use std::os::unix::fs::{PermissionsExt, symlink};

    // Synthetic format fixtures only; never used by ignored real-model tests.
    struct Fixture {
        root: tempfile::TempDir,
        receipt: Value,
        reports: Vec<Value>,
        build: BuildIdentity,
        env: Environment,
    }
    impl Fixture {
        fn new() -> Self {
            let lock: Value = parse(LOCK.as_bytes()).unwrap();
            let compiler = "g++-13 (Ubuntu 13.3.0-6ubuntu2~24.04.1) 13.3.0";
            let context = json!({"mode":"github-ci","source_commit":"1".repeat(40),"source_tree":"2".repeat(40),
                "source_clean":true,"source_snapshot_sha256":"3".repeat(64),"run_id":"123","run_attempt":"1",
                "job":"native-adapter","context_id":"4".repeat(64)});
            let subject = json!({"artifact_manifest_sha256":"c".repeat(64),"upstream_commit":lock["upstream_commit"],
                "patch_set_sha256":lock["patch_set_sha256"],"policy_sha256":lock["policy_sha256"],
                "header_sha256":hash(HEADER),"target":"x86_64-unknown-linux-gnu","compiler":compiler,
                "compiler_sha256":hash(compiler.as_bytes()),"profile":"ubuntu24.04-gcc13.3-cpu-v1",
                "input_lock_sha256":hash(INPUT),"candidate_identity_sha256":mnn_model_store::candidate_digest(),
                "template_sha256":mnn_model_store::TEMPLATE_SHA256});
            let build = BuildIdentity {
                upstream_commit: subject["upstream_commit"].as_str().unwrap().into(),
                patch_sha256: subject["patch_set_sha256"].as_str().unwrap().into(),
                policy_sha256: subject["policy_sha256"].as_str().unwrap().into(),
                artifact_manifest_sha256: "c".repeat(64),
                target: "x86_64-unknown-linux-gnu".into(),
                compiler: compiler.into(),
            };
            let checks = vec![
                json!({"source":{"source_commit":context["source_commit"],"source_tree":context["source_tree"],"source_clean":true},
                    "gcc":"13.3.0","rust":"1.98.1","cmake":"4.4.3","ninja":"1.13.2",
                    "notice_inventory":{"components":12,"files":27,"manifest_sha256":hash(NOTICES),"final_apk_verified":false}}),
                json!({"ndk_sha256":"753611f410d002cfcd3f3dc2ef49aad532089d3180b436c060a90bf0fcb64df2","ndk_revision":"30.0.16248370","model_lock_sha256":hash(INPUT)}),
                json!({"pristine_source_unchanged":true}),
                json!({"unpatched_probe_ctest":true}),
                json!({"artifact":{"manifest_sha256":subject["artifact_manifest_sha256"],"compiler_sha256":subject["compiler_sha256"],
                    "upstream_commit":subject["upstream_commit"],"patch_set_sha256":subject["patch_set_sha256"],"policy_sha256":subject["policy_sha256"],
                    "header_sha256":subject["header_sha256"],"target":subject["target"],
                    "archives":{"nexa-mnn-shim":"a".repeat(64),"MNN":"b".repeat(64)},"archive_object_counts":{"nexa-mnn-shim":1,"MNN":444}},
                    "logging_audit":{"compiled_source_count":10,"header_count":10},"ctest_and_stream_sanitizers":true}),
                json!({"upstream_exact":true,"privacy_canaries":9,"prompt_tokens":21,"completion_tokens":12,
                    "cancel_safe_return_ms":{"cancel_load_checkpoint_0":1,"cancel_load_checkpoint_2":1,"cancel_load_checkpoint_3":1,
                    "cancel_load_checkpoint_4":1,"cancel_load_checkpoint_5":1,"cancel_load_checkpoint_6":1,"cancel_prepare_phase_2":1,
                    "cancel_prepare_phase_3":1,"cancel_phase_4":1,"cancel_phase_5":1}}),
                json!({"clippy":true,"abi_layout":true,"artifact_negative":true,"unit_compile_fail_and_real_model":true,"linked_manifest_sha256":subject["artifact_manifest_sha256"]}),
            ];
            let reports = STAGES.iter().zip(checks).map(|(name, checks)| json!({"schema":2,"context":context,"stage":name,
                "status":"ok","android_run":false,"exit_codes":[0],"failure_case":"none","missing_archive":null,"checks":checks,
                "commands":[{"index":0,"category":"verification","timeout_seconds":600,"timed_out":false,"log_limit_exceeded":false,
                    "cleanup_confirmed":true,"failure_case":"none","exit_code":0,"log_bytes":0,"output_truncated":false,"diagnostics":[]}]})).collect();
            let env = Environment {
                ci: true,
                sha: "1".repeat(40),
                run: "123".into(),
                attempt: "1".into(),
                job: "native-adapter".into(),
                expected_context: context.to_string(),
            };
            Self {
                root: tempfile::tempdir().unwrap(),
                receipt: json!({"schema_version":1,"purpose":"ci-linux-b2-research","research_only":true,
                "production_admitted":false,"android_run":false,"context":context,"subject":subject,"issued_at":1000,"expires_at":3700,"prerequisites":[]}),
                reports,
                build,
                env,
            }
        }
        fn write(&mut self) -> PathBuf {
            fs::set_permissions(self.root.path(), fs::Permissions::from_mode(0o700)).unwrap();
            let mut proofs = Vec::new();
            for (name, report) in STAGES.iter().zip(&self.reports) {
                let data = serde_json::to_vec(report).unwrap();
                let path = self.root.path().join(format!("{name}.json"));
                if path.exists() {
                    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
                }
                fs::write(&path, &data).unwrap();
                fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
                proofs.push(json!({"stage":name,"sha256":hash(&data),"outcome":"success"}));
            }
            self.receipt["prerequisites"] = proofs.into();
            let path = self.root.path().join("receipt.json");
            if path.exists() {
                fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            }
            fs::write(&path, serde_json::to_vec(&self.receipt).unwrap()).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
            fs::set_permissions(self.root.path(), fs::Permissions::from_mode(0o555)).unwrap();
            path
        }
        fn valid(&mut self) -> bool {
            let path = self.write();
            validate(&path, &self.env, 1001, &self.build).is_ok()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::set_permissions(self.root.path(), fs::Permissions::from_mode(0o700));
        }
    }

    #[test]
    fn complete_sealed_fixture_is_accepted() {
        assert!(Fixture::new().valid());
    }

    #[test]
    fn local_namespace_is_explicit_and_cannot_be_used_as_github_evidence() {
        let mut f = Fixture::new();
        let compiler = "c++ (Debian 14.2.0-19) 14.2.0";
        f.receipt["context"]["mode"] = json!("local-verification");
        f.receipt["context"]["source_clean"] = json!(false);
        f.env.ci = false;
        f.env.expected_context = f.receipt["context"].to_string();
        f.receipt["subject"]["compiler"] = json!(compiler);
        f.receipt["subject"]["compiler_sha256"] = json!(hash(compiler.as_bytes()));
        f.receipt["subject"]["profile"] = json!("debian-gcc14.2-local-cpu-v1");
        f.build.compiler = compiler.into();
        for report in &mut f.reports {
            report["context"] = f.receipt["context"].clone();
        }
        f.reports[0]["checks"]["source"]["source_clean"] = json!(false);
        f.reports[0]["checks"]["gcc"] = json!("14.2.0");
        f.reports[4]["checks"]["artifact"]["compiler_sha256"] = json!(hash(compiler.as_bytes()));
        assert!(f.valid());
        f.env.ci = true;
        assert!(!f.valid());
    }

    #[test]
    fn every_proof_is_required_successful_and_same_context() {
        for i in 0..STAGES.len() {
            for (pointer, value) in [
                ("/status", json!("failed")),
                ("/exit_codes/0", json!(1)),
                ("/commands/0/cleanup_confirmed", json!(false)),
                ("/commands/0/timed_out", json!(true)),
                ("/commands/0/log_limit_exceeded", json!(true)),
                ("/context/run_id", json!("999")),
                ("/context/source_tree", json!("f".repeat(40))),
                ("/context/context_id", json!("a".repeat(64))),
                ("/schema", json!(1)),
                ("/checks", json!({})),
                ("/commands/0/category", json!("unknown")),
                ("/commands/0/diagnostics", json!(["private"])),
            ] {
                let mut f = Fixture::new();
                *f.reports[i].pointer_mut(pointer).unwrap() = value;
                assert!(!f.valid(), "{i} {pointer}");
            }
        }
    }
    #[test]
    fn identities_flags_and_time_are_strict() {
        for (pointer, value) in [
            ("/purpose", json!("product")),
            ("/production_admitted", json!(true)),
            ("/research_only", json!(false)),
            ("/android_run", json!(true)),
            ("/schema_version", json!(2)),
            ("/issued_at", json!(1002)),
            ("/expires_at", json!(1001)),
            ("/expires_at", json!(3701)),
            ("/subject/artifact_manifest_sha256", json!("a".repeat(64))),
            ("/subject/patch_set_sha256", json!("a".repeat(64))),
            ("/subject/input_lock_sha256", json!("a".repeat(64))),
            ("/subject/candidate_identity_sha256", json!("a".repeat(64))),
            ("/subject/template_sha256", json!("a".repeat(64))),
            ("/subject/header_sha256", json!("a".repeat(64))),
            ("/subject/target", json!("aarch64-linux-android")),
            ("/subject/compiler", json!("unreviewed")),
            ("/context/run_attempt", json!("2")),
            ("/context/source_clean", json!(false)),
            ("/context/source_snapshot_sha256", json!("a".repeat(64))),
        ] {
            let mut f = Fixture::new();
            *f.receipt.pointer_mut(pointer).unwrap() = value;
            assert!(!f.valid(), "{pointer}");
        }
        let mut f = Fixture::new();
        f.receipt["extra"] = json!(true);
        assert!(!f.valid());
        let mut f = Fixture::new();
        f.reports[0]["checks"]["source"]["extra"] = json!(true);
        assert!(!f.valid());
        let mut f = Fixture::new();
        f.reports[4]["checks"]["logging_audit"]["extra"] = json!(true);
        assert!(!f.valid());
        let mut f = Fixture::new();
        f.reports[5]["checks"]["privacy_canaries"] = json!(8);
        assert!(!f.valid());
        let mut f = Fixture::new();
        f.reports[6]["checks"]["unit_compile_fail_and_real_model"] = json!(false);
        assert!(!f.valid());
    }
    #[test]
    fn proof_names_hashes_outcomes_and_order_are_fixed() {
        for mode in [
            "duplicate",
            "missing",
            "extra",
            "cycle",
            "path",
            "hash",
            "outcome",
            "unknown",
        ] {
            let mut f = Fixture::new();
            let path = f.write();
            match mode {
                "duplicate" => {
                    f.receipt["prerequisites"][0] = f.receipt["prerequisites"][1].clone()
                }
                "missing" => {
                    f.receipt["prerequisites"].as_array_mut().unwrap().pop();
                }
                "extra" => {
                    let proof = f.receipt["prerequisites"][0].clone();
                    f.receipt["prerequisites"]
                        .as_array_mut()
                        .unwrap()
                        .push(proof);
                }
                "cycle" => f.receipt["prerequisites"][0]["stage"] = json!("b2_linux"),
                "path" => f.receipt["prerequisites"][0]["stage"] = json!("../tools"),
                "hash" => f.receipt["prerequisites"][0]["sha256"] = json!("e".repeat(64)),
                "outcome" => f.receipt["prerequisites"][0]["outcome"] = json!("skipped"),
                _ => f.receipt["prerequisites"][0]["unknown"] = json!(true),
            }
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            fs::write(&path, serde_json::to_vec(&f.receipt).unwrap()).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
            assert!(validate(&path, &f.env, 1001, &f.build).is_err(), "{mode}");
        }
    }

    #[test]
    fn bundle_requires_exact_readonly_regular_files() {
        for mode in [
            "missing", "extra", "symlink", "hardlink", "writable", "oversize", "tamper",
        ] {
            let mut f = Fixture::new();
            let receipt = f.write();
            fs::set_permissions(f.root.path(), fs::Permissions::from_mode(0o700)).unwrap();
            let proof = f.root.path().join("tools.json");
            match mode {
                "missing" => fs::remove_file(&proof).unwrap(),
                "extra" => fs::write(f.root.path().join("extra"), b"{}").unwrap(),
                "symlink" => {
                    fs::remove_file(&proof).unwrap();
                    symlink("inputs.json", &proof).unwrap();
                }
                "hardlink" => fs::hard_link(&proof, f.root.path().join("alias")).unwrap(),
                "writable" => {
                    fs::set_permissions(&proof, fs::Permissions::from_mode(0o644)).unwrap()
                }
                _ => {
                    fs::set_permissions(&proof, fs::Permissions::from_mode(0o600)).unwrap();
                    fs::write(
                        &proof,
                        vec![
                            b' ';
                            if mode == "oversize" {
                                MAX as usize + 1
                            } else {
                                2
                            }
                        ],
                    )
                    .unwrap();
                    fs::set_permissions(&proof, fs::Permissions::from_mode(0o444)).unwrap();
                }
            }
            fs::set_permissions(f.root.path(), fs::Permissions::from_mode(0o555)).unwrap();
            assert!(
                validate(&receipt, &f.env, 1001, &f.build).is_err(),
                "{mode}"
            );
        }
    }
}
