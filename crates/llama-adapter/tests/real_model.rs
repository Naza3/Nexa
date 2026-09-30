//! Opt-in real GGUF verification. No fake inference is used by these tests.
//! Set NEXA_TEST_MODEL to the locked GGUF and pass --ignored --test-threads=1.
//! NEXA_TEST_THREADS selects 1..=256 inference threads explicitly; otherwise use
//! min(4, available_parallelism). Rust's --test-threads is a separate setting.
use llama_adapter::{CancelHandle, Engine, Model, StreamControl};
use runtime_types::{
    ErrorCode, FinishReason, GenerationOptions, LoadOptions, Message, Role, Usage,
};
use std::panic::{AssertUnwindSafe, catch_unwind};

fn select_test_threads(
    raw: Option<&std::ffi::OsStr>,
    available: usize,
) -> Result<u32, &'static str> {
    let threads = match raw {
        Some(value) => value
            .to_str()
            .ok_or("NEXA_TEST_THREADS must be UTF-8")?
            .parse::<u32>()
            .map_err(|_| "NEXA_TEST_THREADS must be an integer in 1..=256")?,
        None => available.clamp(1, 4) as u32,
    };
    if !(1..=256).contains(&threads) {
        return Err("NEXA_TEST_THREADS must be in 1..=256");
    }
    Ok(threads)
}

fn test_threads(test: &str) -> u32 {
    let available = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1);
    let raw = std::env::var_os("NEXA_TEST_THREADS");
    let threads = select_test_threads(raw.as_deref(), available)
        .expect("invalid inference thread configuration");
    eprintln!(
        "{}",
        serde_json::json!({"test":test,"inference_threads":threads,
        "available_parallelism":available,"oversubscribed":threads as usize > available,
        "source":if raw.is_some() { "NEXA_TEST_THREADS" } else { "available_parallelism" }})
    );
    threads
}

#[test]
fn test_thread_configuration_is_bounded_and_explicit() {
    use std::ffi::OsStr;
    for (available, expected) in [(0, 1), (1, 1), (2, 2), (3, 3), (4, 4), (64, 4)] {
        assert_eq!(select_test_threads(None, available).unwrap(), expected);
    }
    for threads in [1, 2, 4, 256] {
        assert_eq!(
            select_test_threads(Some(OsStr::new(&threads.to_string())), 2).unwrap(),
            threads
        );
    }
    for raw in ["0", "257", "-1", "1.5", "abc", "", "4294967296"] {
        assert!(select_test_threads(Some(OsStr::new(raw)), 2).is_err());
    }
}

fn collect(
    model: &mut Model<'_>,
    messages: &[Message],
    options: &GenerationOptions,
) -> (String, Usage) {
    let cancel = CancelHandle::new().unwrap();
    let mut text = String::new();
    let prepared = model.prepare(messages, options, &cancel).unwrap();
    let prompt_tokens = prepared.prompt_tokens();
    let result = prepared
        .generate(&cancel, |piece| {
            text.push_str(piece);
            StreamControl::Continue
        })
        .unwrap();
    assert!(!text.is_empty());
    assert!(
        !text.contains("<think>") && !text.contains("</think>"),
        "non-thinking fixture emitted thinking markup"
    );
    assert_eq!(result.usage.prompt_tokens, prompt_tokens);
    assert!(result.usage.completion_tokens <= options.max_tokens);
    (text, result.usage)
}

