//! Explicit ignored gate: needs the real fixed candidate and native build.
//! Run with --ignored --test-threads=1; missing configuration is a failure.
use mnn_adapter::*;
fn cancel() -> Cancellation {
    Cancellation::new().unwrap()
}
fn request<'a>(messages: &'a [Message<'a>]) -> Request<'a> {
    Request {
        messages,
        stops: &[],
        max_tokens: 12,
        temperature: 0.7,
        top_p: 0.9,
        seed: Seed::Fixed(0),
    }
}
fn run(model: &mut Model, request: &Request<'_>) -> (String, Generation) {
    let c = cancel();
    let prepared = model.prepare(request, &c, |_| {}).unwrap();
    let info = prepared.info();
    let mut output = String::new();
    let result = prepared
        .generate(
            &c,
            |s| {
                assert!(!s.is_empty() && s.len() <= 4096);
                output.push_str(s);
                TextAction::Continue
            },
            |_| {},
        )
        .unwrap();
    assert_eq!(result.prompt_tokens, info.prompt_tokens);
    assert_eq!(result.resolved_seed, info.resolved_seed);
    assert!(
        result.completion_tokens > 0 && result.completion_tokens <= u64::from(request.max_tokens)
    );
    (output, result)
}
#[test]
#[ignore = "requires explicitly verified real candidate runtime configuration"]
fn real_seed_isolation_cancel_recovery_and_callback_failure() {
    let path = std::env::var("NEXA_MNN_TEST_CONFIG").expect("NEXA_MNN_TEST_CONFIG is required");
    let digest = std::env::var("NEXA_MNN_TEST_ARTIFACT_SHA256")
        .expect("verified research artifact identity is required");
    let options = LoadOptions {
        runtime_config_path: &path,
        artifact_sha256: &digest,
        logical_context: 2048,
        threads: 2,
        prefill_chunk: 32,
    };
    let c = cancel();
    let mut reached = false;
    let cancelled = Model::load(&options, &c, |p| {
        if p.phase == Phase::Load {
            reached = true;
            c.cancel();
        }
    });
    assert_eq!(cancelled.err().unwrap().kind, ErrorKind::Cancelled);
    assert!(reached);
    let c = cancel();
    let panicked = Model::load(&options, &c, |_| panic!("synthetic load callback panic"));
    assert_eq!(panicked.err().unwrap().kind, ErrorKind::CallbackPanic);
    let mut nested_seen = false;
    let mut model = Model::load(&options, &cancel(), |_| {
        let nested = Model::load(&options, &cancel(), |_| {});
        assert_eq!(nested.err().unwrap().kind, ErrorKind::Busy);
        nested_seen = true;
    })
    .unwrap();
    assert!(nested_seen);
    let messages = [Message {
        role: Role::User,
        content: "Reply with one short sentence about the sky.",
    }];
    let mut req = request(&messages);
    let a = run(&mut model, &req);
    assert!(!a.0.is_empty());
    assert_eq!(a.1.resolved_seed, 0);
    req.seed = Seed::Fixed(42);
    req.temperature = 1.5;
    req.top_p = 0.5;
    run(&mut model, &req);
    req.seed = Seed::Fixed(0);
    req.temperature = 0.7;
    req.top_p = 0.9;
    assert_eq!(a, run(&mut model, &req));
    let stop = a.0.chars().next().unwrap().to_string();
    let stops = [stop.as_str()];
    req.stops = &stops;
    let stopped = run(&mut model, &req);
    assert!(stopped.0.is_empty());
    assert_eq!(stopped.1.finish_reason, FinishReason::Stop);
    req.stops = &[];
    req.max_tokens = 2048 - a.1.prompt_tokens as u32;
    let prepared = model.prepare(&req, &cancel(), |_| {}).unwrap();
    assert_eq!(prepared.info().prompt_tokens, a.1.prompt_tokens);
    drop(prepared);
    req.max_tokens += 1;
    assert_eq!(
        model.prepare(&req, &cancel(), |_| {}).err().unwrap().kind,
        ErrorKind::Budget
    );
    req.max_tokens = 12;
    // Prepared owns native copies: source strings and callback closure are gone
    // before generation starts, and no pointer into their stack is retained.
    let prepared = {
        let temporary = String::from("Say yes.");
        let messages = [Message {
            role: Role::User,
            content: &temporary,
        }];
        let request = request(&messages);
        model.prepare(&request, &cancel(), |_| {}).unwrap()
    };
    prepared
        .generate(&cancel(), |_| TextAction::Continue, |_| {})
        .unwrap();
    let chinese = [
        Message {
            role: Role::System,
            content: "回答简洁。",
        },
        Message {
            role: Role::User,
            content: "请记住竹子。",
        },
        Message {
            role: Role::Assistant,
            content: "已记住竹子。",
        },
        Message {
            role: Role::User,
            content: "我让你记住了什么？",
        },
    ];
    assert!(!run(&mut model, &request(&chinese)).0.is_empty());
    for phase in [Phase::Template, Phase::Tokenize] {
        let c = cancel();
        let mut seen = false;
        let failed = model.prepare(&req, &c, |p| {
            if p.phase == phase {
                seen = true;
                c.cancel();
            }
        });
        assert_eq!(failed.err().unwrap().kind, ErrorKind::Cancelled);
        assert!(seen);
        assert_eq!(a, run(&mut model, &req));
    }
    for phase in [Phase::Prefill, Phase::Decode] {
        let c = cancel();
        let controller = c.clone();
        let (tx, rx) = std::sync::mpsc::sync_channel(0);
        let (ack_tx, ack_rx) = std::sync::mpsc::sync_channel(0);
        let worker = std::thread::spawn(move || {
            rx.recv_timeout(std::time::Duration::from_secs(120))
                .unwrap();
            controller.cancel();
            ack_tx.send(()).unwrap();
        });
        let prepared = model.prepare(&req, &c, |_| {}).unwrap();
        let mut seen = false;
        let failed = prepared
            .generate(
                &c,
                |_| TextAction::Continue,
                |p| {
                    if !seen && p.phase == phase && p.count > 0 {
                        seen = true;
                        tx.send(()).unwrap();
                        ack_rx
                            .recv_timeout(std::time::Duration::from_secs(120))
                            .unwrap();
                    }
                },
            )
            .unwrap_err();
        worker.join().unwrap();
        assert!(seen);
        assert_eq!(failed.error.kind, ErrorKind::Cancelled);
        assert_eq!(failed.usage.finish_reason, FinishReason::Cancelled);
        assert!(failed.usage.prompt_tokens > 0);
        assert_eq!(a, run(&mut model, &req));
    }
    for action in [TextAction::Cancel, TextAction::Fail] {
        let c = cancel();
        let failed = model
            .prepare(&req, &c, |_| {})
            .unwrap()
            .generate(&c, |_| action, |_| {})
            .unwrap_err();
        assert_eq!(
            failed.error.kind,
            if action == TextAction::Cancel {
                ErrorKind::Cancelled
            } else {
                ErrorKind::Callback
            }
        );
        assert!(failed.usage.completion_tokens > 0);
        assert_eq!(a, run(&mut model, &req));
    }
    let c = cancel();
    let failed = model
        .prepare(&req, &c, |_| {})
        .unwrap()
        .generate(&c, |_| panic!("synthetic text callback panic"), |_| {})
        .unwrap_err();
    assert_eq!(failed.error.kind, ErrorKind::CallbackPanic);
    assert_eq!(failed.usage.finish_reason, FinishReason::Error);
    assert_eq!(a, run(&mut model, &req));
    let c = cancel();
    let failed = model
        .prepare(&req, &c, |_| {})
        .unwrap()
        .generate(
            &c,
            |_| TextAction::Continue,
            |_| panic!("synthetic generation progress panic"),
        )
        .unwrap_err();
    assert_eq!(failed.error.kind, ErrorKind::CallbackPanic);
    assert_eq!(a, run(&mut model, &req));
    req.max_tokens = 1;
    assert_eq!(run(&mut model, &req).1.completion_tokens, 1);
    model.close().unwrap();
    model.close().unwrap();
    assert_eq!(
        model.prepare(&req, &cancel(), |_| {}).err().unwrap().kind,
        ErrorKind::Closed
    );
}
