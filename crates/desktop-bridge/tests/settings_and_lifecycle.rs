use desktop_bridge::{ConnectionState, DesktopBridge, DesktopPreferences};
use runtime_api::{
    Config,
    token::{init_private_token, write_private_new},
};
use runtime_cli::instance::{Discovery, InstanceLock};
use std::{path::PathBuf, sync::Arc};
use uuid::Uuid;
fn initialized() -> (tempfile::TempDir, PathBuf, Arc<DesktopBridge>) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("private");
    init_private_token(&root).unwrap();
    let mut config = Config::default();
    config.api.listen = "127.0.0.1:0".parse().unwrap();
    write_private_new(
        &root.join("config.toml"),
        config.to_toml().unwrap().as_bytes(),
    )
    .unwrap();
    let bridge = Arc::new(
        DesktopBridge::new(
            root.clone(),
            temp.path().join(if cfg!(windows) {
                "ai-runtime.exe"
            } else {
                "ai-runtime"
            }),
        )
        .unwrap(),
    );
    (temp, root, bridge)
}
#[tokio::test]
async fn preference_and_idle_publications_are_separate_atomic_and_preserve_token() {
    let (_temp, root, bridge) = initialized();
    let token = std::fs::read(root.join("secrets/api-token")).unwrap();
    let original = std::fs::read(root.join("config.toml")).unwrap();
    let p = DesktopPreferences {
        context_size: 4096,
        threads: 3,
        batch_size: 256,
        max_output_tokens: 123,
        close_runtime_on_exit: true,
        download_source: Default::default(),
    };
    bridge.settings_save(p.clone()).await.unwrap();
    assert_eq!(std::fs::read(root.join("config.toml")).unwrap(), original);
    let preferences = std::fs::read(root.join("desktop-settings.json")).unwrap();
    assert_eq!(
        serde_json::from_slice::<DesktopPreferences>(&preferences).unwrap(),
        p
    );
    let s = bridge.save_idle(86400).await.unwrap();
    assert_eq!(s.settings.idle_unload_seconds, 86400);
    assert_eq!(s.settings.threads, 3);
    assert_eq!(
        std::fs::read(root.join("desktop-settings.json")).unwrap(),
        preferences
    );
    assert_eq!(
        std::fs::read(root.join("secrets/api-token")).unwrap(),
        token
    );
    let c = Config::from_toml(&std::fs::read_to_string(root.join("config.toml")).unwrap()).unwrap();
    assert_eq!(c.inference.context_size, 4096);
    assert_eq!(c.inference.threads, None);
    assert_eq!(c.api.listen.port(), 0);
    assert_eq!(c.runtime.idle_unload_seconds, 86400);
}
#[tokio::test]
async fn invalid_settings_never_partially_save() {
    let (_temp, root, bridge) = initialized();
    let old = std::fs::read(root.join("config.toml")).unwrap();
    for idle in [0, 86401, u64::MAX] {
        assert_eq!(
            bridge.save_idle(idle).await.unwrap_err().code,
            "settings_invalid"
        );
        assert_eq!(std::fs::read(root.join("config.toml")).unwrap(), old);
    }
    assert_eq!(
        bridge
            .settings_save(DesktopPreferences {
                threads: 0,
                ..Default::default()
            })
            .await
            .unwrap_err()
            .code,
        "settings_invalid"
    );
    assert!(!root.join("desktop-settings.json").exists());
}
#[tokio::test]
async fn held_instance_prevents_idle_write_even_with_unreachable_discovery() {
    let (_temp, root, bridge) = initialized();
    let lock = InstanceLock::try_acquire(&root).unwrap().unwrap();
    lock.publish(&Discovery::current(Uuid::new_v4(), "127.0.0.1:1".parse().unwrap()).unwrap())
        .unwrap();
    let old = std::fs::read(root.join("config.toml")).unwrap();
    assert_eq!(
        bridge.save_idle(1).await.unwrap_err().code,
        "runtime_running"
    );
    assert_eq!(std::fs::read(root.join("config.toml")).unwrap(), old);
    assert_eq!(
        bridge.start(false).await.unwrap_err().code,
        "connection_failed"
    );
    assert!(InstanceLock::try_acquire(&root).unwrap().is_none());
}
#[tokio::test]
async fn stale_record_never_becomes_confirmed_stop() {
    let (_temp, root, bridge) = initialized();
    let lock = InstanceLock::try_acquire(&root).unwrap().unwrap();
    let record = Discovery::current(Uuid::new_v4(), "127.0.0.1:1".parse().unwrap()).unwrap();
    lock.publish(&record).unwrap();
    drop(lock);
    assert_eq!(
        bridge.stop().await.unwrap_err().code,
        "runtime_stop_unconfirmed"
    );
    assert_eq!(Discovery::read(&root).unwrap(), record);
    assert!(matches!(
        bridge.snapshot().await.unwrap().connection,
        ConnectionState::Error
    ));
}
#[tokio::test]
async fn no_implicit_init_and_explicit_init_preserves_credential() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("not-created");
    let bridge = DesktopBridge::new(
        root.clone(),
        temp.path().join(if cfg!(windows) {
            "ai-runtime.exe"
        } else {
            "ai-runtime"
        }),
    )
    .unwrap();
    assert!(!bridge.snapshot().await.unwrap().initialized);
    assert_eq!(
        bridge.start(false).await.unwrap_err().code,
        "not_initialized"
    );
    assert!(!root.exists());
    assert_eq!(
        bridge.start(true).await.unwrap_err().code,
        "packaged_runtime_missing"
    );
    let token = std::fs::read(root.join("secrets/api-token")).unwrap();
    assert_eq!(
        bridge.start(true).await.unwrap_err().code,
        "packaged_runtime_missing"
    );
    assert_eq!(
        std::fs::read(root.join("secrets/api-token")).unwrap(),
        token
    );
    assert!(matches!(
        bridge.snapshot().await.unwrap().connection,
        ConnectionState::Stopped
    ));
}
#[cfg(unix)]
#[tokio::test]
async fn settings_symlink_is_not_followed_or_overwritten() {
    use std::os::unix::fs::symlink;
    let (temp, root, bridge) = initialized();
    let victim = temp.path().join("victim");
    std::fs::write(&victim, b"keep").unwrap();
    symlink(&victim, root.join("desktop-settings.json")).unwrap();
    assert!(
        bridge
            .settings_save(DesktopPreferences::default())
            .await
            .is_err()
    );
    assert_eq!(std::fs::read(victim).unwrap(), b"keep");
    assert!(
        std::fs::symlink_metadata(root.join("desktop-settings.json"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[tokio::test]
async fn source_preference_persists_before_runtime_initialization() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("private");
    let bridge = DesktopBridge::new(
        root.clone(),
        temp.path().join(if cfg!(windows) {
            "ai-runtime.exe"
        } else {
            "ai-runtime"
        }),
    )
    .unwrap();
    assert_eq!(
        bridge.snapshot().await.unwrap().settings.download_source,
        desktop_bridge::DownloadSource::Modelscope
    );
    let snapshot = bridge
        .settings_save(DesktopPreferences {
            download_source: desktop_bridge::DownloadSource::Huggingface,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(!snapshot.initialized);
    assert_eq!(
        snapshot.settings.download_source,
        desktop_bridge::DownloadSource::Huggingface
    );
    assert_eq!(
        bridge.snapshot().await.unwrap().settings.download_source,
        desktop_bridge::DownloadSource::Huggingface
    );
    assert!(!root.join("secrets/api-token").exists());
}

#[tokio::test]
async fn lan_settings_are_stopped_only_atomic_and_never_create_or_expose_credentials() {
    let (_temp, root, bridge) = initialized();
    let original = std::fs::read(root.join("config.toml")).unwrap();
    let management_token = std::fs::read(root.join("secrets/api-token")).unwrap();
    assert_eq!(
        bridge.snapshot().await.unwrap().lan_api,
        runtime_api::LanApiConfig::default()
    );
    assert!(!root.join("secrets/lan-api-token").exists());
    assert_eq!(
        bridge.lan_token_for_copy().await.unwrap_err().code,
        "lan_token_unavailable"
    );
    let config = runtime_api::LanApiConfig {
        enabled: true,
        listen: Some("192.168.1.2:18081".parse().unwrap()),
        allowed_cidrs: vec!["192.168.1.3/32".into()],
    };
    let lock = InstanceLock::try_acquire(&root).unwrap().unwrap();
    assert_eq!(
        bridge.save_lan(config.clone()).await.unwrap_err().code,
        "runtime_running"
    );
    assert_eq!(std::fs::read(root.join("config.toml")).unwrap(), original);
    drop(lock);
    let snapshot = bridge.save_lan(config.clone()).await.unwrap();
    assert_eq!(snapshot.lan_api, config);
    assert!(snapshot.runtime.is_none());
    assert!(!root.join("secrets/lan-api-token").exists());
    assert_eq!(
        bridge.lan_token_for_copy().await.unwrap_err().code,
        "lan_token_unavailable"
    );
    let saved = std::fs::read(root.join("config.toml")).unwrap();
    let mut invalid = config;
    invalid.allowed_cidrs = vec!["0.0.0.0/0".into()];
    assert_eq!(
        bridge.save_lan(invalid).await.unwrap_err().code,
        "lan_settings_invalid"
    );
    assert_eq!(std::fs::read(root.join("config.toml")).unwrap(), saved);
    let fixture = runtime_api::token::init_private_lan_token(&root).unwrap();
    assert_eq!(
        fixture.bearer_header_value(),
        bridge
            .lan_token_for_copy()
            .await
            .unwrap()
            .bearer_header_value()
    );
    let json = serde_json::to_string(&bridge.snapshot().await.unwrap()).unwrap();
    let raw = fixture.bearer_header_value();
    let secret = raw.to_str().unwrap().strip_prefix("Bearer ").unwrap();
    assert!(!json.contains(secret));
    assert!(
        !std::fs::read_to_string(root.join("config.toml"))
            .unwrap()
            .contains(secret)
    );
    // Even a private but misconfigured duplicate must never be copied as a LAN key.
    std::fs::write(root.join("secrets/lan-api-token"), &management_token).unwrap();
    assert_eq!(
        bridge.lan_token_for_copy().await.unwrap_err().code,
        "lan_token_unavailable"
    );
    bridge.save_lan(Default::default()).await.unwrap();
    assert_eq!(
        bridge.lan_token_for_copy().await.unwrap_err().code,
        "lan_token_unavailable"
    );
    assert_eq!(
        std::fs::read(root.join("secrets/api-token")).unwrap(),
        management_token
    );
}

#[tokio::test]
async fn lan_save_rejects_uninitialized_and_unclean_instance_without_side_effects() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("missing");
    let bridge = DesktopBridge::new(
        missing.clone(),
        temp.path().join(if cfg!(windows) {
            "ai-runtime.exe"
        } else {
            "ai-runtime"
        }),
    )
    .unwrap();
    assert_eq!(
        bridge.save_lan(Default::default()).await.unwrap_err().code,
        "not_initialized"
    );
    assert!(!missing.exists());
    let (_temp, root, bridge) = initialized();
    let lock = InstanceLock::try_acquire(&root).unwrap().unwrap();
    lock.publish(&Discovery::current(Uuid::new_v4(), "127.0.0.1:1".parse().unwrap()).unwrap())
        .unwrap();
    drop(lock);
    let original = std::fs::read(root.join("config.toml")).unwrap();
    assert_eq!(
        bridge.save_lan(Default::default()).await.unwrap_err().code,
        "runtime_stop_unconfirmed"
    );
    assert_eq!(std::fs::read(root.join("config.toml")).unwrap(), original);
    assert!(!root.join("secrets/lan-api-token").exists());
}

#[tokio::test]
async fn runtime_policy_saves_preserve_each_other_lan_preferences_and_credentials() {
    let (_temp, root, bridge) = initialized();
    let token = std::fs::read(root.join("secrets/api-token")).unwrap();
    let mut config =
        Config::from_toml(&std::fs::read_to_string(root.join("config.toml")).unwrap()).unwrap();
    config.lan_api.listen = Some("192.168.1.20:19090".parse().unwrap());
    config.inference.threads = Some(3);
    std::fs::write(root.join("config.toml"), config.to_toml().unwrap()).unwrap();
    let disabled = bridge.save_idle_policy(600, Some(false)).await.unwrap();
    assert!(!disabled.settings.idle_unload_enabled);
    assert_eq!(disabled.settings.idle_unload_seconds, 600);
    for seconds in [30, 7200, 300] {
        let saved = bridge.save_verification_timeout(seconds).await.unwrap();
        assert_eq!(saved.settings.model_verification_timeout_seconds, seconds);
        assert!(!saved.settings.idle_unload_enabled);
        assert_eq!(saved.settings.idle_unload_seconds, 600);
    }
    let legacy = bridge.save_idle(900).await.unwrap();
    assert!(!legacy.settings.idle_unload_enabled);
    assert_eq!(legacy.settings.idle_unload_seconds, 900);
    let saved =
        Config::from_toml(&std::fs::read_to_string(root.join("config.toml")).unwrap()).unwrap();
    assert_eq!(saved.lan_api.listen, config.lan_api.listen);
    assert_eq!(saved.inference.threads, Some(3));
    assert_eq!(saved.runtime.model_verification_timeout_seconds, 300);
    assert_eq!(
        std::fs::read(root.join("secrets/api-token")).unwrap(),
        token
    );
    assert!(!root.join("desktop-settings.json").exists());
    assert!(!root.join("secrets/lan-api-token").exists());
    let enabled = bridge.save_idle_policy(900, Some(true)).await.unwrap();
    assert!(enabled.settings.idle_unload_enabled);
}

#[tokio::test]
async fn invalid_runtime_policy_saves_are_atomic_even_when_idle_disabled() {
    let (_temp, root, bridge) = initialized();
    let original = std::fs::read(root.join("config.toml")).unwrap();
    for seconds in [0, 29, 7201, u64::MAX] {
        assert_eq!(
            bridge
                .save_verification_timeout(seconds)
                .await
                .unwrap_err()
                .code,
            "settings_invalid"
        );
        assert_eq!(std::fs::read(root.join("config.toml")).unwrap(), original);
    }
    for seconds in [0, 86401, u64::MAX] {
        assert_eq!(
            bridge
                .save_idle_policy(seconds, Some(false))
                .await
                .unwrap_err()
                .code,
            "settings_invalid"
        );
        assert_eq!(std::fs::read(root.join("config.toml")).unwrap(), original);
    }
}

#[tokio::test]
async fn runtime_policy_saves_require_confirmed_stop_and_never_initialize() {
    let (_temp, root, bridge) = initialized();
    let lock = InstanceLock::try_acquire(&root).unwrap().unwrap();
    let original = std::fs::read(root.join("config.toml")).unwrap();
    assert_eq!(
        bridge.save_verification_timeout(30).await.unwrap_err().code,
        "runtime_running"
    );
    assert_eq!(
        bridge
            .save_idle_policy(1, Some(false))
            .await
            .unwrap_err()
            .code,
        "runtime_running"
    );
    lock.publish(&Discovery::current(Uuid::new_v4(), "127.0.0.1:1".parse().unwrap()).unwrap())
        .unwrap();
    drop(lock);
    assert_eq!(
        bridge.save_verification_timeout(30).await.unwrap_err().code,
        "runtime_stop_unconfirmed"
    );
    assert_eq!(
        bridge
            .save_idle_policy(1, Some(false))
            .await
            .unwrap_err()
            .code,
        "runtime_stop_unconfirmed"
    );
    assert_eq!(
        bridge.save_idle(1).await.unwrap_err().code,
        "runtime_stop_unconfirmed"
    );
    assert_eq!(std::fs::read(root.join("config.toml")).unwrap(), original);
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("absent");
    let uninitialized = DesktopBridge::new(
        missing.clone(),
        temp.path().join(if cfg!(windows) {
            "ai-runtime.exe"
        } else {
            "ai-runtime"
        }),
    )
    .unwrap();
    assert_eq!(
        uninitialized
            .save_verification_timeout(300)
            .await
            .unwrap_err()
            .code,
        "not_initialized"
    );
    assert_eq!(
        uninitialized
            .save_idle_policy(300, Some(false))
            .await
            .unwrap_err()
            .code,
        "not_initialized"
    );
    assert!(!missing.exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_windows_never_overwrite_other_runtime_policy_fields() {
    let (_temp, root, first) = initialized();
    let second = Arc::new(
        DesktopBridge::new(
            root.clone(),
            root.join(if cfg!(windows) {
                "ai-runtime.exe"
            } else {
                "ai-runtime"
            }),
        )
        .unwrap(),
    );
    let a = first.clone();
    let b = second.clone();
    let (idle, verification) = tokio::join!(
        tokio::spawn(async move { a.save_idle_policy(900, Some(false)).await }),
        tokio::spawn(async move { b.save_verification_timeout(7200).await }),
    );
    // Fail-fast cross-process locking may admit one or both in sequence. Retry
    // only the known losing action; never replay a whole stale configuration.
    if let Err(error) = idle.unwrap() {
        assert_eq!(error.code, "runtime_running");
        first.save_idle_policy(900, Some(false)).await.unwrap();
    }
    if let Err(error) = verification.unwrap() {
        assert_eq!(error.code, "runtime_running");
        second.save_verification_timeout(7200).await.unwrap();
    }
    let saved =
        Config::from_toml(&std::fs::read_to_string(root.join("config.toml")).unwrap()).unwrap();
    assert!(!saved.runtime.idle_unload_enabled);
    assert_eq!(saved.runtime.idle_unload_seconds, 900);
    assert_eq!(saved.runtime.model_verification_timeout_seconds, 7200);
}

#[test]
fn older_settings_dto_defaults_new_fields_without_changing_old_idle_value() {
    let mut value = serde_json::to_value(desktop_bridge::DesktopSettings::default()).unwrap();
    value.as_object_mut().unwrap().remove("idle_unload_enabled");
    value
        .as_object_mut()
        .unwrap()
        .remove("model_verification_timeout_seconds");
    value["idle_unload_seconds"] = 120.into();
    let settings: desktop_bridge::DesktopSettings = serde_json::from_value(value).unwrap();
    assert!(settings.idle_unload_enabled);
    assert_eq!(settings.idle_unload_seconds, 120);
    assert_eq!(settings.model_verification_timeout_seconds, 300);
}

#[tokio::test]
async fn legacy_long_idle_ttl_is_read_and_preserved_by_unrelated_save() {
    let (_temp, root, bridge) = initialized();
    let mut config = Config::default();
    config.runtime.idle_unload_seconds = 172800;
    std::fs::write(root.join("config.toml"), config.to_toml().unwrap()).unwrap();
    let before = bridge.snapshot().await.unwrap();
    assert_eq!(before.settings.idle_unload_seconds, 172800);
    assert!(before.settings.idle_unload_enabled);
    let after = bridge.save_verification_timeout(7200).await.unwrap();
    assert_eq!(after.settings.idle_unload_seconds, 172800);
    assert_eq!(
        bridge.save_idle(172800).await.unwrap_err().code,
        "settings_invalid"
    );
    let fixed = bridge.save_idle_policy(86400, Some(false)).await.unwrap();
    assert_eq!(fixed.settings.idle_unload_seconds, 86400);
    assert!(!fixed.settings.idle_unload_enabled);
}
