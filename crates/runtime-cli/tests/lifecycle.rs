//! These tests spawn the actual built CLI in isolated private directories. No
//! PATH/CWD worker override or production test hook is used.
use runtime_cli::instance::{Discovery, InstanceLock};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_ai-runtime"))
}
fn run(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(binary())
        .arg("--data-dir")
        .arg(root)
        .args(args)
        .output()
        .unwrap()
}
#[test]
fn init_is_idempotent_and_argument_errors_are_two() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    assert!(run(&root, &["init"]).status.success());
    let token = fs::read(root.join("secrets/api-token")).unwrap();
    let config = fs::read(root.join("config.toml")).unwrap();
    assert!(run(&root, &["init"]).status.success());
    assert_eq!(fs::read(root.join("secrets/api-token")).unwrap(), token);
    assert_eq!(fs::read(root.join("config.toml")).unwrap(), config);
    assert_eq!(run(&root, &["start"]).status.code(), Some(2));
    assert!(run(&root, &["models", "list"]).status.success());
    assert!(run(&root, &["stop"]).status.success());
    assert!(run(&root, &["stop"]).status.success());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(root.join("secrets/api-token"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(root.join("secrets"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
}
#[test]
fn serve_never_initializes_credentials() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("absent");
    assert_eq!(run(&root, &["serve"]).status.code(), Some(1));
    assert!(!root.exists());
    assert!(run(&root, &["stop"]).status.success());
    assert!(!root.exists());
}
struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}
fn packaged_binary(temp: &Path) -> PathBuf {
    let package = temp.join("package");
    fs::create_dir(&package).unwrap();
    let cli = package.join(if cfg!(windows) {
        "ai-runtime.exe"
    } else {
        "ai-runtime"
    });
    fs::copy(binary(), &cli).unwrap();
    // Serve only verifies presence before load; the actual CLI binary itself is
    // a controlled fixture that is never launched as a worker in this empty-store
    // lifecycle test. No fake path enters production code or the environment.
    fs::copy(
        binary(),
        package.join(if cfg!(windows) {
            "ai-runtime-worker.exe"
        } else {
            "ai-runtime-worker"
        }),
    )
    .unwrap();
    cli
}
#[test]
fn actual_serve_lock_discovery_proof_and_stop_lifecycle() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    let cli = packaged_binary(temp.path());
    assert!(run(&root, &["init"]).status.success());
    let config = root.join("config.toml");
    let text = fs::read_to_string(&config)
        .unwrap()
        .replace("127.0.0.1:18080", "127.0.0.1:0");
    fs::write(config, text).unwrap();
    let mut child = Running(
        Command::new(&cli)
            .arg("--data-dir")
            .arg(&root)
            .arg("serve")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    let original = loop {
        if let Ok(record) = Discovery::read(&root) {
            break record;
        }
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "serve exited before discovery"
        );
        assert!(Instant::now() < deadline, "serve did not publish discovery");
        thread::sleep(Duration::from_millis(20));
    };
    assert!(InstanceLock::try_acquire(&root).unwrap().is_none());
    assert_eq!(
        Command::new(&cli)
            .arg("--data-dir")
            .arg(&root)
            .arg("serve")
            .output()
            .unwrap()
            .status
            .code(),
        Some(1)
    );
    assert_eq!(Discovery::read(&root).unwrap(), original);
    for args in [&["status"][..], &["devices"], &["models", "list"]] {
        assert!(run(&root, args).status.success(), "{args:?}");
    }
    let token = fs::read(root.join("secrets/api-token")).unwrap();
    fs::write(root.join("config.toml"), b"damaged configuration fixture").unwrap();
    assert!(run(&root, &["stop"]).status.success());
    assert_eq!(
        fs::read(root.join("config.toml")).unwrap(),
        b"damaged configuration fixture"
    );
    assert_eq!(fs::read(root.join("secrets/api-token")).unwrap(), token);
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.0.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    assert!(InstanceLock::try_acquire(&root).unwrap().is_some());
    assert!(!root.join("runtime/instance.json").exists());
    assert!(run(&root, &["stop"]).status.success());
}
#[test]
fn held_offline_instance_lock_prevents_model_store_open() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    assert!(run(&root, &["init"]).status.success());
    let _lock = InstanceLock::try_acquire(&root).unwrap().unwrap();
    assert_eq!(run(&root, &["models", "list"]).status.code(), Some(1));
    assert!(!root.join("runtime/model-store.lock").exists());
}
#[test]
fn stale_port_attacker_never_receives_cli_bearer() {
    use std::io::{Read, Write};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    assert!(run(&root, &["init"]).status.success());
    let lock = InstanceLock::try_acquire(&root).unwrap().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut record =
        Discovery::current(uuid::Uuid::new_v4(), listener.local_addr().unwrap()).unwrap();
    record.pid = 1;
    record.process_created = "deliberately stale identity; never authority to kill".into();
    lock.publish(&record).unwrap();
    let attack = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut all = Vec::new();
        let mut byte = [0];
        while all.len() < 16384 {
            if socket.read(&mut byte).unwrap_or(0) == 0 {
                break;
            }
            all.push(byte[0]);
            if all.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        let request = String::from_utf8(all).unwrap();
        assert!(!request.to_ascii_lowercase().contains("authorization:"));
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
            .unwrap();
        let mut remaining = Vec::new();
        let _ = socket.read_to_end(&mut remaining);
        assert!(
            !String::from_utf8_lossy(&remaining)
                .to_ascii_lowercase()
                .contains("authorization:")
        );
    });
    assert_eq!(run(&root, &["status"]).status.code(), Some(1));
    attack.join().unwrap();
    assert_eq!(Discovery::read(&root).unwrap().pid, 1);
}
#[test]
fn bind_failure_preserves_existing_discovery_and_releases_lock() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    let cli = packaged_binary(temp.path());
    assert!(run(&root, &["init"]).status.success());
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let config = root.join("config.toml");
    fs::write(
        &config,
        fs::read_to_string(&config)
            .unwrap()
            .replace("127.0.0.1:18080", &address.to_string()),
    )
    .unwrap();
    let lock = InstanceLock::try_acquire(&root).unwrap().unwrap();
    let record = Discovery::current(uuid::Uuid::new_v4(), address).unwrap();
    lock.publish(&record).unwrap();
    drop(lock);
    assert_eq!(
        Command::new(cli)
            .arg("--data-dir")
            .arg(&root)
            .arg("serve")
            .output()
            .unwrap()
            .status
            .code(),
        Some(1)
    );
    assert_eq!(Discovery::read(&root).unwrap(), record);
    assert!(InstanceLock::try_acquire(&root).unwrap().is_some());
}

