//! Real CPU tool generation. The callback never executes tools; this fixture
//! uses a single in-memory lookup and feeds its result into the next request.
use llama_adapter::{CancelHandle, Engine, GeneratedDelta, StreamControl};
use runtime_types::{
    FinishReason, GenerationOptions, GenerationRequest, LoadOptions, Message, ModelId, RequestId,
    Role, ToolCall, ToolCallDelta, ToolChoice, ToolConfig, ToolDefinition,
};
use serde_json::json;

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

#[test]
#[ignore = "requires NEXA_TEST_MODEL with real tool-capable GGUF"]
fn actual_model_two_round_tool_cycle() {
    let path = std::env::var("NEXA_TEST_MODEL").expect("set NEXA_TEST_MODEL");
    let mut engine = Engine::new().expect("native engine");
    let cancel = CancelHandle::new().expect("cancel handle");
    let mut model = engine
        .load(
            path,
            LoadOptions {
                context_size: 4096,
                threads: test_threads("actual_model_two_round_tool_cycle"),
                batch_size: 256,
            },
            &cancel,
        )
        .expect("load model");
    let tools = ToolConfig { definitions: vec![ToolDefinition { name: "lookup_test_color".into(),
        description: Some("Look up the secret color and verification tag for a code. Always call this tool; do not guess.".into()),
        parameters: json!({"type":"object","properties":{"code":{"type":"string","enum":["B7"]}},"required":["code"],"additionalProperties":false}) }],
        choice: ToolChoice::Required, parallel_tool_calls: false };
    let mut request = GenerationRequest {
        request_id: RequestId::new(),
        model: ModelId::new("tool-fixture").expect("id"),
        messages: vec![
            Message::new(
                Role::System,
                "Use the provided lookup tool when asked for a code. After its result, answer with both its color and verification tag.",
            ),
            Message::new(
                Role::User,
                "Look up code B7. What color and verification tag does the lookup return?",
            ),
        ],
        options: GenerationOptions {
            max_tokens: 256,
            temperature: 0.0,
            ..Default::default()
        },
        tools,
    };
    let mut over_budget = request.clone();
    over_budget.tools.definitions[0].description = Some("x ".repeat(8192));
    let error = match model.prepare_request(&over_budget, &cancel) {
        Ok(_) => panic!("tool definitions must count toward the context budget"),
        Err(error) => error,
    };
    assert_eq!(error.code, runtime_types::ErrorCode::ContextLengthExceeded);
    let mut short = request.clone();
    short.options.max_tokens = 1;
    let mut published = 0;
    let error = model
        .prepare_request(&short, &cancel)
        .expect("short prepare")
        .generate_chat_observed(
            &cancel,
            |_| {
                published += 1;
                StreamControl::Continue
            },
            |_| StreamControl::Continue,
        )
        .expect_err("truncated tool generation must fail");
    assert_eq!(
        error.error.code,
        runtime_types::ErrorCode::IncompleteGeneration
    );
    assert_eq!(error.usage.completion_tokens, 1);
    assert_eq!(published, 0);
    let prepared = model
        .prepare_request(&request, &cancel)
        .expect("prepare complete tools prompt");
    let initial_tokens = prepared.prompt_tokens();
    let mut calls: Vec<ToolCall> = vec![];
    let mut content = String::new();
    let result = prepared
        .generate_chat_observed(
            &cancel,
            |delta| {
                match delta {
                    GeneratedDelta::Text(text) => content.push_str(text),
                    GeneratedDelta::ToolCall(ToolCallDelta::Start { index, id, name }) => {
                        assert_eq!(index as usize, calls.len());
                        calls.push(ToolCall {
                            id,
                            name,
                            arguments: String::new(),
                        });
                    }
                    GeneratedDelta::ToolCall(ToolCallDelta::Arguments { index, arguments }) => {
                        calls[index as usize].arguments.push_str(&arguments)
                    }
                }
                StreamControl::Continue
            },
            |_| StreamControl::Continue,
        )
        .expect("actual tool generation");
    assert_eq!(result.finish_reason, FinishReason::ToolCalls);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "lookup_test_color");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&calls[0].arguments).expect("raw JSON"),
        json!({"code":"B7"})
    );
    assert!(result.usage.completion_tokens > 0);
    let id = calls[0].id.clone();
    request.messages.push(Message {
        role: Role::Assistant,
        content: if content.is_empty() {
            None
        } else {
            Some(content)
        },
        tool_calls: calls,
        tool_call_id: None,
        image: None,
    });
    request.messages.push(Message {
        role: Role::Tool,
        content: Some("{\"color\":\"ultramarine\",\"verification_tag\":\"fixture-b7-v1\"}".into()),
        tool_calls: vec![],
        tool_call_id: Some(id),
        image: None,
    });
    request.tools.choice = ToolChoice::None;
    request.request_id = RequestId::new();
    let prepared = model
        .prepare_request(&request, &cancel)
        .expect("prepare full assistant/tool history");
    assert!(prepared.prompt_tokens() > initial_tokens);
    drop(prepared);
    // A caller may remove tools after a call; the full original history remains.
    request.tools.definitions.clear();
    let prepared = model
        .prepare_request(&request, &cancel)
        .expect("tool history without current definitions");
    let mut answer = String::new();
    let result = prepared
        .generate_chat_observed(
            &cancel,
            |delta| {
                match delta {
                    GeneratedDelta::Text(text) => answer.push_str(text),
                    GeneratedDelta::ToolCall(_) => panic!("no tools requested"),
                }
                StreamControl::Continue
            },
            |_| StreamControl::Continue,
        )
        .expect("actual tool result answer");
    assert_eq!(result.finish_reason, FinishReason::Stop);
    assert!(answer.to_lowercase().contains("ultramarine"));
    assert!(answer.contains("fixture-b7-v1"));
}
