//! Direct public Executor research lane. It never changes model admission.
use crate::{
    host::{Host, Result, failure, runtime_error},
    output::Operation,
};
use mnn_executor::MnnExecutor;
use runtime_core::{
    ExecutionEventSink, ExecutionEvents, Executor, ExecutorCommand, ExecutorEvent, ModelResolver,
};
use runtime_types::{
    GenerationOptions, GenerationRequest, LoadOptions, Message, ModelId, RequestId, Role,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
// The full fixed input/parameter implementation and suite ID enter the digest.
pub const SUITE_SPEC: &str = include_str!("runner.rs");
fn id() -> ModelId {
    ModelId::new(crate::host::MODEL).unwrap()
}
fn options() -> LoadOptions {
    LoadOptions {
        context_size: 2048,
        threads: 2,
        batch_size: 32,
    }
}
fn request(messages: Vec<Message>) -> GenerationRequest {
    GenerationRequest {
        request_id: RequestId::new(),
        model: id(),
        messages,
        options: GenerationOptions {
            max_tokens: 256,
            temperature: 0.,
            top_p: 1.,
            seed: 0,
            stops: vec![],
        },
    }
}
struct Sink {
    op: Arc<Operation>,
    terminal: Mutex<Option<ExecutorEvent>>,
    changed: Condvar,
    output: Mutex<(Sha256, u64)>,
    prompt: Mutex<Option<u32>>,
    first_text: AtomicBool,
}
impl Sink {
    fn new(op: Arc<Operation>) -> Arc<Self> {
        Arc::new(Self {
            op,
            terminal: Mutex::new(None),
            changed: Condvar::new(),
            output: Mutex::new((Sha256::new(), 0)),
            prompt: Mutex::new(None),
            first_text: AtomicBool::new(false),
        })
    }
}
impl ExecutionEventSink for Sink {
    fn emit(&self, event: ExecutorEvent) -> bool {
        match event {
            ExecutorEvent::TextDelta(ref text) => {
                let mut out = self.output.lock().unwrap();
                out.0.update(text.as_bytes());
                out.1 += text.len() as u64;
                drop(out);
                self.first_text.store(true, Ordering::Release);
                self.op.text(text)
            }
            ExecutorEvent::Prepared { prompt_tokens } => {
                *self.prompt.lock().unwrap() = Some(prompt_tokens);
                true
            }
            terminal => {
                let mut t = self.terminal.lock().unwrap();
                if t.is_some() {
                    self.op.stop("native_protocol");
                    return false;
                }
                *t = Some(terminal);
                self.changed.notify_all();
                true
            }
        }
    }
}
fn command(
    executor: &mut impl Executor,
    op: &Arc<Operation>,
    command: ExecutorCommand,
    cancel_on_text: bool,
) -> Result<(ExecutorEvent, Value)> {
    let profile = match &command {
        ExecutorCommand::Generate { request } => {
            json!({"requested":{"max_tokens":request.options.max_tokens,"temperature":request.options.temperature,"top_p":request.options.top_p,"seed":request.options.seed,"stop_count":request.options.stops.len()},"effective":{"backend":"cpu","context_size":2048,"threads":2,"batch_size":32}})
        }
        ExecutorCommand::Load { options, .. } => json!({"requested":options,"effective":options}),
        ExecutorCommand::Unload => Value::Null,
    };
    let sink = Sink::new(op.clone());
    let now = Instant::now();
    let control = executor
        .start(command, ExecutionEvents::from_sink(1, sink.clone()))
        .map_err(runtime_error)?;
    op.bind(control.clone());
    let mut cancel_at = None;
    let event = loop {
        let mut t = sink.terminal.lock().unwrap();
        if let Some(t) = t.take() {
            break t;
        }
        if cancel_on_text && sink.first_text.load(Ordering::Acquire) && cancel_at.is_none() {
            cancel_at = Some(Instant::now());
            control.cancel();
        }
        t = sink
            .changed
            .wait_timeout(t, Duration::from_millis(10))
            .unwrap()
            .0;
        drop(t);
    };
    op.clear_control();
    let out = sink.output.lock().unwrap();
    let usage = match &event {
        ExecutorEvent::Completed { usage, .. } | ExecutorEvent::GenerationFailed { usage, .. } => {
            Some(*usage)
        }
        _ => None,
    };
    let metrics = json!({"profile":profile,"duration_ms":now.elapsed().as_millis(),"prompt_tokens":usage.map(|u|u.prompt_tokens).or(*sink.prompt.lock().unwrap()),"completion_tokens":usage.map(|u|u.completion_tokens),"output_bytes":out.1,"output_sha256":format!("{:x}",out.0.clone().finalize()),"cancel_to_return_ms":cancel_at.map(|t|t.elapsed().as_millis()),"usage_reason":if matches!(event,ExecutorEvent::Faulted(_)){Some("fatal_completion_unknown")}else{None}});
    if matches!(event, ExecutorEvent::CleanupUnconfirmed(_)) {
        return Err(failure("cleanup_unconfirmed"));
    }
    Ok((event, metrics))
}
fn record(
    op: &Operation,
    cases: &mut Vec<Value>,
    case: &str,
    layer: &str,
    passed: bool,
    metrics: Value,
) {
    let result = json!({"case_id":case,"layer":layer,"verdict":if passed{"passed"}else if op.stopped(){"inconclusive"}else{"failed"},"reason_code":if passed{None}else if op.stopped(){Some("operation_cancelled")}else{Some("unexpected_result")},"metrics":metrics});
    op.emit("case_result",json!({"case_id":case,"layer":layer,"verdict":result["verdict"],"reason_code":result["reason_code"],"duration_ms":result["metrics"]["duration_ms"]}));
    op.cases.lock().unwrap().push(result.clone());
    cases.push(result);
}
fn began(op: &Operation, case: &str, layer: &str) {
    op.emit("case_started", json!({"case_id":case,"layer":layer}));
}
fn checkpoint(op: &Operation) -> Result<()> {
    if op.stopped() {
        Err(failure("request_cancelled"))
    } else {
        Ok(())
    }
}
pub fn run(h: &Arc<Host>, op: &Arc<Operation>) -> Result<Vec<Value>> {
    let snapshot = h.store.lock().unwrap().snapshot().map_err(runtime_error)?;
    let resolved = snapshot.resolve_candidate(&id()).map_err(runtime_error)?;
    if resolved.validated {
        return Err(failure("integrity_failure"));
    }
    let (resolver, mut executor) = MnnExecutor::composition(snapshot).map_err(runtime_error)?;
    let mut cases = vec![];
    let production_rejected = resolver
        .resolve(&id())
        .is_err_and(|e| e.code == runtime_types::ErrorCode::UnsupportedModel);
    drop(resolver);
    record(
        op,
        &mut cases,
        "production_resolver_rejects",
        "executor",
        production_rejected,
        json!({}),
    );
    if !production_rejected {
        return Err(failure("integrity_failure"));
    }
    let result = (|| {
        checkpoint(op)?;
        op.emit(
            "progress",
            json!({"stage":"loading","completed_bytes":null,"total_bytes":null}),
        );
        let (event, metrics) = command(
            &mut executor,
            op,
            ExecutorCommand::Load {
                model: resolved.clone(),
                options: options(),
            },
            false,
        )?;
        let loaded = matches!(event, ExecutorEvent::Loaded);
        record(op, &mut cases, "load", "executor", loaded, metrics);
        if !loaded {
            return Err(failure("native_failure"));
        }
        let english = vec![Message::new(
            Role::User,
            "In one short sentence, explain why the sky appears blue.",
        )];
        let chinese = vec![
            Message::new(Role::System, "请使用中文，用一句简短的话回答。"),
            Message::new(Role::User, "我喜欢海边。"),
            Message::new(Role::Assistant, "海边能让人放松。"),
            Message::new(Role::User, "请给我一个相关的出行建议。"),
        ];
        for (case, messages) in [
            ("english_stream", english.clone()),
            ("chinese_system_multiturn", chinese),
        ] {
            checkpoint(op)?;
            began(op, case, "executor");
            let (event, m) = command(
                &mut executor,
                op,
                ExecutorCommand::Generate {
                    request: request(messages),
                },
                false,
            )?;
            let passed = matches!(event, ExecutorEvent::Completed { .. })
                && m["output_bytes"].as_u64().unwrap_or(0) > 0;
            record(op, &mut cases, case, "executor", passed, m);
            if !passed {
                return Err(failure("native_failure"));
            }
        }
        checkpoint(op)?;
        began(op, "eos_or_stop_completion", "executor");
        let mut stop = request(vec![Message::new(
            Role::User,
            "Repeat exactly this word: Hello",
        )]);
        stop.options.stops = vec!["Hello".into(), "hello".into()];
        let (event, m) = command(
            &mut executor,
            op,
            ExecutorCommand::Generate { request: stop },
            false,
        )?;
        // This public layer deliberately merges EOS and explicit stop.
        record(
            op,
            &mut cases,
            "eos_or_stop_completion",
            "executor",
            matches!(
                event,
                ExecutorEvent::Completed {
                    finish_reason: runtime_types::FinishReason::Stop,
                    ..
                }
            ),
            m,
        );
        checkpoint(op)?;
        began(op, "active_cancel", "executor");
        let (event, m) = command(
            &mut executor,
            op,
            ExecutorCommand::Generate {
                request: request(vec![Message::new(
                    Role::User,
                    "List the integers from one to one hundred, with a short sentence for each number.",
                )]),
            },
            true,
        )?;
        let cancelled = matches!(event,ExecutorEvent::GenerationFailed{ref error,..}if error.code==runtime_types::ErrorCode::RequestCancelled);
        record(op, &mut cases, "active_cancel", "executor", cancelled, m);
        checkpoint(op)?;
        began(op, "cancel_recovery", "executor");
        let mut recovery = request(english);
        recovery.options.max_tokens = 8;
        let (event, m) = command(
            &mut executor,
            op,
            ExecutorCommand::Generate { request: recovery },
            false,
        )?;
        record(
            op,
            &mut cases,
            "cancel_recovery",
            "executor",
            matches!(event, ExecutorEvent::Completed { .. }),
            m,
        );
        Ok(())
    })();
    // A cleanup fault must survive all later errors. No timeout destroys owner resources.
    op.emit(
        "progress",
        json!({"stage":"unloading","completed_bytes":null,"total_bytes":null}),
    );
    let metrics = finish_executor(
        &mut executor,
        op,
        result
            .as_ref()
            .err()
            .is_some_and(|e| e.code == "cleanup_unconfirmed"),
    )?;
    drop(executor);
    record(op, &mut cases, "unload_close", "executor", true, metrics);
    result?;
    checkpoint(op)?;
    adapter_cases(h, op, &mut cases)?;
    for (case, layer, reason) in [
        (
            "native_background_cancel",
            "app",
            "manual_device_action_required",
        ),
        ("SAF_faults", "app", "manual_device_action_required"),
        ("process_restart", "app", "manual_device_action_required"),
        ("logcat_canary", "app", "external_capture_required"),
        ("stability", "app", "not_implemented_disabled"),
        ("core_scheduler", "core", "b3b_not_admitted"),
    ] {
        let value = json!({"case_id":case,"layer":layer,"verdict":"not_run","reason_code":reason});
        op.cases.lock().unwrap().push(value.clone());
        cases.push(value);
    }
    Ok(cases)
}
fn adapter_cases(h: &Arc<Host>, op: &Arc<Operation>, cases: &mut Vec<Value>) -> Result<()> {
    use mnn_adapter::{Cancellation, Model, Request, Seed, TextAction};
    let snapshot = h.store.lock().unwrap().snapshot().map_err(runtime_error)?;
    let resolved = snapshot.resolve_candidate(&id()).map_err(runtime_error)?;
    let lease = snapshot
        .acquire(&resolved, options(), || op.stopped())
        .map_err(runtime_error)?;
    let path = lease
        .runtime_config()
        .to_str()
        .ok_or_else(|| failure("invalid_manifest"))?;
    let cancel = Cancellation::new().map_err(|_| failure("native_failure"))?;
    let controller = cancel.clone();
    op.bind(runtime_core::CancellationHandle::new(move || {
        controller.cancel()
    }));
    let now = Instant::now();
    let mut model = Model::load(
        &mnn_adapter::LoadOptions {
            runtime_config_path: path,
            artifact_sha256: lease.artifact_digest(),
            logical_context: 2048,
            threads: 2,
            prefill_chunk: 32,
        },
        &cancel,
        |_| {},
    )
    .map_err(|_| failure("native_failure"))?;
    let result = (|| {
        let messages = [mnn_adapter::Message {
            role: mnn_adapter::Role::User,
            content: "Explain in a short sentence why leaves are green.",
        }];
        let mut request = Request {
            messages: &messages,
            stops: &[],
            max_tokens: 1,
            temperature: 0.,
            top_p: 1.,
            seed: Seed::Fixed(0),
        };
        let mut probe = model
            .prepare(&request, &cancel, |_| {})
            .map_err(|_| failure("native_failure"))?;
        let tokens = probe.info().prompt_tokens;
        if probe.close().is_err() {
            std::mem::forget(probe);
            return Err(failure("cleanup_unconfirmed"));
        }
        drop(probe);
        request.max_tokens = 2048 - tokens as u32;
        let exact = match model.prepare(&request, &cancel, |_| {}) {
            Ok(mut p) => {
                if p.close().is_err() {
                    std::mem::forget(p);
                    return Err(failure("cleanup_unconfirmed"));
                }
                true
            }
            Err(_) => false,
        };
        record(
            op,
            cases,
            "exact_budget_prepare",
            "adapter",
            exact,
            json!({"prompt_tokens":tokens,"reserved_completion_tokens":request.max_tokens}),
        );
        request.max_tokens += 1;
        let over = match model.prepare(&request, &cancel, |_| {}) {
            Err(e) => e.kind == mnn_adapter::ErrorKind::Budget,
            Ok(mut prepared) => {
                if prepared.close().is_err() {
                    std::mem::forget(prepared);
                    return Err(failure("cleanup_unconfirmed"));
                }
                false
            }
        };
        record(op, cases, "over_one_budget", "adapter", over, json!({}));
        request.max_tokens = 16;
        let prepared = model
            .prepare(&request, &cancel, |_| {})
            .map_err(|_| failure("native_failure"))?;
        let generation = prepared.generate(
            &cancel,
            |s| {
                if op.text(s) {
                    TextAction::Continue
                } else {
                    TextAction::Cancel
                }
            },
            |_| {},
        );
        confirm_generation_cleanup(&generation)?;
        record(
            op,
            cases,
            "adapter_reload_stream",
            "adapter",
            generation.is_ok(),
            json!({"duration_ms":now.elapsed().as_millis()}),
        );
        let mut baseline = String::new();
        request.max_tokens = 32;
        let generated = model
            .prepare(&request, &cancel, |_| {})
            .map_err(|_| failure("native_failure"))?
            .generate(
                &cancel,
                |text| {
                    if baseline.len() < 4096 {
                        baseline.push_str(text);
                    }
                    TextAction::Continue
                },
                |_| {},
            );
        confirm_generation_cleanup(&generated)?;
        if generated.is_err() {
            return Err(failure("native_failure"));
        }
        let stop = baseline
            .chars()
            .next()
            .map(|c| c.to_string())
            .ok_or_else(|| failure("native_protocol"))?;
        let stops = [stop.as_str()];
        request.stops = &stops;
        let stopped = model
            .prepare(&request, &cancel, |_| {})
            .map_err(|_| failure("native_failure"))?
            .generate(
                &cancel,
                |text| {
                    if op.text(text) {
                        TextAction::Continue
                    } else {
                        TextAction::Cancel
                    }
                },
                |_| {},
            );
        confirm_generation_cleanup(&stopped)?;
        record(
            op,
            cases,
            "exact_stop_string",
            "adapter",
            stopped.is_ok_and(|g| g.finish_reason == mnn_adapter::FinishReason::Stop),
            json!({"stop_source":"first_unicode_scalar_of_same_seed_baseline","stop_text_exported":false}),
        );
        Ok(())
    })();
    if result
        .as_ref()
        .err()
        .is_some_and(|e| e.code == "cleanup_unconfirmed")
    {
        std::mem::forget(model);
        std::mem::forget(lease);
        return result;
    }
    if model.close().is_err() {
        std::mem::forget(model);
        std::mem::forget(lease);
        return Err(failure("cleanup_unconfirmed"));
    }
    op.clear_control();
    drop(model);
    result?;
    for (name, phase) in [
        ("load", mnn_adapter::Phase::Load),
        ("template", mnn_adapter::Phase::Template),
        ("tokenize", mnn_adapter::Phase::Tokenize),
        ("prefill", mnn_adapter::Phase::Prefill),
        ("decode", mnn_adapter::Phase::Decode),
    ] {
        checkpoint(op)?;
        let case = format!("adapter_cancel_{name}");
        began(op, &case, "adapter");
        let cancel = Cancellation::new().map_err(|_| failure("native_failure"))?;
        let controller = cancel.clone();
        op.bind(runtime_core::CancellationHandle::new(move || {
            controller.cancel()
        }));
        let (send, receive) = std::sync::mpsc::sync_channel::<()>(1);
        let (done, ack) = std::sync::mpsc::sync_channel::<Instant>(1);
        let cross_thread = cancel.clone();
        let worker = std::thread::spawn(move || {
            if receive.recv().is_ok() {
                let at = Instant::now();
                cross_thread.cancel();
                let _ = done.send(at);
            }
        });
        let mut send = Some(send);
        let mut requested = None;
        let mut progress = |p: mnn_adapter::Progress| {
            if p.phase == phase
                && (phase != mnn_adapter::Phase::Prefill || p.count > 0)
                && let Some(send) = send.take()
            {
                let _ = send.send(());
                requested = ack.recv().ok();
            }
        };
        let loaded = Model::load(
            &mnn_adapter::LoadOptions {
                runtime_config_path: path,
                artifact_sha256: lease.artifact_digest(),
                logical_context: 2048,
                threads: 2,
                prefill_chunk: 32,
            },
            &cancel,
            &mut progress,
        );
        let mut native = None;
        let mut cleanup_unconfirmed = false;
        let cancelled = match loaded {
            Err(e) => e.kind == mnn_adapter::ErrorKind::Cancelled,
            Ok(m) => {
                native = Some(m);
                let messages = [mnn_adapter::Message {
                    role: mnn_adapter::Role::User,
                    content: "Please list several facts about the ocean in short sentences.",
                }];
                let request = Request {
                    messages: &messages,
                    stops: &[],
                    max_tokens: 32,
                    temperature: 0.,
                    top_p: 1.,
                    seed: Seed::Fixed(0),
                };
                match native
                    .as_mut()
                    .unwrap()
                    .prepare(&request, &cancel, &mut progress)
                {
                    Err(e) => e.kind == mnn_adapter::ErrorKind::Cancelled,
                    Ok(prepared) => prepared
                        .generate(
                            &cancel,
                            |s| {
                                if op.text(s) {
                                    TextAction::Continue
                                } else {
                                    TextAction::Cancel
                                }
                            },
                            &mut progress,
                        )
                        .is_err_and(|e| {
                            cleanup_unconfirmed = e.cleanup_error.is_some();
                            e.error.kind == mnn_adapter::ErrorKind::Cancelled
                        }),
                }
            }
        };
        drop(send);
        let _ = worker.join();
        let returned = requested.map(|t| t.elapsed().as_micros());
        if cleanup_unconfirmed {
            std::mem::forget(native);
            std::mem::forget(lease);
            return Err(failure("cleanup_unconfirmed"));
        }
        if let Some(mut model) = native
            && model.close().is_err()
        {
            std::mem::forget(model);
            std::mem::forget(lease);
            return Err(failure("cleanup_unconfirmed"));
        }
        op.clear_control();
        record(
            op,
            cases,
            &case,
            "adapter",
            cancelled && requested.is_some(),
            json!({"observed_phase":name,"cancel_to_safe_return_us":returned,"cancel_to_close_us":requested.map(|t|t.elapsed().as_micros()),"method":"cross_thread_cancel_at_public_checkpoint","arbitrary_kernel_latency_guarantee":false}),
        );
    }
    drop(lease);
    drop(snapshot);
    Ok(())
}
pub fn complete_case_journal(op: &Operation) {
    if op.kind != "suite" {
        return;
    }
    let mut cases = op.cases.lock().unwrap();
    for (case, layer) in [
        ("production_resolver_rejects", "executor"),
        ("load", "executor"),
        ("english_stream", "executor"),
        ("chinese_system_multiturn", "executor"),
        ("eos_or_stop_completion", "executor"),
        ("active_cancel", "executor"),
        ("cancel_recovery", "executor"),
        ("unload_close", "executor"),
        ("exact_budget_prepare", "adapter"),
        ("over_one_budget", "adapter"),
        ("adapter_reload_stream", "adapter"),
        ("exact_stop_string", "adapter"),
        ("adapter_cancel_load", "adapter"),
        ("adapter_cancel_template", "adapter"),
        ("adapter_cancel_tokenize", "adapter"),
        ("adapter_cancel_prefill", "adapter"),
        ("adapter_cancel_decode", "adapter"),
        ("native_background_cancel", "app"),
        ("SAF_faults", "app"),
        ("process_restart", "app"),
        ("logcat_canary", "app"),
        ("stability", "app"),
        ("core_scheduler", "core"),
    ] {
        if !cases.iter().any(|v| v["case_id"] == case) {
            cases.push(json!({"case_id":case,"layer":layer,"verdict":"not_run","reason_code":"execution_not_reached"}));
        }
    }
}

fn confirm_generation_cleanup(
    result: &std::result::Result<mnn_adapter::Generation, mnn_adapter::GenerationFailure>,
) -> Result<()> {
    if result
        .as_ref()
        .err()
        .is_some_and(|failure| failure.cleanup_error.is_some())
    {
        Err(failure("cleanup_unconfirmed"))
    } else {
        Ok(())
    }
}
fn finish_executor(
    executor: &mut impl Executor,
    op: &Arc<Operation>,
    already_unconfirmed: bool,
) -> Result<Value> {
    if already_unconfirmed {
        return Err(failure("cleanup_unconfirmed"));
    }
    let (event, metrics) = command(executor, op, ExecutorCommand::Unload, false)
        .map_err(|_| failure("cleanup_unconfirmed"))?;
    if !matches!(event, ExecutorEvent::Unloaded) {
        return Err(failure("cleanup_unconfirmed"));
    }
    executor
        .close()
        .map_err(|_| failure("cleanup_unconfirmed"))?;
    Ok(metrics)
}
#[cfg(test)]
mod tests {
    use super::*;
    struct ControlExecutor {
        calls: usize,
        fail_start: bool,
        cleanup_event: bool,
        fail_close: bool,
    }
    impl Executor for ControlExecutor {
        fn start(
            &mut self,
            _: ExecutorCommand,
            events: ExecutionEvents,
        ) -> std::result::Result<runtime_core::CancellationHandle, runtime_types::RuntimeError>
        {
            self.calls += 1;
            if self.fail_start {
                return Err(runtime_types::RuntimeError::new(
                    runtime_types::ErrorCode::ExecutorUnavailable,
                    "synthetic control failure",
                ));
            }
            if self.cleanup_event {
                events.emit(ExecutorEvent::CleanupUnconfirmed(
                    runtime_types::RuntimeError::new(
                        runtime_types::ErrorCode::ExecutorCleanupUnconfirmed,
                        "synthetic control failure",
                    ),
                ));
            } else {
                events.emit(ExecutorEvent::Unloaded);
            }
            Ok(runtime_core::CancellationHandle::noop())
        }
        fn close(&mut self) -> std::result::Result<(), runtime_types::RuntimeError> {
            if self.fail_close {
                Err(runtime_types::RuntimeError::new(
                    runtime_types::ErrorCode::ExecutorCleanupUnconfirmed,
                    "synthetic control failure",
                ))
            } else {
                Ok(())
            }
        }
    }
    #[test]
    fn adapter_cleanup_error_is_never_reclassified_as_cancellation() {
        let failure = mnn_adapter::GenerationFailure {
            error: mnn_adapter::Error {
                kind: mnn_adapter::ErrorKind::Cancelled,
            },
            usage: mnn_adapter::Generation {
                prompt_tokens: 3,
                completion_tokens: 1,
                resolved_seed: 0,
                finish_reason: mnn_adapter::FinishReason::Cancelled,
            },
            cleanup_error: Some(mnn_adapter::Error {
                kind: mnn_adapter::ErrorKind::Native,
            }),
        };
        assert_eq!(
            confirm_generation_cleanup(&Err(failure)).unwrap_err().code,
            "cleanup_unconfirmed"
        );
    }
    #[test]
    fn cleanup_fault_cannot_be_downgraded_by_unload() {
        let op = Operation::new("suite", None);
        for (fail_start, cleanup_event, fail_close) in [
            (true, false, false),
            (false, true, false),
            (false, false, true),
        ] {
            let mut e = ControlExecutor {
                calls: 0,
                fail_start,
                cleanup_event,
                fail_close,
            };
            assert_eq!(
                finish_executor(&mut e, &op, false).unwrap_err().code,
                "cleanup_unconfirmed"
            );
        }
        let mut e = ControlExecutor {
            calls: 0,
            fail_start: true,
            cleanup_event: false,
            fail_close: false,
        };
        assert_eq!(
            finish_executor(&mut e, &op, true).unwrap_err().code,
            "cleanup_unconfirmed"
        );
        assert_eq!(e.calls, 0);
        e.fail_start = false;
        assert!(finish_executor(&mut e, &op, false).is_ok());
    }
}
