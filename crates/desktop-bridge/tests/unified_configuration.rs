use desktop_bridge::{ConnectionState, DesktopBridge, DesktopPreferences, ModelLoadProfileRequest};
use runtime_api::configuration::*;
use runtime_cli::instance::{Discovery, InstanceLock};
use runtime_types::ModelId;
fn bridge(temp: &tempfile::TempDir) -> DesktopBridge {
    DesktopBridge::new(
        temp.path().join("private"),
        temp.path().join(if cfg!(windows) {
            "ai-runtime.exe"
        } else {
            "ai-runtime"
        }),
    )
    .unwrap()
}
#[tokio::test]
async fn initialize_is_an_offline_explicit_action_and_cas_groups_remain_separate() {
    let temp = tempfile::tempdir().unwrap();
    let b = bridge(&temp);
    let root = temp.path().join("private");
    assert_eq!(b.configuration_get().await.unwrap().revision, "absent");
    assert!(!root.exists());
    let snapshot = b.initialize().await.unwrap();
    assert!(snapshot.initialized);
    assert!(matches!(snapshot.connection, ConnectionState::Stopped));
    assert!(snapshot.runtime.is_none());
    assert!(!root.join("secrets/lan-api-token").exists());
    assert!(!root.join("models").exists());
    let token = std::fs::read(root.join("secrets/api-token")).unwrap();
    let initial = snapshot.configuration.unwrap();
    let saved = b
        .configuration_save(ConfigurationSaveRequest {
            expected_revision: initial.revision.clone(),
            update: ConfigurationUpdate::ModelProfile {
                model_id: ModelId::new("a").unwrap(),
                load_overrides: LoadOverrides {
                    context_size: Some(2048),
                    threads: Some(2),
                    batch_size: Some(128),
                },
            },
        })
        .await
        .unwrap();
    let model = b.configuration_model_get("a".into()).await.unwrap();
    assert_eq!(model.saved_effective.context_size, 2048);
    assert!(model.current_load_options.is_none());
    assert!(model.restore_load_options.is_none());
    assert_eq!(
        b.settings_save(DesktopPreferences::default())
            .await
            .unwrap_err()
            .code,
        "configuration_revision_required"
    );
    let ui = b.ui_preferences_get().unwrap();
    b.ui_preferences_save(UiPreferencesSaveRequest {
        expected_revision: ui.revision,
        preferences: UiPreferences {
            close_runtime_on_exit: true,
            download_source: "huggingface".into(),
        },
    })
    .await
    .unwrap();
    assert_eq!(
        b.configuration_get().await.unwrap().revision,
        saved.revision
    );
    assert_eq!(
        std::fs::read(root.join("secrets/api-token")).unwrap(),
        token
    );
}
#[tokio::test]
async fn initialize_never_repairs_partial_data_or_uncertain_shutdown() {
    let temp = tempfile::tempdir().unwrap();
    let b = bridge(&temp);
    let root = temp.path().join("private");
    let lock = InstanceLock::try_acquire(&root).unwrap().unwrap();
    lock.publish(
        &Discovery::current(uuid::Uuid::new_v4(), "127.0.0.1:18080".parse().unwrap()).unwrap(),
    )
    .unwrap();
    drop(lock);
    assert_eq!(
        b.initialize().await.unwrap_err().code,
        "runtime_stop_unconfirmed"
    );
    assert!(!root.join("config.toml").exists());
    assert!(!root.join("secrets/api-token").exists());
}
#[test]
fn temporary_overrides_are_sparse_explicit_and_null_is_invalid() {
    let request: ModelLoadProfileRequest = serde_json::from_str(r#"{"model_id":"a"}"#).unwrap();
    assert!(request.load_overrides.threads.is_none());
    assert!(
        serde_json::from_str::<ModelLoadProfileRequest>(
            r#"{"model_id":"a","load_overrides":{"threads":null}}"#
        )
        .is_err()
    );
    assert!(
        serde_json::from_str::<ModelLoadProfileRequest>(
            r#"{"model_id":"a","load_overrides":{"threads":2}}"#
        )
        .is_ok()
    );
}
