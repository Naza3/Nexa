//! Real subprocesses share the same configuration inode lock; these are not
//! merely two async tasks protected by the HTTP owner's in-memory mutex.
use runtime_api::configuration::*;
use runtime_types::ModelId;
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
#[test]
fn subprocess_writer() {
    let Ok(root) = std::env::var("NEXA_TEST_CONFIGURATION_ROOT") else {
        return;
    };
    let name = std::env::var("NEXA_TEST_WRITER").unwrap();
    let revision = std::env::var("NEXA_TEST_REVISION").unwrap();
    let started = Instant::now();
    while !Path::new(&root).join("go").exists() {
        assert!(started.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(5));
    }
    let result = loop {
        let request = ConfigurationSaveRequest {
            expected_revision: revision.clone(),
            update: ConfigurationUpdate::ModelProfile {
                model_id: ModelId::new(&name).unwrap(),
                load_overrides: LoadOverrides {
                    context_size: Some(2048),
                    threads: Some(2),
                    batch_size: Some(128),
                },
            },
        };
        match save(Path::new(&root), request, None, &BTreeMap::new()) {
            Ok(_) => break "ok",
            Err(e)
                if e.code == "configuration_busy"
                    && started.elapsed() < Duration::from_secs(10) =>
            {
                std::thread::sleep(Duration::from_millis(5))
            }
            Err(e) => break e.code,
        }
    };
    fs::write(Path::new(&root).join(format!("{name}.result")), result).unwrap();
}
#[test]
fn two_process_cas_allows_exactly_one_save_and_preserves_winner() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("private");
    initialize(&root).unwrap();
    let before = read(&root).unwrap();
    let exe = std::env::current_exe().unwrap();
    let mut children = Vec::new();
    for name in ["a", "b"] {
        children.push(
            Command::new(&exe)
                .args(["--exact", "subprocess_writer", "--nocapture"])
                .env("NEXA_TEST_CONFIGURATION_ROOT", &root)
                .env("NEXA_TEST_WRITER", name)
                .env("NEXA_TEST_REVISION", &before.revision)
                .stdout(Stdio::null())
                .spawn()
                .unwrap(),
        );
    }
    fs::write(root.join("go"), b"").unwrap();
    for child in &mut children {
        assert!(child.wait().unwrap().success());
    }
    let a = fs::read_to_string(root.join("a.result")).unwrap();
    let b = fs::read_to_string(root.join("b.result")).unwrap();
    assert!(
        (a == "ok" && b == "configuration_conflict")
            || (b == "ok" && a == "configuration_conflict"),
        "{a}, {b}"
    );
    let after = read(&root).unwrap();
    assert_eq!(after.config.model_profiles.len(), 1);
    assert_eq!(after.config.inference, before.config.inference);
    assert_eq!(after.config.lan_api, before.config.lan_api);
    assert_eq!(after.config.runtime, before.config.runtime);
}
