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

struct NoInference;
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
            ExecutorCommand::Generate { .. } => {
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
    state: ApiState,
    router: Router,
    bearer: HeaderValue,
}
impl Harness {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let store = ApiState::open_store(root.path().to_path_buf())
            .await
            .unwrap();
        let config = Config::default();
        let resolver = store.clone();
        let runtime = Runtime::spawn(
            config.runtime_config(),
            move |id: &ModelId| resolver.resolve(id),
            NoInference,
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
            _root: root,
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
        std::fs::read_dir(harness._root.path().join("imports"))
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
                context_limit: 4096,
                default_context: 4096,
                loadable: true,
            })
        },
        NoInference,
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
