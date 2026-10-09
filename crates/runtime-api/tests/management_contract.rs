//! In-process HTTP/actor/store contract tests. No fake executor is called model inference.
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderValue, Request, StatusCode},
};
use runtime_api::{
    ApiState, Config, router,
    security::{PeerEndpoints, SecurityContext},
    token::SecretToken,
};
use runtime_core::{
    CancellationHandle, ExecutionEvents, Executor, ExecutorCommand, ExecutorEvent, Runtime,
};
use runtime_types::{ErrorCode, ModelId, RuntimeError};
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;

struct NoInference(std::sync::Arc<std::sync::Mutex<Option<runtime_types::GenerationOptions>>>);
impl Executor for NoInference {
    fn start(
        &mut self,
        command: ExecutorCommand,
        events: ExecutionEvents,
    ) -> Result<CancellationHandle, RuntimeError> {
        match command {
            ExecutorCommand::Load { .. } => {
                events.emit(ExecutorEvent::Loaded);
            }
            ExecutorCommand::Unload => {
                events.emit(ExecutorEvent::Unloaded);
            }
            ExecutorCommand::Generate { request } => {
                *self.0.lock().unwrap() = Some(request.options);
                return Err(RuntimeError::new(
                    ErrorCode::UnsupportedModel,
                    "not an inference test",
                ));
            }
        }
        Ok(CancellationHandle::noop())
    }
}
struct Harness {
    _root: tempfile::TempDir,
    root: std::path::PathBuf,
    seen: std::sync::Arc<std::sync::Mutex<Option<runtime_types::GenerationOptions>>>,
    state: ApiState,
    router: Router,
    bearer: HeaderValue,
}
impl Harness {
    async fn new() -> Self {
        Self::configured(false).await
    }
    async fn configured(unified: bool) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("private");
        runtime_api::token::create_private_dir(&root).unwrap();
        runtime_api::token::create_private_dir(&root.join("runtime")).unwrap();
        let mut config = Config::default();
        if unified {
            config.schema_version = 2;
        }
        runtime_api::token::write_private_new(
            &root.join("config.toml"),
            config.to_toml().unwrap().as_bytes(),
        )
        .unwrap();
        let store = ApiState::open_store(root.clone()).await.unwrap();
        let resolver = store.clone();
        let seen = Arc::new(std::sync::Mutex::new(None));
        let runtime = Runtime::spawn(
            config.runtime_config(),
            move |id: &ModelId| resolver.resolve(id),
            NoInference(seen.clone()),
        )
        .unwrap();
        let state = ApiState::new(runtime, store, config.clone(), None);
        state.initialize_registry().await.unwrap();
        let token = SecretToken::generate().unwrap();
        let bearer = token.bearer_header_value();
        let security = Arc::new(
            SecurityContext::new(token, uuid::Uuid::new_v4(), config.api.listen, vec![]).unwrap(),
        );
        let router = router(state.clone(), security);
        Self {
            _root: temp,
            seen,
            root,
            state,
            router,
            bearer,
        }
    }
    async fn call(&self, method: &str, path: &str, body: &str) -> (StatusCode, Value) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("host", "127.0.0.1:18080")
            .header("authorization", &self.bearer)
            .header("content-type", "application/json")
            .body(Body::from(body.to_owned()))
            .unwrap();
        request.extensions_mut().insert(PeerEndpoints {
            client: "127.0.0.1:30000".parse().unwrap(),
            server: "127.0.0.1:18080".parse().unwrap(),
        });
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap())
    }
    async fn close(&self) {
        self.state.shutdown.begin();
        self.state.shutdown.wait().await.unwrap();
    }
}
fn synthetic_gguf() -> Vec<u8> {
    fn text(bytes: &mut Vec<u8>, value: &str) {
        bytes.extend((value.len() as u64).to_le_bytes());
        bytes.extend(value.as_bytes());
    }
    let mut bytes = b"GGUF".to_vec();
    bytes.extend(3_u32.to_le_bytes());
    bytes.extend(1_u64.to_le_bytes());
    bytes.extend(4_u64.to_le_bytes());
    for (name, value) in [
        ("general.architecture", "qwen3"),
        ("tokenizer.chat_template", "synthetic test template"),
    ] {
        text(&mut bytes, name);
        bytes.extend(8_u32.to_le_bytes());
        text(&mut bytes, value);
    }
    for (name, value) in [
        ("general.file_type", 7_u32),
        ("qwen3.context_length", 40960),
    ] {
        text(&mut bytes, name);
        bytes.extend(4_u32.to_le_bytes());
        bytes.extend(value.to_le_bytes());
    }
    text(&mut bytes, "synthetic.weight");
    bytes.extend(1_u32.to_le_bytes());
    bytes.extend(16_u64.to_le_bytes());
    bytes.extend(0_u32.to_le_bytes());
    bytes.extend(0_u64.to_le_bytes());
    bytes.resize(bytes.len().next_multiple_of(32), 0);
    bytes.resize(bytes.len() + 64, 0);
    bytes
}
#[tokio::test]
async fn v1_model_names_preserve_ids_duplicates_and_lan_resident_scope() {
    async fn lan_models(state: ApiState) -> Value {
        let config = runtime_api::LanApiConfig {
            enabled: true,
            listen: Some("192.168.10.2:18081".parse().unwrap()),
            allowed_cidrs: vec!["192.168.10.3/32".into()],
        };
        let token = SecretToken::generate().unwrap();
        let bearer = token.bearer_header_value();
        let security = Arc::new(runtime_api::lan::LanSecurityContext::new(token, &config).unwrap());
        let mut request = Request::builder()
            .uri("/v1/models")
            .header("host", "192.168.10.2:18081")
            .header("authorization", bearer)
            .body(Body::empty())
            .unwrap();
        request.extensions_mut().insert(PeerEndpoints {
            client: "192.168.10.3:25000".parse().unwrap(),
            server: config.listen.unwrap(),
        });
        let response = runtime_api::routes::lan_router(state, security)
            .oneshot(request)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap()).unwrap()
    }

    let harness = Harness::new().await;
    let source = tempfile::tempdir().unwrap();
    let path = source.path().join("synthetic.gguf");
    std::fs::write(&path, synthetic_gguf()).unwrap();
    let name = "自定义 Qwen 模型 · Q8_0 \"副本\"";
    for id in ["ext-a", "ext-b", "legacy-id"] {
        let mut import = json!({"id":id,"file":path});
        if id != "legacy-id" {
            import["display_name"] = json!(name);
        }
        let (status, result) = harness
            .call("POST", "/runtime/models/import", &import.to_string())
            .await;
        assert_eq!(status, StatusCode::OK, "{result}");
    }
    let (_, local) = harness.call("GET", "/v1/models", "").await;
    assert_eq!(
        local["data"],
        json!([
            {"id":"ext-a","display_name":name,"object":"model","owned_by":"local"},
            {"id":"ext-b","display_name":name,"object":"model","owned_by":"local"},
            {"id":"legacy-id","display_name":"legacy-id","object":"model","owned_by":"local"}
        ])
    );
    let (_, page) = harness.call("GET", "/v1/models?limit=1", "").await;
    assert_eq!(page["data"], json!([local["data"][0].clone()]));
    assert_eq!(page["next_after"], "ext-a");
    assert_eq!(lan_models(harness.state.clone()).await["data"], json!([]));

    // Equal display names do not become aliases or merge identities. Existing
    // ID-based loads still work and LAN only advertises the resident model.
    for (index, id) in ["ext-a", "ext-b"].into_iter().enumerate() {
        let (status, result) = harness
            .call(
                "POST",
                "/runtime/load",
                &json!({"model":id,"context_size":2048}).to_string(),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{result}");
        let lan = lan_models(harness.state.clone()).await;
        assert_eq!(lan["data"], json!([local["data"][index].clone()]));
        let (status, _) = harness.call("POST", "/runtime/unload", "{}").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(lan_models(harness.state.clone()).await["data"], json!([]));
    }
    harness.close().await;
}