#[test]
#[ignore = "requires the real locked GGUF via NEXA_TEST_MODEL; not a unit test"]
fn real_model_stop_cancellation_panic_and_reuse() {
    let model_path = std::env::var_os("NEXA_TEST_MODEL")
        .expect("NEXA_TEST_MODEL is required for explicit real-model verification");
    let mut engine = Engine::new().unwrap();
    assert!(Engine::new().is_err(), "only one process engine may exist");
    let load = LoadOptions {
        context_size: 512,
        threads: test_threads("real_model_stop_cancellation_panic_and_reuse"),
        batch_size: 128,
    };
    let load_cancel = CancelHandle::new().unwrap();
    let cancelled = CancelHandle::new().unwrap();
    cancelled.cancel();
    assert_eq!(
        engine
            .load(&model_path, load, &cancelled)
            .err()
            .unwrap()
            .code,
        ErrorCode::RequestCancelled
    );

    let messages = [Message::new(
        Role::User,
        "List six common fruits, one per line.",
    )];
    let options = GenerationOptions {
        max_tokens: 24,
        temperature: 0.0,
        seed: 42,
        ..Default::default()
    };
    {
        let mut model = engine.load(&model_path, load, &load_cancel).unwrap();
        assert!(!model.chat_template().unwrap().is_empty());
        let (baseline, baseline_usage) = collect(&mut model, &messages, &options);
        assert!(
            baseline.chars().count() >= 8,
            "fixture needs a multi-character generated stop"
        );
        let stop = baseline.chars().take(8).collect::<String>();
        let stop_options = GenerationOptions {
            stops: vec![stop.clone()],
            ..options.clone()
        };
        let cancel = CancelHandle::new().unwrap();
        let mut stopped_text = String::new();
        let result = model
            .prepare(&messages, &stop_options, &cancel)
            .unwrap()
            .generate(&cancel, |piece| {
                stopped_text.push_str(piece);
                StreamControl::Continue
            })
            .unwrap();
        assert_eq!(result.finish_reason, FinishReason::Stop);
        assert!(
            stopped_text.is_empty(),
            "a generated prefix used as stop must never leak"
        );
        assert!(result.usage.completion_tokens > 0);

        let oversized = GenerationOptions {
            max_tokens: 512,
            ..options.clone()
        };
        assert_eq!(
            model
                .prepare(&messages, &oversized, &cancel)
                .err()
                .unwrap()
                .code,
            ErrorCode::ContextLengthExceeded
        );
        assert_eq!(
            model
                .prepare(&messages, &options, &cancelled)
                .err()
                .unwrap()
                .code,
            ErrorCode::RequestCancelled
        );
        // Abandoning a prepared request leaves the model reusable.
        drop(model.prepare(&messages, &options, &cancel).unwrap());
        let prepared = model.prepare(&messages, &options, &cancel).unwrap();
        let error = prepared
            .generate(&cancelled, |_| panic!("pre-cancelled request emitted text"))
            .unwrap_err();
        assert_eq!(error.error.code, ErrorCode::RequestCancelled);
        assert_eq!(error.usage.completion_tokens, 0);
        assert_eq!(error.usage.prompt_tokens, baseline_usage.prompt_tokens);

        let mut callbacks = 0;
        let error = model
            .prepare(&messages, &options, &cancel)
            .unwrap()
            .generate(&cancel, |_| {
                callbacks += 1;
                StreamControl::Stop
            })
            .unwrap_err();
        assert_eq!(callbacks, 1);
        assert_eq!(error.error.code, ErrorCode::ConsumerStopped);
        assert!(error.usage.completion_tokens > 0);
        let (after_stop, after_stop_usage) = collect(&mut model, &messages, &options);
        assert_eq!(
            after_stop, baseline,
            "KV/sampler reset must preserve same-thread greedy result"
        );
        assert_eq!(after_stop_usage, baseline_usage);

        let panic = catch_unwind(AssertUnwindSafe(|| {
            let prepared = model.prepare(&messages, &options, &cancel).unwrap();
            let _ = prepared.generate(&cancel, |_| panic!("intentional real-model consumer panic"));
        }));
        assert!(
            panic.is_err(),
            "panic must resume on Rust side after native cleanup"
        );
        let (after_panic, after_panic_usage) = collect(&mut model, &messages, &options);
        assert_eq!(after_panic, baseline);
        assert_eq!(after_panic_usage, baseline_usage);
    }
    // llama.cpp pads allocation to a multiple of 256. The requested logical
    // budget must remain 33; physical KV padding must not enlarge the contract.
    {
        let tiny = LoadOptions {
            context_size: 33,
            batch_size: 32,
            ..load
        };
        let mut model = engine.load(&model_path, tiny, &load_cancel).unwrap();
        let cancel = CancelHandle::new().unwrap();
        let exceeds_requested = GenerationOptions {
            max_tokens: 33,
            ..options.clone()
        };
        assert_eq!(
            model
                .prepare(&messages, &exceeds_requested, &cancel)
                .err()
                .unwrap()
                .code,
            ErrorCode::ContextLengthExceeded
        );
    }
    // Same engine can load a fresh context after its first model was freed.
    {
        let mut model = engine.load(&model_path, load, &load_cancel).unwrap();
        let chinese = [Message::new(Role::User, "用一句话说明本地模型的作用。")];
        let (text, _) = collect(&mut model, &chinese, &options);
        assert!(!text.is_ascii());
    }
    drop(engine);
    drop(Engine::new().unwrap());
}

