//! Opt-in real parent/worker/HTTP OCR regression, with isolated managed assets.
//! The fixed files and a synthetic image are supplied by the caller; no download.
use http_body_util::BodyExt;
use hyper::{HeaderMap, Method};
use runtime_api::{Config, token::load_private_token};
use runtime_cli::{
    client::{RequestBody, VerifiedConnection},
    instance::Discovery,
};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}
async fn connection(root: &Path, instance: &Discovery) -> VerifiedConnection {
    VerifiedConnection::connect(
        instance.listen,
        instance.instance_id,
        load_private_token(root).unwrap(),
    )
    .await
    .unwrap()
}
fn input(name: &str) -> PathBuf {
    PathBuf::from(
        std::env::var_os(name).unwrap_or_else(|| panic!("set {name} to the fixed test asset")),
    )
    .canonicalize()
    .unwrap()
}
async fn stream_text(client: &mut VerifiedConnection, request: &Value) -> (String, Value) {
    let response = client
        .request(
            Method::POST,
            "/v1/chat/completions",
            RequestBody::fixed(serde_json::to_vec(request).unwrap()),
            HeaderMap::new(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let mut body = response.into_body();
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        if let Ok(data) = frame.unwrap().into_data() {
            assert!(bytes.len() + data.len() <= 1024 * 1024);
            bytes.extend_from_slice(&data);
        }
    }
    let wire = String::from_utf8(bytes).unwrap();
    assert_eq!(wire.matches("data: [DONE]").count(), 1);
    let mut text = String::new();
    let mut usage = Value::Null;
    for line in wire.lines().filter_map(|line| line.strip_prefix("data: ")) {
        if line == "[DONE]" {
            continue;
        }
        let value: Value = serde_json::from_str(line).unwrap();
        assert!(value.get("error").is_none());
        if let Some(delta) = value["choices"][0]["delta"]["content"].as_str() {
            text.push_str(delta);
        }
        if value["usage"].is_object() {
            usage = value["usage"].clone();
        }
    }
    (text, usage)
}

#[tokio::test]
#[ignore = "requires built sibling worker, official GLM-OCR Q8 pair, and synthetic invoice data URL"]
async fn real_pair_import_load_image_http_cancel_and_recovery() {
    let model = input("NEXA_OCR_MODEL");
    let projector = input("NEXA_OCR_PROJECTOR");
    let image = fs::read_to_string(input("NEXA_OCR_IMAGE_DATA_URL")).unwrap();
    let image = image.trim();
    runtime_types::ImageInput::from_data_url(image).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("data");
    let binary = Path::new(env!("CARGO_BIN_EXE_ai-runtime"));
    assert!(
        binary
            .with_file_name(if cfg!(windows) {
                "ai-runtime-worker.exe"
            } else {
                "ai-runtime-worker"
            })
            .is_file()
    );
    assert!(
        Command::new(binary)
            .arg("--data-dir")
            .arg(&root)
            .arg("init")
            .output()
            .unwrap()
            .status
            .success()
    );
    let mut config = Config::default();
    config.api.listen = "127.0.0.1:0".parse().unwrap();
    config.inference.context_size = 8192;
    config.inference.batch_size = 256;
    config.inference.threads = Some(4);
    config.runtime.idle_unload_enabled = false;
    fs::write(root.join("config.toml"), config.to_toml().unwrap()).unwrap();
    let mut child = Running(
        Command::new(binary)
            .arg("--data-dir")
            .arg(&root)
            .arg("serve")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    let instance = loop {
        if let Ok(instance) = Discovery::read(&root) {
            break instance;
        }
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "test service exited before publishing discovery"
        );
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(25)).await;
    };
    assert_eq!(instance.pid, child.0.id());
    let mut client = connection(&root, &instance).await;
    client.json(Method::POST, "/runtime/models/import", Some(&json!({
        "id":"glm-ocr-real", "file":model,
        "expected_sha256":"45bc244a6446aff850521dc41f18bc8d7105ad5f0c2c8c28af04e7cc4f4d50b1",
        "projector":{"file":projector,"expected_sha256":"9c4b58e33e316ed142eb5dcb41abec3844d3e6e5dc361ffb782c3fa9d175141f"}
    }))).await.unwrap();
    let inventory = client
        .json(Method::GET, "/runtime/models", None)
        .await
        .unwrap();
    assert_eq!(inventory["data"][0]["has_projector"], true);
    assert_eq!(inventory["data"][0]["projector_size_bytes"], 484403648u64);
    let load = json!({"model":"glm-ocr-real","context_size":8192,"batch_size":256,"threads":4});
    let loaded = client
        .json(Method::POST, "/runtime/load-and-test", Some(&load))
        .await
        .unwrap();
    assert_eq!(loaded["local_validation"]["state"], "loaded");
    assert_eq!(loaded["local_validation"]["load_success"], true);
    assert_eq!(loaded["local_validation"]["generation_pass"], false);
    let mut request = json!({"model":"glm-ocr-real","messages":[{"role":"user","content":[
        {"type":"image_url","image_url":{"url":image}},{"type":"text","text":"Text Recognition:"}
    ]}],"max_tokens":256,"temperature":0,"stream":true,"stream_options":{"include_usage":true}});
    let started = Instant::now();
    let (text, usage) = stream_text(&mut client, &request).await;
    for expected in [
        "Nexa OCR baseline",
        "Invoice ID: INV-2026-1007",
        "Total: 123.45 USD",
    ] {
        assert!(
            text.contains(expected),
            "synthetic OCR anchor missing: {expected}"
        );
    }
    assert!(
        usage["prompt_tokens"].as_u64().unwrap() > 100,
        "image tokens must count toward usage"
    );
    eprintln!(
        "real HTTP OCR: 3/3 anchors, prompt_tokens={}, elapsed_ms={}",
        usage["prompt_tokens"],
        started.elapsed().as_millis()
    );

    // A separately authenticated control connection must cancel a pending
    // image request before its response headers without occupying its FIFO.
    let id = uuid::Uuid::new_v4();
    let mut pending_client = connection(&root, &instance).await;
    let pending_body = serde_json::to_vec(&request).unwrap();
    let pending = tokio::spawn(async move {
        let mut headers = HeaderMap::new();
        headers.insert("x-request-id", id.to_string().parse().unwrap());
        let response = pending_client
            .request(
                Method::POST,
                "/v1/chat/completions",
                RequestBody::fixed(pending_body),
                headers,
            )
            .await
            .unwrap();
        assert!(
            !response.status().is_success(),
            "early cancelled request must not produce successful SSE"
        );
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let error: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(error["error"]["code"], "request_cancelled");
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let status = client
            .json(Method::GET, "/runtime/status", None)
            .await
            .unwrap();
        if status["active_request"] == id.to_string() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "request did not enter active slot"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    client
        .json(
            Method::POST,
            &format!("/runtime/requests/{id}/cancel"),
            None,
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(30), pending)
        .await
        .unwrap()
        .unwrap();
    request["stream"] = false.into();
    request.as_object_mut().unwrap().remove("stream_options");
    let recovered = client
        .json(Method::POST, "/v1/chat/completions", Some(&request))
        .await
        .unwrap();
    assert!(
        recovered["choices"][0]["message"]["content"]
            .as_str()
            .unwrap()
            .contains("INV-2026-1007")
    );
    client
        .json(Method::POST, "/runtime/shutdown", None)
        .await
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
