fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let attributes = tauri_build::Attributes::new().app_manifest(
            tauri_build::AppManifest::new().commands(&[
                "desktop_snapshot",
                "runtime_start",
                "model_pick",
                "model_import",
                "model_directory_pick",
                "model_directory_apply",
                "model_directory_discover",
                "model_catalog",
                "model_download_start",
                "model_download_next",
                "model_download_cancel",
                "models_scan",
                "models_reconcile",
                "model_test",
                "model_library_next",
                "model_library_cancel",
                "models_page",
                "model_load",
                "model_unload",
                "chat_start",
                "chat_next",
                "chat_cancel",
                "settings_save",
                "runtime_idle_save",
                "token_copy",
                "runtime_stop",
                "desktop_close",
            ]),
        );
        tauri_build::try_build(attributes).expect("Nexa desktop build configuration is invalid");
    }
}
