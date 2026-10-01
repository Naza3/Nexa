//! Non-native management process: the real worker is an external executable.
use process_host::{ProcessHost, ProcessHostConfig};
use runtime_core::{Runtime, RuntimeHandle};
use runtime_types::*;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
fn request(model: ModelId, text: &str, max_tokens: u32) -> GenerationRequest {
    GenerationRequest {
        request_id: RequestId::new(),
        model,
        messages: vec![Message::new(Role::User, text)],
        options: GenerationOptions {
            max_tokens,
            temperature: 0.0,
            ..Default::default()
        },
    }
}
fn config() -> RuntimeConfig {
    RuntimeConfig {
        load_options: LoadOptions {
            context_size: 2048,
            threads: 2,
            batch_size: 128,
        },
        idle_unload: Duration::from_secs(120),
        ..Default::default()
    }
}
fn runtime(host: ProcessHost, model_path: PathBuf) -> Runtime {
    Runtime::spawn(
        config(),
        move |id: &ModelId| {
            Ok(ResolvedModel {
                id: id.clone(),
                path: model_path.clone(),
                context_limit: 2048,
                default_context: 2048,
                validated: true,
            })
        },
        host,
    )
    .unwrap()
}
fn complete(handle: &RuntimeHandle, request: GenerationRequest) -> (usize, Usage) {
    let events = handle.submit(request).unwrap();
    let mut bytes = 0;
    let mut terminal = None;
    while let Some(lease) = events.recv_leased() {
        match &lease.event().kind {
            RequestEventKind::TextDelta(text) => bytes += text.len(),
            RequestEventKind::Completed { usage, .. } => terminal = Some(*usage),
            RequestEventKind::Failed { error, .. } => panic!("generation failed: {error}"),
            RequestEventKind::Cancelled { reason, .. } => {
                panic!("generation cancelled: {reason:?}")
            }
            _ => {}
        }
    }
    (bytes, terminal.expect("one completed terminal"))
}
fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() < 3 {
        eprintln!(
            "usage: nexa-process-harness real WORKER MODEL | parent-exit FIXTURE PID-FILE [normal|abrupt]"
        );
        std::process::exit(2);
    }
    if args[0] == "startup-exit" {
        struct Sink;
        impl runtime_core::ExecutionEventSink for Sink {
            fn emit(&self, _event: runtime_core::ExecutorEvent) -> bool {
                true
            }
        }
        use runtime_core::Executor;
        let mut config = ProcessHostConfig::new(PathBuf::from(&args[1]));
        config.worker_args = vec!["normal".into(), args[2].clone()];
        let mut host = ProcessHost::new(config).unwrap();
        host.start(
            runtime_core::ExecutorCommand::Load {
                model: ResolvedModel {
                    id: ModelId::new("fixture").unwrap(),
                    path: PathBuf::from("fixture.gguf"),
                    context_limit: 2048,
                    default_context: 2048,
                    validated: true,
                },
                options: self::config().load_options,
            },
            runtime_core::ExecutionEvents::from_sink(1, std::sync::Arc::new(Sink)),
        )
        .unwrap();
        let delay = args.get(3).unwrap().to_str().unwrap().parse().unwrap();
        std::thread::sleep(Duration::from_millis(delay));
        std::process::exit(74);
    }
    if args[0] == "parent-exit" {
        let abrupt = args.get(3).is_some_and(|a| a == "abrupt");
        let descendants = cfg!(windows) || !abrupt;
        let mut host_config = ProcessHostConfig::new(PathBuf::from(&args[1]));
        host_config.worker_args = vec![
            if descendants { "descendants" } else { "normal" }.into(),
            args[2].clone(),
        ];
        let host = ProcessHost::new(host_config).unwrap();
        let runtime = runtime(host, PathBuf::from("fixture.gguf"));
        let handle = runtime.handle();
        handle
            .load(ModelId::new("fixture").unwrap(), config().load_options)
            .unwrap();
        let descendant = PathBuf::from(format!("{}.descendant", args[2].to_string_lossy()));
        let until = Instant::now() + Duration::from_secs(5);
        while descendants && !descendant.exists() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
        if abrupt {
            std::process::exit(73);
        }
        runtime.shutdown().unwrap();
        return;
    }
    if args[0] != "real" {
        std::process::exit(2);
    }
    let host = ProcessHost::new(ProcessHostConfig::new(PathBuf::from(&args[1]))).unwrap();
    let diagnostics = host.diagnostics();
    let model_path = std::fs::canonicalize(&args[2]).unwrap();
    let runtime = runtime(host, model_path);
    let handle = runtime.handle();
    let model = ModelId::new("qwen").unwrap();
    let started = Instant::now();
    handle.load(model.clone(), config().load_options).unwrap();
    let (bytes, usage) = complete(
        &handle,
        request(model.clone(), "用一句中文介绍本地人工智能。", 48),
    );
    assert!(bytes > 0 && usage.prompt_tokens > 0);
    handle.unload().unwrap();
    handle.load(model.clone(), config().load_options).unwrap();
    let (recovery_bytes, _) =
        complete(&handle, request(model.clone(), "Say hello in English.", 32));
    assert!(recovery_bytes > 0);
    let pending = request(model, "Count as many numbers as you can.", 512);
    let id = pending.request_id;
    let events = handle.submit(pending).unwrap();
    let mut cancelled = false;
    while let Some(event) = events.recv() {
        match event.kind {
            RequestEventKind::Started { .. } => {
                handle.cancel(id).unwrap();
            }
            RequestEventKind::Cancelled { reason, .. } => {
                assert_eq!(reason, ErrorCode::RequestCancelled);
                cancelled = true;
            }
            RequestEventKind::Failed { error, .. } => panic!("cancel failed: {error}"),
            _ => {}
        }
    }
    assert!(cancelled);
    runtime.shutdown().unwrap();
    assert!(diagnostics.worker_pid().is_none());
    assert_eq!(
        diagnostics.sessions_started(),
        diagnostics.sessions_reaped()
    );
    println!(
        "{}",
        serde_json::json!({"harness": "non_native_management", "real_worker_ipc": "pass", "text_bytes": bytes, "recovery_text_bytes": recovery_bytes, "usage_prompt": usage.prompt_tokens, "usage_completion": usage.completion_tokens, "cancel": "pass", "sessions": diagnostics.sessions_started(), "reaped": diagnostics.sessions_reaped(), "elapsed_ms": started.elapsed().as_millis()})
    );
}