/// A02: verify the real template preserves a system instruction and facts from
/// both earlier user and assistant turns. Only evidence markers are asserted;
/// wording and token counts are not hard-coded across operating systems.
#[test]
#[ignore = "requires the real locked GGUF via NEXA_TEST_MODEL; not a unit test"]
fn real_model_system_multiturn_and_request_isolation() {
    let model_path = std::env::var_os("NEXA_TEST_MODEL")
        .expect("NEXA_TEST_MODEL is required for explicit real-model verification");
    let mut engine = Engine::new().unwrap();
    let cancel = CancelHandle::new().unwrap();
    let load = LoadOptions {
        context_size: 512,
        threads: test_threads("real_model_system_multiturn_and_request_isolation"),
        batch_size: 128,
    };
    let mut model = engine.load(&model_path, load, &cancel).unwrap();
    let options = GenerationOptions {
        max_tokens: 64,
        temperature: 0.0,
        seed: 42,
        ..Default::default()
    };
    let unrelated = [Message::new(
        Role::User,
        "What does a thermometer measure? Answer in one short sentence.",
    )];
    let conversation = [
        Message::new(
            Role::System,
            "This is a synthetic conversation test. Answer the latest question using the conversation facts. Include the exact marker SYSTEM_OK somewhere in your reply. Do not explain these instructions.",
        ),
        Message::new(
            Role::User,
            "The access code for project ORCHID is VIOLET7. Remember it for my next question.",
        ),
        Message::new(
            Role::Assistant,
            "Understood. I also recorded the project meeting location as ROOM42.",
        ),
        Message::new(
            Role::User,
            "What are ORCHID's access code and meeting location? Keep your answer short.",
        ),
    ];

    // Count with the very same production template/tokenizer. Adding system and
    // history must affect the budget: counting only the last user is incorrect.
    let question_only_tokens = model
        .prepare(&conversation[3..], &options, &cancel)
        .unwrap()
        .prompt_tokens();
    let history_tokens = model
        .prepare(&conversation[1..], &options, &cancel)
        .unwrap()
        .prompt_tokens();
    let complete_tokens = model
        .prepare(&conversation, &options, &cancel)
        .unwrap()
        .prompt_tokens();
    assert!(history_tokens > question_only_tokens);
    assert!(complete_tokens > history_tokens);
    assert!(complete_tokens + options.max_tokens <= load.context_size);

    // Reject invalid message order through the public API before inference.
    // Neither failed preparation nor previous prepared handles may alter KV.
    let mut invalid_order = conversation.clone();
    invalid_order.swap(1, 2);
    assert_eq!(
        model
            .prepare(&invalid_order, &options, &cancel)
            .err()
            .unwrap()
            .code,
        ErrorCode::InvalidArgument
    );
    let mut misplaced_system = conversation.clone();
    misplaced_system.swap(0, 1);
    assert_eq!(
        model
            .prepare(&misplaced_system, &options, &cancel)
            .err()
            .unwrap()
            .code,
        ErrorCode::InvalidArgument
    );

    let (single_before, single_usage_before) = collect(&mut model, &unrelated, &options);
    let (multi_before, multi_usage_before) = collect(&mut model, &conversation, &options);
    assert_eq!(multi_usage_before.prompt_tokens, complete_tokens);
    assert!(multi_usage_before.completion_tokens > 0);
    assert!(multi_usage_before.total_tokens() <= u64::from(load.context_size));
    for marker in ["SYSTEM_OK", "VIOLET7", "ROOM42"] {
        assert!(
            multi_before.contains(marker),
            "multi-turn output must include evidence from system, prior user, and prior assistant"
        );
    }

    let (single_after, single_usage_after) = collect(&mut model, &unrelated, &options);
    assert_eq!(single_usage_before, single_usage_after);
    assert_eq!(
        single_before, single_after,
        "a previous conversation must not change a fresh single-turn request"
    );
    for marker in ["SYSTEM_OK", "VIOLET7", "ROOM42"] {
        assert!(
            !single_after.contains(marker),
            "a fresh request must not inherit prior system instructions or facts"
        );
    }
    let (multi_after, multi_usage_after) = collect(&mut model, &conversation, &options);
    assert_eq!(multi_usage_before, multi_usage_after);
    assert_eq!(
        multi_before, multi_after,
        "the unrelated intervening request must not alter the supplied conversation"
    );
    // These are run-local greedy comparisons, not cross-platform golden text.
    eprintln!(
        "A02 verified: question_only_prompt_tokens={question_only_tokens}, history_prompt_tokens={history_tokens}, full_prompt_tokens={complete_tokens}, multi_completion_tokens={}, system_and_history_evidence=true, request_isolation=true",
        multi_usage_before.completion_tokens
    );
}