#[tokio::test]
async fn safe_model_summaries_paginate_without_exposing_manifest_or_sources() {
    let harness = Harness::new().await;
    let source = tempfile::tempdir().unwrap();
    let path = source.path().join("synthetic.gguf");
    std::fs::write(&path, synthetic_gguf()).unwrap();
    for id in ["a", "b", "c"] {
        let (status, result) = harness
            .call(
                "POST",
                "/runtime/models/import",
                &json!({"id":id,"file":path}).to_string(),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{result}");
        assert_eq!(result["id"], id);
        assert_eq!(result["model"]["validated"], false);
        assert_eq!(result["model"]["compatibility"], "unvalidated");
        assert_eq!(result["model"]["loadable"], true);
        assert_eq!(result["model"]["available"], true);
        assert_eq!(result["model"]["context_limit"], 40960);
        assert!(result["model"]["availability_error"].is_null());
    }
    let (status, page) = harness.call("GET", "/runtime/models?limit=2", "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["data"].as_array().unwrap().len(), 2);
    assert_eq!(page["next_after"], "b");
    assert_eq!(page["data"][0]["compatibility"], "unvalidated");
    assert!(page["data"][0]["availability_error"].is_null());
    assert_eq!(page["data"][0]["loadable"], true);
    // The controlled candidate reaches the executor without promoting evidence.
    // This executor ACK is a scheduling test, never native-model verification.
    let (status, _) = harness
        .call(
            "POST",
            "/runtime/load",
            &json!({"model":"a","context_size":2048}).to_string(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, error) = harness
        .call(
            "POST",
            "/v1/chat/completions",
            &json!({"model":"a","messages":[{"role":"user","content":"test"}]}).to_string(),
        )
        .await;
    assert_eq!(error["error"]["code"], "unsupported_model");
    let encoded = page.to_string();
    for forbidden in [
        "source",
        "relative_file",
        "extra",
        source.path().to_str().unwrap(),
    ] {
        assert!(!encoded.contains(forbidden));
    }
    let (_, last) = harness
        .call("GET", "/runtime/models?limit=2&after=b", "")
        .await;
    assert_eq!(last["data"][0]["id"], "c");
    assert!(last["next_after"].is_null());
    let (_, available) = harness.call("GET", "/v1/models", "").await;
    assert_eq!(available["data"].as_array().unwrap().len(), 3);
    for query in [
        "limit=0",
        "limit=129",
        "after=../a",
        "limit=2&limit=3",
        "surprise=1",
    ] {
        assert_eq!(
            harness
                .call("GET", &format!("/runtime/models?{query}"), "")
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }
    harness.close().await;
}
#[tokio::test]
async fn management_errors_status_and_shutdown_use_the_public_envelope() {
    let harness = Harness::new().await;
    let (_, status) = harness.call("GET", "/runtime/status", "").await;
    assert_eq!(status["state"], "unloaded");
    assert!(status["memory"]["worker_private_bytes"].is_null());
    assert!(status["backend"].is_null());
    for (method, path, body, expected, code) in [
        (
            "POST",
            "/runtime/load",
            r#"{"model":"a"}"#,
            StatusCode::NOT_FOUND,
            "model_not_found",
        ),
        (
            "POST",
            "/runtime/load",
            r#"{"model":"/tmp/model","context_size":2048}"#,
            StatusCode::BAD_REQUEST,
            "invalid_request",
        ),
        (
            "POST",
            "/runtime/unload",
            r#"{"unknown":true}"#,
            StatusCode::BAD_REQUEST,
            "invalid_request",
        ),
        ("GET", "/missing", "", StatusCode::NOT_FOUND, "not_found"),
        (
            "GET",
            "/runtime/load",
            "",
            StatusCode::METHOD_NOT_ALLOWED,
            "method_not_allowed",
        ),
    ] {
        let (status, error) = harness.call(method, path, body).await;
        assert_eq!(status, expected, "{error}");
        assert_eq!(error["error"]["code"], code);
    }
    let (status, body) = harness.call("POST", "/runtime/shutdown", "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "stopped");
    harness.state.shutdown.wait().await.unwrap();
}
#[tokio::test]
async fn reservation_is_visible_and_controls_remain_available_over_http() {
    let harness = Harness::new().await;
    let lease = harness
        .state
        .control(|runtime| runtime.reserve_registry())
        .await
        .unwrap();
    let (_, status) = harness.call("GET", "/runtime/status", "").await;
    assert_eq!(status["registry_busy"], true);
    let (status, body) = harness.call("POST", "/runtime/unload", "").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"]["code"], "runtime_busy");
    let (status, _) = harness
        .call(
            "POST",
            &format!("/runtime/requests/{}/cancel", uuid::Uuid::new_v4()),
            "",
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    drop(lease);
    harness.close().await;
}

#[tokio::test]
async fn dropped_import_future_cancels_only_that_copy_and_releases_after_cleanup() {
    let harness = Harness::new().await;
    let sources = tempfile::tempdir().unwrap();
    let large = sources.path().join("cancelled.gguf");
    std::fs::File::create(&large)
        .unwrap()
        .set_len(32 * 1024 * 1024)
        .unwrap();
    let state = harness.state.clone();
    let request = runtime_api::dto::ImportModelRequest::parse(
        &serde_json::to_vec(&json!({"id":"cancelled","file":large})).unwrap(),
    )
    .unwrap();
    let operation = tokio::spawn(async move { state.import(request).await });
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if harness
            .state
            .control(|runtime| runtime.status())
            .await
            .unwrap()
            .registry_busy
        {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline);
        tokio::task::yield_now().await;
    }
    operation.abort();
    let _ = operation.await;
    loop {
        if !harness
            .state
            .control(|runtime| runtime.status())
            .await
            .unwrap()
            .registry_busy
        {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
    assert_eq!(
        std::fs::read_dir(harness.root.join("imports"))
            .unwrap()
            .count(),
        0
    );
    let good = sources.path().join("good.gguf");
    std::fs::write(&good, synthetic_gguf()).unwrap();
    let (status, model) = harness
        .call(
            "POST",
            "/runtime/models/import",
            &json!({"id":"next","file":good}).to_string(),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{model}");
    harness.close().await;
}

#[tokio::test]
async fn current_model_submission_never_prepares_an_external_registration() {
    use model_store::library::{LIBRARY_FILE, ScanControl, scan_directory};
    use runtime_types::{
        GenerationOptions, GenerationRequest, LoadOptions, Message, RequestId, ResolvedModel, Role,
    };
    let root = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let file = source.path().join("external.gguf");
    std::fs::write(&file, synthetic_gguf()).unwrap();
    let outcome =
        scan_directory(root.path(), source.path(), None, &ScanControl::default()).unwrap();
    let library = outcome.library().unwrap();
    let model = library.models[0].manifest.id.clone();
    std::fs::write(root.path().join(LIBRARY_FILE), library.encode().unwrap()).unwrap();
    drop(outcome);
    let store = ApiState::open_store(root.path().to_path_buf())
        .await
        .unwrap();
    assert!(store.needs_external_preparation(&model).unwrap());
    // Synthetic loaded executor state is deliberately independent of storage:
    // a current request must not hash/open/prepare the registered file at all.
    let runtime = Runtime::spawn(
        Config::default().runtime_config(),
        |id: &ModelId| {
            Ok(ResolvedModel {
                id: id.clone(),
                path: "not-read".into(),
                projector_path: None,
                context_limit: 4096,
                default_context: 4096,
                loadable: true,
            })
        },
        NoInference(Arc::new(std::sync::Mutex::new(None))),
    )
    .unwrap();
    let state = ApiState::new(runtime, store.clone(), Config::default(), None);
    state.initialize_registry().await.unwrap();
    let selected = model.clone();
    state
        .control(move |runtime| runtime.load(selected, LoadOptions::default()))
        .await
        .unwrap();
    // If current admission accidentally uses the explicit prepare path, this
    // unavailable external source must fail before any request is accepted.
    std::fs::remove_file(file).unwrap();
    let request = GenerationRequest {
        request_id: RequestId::new(),
        model: model.clone(),
        messages: vec![Message::new(Role::User, "synthetic")],
        options: GenerationOptions::default(),
    };
    let (actual, events) = state
        .submit_current(
            request.request_id,
            request.messages.clone(),
            request.options.clone(),
        )
        .await
        .unwrap();
    assert_eq!(actual, model);
    tokio::task::spawn_blocking(move || {
        assert!(matches!(
            events.recv().unwrap().kind,
            runtime_types::RequestEventKind::Accepted
        ));
        while let Some(event) = events.recv() {
            if event.kind.is_terminal() {
                break;
            }
        }
    })
    .await
    .unwrap();
    assert!(store.needs_external_preparation(&model).unwrap());
    assert!(
        state.submit(request).await.is_err(),
        "explicit admission must still prepare external files"
    );
    state.shutdown.begin();
    state.shutdown.wait().await.unwrap();
}

#[tokio::test]
async fn unified_profiles_live_save_explicit_override_active_snapshot_and_external_edit() {
    let h = Harness::configured(true).await;
    let source = tempfile::tempdir().unwrap();
    let path = source.path().join("synthetic.gguf");
    std::fs::write(&path, synthetic_gguf()).unwrap();
    for id in ["a", "b"] {
        assert_eq!(
            h.call(
                "POST",
                "/runtime/models/import",
                &json!({"id":id,"file":path}).to_string()
            )
            .await
            .0,
            StatusCode::OK
        );
    }
    let (status, initial) = h.call("GET", "/runtime/configuration", "").await;
    assert_eq!(status, StatusCode::OK);
    let update = |revision: &Value, id: &str, context: u32| {
        json!({"expected_revision":revision,"update":{"kind":"model_profile","model_id":id,"load_overrides":{"context_size":context,"threads":2,"batch_size":128}}}).to_string()
    };
    let (status, a) = h
        .call(
            "PUT",
            "/runtime/configuration",
            &update(&initial["revision"], "a", 2048),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{a}");
    let (_, b) = h
        .call(
            "PUT",
            "/runtime/configuration",
            &update(&a["revision"], "b", 1024),
        )
        .await;
    let _ = h
        .call(
            "POST",
            "/v1/chat/completions",
            r#"{"model":"a","messages":[{"role":"user","content":"cold"}]}"#,
        )
        .await;
    let (_, cold) = h.call("GET", "/runtime/status", "").await;
    assert_eq!(
        cold["load_options"],
        json!({"context_size":2048,"threads":2,"batch_size":128})
    );
    let (status, loaded) = h
        .call("POST", "/runtime/load", r#"{"model":"a","threads":3}"#)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        loaded["load_options"],
        json!({"context_size":2048,"threads":3,"batch_size":128})
    );
    let (status, saved) = h
        .call(
            "PUT",
            "/runtime/configuration",
            &update(&b["revision"], "a", 3072),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, model) = h.call("GET", "/runtime/configuration/models/a", "").await;
    assert_eq!(model["current_load_options"]["context_size"], 2048);
    assert_eq!(model["saved_effective"]["context_size"], 3072);
    assert_eq!(model["pending_apply"], true);
    let (status, _) = h
        .call(
            "POST",
            "/runtime/model-test",
            r#"{"model":"a","backend":"cuda"}"#,
        )
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let (_, same) = h.call("GET", "/runtime/status", "").await;
    assert_eq!(same["load_options"], loaded["load_options"]);
    let (_, reloaded) = h.call("POST", "/runtime/load", r#"{"model":"a"}"#).await;
    assert_eq!(reloaded["load_options"]["context_size"], 3072);
    assert_eq!(reloaded["load_options"]["threads"], 2);
    let (status,changed)=h.call("PUT","/runtime/configuration",&json!({"expected_revision":saved["revision"],"update":{"kind":"request_defaults","request_defaults":{"max_output_tokens":99,"temperature":0.2,"top_p":0.8}}}).to_string()).await;
    assert_eq!(status, StatusCode::OK, "{changed}");
    assert_eq!(
        h.state.active_config().unwrap().inference.max_output_tokens,
        99
    );
    let _ = h
        .call(
            "POST",
            "/v1/chat/completions",
            r#"{"model":"a","messages":[{"role":"user","content":"new-default"}]}"#,
        )
        .await;
    assert_eq!(h.seen.lock().unwrap().as_ref().unwrap().max_tokens, 99);
    assert_eq!(h.seen.lock().unwrap().as_ref().unwrap().temperature, 0.2);

    let mut disk = runtime_api::configuration::read(&h.root).unwrap().config;
    disk.inference.context_size = 8192;
    runtime_api::token::atomic_replace_private(
        &h.root.join("config.toml"),
        disk.to_toml().unwrap().as_bytes(),
    )
    .unwrap();
    let (_, pending) = h.call("GET", "/runtime/configuration", "").await;
    assert_eq!(pending["pending_restart"], true);
    assert_eq!(pending["saved"]["global_defaults"]["context_size"], 8192);
    assert_eq!(
        pending["runtime_effective"]["values"]["global_defaults"]["context_size"],
        4096
    );
    let (status, error) = h
        .call(
            "PUT",
            "/runtime/configuration",
            &update(&pending["revision"], "b", 2048),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["error"]["code"], "configuration_restart_required");
    let (_, switched) = h.call("POST", "/runtime/load", r#"{"model":"b"}"#).await;
    assert_eq!(switched["load_options"]["context_size"], 1024);
    assert!(!pending.to_string().contains("token_file"));
    h.close().await;
}

struct QueueProbe {
    seen: Arc<std::sync::Mutex<Vec<runtime_types::GenerationOptions>>>,
    held: Arc<std::sync::Mutex<Option<ExecutionEvents>>>,
}
impl Executor for QueueProbe {
    fn start(
        &mut self,
        command: ExecutorCommand,
        events: ExecutionEvents,
    ) -> Result<CancellationHandle, RuntimeError> {
        match command {
            ExecutorCommand::Load { .. } => {
                events.emit(ExecutorEvent::Loaded);
            }
            ExecutorCommand::Unload => {
                events.emit(ExecutorEvent::Unloaded);
            }
            ExecutorCommand::Generate { request } => {
                let mut seen = self.seen.lock().unwrap();
                seen.push(request.options);
                let first = seen.len() == 1;
                drop(seen);
                events.emit(ExecutorEvent::Prepared { prompt_tokens: 8 });
                if first {
                    *self.held.lock().unwrap() = Some(events.clone());
                    return Ok(CancellationHandle::new(move || {
                        events.emit(ExecutorEvent::GenerationFailed {
                            error: RuntimeError::new(ErrorCode::RequestCancelled, "cancelled"),
                            usage: runtime_types::Usage {
                                prompt_tokens: 8,
                                completion_tokens: 0,
                            },
                        });
                    }));
                }
                finish_probe(&events);
            }
        }
        Ok(CancellationHandle::noop())
    }
}
fn finish_probe(events: &ExecutionEvents) {
    let permit = events.try_reserve_text(120 * 1024).unwrap();
    assert!(events.emit_reserved_text("ok".into(), permit));
    events.emit(ExecutorEvent::Completed {
        timings: None,
        usage: runtime_types::Usage {
            prompt_tokens: 8,
            completion_tokens: 1,
        },
        finish_reason: runtime_types::FinishReason::Stop,
    });
}
#[tokio::test]
async fn http_busy_profile_and_sampling_saves_preserve_queued_requests_and_idle_session() {
    use runtime_api::configuration;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("private");
    configuration::initialize(&root).unwrap();
    let mut config = configuration::read(&root).unwrap().config;
    config.runtime.idle_unload_seconds = 1;
    config.inference.max_output_tokens = 100;
    config.model_profiles.insert(
        ModelId::new("a").unwrap(),
        configuration::LoadOverrides {
            context_size: Some(2048),
            threads: Some(2),
            batch_size: Some(128),
        },
    );
    runtime_api::token::atomic_replace_private(
        &root.join("config.toml"),
        config.to_toml().unwrap().as_bytes(),
    )
    .unwrap();
    let store = ApiState::open_store(root.clone()).await.unwrap();
    let seen = Arc::new(std::sync::Mutex::new(vec![]));
    let held = Arc::new(std::sync::Mutex::new(None));
    let runtime = Runtime::spawn(
        config.runtime_config(),
        |id: &ModelId| {
            Ok(runtime_types::ResolvedModel {
                id: id.clone(),
                path: "synthetic".into(),
                projector_path: None,
                context_limit: 4096,
                default_context: 2048,
                loadable: true,
            })
        },
        QueueProbe {
            seen: seen.clone(),
            held: held.clone(),
        },
    )
    .unwrap();
    let state = ApiState::new(runtime, store, config.clone(), None);
    state.initialize_registry().await.unwrap();
    let token = SecretToken::generate().unwrap();
    let bearer = token.bearer_header_value();
    let security = Arc::new(
        SecurityContext::new(token, uuid::Uuid::new_v4(), config.api.listen, vec![]).unwrap(),
    );
    let h = Arc::new(Harness {
        _root: temp,
        root,
        state: state.clone(),
        router: router(state.clone(), security),
        bearer,
        seen: Arc::new(std::sync::Mutex::new(None)),
    });
    let body = r#"{"model":"a","messages":[{"role":"user","content":"test"}]}"#;
    let h1 = h.clone();
    let first = tokio::spawn(async move { h1.call("POST", "/v1/chat/completions", body).await });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while held.lock().unwrap().is_none() {
        assert!(std::time::Instant::now() < deadline);
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let h2 = h.clone();
    let second = tokio::spawn(async move { h2.call("POST", "/v1/chat/completions", body).await });
    while state.control(|r| r.status()).await.unwrap().queued_jobs != 1 {
        assert!(std::time::Instant::now() < deadline);
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let (_, before) = h.call("GET", "/runtime/configuration", "").await;
    let (status,profile)=h.call("PUT","/runtime/configuration",&json!({"expected_revision":before["revision"],"update":{"kind":"model_profile","model_id":"a","load_overrides":{"context_size":3072,"threads":3,"batch_size":256}}}).to_string()).await;
    assert_eq!(status, StatusCode::OK, "{profile}");
    let (status,sampling)=h.call("PUT","/runtime/configuration",&json!({"expected_revision":profile["revision"],"update":{"kind":"request_defaults","request_defaults":{"max_output_tokens":200,"temperature":0.1,"top_p":0.8}}}).to_string()).await;
    assert_eq!(status, StatusCode::OK, "{sampling}");
    let running = state.control(|r| r.status()).await.unwrap();
    assert_eq!(running.queued_jobs, 1);
    assert_eq!(running.load_options.unwrap().context_size, 2048);
    finish_probe(&held.lock().unwrap().take().unwrap());
    let first_result = first.await.unwrap();
    assert_eq!(first_result.0, StatusCode::OK, "{}", first_result.1);
    let second_result = second.await.unwrap();
    assert_eq!(second_result.0, StatusCode::OK, "{}", second_result.1);
    assert_eq!(seen.lock().unwrap()[0].max_tokens, 100);
    assert_eq!(seen.lock().unwrap()[1].max_tokens, 100);
    assert_eq!(
        h.call("POST", "/v1/chat/completions", body).await.0,
        StatusCode::OK
    );
    assert_eq!(seen.lock().unwrap()[2].max_tokens, 200);
    assert_eq!(seen.lock().unwrap()[2].temperature, 0.1);

    // The separate inference-only LAN router consumes the same active request
    // defaults, while caller fields still override only their own parameters.
    let lan_config = runtime_api::LanApiConfig {
        enabled: true,
        listen: Some("192.168.10.2:18081".parse().unwrap()),
        allowed_cidrs: vec!["192.168.10.3/32".into()],
    };
    let lan_token = SecretToken::generate().unwrap();
    let lan_bearer = lan_token.bearer_header_value();
    let lan_context =
        Arc::new(runtime_api::lan::LanSecurityContext::new(lan_token, &lan_config).unwrap());
    let lan = runtime_api::routes::lan_router(state.clone(), lan_context);
    for (body, expected_max, expected_top_p) in [
        (
            r#"{"messages":[{"role":"user","content":"lan-default"}]}"#,
            200,
            0.8,
        ),
        (
            r#"{"messages":[{"role":"user","content":"lan-explicit"}],"max_tokens":77,"top_p":0.4}"#,
            77,
            0.4,
        ),
    ] {
        let mut request = Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .header("host", "192.168.10.2:18081")
            .header("authorization", &lan_bearer)
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();
        request.extensions_mut().insert(PeerEndpoints {
            client: "192.168.10.3:25000".parse().unwrap(),
            server: "192.168.10.2:18081".parse().unwrap(),
        });
        let response = lan.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let _ = to_bytes(response.into_body(), 65536).await.unwrap();
        let seen = seen.lock().unwrap();
        assert_eq!(seen.last().unwrap().max_tokens, expected_max);
        assert_eq!(seen.last().unwrap().temperature, 0.1);
        assert_eq!(seen.last().unwrap().top_p, expected_top_p);
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
    while state.control(|r| r.status()).await.unwrap().state != runtime_types::ModelState::Unloaded
    {
        assert!(std::time::Instant::now() < deadline);
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let (_, empty) = h
        .call(
            "POST",
            "/v1/chat/completions",
            r#"{"messages":[{"role":"user","content":"empty"}]}"#,
        )
        .await;
    assert_eq!(empty["error"]["code"], "model_not_loaded");
    assert_eq!(
        h.call("POST", "/v1/chat/completions", body).await.0,
        StatusCode::OK
    );
    assert_eq!(
        state
            .control(|r| r.status())
            .await
            .unwrap()
            .load_options
            .unwrap()
            .context_size,
        2048
    );
    let (status, reloaded) = h.call("POST", "/runtime/load", r#"{"model":"a"}"#).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(reloaded["load_options"]["context_size"], 3072);
    assert_eq!(reloaded["load_options"]["threads"], 3);
    h.close().await;
}

#[tokio::test]
async fn owned_load_routes_are_strict_scoped_and_return_json_for_bad_identity() {
    let h = Harness::new().await;
    for path in [
        "/runtime/load-operations/not-a-uuid",
        "/runtime/load-operations/not-a-uuid/cancel",
    ] {
        let method = if path.ends_with("cancel") {
            "POST"
        } else {
            "GET"
        };
        let (status, body) = h.call(method, path, "{}").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["code"], "invalid_request");
    }
    for body in [
        json!({"model":"absent"}),
        json!({"model":"absent","operation_id":uuid::Uuid::nil()}),
        json!({"model":"absent","operation_id":uuid::Uuid::new_v4(),"only_if_unloaded":"yes"}),
        json!({"model":"absent","operation_id":uuid::Uuid::new_v4(),"path":"not allowed"}),
    ] {
        assert_eq!(
            h.call("POST", "/runtime/load-operations", &body.to_string())
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }
    let id = uuid::Uuid::new_v4();
    assert_eq!(
        h.call("GET", &format!("/runtime/load-operations/{id}"), "")
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        h.call(
            "POST",
            &format!("/runtime/load-operations/{id}/cancel"),
            "{}"
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let (status, body) = h
        .call(
            "POST",
            "/runtime/load-operations",
            &json!({"model":"absent","operation_id":id}).to_string(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["operation_id"], id.to_string());
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let (status, result) = h
                .call("GET", &format!("/runtime/load-operations/{id}"), "")
                .await;
            assert_eq!(status, StatusCode::OK);
            if result["terminal"] == true {
                assert_eq!(result["status"], "failed");
                assert_eq!(result["error"]["code"], "model_not_found");
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    h.close().await;
}

#[tokio::test]
async fn unregister_requires_exact_snapshot_is_strict_and_preserves_files_and_selected_safety() {
    let h = Harness::new().await;
    let source = h.root.join("original.gguf");
    let bytes = synthetic_gguf();
    std::fs::write(&source, &bytes).unwrap();
    let (status, _) = h
        .call(
            "POST",
            "/runtime/models/import",
            &json!({"id":"remove-me","file":source}).to_string(),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, page) = h.call("GET", "/runtime/models", "").await;
    let body = json!({"model_id":"remove-me","generation":page["generation"]});
    for invalid in [
        json!({"model_id":"remove-me"}),
        json!({"model_id":"remove-me","generation":uuid::Uuid::nil()}),
        json!({"model_id":"remove-me","generation":page["generation"],"delete_files":true}),
    ] {
        assert_eq!(
            h.call("POST", "/runtime/models/unregister", &invalid.to_string())
                .await
                .0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        h.call(
            "POST",
            "/runtime/models/unregister",
            &json!({"model_id":"remove-me","generation":uuid::Uuid::new_v4()}).to_string()
        )
        .await
        .1["error"]["code"],
        "model_list_changed"
    );
    let (status, response) = h
        .call("POST", "/runtime/load", r#"{"model":"remove-me"}"#)
        .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    let (_, rejected) = h
        .call("POST", "/runtime/models/unregister", &body.to_string())
        .await;
    assert_eq!(rejected["error"]["code"], "model_unregister_loaded");
    assert_eq!(
        h.call("POST", "/runtime/unload", "{}").await.0,
        StatusCode::OK
    );
    let (status, result) = h
        .call("POST", "/runtime/models/unregister", &body.to_string())
        .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(
        result,
        json!({"model_id":"remove-me","removed":true,"files_preserved":true})
    );
    let (_, status) = h.call("GET", "/runtime/status", "").await;
    assert!(status["selected_model"].is_null());
    assert!(status["load_options"].is_null());
    assert_eq!(
        h.call("POST", "/runtime/models/unregister", &body.to_string())
            .await
            .1["error"]["code"],
        "model_list_changed"
    );
    let (_, page) = h.call("GET", "/runtime/models", "").await;
    assert!(page["data"].as_array().unwrap().is_empty());
    assert_eq!(
        h.call("POST", "/runtime/load", r#"{"model":"remove-me"}"#)
            .await
            .1["error"]["code"],
        "model_not_found"
    );
    assert_eq!(std::fs::read(&source).unwrap(), bytes);
    assert_eq!(
        std::fs::read(h.root.join("models/remove-me/model.gguf")).unwrap(),
        bytes
    );
    h.close().await;
}

#[tokio::test]
async fn execution_timeout_configuration_requires_auth_and_stopped_runtime() {
    let h = Harness::configured(true).await;
    let (status, initial) = h.call("GET", "/runtime/configuration", "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        initial["saved"]["runtime"]["execution_timeout_seconds"],
        300
    );
    let before = std::fs::read(h.root.join("config.toml")).unwrap();
    let mut policies = initial["saved"]["runtime"].clone();
    policies["execution_timeout_seconds"] = json!(600);
    let body = json!({"expected_revision": initial["revision"], "update": {
        "kind": "runtime", "runtime": policies
    }})
    .to_string();
    let mut request = Request::builder()
        .method("PUT")
        .uri("/runtime/configuration")
        .header("host", "127.0.0.1:18080")
        .header("content-type", "application/json")
        .body(Body::from(body.clone()))
        .unwrap();
    request.extensions_mut().insert(PeerEndpoints {
        client: "127.0.0.1:30000".parse().unwrap(),
        server: "127.0.0.1:18080".parse().unwrap(),
    });
    let response = h.router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(std::fs::read(h.root.join("config.toml")).unwrap(), before);
    let (status, error) = h.call("PUT", "/runtime/configuration", &body).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error["error"]["code"], "runtime_running");
    assert_eq!(std::fs::read(h.root.join("config.toml")).unwrap(), before);
    let (_, unchanged) = h.call("GET", "/runtime/configuration", "").await;
    assert_eq!(unchanged["revision"], initial["revision"]);
    assert_eq!(
        unchanged["runtime_effective"]["values"]["runtime"]["execution_timeout_seconds"],
        300
    );
    assert_eq!(unchanged["pending_restart"], false);
    h.close().await;
}