#[test]
fn init_refuses_free_lock_with_uncertain_discovery_without_creating_credentials() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("private");
    let lock = InstanceLock::try_acquire(&root).unwrap().unwrap();
    lock.publish(
        &Discovery::current(uuid::Uuid::new_v4(), "127.0.0.1:18080".parse().unwrap()).unwrap(),
    )
    .unwrap();
    drop(lock);
    let result = run(&root, &["init"]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("runtime_stop_unconfirmed"));
    assert!(!root.join("config.toml").exists());
    assert!(!root.join("secrets/api-token").exists());
}

#[test]
fn unavailable_lan_keeps_authenticated_management_and_explicit_restart_recovers() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    let cli = packaged_binary(temp.path());
    assert!(run(&root, &["init"]).status.success());
    // Find an actually unavailable private address, without exposing a service.
    let unavailable = ["192.168.254.253:18081", "10.254.253.252:18081", "172.31.254.253:18081"]
        .into_iter()
        .find(|address| matches!(std::net::TcpListener::bind(address), Err(error) if error.kind() == std::io::ErrorKind::AddrNotAvailable))
        .expect("fixture requires one unavailable private address");
    let mut config = runtime_api::Config::default();
    config.api.listen = "127.0.0.1:0".parse().unwrap();
    config.lan_api = runtime_api::LanApiConfig {
        enabled: true,
        listen: Some(unavailable.parse().unwrap()),
        allowed_cidrs: vec!["192.168.254.252/32".into()],
    };
    let management_token = fs::read(root.join("secrets/api-token")).unwrap();
    for (attempt, enabled) in [true, true, false].into_iter().enumerate() {
        if attempt == 1 {
            runtime_api::token::init_private_lan_token(&root).unwrap();
        }
        let lan_token = fs::read(root.join("secrets/lan-api-token")).ok();
        config.lan_api.enabled = enabled;
        let saved = config.to_toml().unwrap();
        fs::write(root.join("config.toml"), &saved).unwrap();
        let mut child = Running(
            Command::new(&cli)
                .arg("--data-dir")
                .arg(&root)
                .arg("serve")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        let discovery = loop {
            if let Ok(record) = Discovery::read(&root) {
                break record;
            }
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "serve exited before discovery"
            );
            assert!(Instant::now() < deadline, "discovery timeout");
            thread::sleep(Duration::from_millis(20));
        };
        assert!(InstanceLock::try_acquire(&root).unwrap().is_none());
        let output = run(&root, &["status"]);
        assert!(
            output.status.success(),
            "authenticated status must stay available"
        );
        let status: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(status["lan_api"]["enabled"], enabled);
        assert_eq!(status["lan_api"]["running"], false);
        assert_eq!(
            status["lan_api"]["startup_error"],
            if enabled {
                serde_json::json!("address_unavailable")
            } else {
                serde_json::Value::Null
            }
        );
        assert_eq!(status["state"], "unloaded");
        assert_eq!(fs::read_to_string(root.join("config.toml")).unwrap(), saved);
        assert_eq!(
            fs::read(root.join("secrets/api-token")).unwrap(),
            management_token
        );
        assert_eq!(fs::read(root.join("secrets/lan-api-token")).ok(), lan_token);
        assert!(run(&root, &["stop"]).status.success());
        let deadline = Instant::now() + Duration::from_secs(10);
        while child.0.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "shutdown timeout");
            thread::sleep(Duration::from_millis(10));
        }
        assert!(InstanceLock::try_acquire(&root).unwrap().is_some());
        assert!(!root.join("runtime/instance.json").exists());
        // The published record is the usual local-only management discovery.
        assert!(discovery.listen.ip().is_loopback());
    }
}
