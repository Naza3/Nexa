//! Independent black-box API oracle. It uses the proved client transport, but
//! never invokes the server's DTO, validation, SSE encoder, or scheduler code.
use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::{HeaderMap, Method, body::Incoming};
use runtime_cli::{
    client::{ClientError, RequestBody, VerifiedConnection, collect_bounded, connect_data_dir},
    instance::{Discovery, wait_stopped},
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::{ExitCode, Stdio},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use uuid::Uuid;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
#[derive(Serialize)]
struct Check {
    id: String,
    status: &'static str,
    detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    diagnostic: Option<ProbeFailure>,
}
/// Only fixed local categories and sanitized public error codes enter reports.
/// Raw errors, headers, response bodies, credentials, and paths never do.
#[derive(Clone, Debug, Serialize)]
struct ProbeFailure {
    stage: &'static str,
    category: &'static str,
    http_status: Option<u16>,
    api_code: Option<String>,
    idle_poll: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    transport: Option<TransportDiagnostic>,
}
#[derive(Clone, Debug, Serialize)]
struct TransportDiagnostic {
    stage: &'static str,
    error_canceled: bool,
    error_closed: bool,
    error_incomplete_message: bool,
    error_parse: bool,
    error_body_write_aborted: bool,
    error_timeout: bool,
    io_kind: Option<&'static str>,
    sender_ready: bool,
    sender_closed: bool,
}
impl std::fmt::Display for ProbeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.stage, self.category)
    }
}
impl std::error::Error for ProbeFailure {}
impl ProbeFailure {
    fn new(stage: &'static str, category: &'static str) -> Self {
        Self {
            stage,
            category,
            http_status: None,
            api_code: None,
            idle_poll: None,
            transport: None,
        }
    }
    fn from_error(stage: &'static str, error: &(dyn std::error::Error + 'static)) -> Self {
        if let Some(client) = error.downcast_ref::<ClientError>() {
            return match client {
                ClientError::Connection(message) => Self::new(stage, message),
                ClientError::Transport {
                    stage: transport_stage,
                    error_canceled,
                    error_closed,
                    error_incomplete_message,
                    error_parse,
                    error_body_write_aborted,
                    error_timeout,
                    io_kind,
                    sender_ready,
                    sender_closed,
                } => Self {
                    stage,
                    category: "hyper_transport",
                    http_status: None,
                    api_code: None,
                    idle_poll: None,
                    transport: Some(TransportDiagnostic {
                        stage: transport_stage,
                        error_canceled: *error_canceled,
                        error_closed: *error_closed,
                        error_incomplete_message: *error_incomplete_message,
                        error_parse: *error_parse,
                        error_body_write_aborted: *error_body_write_aborted,
                        error_timeout: *error_timeout,
                        io_kind: *io_kind,
                        sender_ready: *sender_ready,
                        sender_closed: *sender_closed,
                    }),
                },
                ClientError::Api { status, code, .. } => Self {
                    stage,
                    category: "api_error",
                    http_status: Some(*status),
                    api_code: code.as_deref().and_then(safe_symbol).map(str::to_owned),
                    idle_poll: None,
                    transport: None,
                },
            };
        }
        if let Some(io) = error.downcast_ref::<std::io::Error>() {
            let category = match io.kind() {
                std::io::ErrorKind::NotFound => "local_state_not_found",
                std::io::ErrorKind::PermissionDenied => "local_state_permission_denied",
                std::io::ErrorKind::InvalidData => "local_state_invalid_data",
                std::io::ErrorKind::ConnectionRefused => "connection_refused",
                std::io::ErrorKind::ConnectionReset => "connection_reset",
                std::io::ErrorKind::TimedOut => "io_timed_out",
                _ => "io_other",
            };
            return Self::new(stage, category);
        }
        if let Some(json) = error.downcast_ref::<serde_json::Error>() {
            let category = match json.classify() {
                serde_json::error::Category::Io => "json_io",
                serde_json::error::Category::Syntax => "json_syntax",
                serde_json::error::Category::Data => "json_data",
                serde_json::error::Category::Eof => "json_eof",
            };
            return Self::new(stage, category);
        }
        Self::new(stage, "unclassified_error")
    }
}
#[derive(Serialize)]
pub struct Report {
    schema_version: u32,
    kind: &'static str,
    platform: String,
    model: String,
    model_sha256: Option<String>,
    inference: Value,
    disconnect_cycles_requested: u32,
    disconnect_cycles_attempted: u32,
    disconnect_cycles_passed: u32,
    checks: Vec<Check>,
}
pub struct Options {
    pub address: SocketAddr,
    pub root: PathBuf,
    pub model: String,
    pub out: PathBuf,
    pub disconnect_cycles: u32,
    pub cli: Option<PathBuf>,
    pub release_acceptance: bool,
}
struct Oracle {
    options: Options,
    report: Report,
    phase: &'static str,
}
impl Oracle {
    fn check(&mut self, id: &str, pass: bool, detail: &str) {
        self.report.checks.push(Check {
            id: id.into(),
            status: if pass { "pass" } else { "fail" },
            detail: detail.into(),
            diagnostic: None,
        });
    }
    fn skip(&mut self, id: &str, detail: &str) {
        self.report.checks.push(Check {
            id: id.into(),
            status: "skipped",
            detail: detail.into(),
            diagnostic: None,
        });
    }
    async fn connect(&self) -> Result<VerifiedConnection> {
        connect_data_dir(&self.options.root).await
    }
    async fn request(
        &self,
        method: Method,
        path: &str,
        body: Vec<u8>,
        extra: HeaderMap,
    ) -> std::result::Result<(u16, Value), ProbeFailure> {
        let mut client = self
            .connect()
            .await
            .map_err(|e| ProbeFailure::from_error("connect_or_proof", e.as_ref()))?;
        let response = client
            .request(method, path, RequestBody::fixed(body), extra)
            .await
            .map_err(|e| ProbeFailure::from_error("http_request", &e))?;
        let status = response.status().as_u16();
        let data = collect_bounded(response.into_body(), 1024 * 1024, Duration::from_secs(750))
            .await
            .map_err(|e| {
                let mut diagnostic = ProbeFailure::from_error("response_body", &e);
                diagnostic.http_status = Some(status);
                diagnostic
            })?;
        let value = serde_json::from_slice(&data).map_err(|e| {
            let mut diagnostic = ProbeFailure::from_error("response_json", &e);
            diagnostic.http_status = Some(status);
            diagnostic
        })?;
        Ok((status, value))
    }
    async fn get(&self, path: &str) -> std::result::Result<(u16, Value), ProbeFailure> {
        self.request(Method::GET, path, Vec::new(), HeaderMap::new())
            .await
    }
    async fn post(&self, path: &str, value: Value) -> Result<(u16, Value)> {
        Ok(self
            .request(
                Method::POST,
                path,
                serde_json::to_vec(&value)?,
                HeaderMap::new(),
            )
            .await?)
    }
    fn chat(&self, tokens: u32, stream: bool) -> Value {
        json!({"model":self.options.model,"messages":[{"role":"user","content":"用中文和 English 简洁解释本地推理。"}],"max_tokens":tokens,"temperature":0,"seed":42,"stream":stream})
    }
    async fn idle(&self) -> std::result::Result<Value, ProbeFailure> {
        let start = tokio::time::Instant::now();
        let mut polls = 0;
        loop {
            polls += 1;
            let (status, value) = self
                .get("/runtime/status")
                .await
                .map_err(|mut diagnostic| {
                    diagnostic.idle_poll = Some(polls);
                    diagnostic
                })?;
            if status != 200 {
                return Err(ProbeFailure {
                    stage: "http_status",
                    category: "non_success_status",
                    http_status: Some(status),
                    api_code: value["error"]["code"]
                        .as_str()
                        .and_then(safe_symbol)
                        .map(str::to_owned),
                    idle_poll: Some(polls),
                    transport: None,
                });
            }
            if value.get("active_request").is_some_and(Value::is_null)
                && value.get("queued_jobs").and_then(Value::as_u64) == Some(0)
            {
                return Ok(value);
            }
            if start.elapsed() > Duration::from_secs(20) {
                return Err(ProbeFailure {
                    stage: "idle_deadline",
                    category: "runtime_still_active",
                    http_status: Some(status),
                    api_code: None,
                    idle_poll: Some(polls),
                    transport: None,
                });
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
    async fn run(&mut self) -> Result<()> {
        let record = Discovery::read(&self.options.root)?;
        if record.listen != self.options.address {
            return Err("--base-url must exactly match local discovery".into());
        }
        let _proof = self.connect().await?;
        self.check(
            "server_identity_proof",
            true,
            "HMAC proof verified on the exact loopback connection before bearer release",
        );
        for path in [
            "/v1/models",
            "/runtime/models?limit=1",
            "/runtime/status",
            "/runtime/devices",
        ] {
            let (status, value) = self.get(path).await?;
            self.check(
                &format!("route:{path}"),
                status == 200 && value.is_object(),
                "authenticated route returns a JSON object",
            );
        }
        let (status, list) = self.get("/runtime/models?limit=128").await?;
        self.report.model_sha256 = list["data"]
            .as_array()
            .and_then(|a| a.iter().find(|v| v["id"] == self.options.model))
            .and_then(|v| v["sha256"].as_str())
            .map(str::to_owned);
        self.check(
            "registered_model",
            status == 200
                && list["data"].as_array().is_some_and(|a| {
                    a.iter()
                        .any(|v| v["id"] == self.options.model && v["validated"] == true)
                }),
            "configured model appears as independently verified registry summary",
        );
        self.pagination().await?;
        let unauthorized = raw_no_auth(
            self.options.address,
            "/runtime/status",
            &self.options.address.to_string(),
            None,
        )
        .await?;
        self.check(
            "authentication",
            unauthorized == 401,
            "missing bearer is rejected before runtime work",
        );
        for (id, header, value) in [
            ("host_rejection", "host", "untrusted.invalid"),
            ("origin_rejection", "origin", "https://untrusted.invalid"),
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(
                hyper::header::HeaderName::from_bytes(header.as_bytes())?,
                value.parse()?,
            );
            let (status, _) = self
                .request(Method::GET, "/runtime/status", Vec::new(), headers)
                .await?;
            self.check(
                id,
                status == 400 || status == 403,
                "untrusted browser origin or Host is rejected",
            );
        }
        for (id,body,code) in [
            ("unknown_chat_field",{let mut b=self.chat(1,false);b["unexpected"]=json!(true);serde_json::to_vec(&b)?},"invalid_request"),
            ("unsupported_tools",{let mut b=self.chat(1,false);b["tools"]=json!([]);serde_json::to_vec(&b)?},"unsupported_parameter"),
            ("duplicate_json_keys",format!("{{\"model\":\"{}\",\"model\":\"{}\",\"messages\":[{{\"role\":\"user\",\"content\":\"hello\"}}]}}",self.options.model,self.options.model).into_bytes(),"invalid_request"),
            ("token_alias_conflict",{let mut b=self.chat(1,false);b["max_completion_tokens"]=json!(1);serde_json::to_vec(&b)?},"invalid_request"),
        ]{let(status,value)=self.request(Method::POST,"/v1/chat/completions",body,HeaderMap::new()).await?;self.check(id,status==400&&value["error"]["code"]==code,"HTTP 400 and exact public error code independently checked");}
        let(status,value)=self.post("/runtime/load",json!({"model":self.options.model,"backend":"cpu","context_size":2048,"gpu_layers":0,"threads":2,"batch_size":128})).await?;
        self.check(
            "explicit_cpu_load",
            status == 200 && value["state"] == "ready",
            "explicit 2048 context, 2 threads, batch128, CPU, zero GPU layers",
        );
        if status != 200 {
            return Err("explicit model load failed".into());
        }
        let mut over = self.chat(4096, false);
        over["messages"][0]["content"] = json!("hello");
        let (status, value) = self.post("/v1/chat/completions", over).await?;
        self.check(
            "A05_context_budget",
            status == 400 && value["error"]["code"] == "context_length_exceeded",
            "budget overflow is rejected without truncation",
        );
        let (status, completion) = self
            .post("/v1/chat/completions", self.chat(24, false))
            .await?;
        self.check("A01_A03_nonstream_usage",status==200&&valid_completion(&completion,&self.options.model)&&completion["choices"][0]["message"]["content"].as_str().is_some_and(|s|!s.trim().is_empty()),"completion schema, assistant text, finish reason, and arithmetic token usage independently checked");
        self.phase = "messages_stop_isolation";
        self.messages_stop(&completion).await?;
        self.phase = "streaming";
        self.streaming().await?;
        self.phase = "chunked_and_limits";
        self.chunked_and_limits().await?;
        self.phase = "concurrent_cancel";
        self.concurrent_cancel().await?;
        self.phase = "disconnect";
        self.disconnect().await?;
        self.phase = "cli_management";
        self.cli_checks().await;
        let (status, _) = self.post("/runtime/unload", json!({})).await?;
        let (_, after) = self.get("/runtime/status").await?;
        self.check(
            "explicit_unload",
            status == 200 && after["active_request"].is_null() && after["queued_jobs"] == 0,
            "unload completes with no active or queued request",
        );
        for (id, why) in [
            (
                "A08_exact_prefill_decode",
                "Remote timing cannot identify native prefill/decode phase; independent executor integration tests are required",
            ),
            (
                "A11_idle_unload_race",
                "Exact idle-unload versus arrival timing requires a controlled scheduler contract test",
            ),
            (
                "A12_load_deadline",
                "Exact load-deadline boundary requires a controlled executor contract test",
            ),
            (
                "A12_queue_deadline",
                "Default queue deadline is 120 seconds; deterministic fake-executor contract suite owns the exact deadline boundary",
            ),
            (
                "A12_execution_deadline",
                "Exact execution-deadline boundary requires controlled executor contract tests",
            ),
            (
                "A09_slow_writer_limit",
                "Small real model cannot deterministically fill all OS socket buffers; bounded-byte/slow-writer transport contract tests must prove this",
            ),
            (
                "worker_crash_containment",
                "Worker fault injection and OS containment require the separate process-host integration suite",
            ),
            (
                "long_term_memory",
                "Long-duration memory and target-device acceptance are outside this short smoke window",
            ),
        ] {
            self.skip(id, why);
        }
        let (status, value) = if self.options.release_acceptance {
            let binary = self
                .options
                .cli
                .as_ref()
                .ok_or("explicit CLI is required")?;
            (
                200,
                cli_json(binary, &self.options.root, &["stop".into()]).await?,
            )
        } else {
            self.post("/runtime/shutdown", json!({})).await?
        };
        let stopped = wait_stopped(
            &self.options.root,
            record.instance_id,
            Duration::from_secs(30),
        )
        .await;
        self.check("A14_shutdown_reap_partial",status==200&&value["status"]=="stopped"&&stopped.is_ok(),"shutdown acknowledged after actual cleanup; original instance marker disappeared and lock released");
        Ok(())
    }
    async fn pagination(&mut self) -> Result<()> {
        for route in ["/runtime/models", "/v1/models"] {
            let mut after: Option<String> = None;
            let mut passed = true;
            let mut pages = 0;
            loop {
                pages += 1;
                if pages > 10000 {
                    return Err("model pagination did not terminate".into());
                }
                let path = match &after {
                    Some(cursor) => format!("{route}?limit=1&after={cursor}"),
                    None => format!("{route}?limit=1"),
                };
                let (status, page) = self.get(&path).await?;
                let entries = page["data"].as_array().ok_or("model page missing data")?;
                passed &= status == 200 && page["object"] == "list" && entries.len() <= 1;
                if let Some(entry) = entries.first() {
                    let id = entry["id"].as_str().ok_or("model entry missing ID")?;
                    passed &= after.as_ref().is_none_or(|old| old.as_str() < id)
                        && entry.get("relative_file").is_none()
                        && entry.get("source").is_none();
                }
                let Some(next) = page["next_after"].as_str() else {
                    break;
                };
                if entries.last().and_then(|v| v["id"].as_str()) != Some(next)
                    || after.as_ref().is_some_and(|old| old.as_str() >= next)
                {
                    passed = false;
                    break;
                }
                after = Some(next.to_owned());
            }
            let (status, _) = self.get(&format!("{route}?limit=129")).await?;
            self.check(&format!("pagination:{route}"),passed&&status==400,"one-entry pages use ordered exclusive cursors; oversize page limits are rejected and summaries hide paths");
        }
        Ok(())
    }
    async fn messages_stop(&mut self, baseline: &Value) -> Result<()> {
        let mut body = self.chat(24, false);
        body["messages"] = json!([{"role":"system","content":"Reply concisely in Chinese and English."},{"role":"user","content":"Remember the synthetic code ALPHA."},{"role":"assistant","content":"The synthetic code is ALPHA."},{"role":"user","content":"What was the code?"}]);
        let (status, multi) = self.post("/v1/chat/completions", body).await?;
        let (status2, repeated) = self
            .post("/v1/chat/completions", self.chat(24, false))
            .await?;
        self.check("A02_messages_isolation_partial",status==200&&status2==200&&valid_completion(&multi,&self.options.model)&&multi["usage"]["prompt_tokens"].as_u64()>baseline["usage"]["prompt_tokens"].as_u64()&&repeated["usage"]["prompt_tokens"]==baseline["usage"]["prompt_tokens"]&&repeated["choices"][0]["message"]["content"]==baseline["choices"][0]["message"]["content"],"full system/multiturn messages are processed; repeating independent greedy request preserves prompt accounting and output");
        if let Some(content) = baseline["choices"][0]["message"]["content"]
            .as_str()
            .filter(|s| !s.is_empty())
        {
            let stop: String = content.chars().take(2).collect();
            let mut body = self.chat(24, false);
            body["stop"] = json!([stop]);
            let (status, stopped) = self.post("/v1/chat/completions", body).await?;
            self.check("A04_stop_no_leak_partial",status==200&&stopped["choices"][0]["message"]["content"]==""&&stopped["choices"][0]["finish_reason"]=="stop","deterministic output prefix used as stop; matched text is absent and one stop terminal returned");
        }
        self.skip("A04_exact_utf8_stop_split","Remote reads cannot prove native token/chunk split positions; controlled stream-buffer contract tests own those boundaries");
        Ok(())
    }
    async fn streaming(&mut self) -> Result<()> {
        let mut client = self.connect().await?;
        let mut body = self.chat(48, true);
        body["stream_options"] = json!({"include_usage":true});
        let response = client
            .request(
                Method::POST,
                "/v1/chat/completions",
                RequestBody::fixed(serde_json::to_vec(&body)?),
                HeaderMap::new(),
            )
            .await?;
        let status = response.status().as_u16();
        let content = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .starts_with("text/event-stream");
        let (bytes, frames) = stream_bytes(response.into_body()).await?;
        let validation = validate_sse(&bytes, &self.options.model, true);
        self.check("A03_streaming_sse",status==200&&content&&validation.is_ok(),&format!("SSE independently checks role, stable identity, text, one finish, final usage, [DONE]; observed {frames} HTTP body frames"));
        Ok(())
    }
    async fn chunked_and_limits(&mut self) -> Result<()> {
        self.phase = "valid_chunked_proof";
        let mut client = self.connect().await?;
        let body = serde_json::to_vec(&self.chat(1, false))?;
        let parts = body.chunks(7).map(Bytes::copy_from_slice).collect();
        self.phase = "valid_chunked_send";
        let response = client
            .request(
                Method::POST,
                "/v1/chat/completions",
                RequestBody::chunked(parts),
                HeaderMap::new(),
            )
            .await?;
        let status = response.status().as_u16();
        self.phase = "valid_chunked_response_body_json";
        let value: Value = serde_json::from_slice(
            &collect_bounded(response.into_body(), 128 * 1024, Duration::from_secs(60)).await?,
        )?;
        self.check(
            "chunked_request_body",
            status == 200 && valid_completion(&value, &self.options.model),
            "valid JSON split across seven-byte HTTP chunks works",
        );
        for chunked in [false, true] {
            self.phase = if chunked {
                "oversize_chunked_proof"
            } else {
                "oversize_fixed_proof"
            };
            let mut client = self.connect().await?;
            let bytes = vec![b' '; 1_048_577];
            let body = if chunked {
                RequestBody::chunked(bytes.chunks(8192).map(Bytes::copy_from_slice).collect())
            } else {
                RequestBody::fixed(bytes)
            };
            self.phase = if chunked {
                "oversize_chunked_send"
            } else {
                "oversize_fixed_send"
            };
            let response = client
                .request(Method::POST, "/v1/chat/completions", body, HeaderMap::new())
                .await?;
            let status = response.status().as_u16();
            self.phase = if chunked {
                "oversize_chunked_response_body"
            } else {
                "oversize_fixed_response_body"
            };
            let bytes = collect_bounded(response.into_body(), 16384, Duration::from_secs(30))
                .await
                .map_err(|error| {
                    let mut diagnostic = ProbeFailure::from_error(self.phase, &error);
                    diagnostic.http_status = Some(status);
                    diagnostic
                })?;
            self.phase = if chunked {
                "oversize_chunked_response_json"
            } else {
                "oversize_fixed_response_json"
            };
            let value: Value = serde_json::from_slice(&bytes).map_err(|error| {
                let mut diagnostic = ProbeFailure::from_error(self.phase, &error);
                diagnostic.http_status = Some(status);
                diagnostic
            })?;

            self.check(
                if chunked {
                    "oversize_chunked_body"
                } else {
                    "oversize_fixed_body"
                },
                status == 413 && value["error"]["code"]=="request_too_large",
                "complete HTTP 413 and request_too_large JSON are required; connection failure is never accepted",
            );
        }
        Ok(())
    }
    async fn concurrent_cancel(&mut self) -> Result<()> {
        let mut active = self.connect().await?;
        let id = Uuid::new_v4();
        let mut headers = HeaderMap::new();
        headers.insert("x-request-id", id.to_string().parse()?);
        let mut long = self.chat(512, true);
        long["messages"][0]["content"] =
            json!("逐行写出从1到500的数字，每行都给出中文解释，继续直到输出预算耗尽。");
        let response = active
            .request(
                Method::POST,
                "/v1/chat/completions",
                RequestBody::fixed(serde_json::to_vec(&long)?),
                headers.clone(),
            )
            .await?;
        if response.status() != 200 {
            return Err("long streaming request failed".into());
        }
        let (status, duplicate) = self
            .request(
                Method::POST,
                "/v1/chat/completions",
                serde_json::to_vec(&self.chat(1, false))?,
                headers,
            )
            .await?;
        self.check(
            "duplicate_request_id",
            status == 409 && duplicate["error"]["code"] == "duplicate_request_id",
            "same live request UUID is rejected without enqueue",
        );
        let (status, busy) = self.post("/runtime/unload", json!({})).await?;
        self.check(
            "A10_management_busy",
            status == 409 && busy["error"]["code"] == "runtime_busy",
            "management mutation cannot interleave with a running generation",
        );
        let mut queued_ids = Vec::new();
        let mut queued_tasks = Vec::new();
        for index in 0..8 {
            let queued_id = Uuid::new_v4();
            let client = self.connect().await?;
            let mut queued = self.chat(if index == 0 { 512 } else { 1 }, false);
            if index == 0 {
                queued["messages"][0]["content"] =
                    json!("逐行写出从1到500的数字，每行给出中文解释。");
            }
            queued_tasks.push(spawn_chat(client, queued_id, serde_json::to_vec(&queued)?));
            queued_ids.push(queued_id);
            self.wait_queued(index + 1).await?;
        }
        let mut excess = self.connect().await?;
        let over = excess
            .request(
                Method::POST,
                "/v1/chat/completions",
                RequestBody::fixed(serde_json::to_vec(&self.chat(1, false))?),
                HeaderMap::new(),
            )
            .await?;
        let full = over.status() == 429
            && over
                .headers()
                .get("retry-after")
                .and_then(|h| h.to_str().ok())
                .and_then(|v| v.parse::<u32>().ok())
                .is_some_and(|v| v > 0);
        let over_body: Value = serde_json::from_slice(
            &collect_bounded(over.into_body(), 16384, Duration::from_secs(10)).await?,
        )?;
        self.check("A06_queue_full",full&&over_body["error"]["code"]=="queue_full","one running plus eight queued; tenth rejected with 429 queue_full and positive Retry-After");
        let queued_cancel = queued_ids[4];
        let (status, cancel) = self
            .post(
                &format!("/runtime/requests/{queued_cancel}/cancel"),
                json!({}),
            )
            .await?;
        self.wait_queued(7).await?;
        self.check(
            "A07_cancel_queued_partial",
            status == 202 && cancel["status"] == "cancelling",
            "queued cancellation removes that UUID while seven other requests remain queued",
        );
        let (status, cancel) = self
            .post(&format!("/runtime/requests/{id}/cancel"), json!({}))
            .await?;
        self.check(
            "A08_cancel_active_partial",
            status == 202 && cancel["status"] == "cancelling",
            "active request cancellation accepted; exact native phase belongs to integration tests",
        );
        let fifo = self.wait_active(queued_ids[0]).await.is_ok();
        let _ = self
            .post(
                &format!("/runtime/requests/{}/cancel", queued_ids[0]),
                json!({}),
            )
            .await?;
        let bytes = collect_bounded(
            response.into_body(),
            2 * 1024 * 1024,
            Duration::from_secs(30),
        )
        .await?;
        let text = std::str::from_utf8(&bytes)?;
        self.check(
            "A08_sse_cancel_terminal_partial",
            text.contains("request_cancelled") && !text.contains("data: [DONE]"),
            "cancelled SSE has an error and no success DONE terminal",
        );
        let mut queued_terminal = true;
        let mut cancelled_terminal = false;
        for (index, task) in queued_tasks.into_iter().enumerate() {
            let (status, _) = task.await??;
            if index == 4 {
                cancelled_terminal = status == 408;
            } else if index == 0 {
                queued_terminal &= matches!(status, 200 | 408);
            } else {
                queued_terminal &= status == 200;
            }
        }
        self.check("A06_FIFO_partial",fifo&&queued_terminal,"first queued UUID enters active slot first; its cancellation releases remaining requests, each completing successfully");
        self.check(
            "A07_cancel_queued_terminal",
            cancelled_terminal,
            "cancelled queued HTTP request receives 408; all unrelated requests complete",
        );
        self.idle().await?;
        let (status, unknown) = self
            .post(
                &format!("/runtime/requests/{}/cancel", Uuid::new_v4()),
                json!({}),
            )
            .await?;
        self.check(
            "cancel_unknown",
            status == 404 && unknown["error"]["code"] == "request_not_found",
            "unknown cancellation ID reports 404",
        );
        Ok(())
    }
    async fn wait_active(&self, id: Uuid) -> Result<()> {
        let start = tokio::time::Instant::now();
        loop {
            let (_, value) = self.get("/runtime/status").await?;
            if value["active_request"].as_str() == Some(id.to_string().as_str()) {
                return Ok(());
            }
            if start.elapsed() > Duration::from_secs(5) {
                return Err("first FIFO request was not observed active".into());
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
    async fn wait_queued(&self, count: u64) -> Result<()> {
        let start = tokio::time::Instant::now();
        loop {
            let (_, value) = self.get("/runtime/status").await?;
            if value["queued_jobs"].as_u64() == Some(count) {
                return Ok(());
            }
            if start.elapsed() > Duration::from_secs(5) {
                return Err("could not establish deterministic live queue".into());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    async fn disconnect(&mut self) -> Result<()> {
        for iteration in 1..=self.options.disconnect_cycles {
            self.report.disconnect_cycles_attempted += 1;
            let mut client = self.connect().await?;
            let request_id = Uuid::new_v4();
            let mut headers = HeaderMap::new();
            headers.insert("x-request-id", request_id.to_string().parse()?);
            let response = client
                .request(
                    Method::POST,
                    "/v1/chat/completions",
                    RequestBody::fixed(serde_json::to_vec(&self.chat(512, true))?),
                    headers,
                )
                .await?;
            let initial_status = response.status().as_u16();
            drop(response);
            drop(client);
            let cleanup_started = std::time::Instant::now();
            let idle_result = self.idle().await;
            let idle = idle_result.is_ok();
            let cleanup_ms = cleanup_started.elapsed().as_millis();
            let snapshot = idle_result.as_ref().ok();
            let state = snapshot
                .and_then(|v| v["state"].as_str())
                .and_then(safe_symbol)
                .unwrap_or("unknown");
            let prior_error = snapshot
                .and_then(|v| v["last_error"]["code"].as_str())
                .and_then(safe_symbol)
                .unwrap_or("none");
            let sessions_started = snapshot.and_then(|v| v["worker"]["sessions_started"].as_u64());
            let sessions_reaped = snapshot.and_then(|v| v["worker"]["sessions_reaped"].as_u64());
            let (status, next) = self
                .post("/v1/chat/completions", self.chat(1, false))
                .await?;
            let valid = valid_completion(&next, &self.options.model);
            let error = next["error"]["code"]
                .as_str()
                .and_then(safe_symbol)
                .unwrap_or("none");
            let finish = next["choices"][0]["finish_reason"]
                .as_str()
                .and_then(safe_symbol)
                .unwrap_or("none");
            let pass = initial_status == 200 && idle && status == 200 && valid;
            if pass {
                self.report.disconnect_cycles_passed += 1;
            }
            self.check(&format!("A09_disconnect_then_next_{iteration}"),pass,&format!("request_id={request_id}, initial_status={initial_status}, idle={idle}, cleanup_ms={cleanup_ms}, state={state}, prior_error={prior_error}, sessions_started={sessions_started:?}, sessions_reaped={sessions_reaped:?}, next_status={status}, error={error}, finish={finish}, valid_completion={valid}, prompt_tokens={:?}, completion_tokens={:?}, total_tokens={:?}",next["usage"]["prompt_tokens"].as_u64(),next["usage"]["completion_tokens"].as_u64(),next["usage"]["total_tokens"].as_u64()));
            if let Err(diagnostic) = idle_result {
                self.report
                    .checks
                    .last_mut()
                    .expect("check was just recorded")
                    .diagnostic = Some(diagnostic);
            }
            if !pass {
                break;
            }
        }
        Ok(())
    }
    async fn cli_checks(&mut self) {
        let binary = self.options.cli.clone().or_else(|| {
            if self.options.release_acceptance {
                return None;
            }
            std::env::current_exe().ok().and_then(|p| {
                p.parent().map(|p| {
                    p.join(if cfg!(windows) {
                        "ai-runtime.exe"
                    } else {
                        "ai-runtime"
                    })
                })
            })
        });
        let Some(binary) = binary.filter(|p| p.is_file()) else {
            if self.options.release_acceptance || self.options.cli.is_some() {
                self.check(
                    "actual_cli_management",
                    false,
                    "explicit product CLI is missing; required release checks cannot be skipped",
                );
            } else {
                self.skip("actual_cli_management", "ai-runtime binary is not beside xtask; provide --cli or build both before running this developer check");
            }
            return;
        };
        for args in [
            &["status"][..],
            &["devices"],
            &["models", "list"],
            &["version", "--json"],
        ] {
            let values: Vec<_> = args.iter().map(OsString::from).collect();
            let result = cli_json(&binary, &self.options.root, &values).await;
            let pass = result.is_ok_and(|v| match args {
                ["status"] => v["active_request"].is_null() && v["queued_jobs"] == 0,
                ["devices"] => {
                    v["build_backends"]
                        .as_array()
                        .is_some_and(|backends| backends.iter().any(|b| b == "cpu"))
                        && v["devices"].as_array().is_some_and(|devices| {
                            devices
                                .iter()
                                .any(|d| d["id"] == "cpu" && d["kind"] == "cpu")
                        })
                }
                ["models", "list"] => v["data"].as_array().is_some_and(|a| {
                    a.iter()
                        .any(|m| m["id"] == self.options.model && m["validated"] == true)
                }),
                ["version", "--json"] => {
                    v["protocol_version"] == 1
                        && v["management_native_linkage"] == false
                        && v["target_arch"] == std::env::consts::ARCH
                }
                _ => false,
            });
            self.check(&format!("actual_cli:{}", args.join("_")), pass,
                "explicit product CLI exits zero with independently checked JSON; no response bodies are recorded");
        }
    }
}
fn spawn_chat(
    mut client: VerifiedConnection,
    id: Uuid,
    body: Vec<u8>,
) -> tokio::task::JoinHandle<Result<(u16, std::time::Instant)>> {
    tokio::spawn(async move {
        let mut headers = HeaderMap::new();
        headers.insert("x-request-id", id.to_string().parse()?);
        let response = client
            .request(
                Method::POST,
                "/v1/chat/completions",
                RequestBody::fixed(body),
                headers,
            )
            .await?;
        let status = response.status().as_u16();
        let _ = collect_bounded(response.into_body(), 128 * 1024, Duration::from_secs(60)).await?;
        Ok((status, std::time::Instant::now()))
    })
}
fn safe_symbol(value: &str) -> Option<&str> {
    (!value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_'))
    .then_some(value)
}
fn valid_usage(value: &Value) -> bool {
    let Some(p) = value["prompt_tokens"].as_u64() else {
        return false;
    };
    let Some(c) = value["completion_tokens"].as_u64() else {
        return false;
    };
    p > 0 && c > 0 && value["total_tokens"].as_u64() == p.checked_add(c)
}
fn valid_completion(value: &Value, model: &str) -> bool {
    value["object"] == "chat.completion"
        && value["model"] == model
        && value["id"]
            .as_str()
            .is_some_and(|s| s.starts_with("chatcmpl-"))
        && value["created"].as_u64().is_some()
        && value["choices"].as_array().is_some_and(|c| c.len() == 1)
        && value["choices"][0]["message"]["role"] == "assistant"
        && value["choices"][0]["message"]["content"].is_string()
        && matches!(
            value["choices"][0]["finish_reason"].as_str(),
            Some("stop" | "length")
        )
        && valid_usage(&value["usage"])
}
async fn stream_bytes(mut body: Incoming) -> Result<(Vec<u8>, usize)> {
    tokio::time::timeout(Duration::from_secs(120), async {
        let mut output = Vec::new();
        let mut frames = 0;
        while let Some(frame) = body.frame().await {
            if let Ok(bytes) = frame?.into_data() {
                if output.len() + bytes.len() > 2 * 1024 * 1024 {
                    return Err("stream exceeded bound".into());
                }
                output.extend_from_slice(&bytes);
                frames += 1;
            }
        }
        Ok((output, frames))
    })
    .await?
}
fn validate_sse(bytes: &[u8], model: &str, usage: bool) -> Result<()> {
    let text = std::str::from_utf8(bytes)?;
    if !text.ends_with("\n\n") {
        return Err("SSE ends mid-event".into());
    }
    let mut identity = None;
    let mut first = true;
    let mut finish = false;
    let mut used = false;
    let mut done = false;
    let mut text_chunks = 0;
    for event in text.split("\n\n").filter(|v| !v.is_empty()) {
        let payload = event.strip_prefix("data: ").ok_or("non-data SSE event")?;
        if done {
            return Err("event follows DONE".into());
        }
        if payload == "[DONE]" {
            if !finish || usage != used {
                return Err("invalid DONE order".into());
            }
            done = true;
            continue;
        }
        let value: Value = serde_json::from_str(payload)?;
        if value.get("error").is_some() {
            return Err("SSE contains error".into());
        }
        let current = (
            value["id"].clone(),
            value["created"].clone(),
            value["model"].clone(),
        );
        if value["object"] != "chat.completion.chunk"
            || value["model"] != model
            || value["created"].as_u64().is_none()
        {
            return Err("invalid SSE envelope".into());
        }
        if let Some(previous) = &identity {
            if previous != &current {
                return Err("SSE identity changed".into());
            }
        } else {
            identity = Some(current);
        }
        let choices = value["choices"].as_array().ok_or("missing SSE choices")?;
        if choices.is_empty() {
            if !finish || !usage || used || !valid_usage(&value["usage"]) {
                return Err("invalid usage chunk".into());
            }
            used = true;
            continue;
        }
        if finish || choices.len() != 1 || choices[0]["index"] != 0 {
            return Err("invalid SSE choice order".into());
        }
        let choice = &choices[0];
        if first {
            if choice["delta"]["role"] != "assistant" {
                return Err("first chunk lacks assistant role".into());
            }
            first = false;
        }
        if let Some(content) = choice["delta"]["content"].as_str()
            && !content.is_empty()
        {
            text_chunks += 1;
        }
        if let Some(reason) = choice["finish_reason"].as_str() {
            if !matches!(reason, "stop" | "length") {
                return Err("invalid finish reason".into());
            }
            finish = true;
        }
    }
    if !done || !finish || first || text_chunks == 0 {
        return Err("incomplete SSE sequence".into());
    }
    Ok(())
}
async fn raw_no_auth(
    address: SocketAddr,
    path: &str,
    host: &str,
    origin: Option<&str>,
) -> Result<u16> {
    let mut socket = TcpStream::connect(address).await?;
    let extra = origin
        .map(|o| format!("Origin: {o}\r\n"))
        .unwrap_or_default();
    socket
        .write_all(
            format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n{extra}Connection: close\r\n\r\n")
                .as_bytes(),
        )
        .await?;
    let mut bytes = vec![0; 4096];
    let n = tokio::time::timeout(Duration::from_secs(10), socket.read(&mut bytes)).await??;
    let head = std::str::from_utf8(&bytes[..n])?;
    Ok(head
        .split_whitespace()
        .nth(1)
        .ok_or("invalid HTTP response")?
        .parse()?)
}
fn parse(args: &[OsString]) -> Result<Options> {
    let mut values = BTreeMap::new();
    for pair in args.chunks(2) {
        if pair.len() != 2 {
            return Err("api-smoke options require values".into());
        }
        let key = pair[0].to_str().ok_or("invalid option")?;
        if ![
            "--base-url",
            "--data-dir",
            "--model",
            "--out",
            "--disconnect-cycles",
            "--cli",
            "--release-acceptance",
        ]
        .contains(&key)
            || values.insert(key, pair[1].clone()).is_some()
        {
            return Err("unknown or duplicate api-smoke option".into());
        }
    }
    let base = values.remove("--base-url").ok_or("--base-url required")?;
    let base = base.to_str().ok_or("base URL must be UTF-8")?;
    let address: SocketAddr = base
        .strip_prefix("http://")
        .ok_or("only literal loopback HTTP addresses are supported")?
        .trim_end_matches('/')
        .parse()?;
    if !address.ip().is_loopback() || address.port() == 0 {
        return Err("base URL must be a concrete loopback endpoint".into());
    }
    let root = values
        .remove("--data-dir")
        .ok_or("--data-dir required")?
        .into();
    let model = values
        .remove("--model")
        .ok_or("--model required")?
        .into_string()
        .map_err(|_| "model ID must be UTF-8")?;
    if model.is_empty()
        || model.len() > 64
        || !model.bytes().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'.' | b'_' | b'-')
        })
    {
        return Err("invalid model ID".into());
    }
    let out = values.remove("--out").ok_or("--out required")?.into();
    let disconnect_cycles = match values.remove("--disconnect-cycles") {
        Some(value) => value
            .to_str()
            .ok_or("disconnect cycles must be UTF-8")?
            .parse::<u32>()
            .map_err(|_| "disconnect cycles must be an integer in 1..=50")?,
        None => 5,
    };
    if !(1..=50).contains(&disconnect_cycles) {
        return Err("disconnect cycles must be in 1..=50".into());
    }

    let cli = values.remove("--cli").map(PathBuf::from);
    let release_acceptance = match values.remove("--release-acceptance") {
        None => false,
        Some(value) if value == "true" => true,
        Some(value) if value == "false" => false,
        Some(_) => return Err("--release-acceptance requires true or false".into()),
    };
    if release_acceptance && cli.is_none() {
        return Err("--release-acceptance requires an explicit --cli".into());
    }
    if cli.as_ref().is_some_and(|p| !p.is_absolute()) {
        return Err("--cli must be an absolute product executable path".into());
    }
    Ok(Options {
        address,
        root,
        model,
        out,
        disconnect_cycles,
        cli,
        release_acceptance,
    })
}
/// One oracle for developer HTTP checks and the standalone package verifier.
/// Failure still attempts authenticated unified shutdown; reports never contain raw I/O.
pub async fn run(options: Options) -> Report {
    let report = Report {
        schema_version: 1,
        kind: "nexa-http-cli-black-box-smoke",
        platform: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        model: options.model.clone(),
        model_sha256: None,
        inference: json!({"backend":"cpu","context_size":2048,"threads":2,"batch_size":128,"gpu_layers":0}),
        disconnect_cycles_requested: options.disconnect_cycles,
        disconnect_cycles_attempted: 0,
        disconnect_cycles_passed: 0,
        checks: Vec::new(),
    };
    let mut oracle = Oracle {
        options,
        report,
        phase: "routes_auth_validation_load",
    };
    if let Err(error) = oracle.run().await {
        let detail = format!(
            "verification stopped during {}; no raw response, token, prompt, or private path is recorded",
            oracle.phase
        );
        oracle.check("smoke_execution", false, &detail);
        let diagnostic = error
            .downcast_ref::<ProbeFailure>()
            .cloned()
            .unwrap_or_else(|| ProbeFailure::from_error(oracle.phase, error.as_ref()));
        oracle
            .report
            .checks
            .last_mut()
            .expect("check was just recorded")
            .diagnostic = Some(diagnostic);
        let cleaned = async {
            let record = Discovery::read(&oracle.options.root)?;
            let (status, value) = oracle.post("/runtime/shutdown", json!({})).await?;
            if status != 200 || value["status"] != "stopped" {
                return Err("cleanup failed".into());
            }
            wait_stopped(
                &oracle.options.root,
                record.instance_id,
                Duration::from_secs(30),
            )
            .await?;
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        }
        .await
        .is_ok();
        oracle.check(
            "cleanup_after_failed_smoke",
            cleaned,
            "best-effort unified shutdown and instance-lock release after a failed test",
        );
    }
    oracle.report
}
impl Report {
    pub fn passed(&self) -> bool {
        self.checks.iter().all(|check| check.status != "fail")
    }
}

/// Never inherit developer DLL search paths or credentials into a product process.
pub fn product_command(binary: &Path) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(binary);
    command.env_clear();
    for name in ["SystemRoot", "WINDIR", "TEMP", "TMP"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SystemRoot") {
        let root = PathBuf::from(root);
        if let Ok(path) = std::env::join_paths([root.join("System32"), root]) {
            command.env("PATH", path);
        }
    }
    #[cfg(not(windows))]
    command.env("PATH", "/usr/bin:/bin");
    command
}

/// Execute only the selected product CLI from an isolated data-directory CWD.
/// Bound both execution time and output; stderr is deliberately never collected.
pub async fn cli_json(binary: &Path, root: &Path, args: &[OsString]) -> Result<Value> {
    let root = if root.is_absolute() {
        root.to_path_buf()
    } else {
        std::env::current_dir()?.join(root)
    };
    let mut child = product_command(binary)
        .arg("--data-dir")
        .arg(&root)
        .args(args)
        .current_dir(root.parent().unwrap_or(&root))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let mut output = child
        .stdout
        .take()
        .ok_or("CLI stdout unavailable")?
        .take(1024 * 1024 + 1);
    let result = tokio::time::timeout(Duration::from_secs(750), async {
        let mut bytes = Vec::new();
        output.read_to_end(&mut bytes).await?;
        if bytes.len() > 1024 * 1024 {
            return Err("CLI output exceeds limit".into());
        }
        if !child.wait().await?.success() {
            return Err("CLI returned failure".into());
        }
        Ok::<Value, Box<dyn std::error::Error + Send + Sync>>(serde_json::from_slice(&bytes)?)
    })
    .await;
    match result {
        Ok(Ok(value)) => Ok(value),
        outcome => {
            // This is an owned Child handle, never a PID obtained from discovery.
            let _ = child.kill().await;
            let _ = child.wait().await;
            match outcome {
                Ok(Err(e)) => Err(e),
                _ => Err("CLI deadline expired".into()),
            }
        }
    }
}

pub fn main(args: &[OsString]) -> ExitCode {
    let options = match parse(args) {
        Ok(v) => v,
        Err(error) => {
            eprintln!("api-smoke: {error}");
            return ExitCode::from(2);
        }
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(v) => v,
        Err(_) => {
            eprintln!("api-smoke: cannot start test runtime");
            return ExitCode::FAILURE;
        }
    };
    let out = options.out.clone();
    let report = runtime.block_on(run(options));
    let pass = report.passed();
    let written = (|| -> Result<()> {
        if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&out, serde_json::to_vec_pretty(&report)?)?;
        Ok(())
    })();
    if written.is_err() {
        eprintln!("api-smoke: could not write sanitized report");
        return ExitCode::FAILURE;
    }
    println!(
        "api-smoke: {} ({} checks)",
        if pass { "pass" } else { "fail" },
        report.checks.len()
    );
    if pass {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn oracle_rejects_fake_success_and_wrong_usage() {
        assert!(!valid_completion(&json!({"object":"chat.completion"}), "m"));
        assert!(!valid_usage(
            &json!({"prompt_tokens":2,"completion_tokens":3,"total_tokens":6})
        ));
        assert!(validate_sse(b"data: [DONE]\n\n", "m", false).is_err());
    }
    #[test]
    fn smoke_rejects_nonlocal_base_url() {
        assert!(
            parse(
                &[
                    "--base-url",
                    "http://example.com:80",
                    "--data-dir",
                    "x",
                    "--model",
                    "a",
                    "--out",
                    "a"
                ]
                .map(OsString::from)
            )
            .is_err()
        );
    }
    #[test]
    fn disconnect_cycles_are_explicit_and_bounded() {
        let base = [
            "--base-url",
            "http://127.0.0.1:12345",
            "--data-dir",
            "private",
            "--model",
            "qa-small",
            "--out",
            "report.json",
        ]
        .map(OsString::from)
        .to_vec();
        assert_eq!(parse(&base).unwrap().disconnect_cycles, 5);
        for count in ["1", "5", "50"] {
            let mut args = base.clone();
            args.extend([OsString::from("--disconnect-cycles"), OsString::from(count)]);
            assert_eq!(
                parse(&args).unwrap().disconnect_cycles,
                count.parse::<u32>().unwrap()
            );
        }
        for count in ["0", "51", "-1", "abc", "1.0", ""] {
            let mut args = base.clone();
            args.extend([OsString::from("--disconnect-cycles"), OsString::from(count)]);
            assert!(parse(&args).is_err());
        }
        let mut args = base;
        args.extend(["--disconnect-cycles", "5", "--disconnect-cycles", "5"].map(OsString::from));
        assert!(parse(&args).is_err());
    }
    #[test]
    fn release_mode_requires_an_explicit_absolute_cli() {
        let base = [
            "--base-url",
            "http://127.0.0.1:12345",
            "--data-dir",
            "private",
            "--model",
            "qa-small",
            "--out",
            "report.json",
            "--release-acceptance",
            "true",
        ]
        .map(OsString::from)
        .to_vec();
        assert!(parse(&base).is_err());
        let mut relative = base.clone();
        relative.extend(["--cli", "relative-cli"].map(OsString::from));
        assert!(parse(&relative).is_err());
        let mut explicit = base;
        explicit.push("--cli".into());
        explicit.push(
            std::env::current_dir()
                .unwrap()
                .join("missing-cli")
                .into_os_string(),
        );
        assert!(parse(&explicit).unwrap().release_acceptance);
    }
    #[tokio::test]
    async fn release_mode_missing_cli_is_a_failure_not_a_skip() {
        let temp = tempfile::tempdir().unwrap();
        let options = Options {
            address: "127.0.0.1:12345".parse().unwrap(),
            root: temp.path().into(),
            model: "fixed".into(),
            out: PathBuf::new(),
            disconnect_cycles: 1,
            cli: Some(temp.path().join("missing-product-cli")),
            release_acceptance: true,
        };
        let mut oracle = Oracle {
            options,
            phase: "cli_management",
            report: Report {
                schema_version: 1,
                kind: "test",
                platform: "test".into(),
                model: "fixed".into(),
                model_sha256: None,
                inference: Value::Null,
                disconnect_cycles_requested: 1,
                disconnect_cycles_attempted: 0,
                disconnect_cycles_passed: 0,
                checks: Vec::new(),
            },
        };
        oracle.cli_checks().await;
        assert_eq!(oracle.report.checks[0].status, "fail");
        assert!(!oracle.report.passed());
    }
    #[test]
    fn probe_diagnostics_never_serialize_raw_errors_or_parameters() {
        let error = std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "secret token /private/path raw-body",
        );
        let diagnostic = ProbeFailure::from_error("connect_or_proof", &error);
        let encoded = serde_json::to_string(&diagnostic).unwrap();
        assert!(encoded.contains("local_state_permission_denied"));
        for forbidden in ["secret token", "/private/path", "raw-body"] {
            assert!(!encoded.contains(forbidden));
        }
        let api = ClientError::Api {
            status: 503,
            code: Some("../../sensitive".into()),
            param: Some("secret".into()),
        };
        let encoded =
            serde_json::to_string(&ProbeFailure::from_error("http_request", &api)).unwrap();
        assert!(!encoded.contains("sensitive"));
        assert!(!encoded.contains("secret"));
    }
    #[test]
    fn top_level_transport_diagnostic_preserves_safe_flags() {
        let error = ClientError::Transport {
            stage: "send_request",
            error_canceled: false,
            error_closed: false,
            error_incomplete_message: true,
            error_parse: false,
            error_body_write_aborted: false,
            error_timeout: false,
            io_kind: Some("connection_reset"),
            sender_ready: true,
            sender_closed: false,
        };
        let diagnostic =
            serde_json::to_value(ProbeFailure::from_error("oversize_fixed_send", &error)).unwrap();
        assert_eq!(diagnostic["stage"], "oversize_fixed_send");
        assert_eq!(diagnostic["transport"]["io_kind"], "connection_reset");
        assert_eq!(diagnostic["transport"]["error_incomplete_message"], true);
        assert_eq!(diagnostic["transport"]["sender_ready"], true);
    }
}
