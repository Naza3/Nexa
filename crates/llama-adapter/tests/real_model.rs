//! Opt-in real GGUF verification. No fake inference is used by these tests.
//! Set NEXA_TEST_MODEL to the locked GGUF and pass --ignored --test-threads=1.
use llama_adapter::{CancelHandle, Engine, Model, StreamControl};
use runtime_types::{
    ErrorCode, FinishReason, GenerationOptions, LoadOptions, Message, Role, Usage,
};
use std::panic::{AssertUnwindSafe, catch_unwind};

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
        threads: 4,
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
