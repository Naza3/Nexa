//! Real-model verification entry. Emits JSON measurements, never request/output text.
use llama_adapter::{CancelHandle, Engine, StreamControl};
use runtime_types::{
    ErrorCode, FinishReason, GenerationOptions, LoadOptions, Message, Role, Usage,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, fs::File, io::Read, path::PathBuf, process::ExitCode, sync::mpsc};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const HELP: &str = "native-smoke --model PATH --prompt-file PATH [--max-tokens 64] [--context-size 2048] [--repeat 1] [--mode generate|budget|cancel-before-load|cancel-before-prepare|cancel-before-generate|cancel-during|consumer-stop] [--stop TEXT]";

struct Options {
    model: PathBuf,
    prompt: String,
    max_tokens: u32,
    context_size: u32,
    repeat: u32,
    mode: String,
    stops: Vec<String>,
}

fn parse() -> Result<Options> {
    let mut arguments = std::env::args_os().skip(1);
    let mut values = HashMap::new();
    while let Some(flag) = arguments.next() {
        let flag = flag
            .into_string()
            .map_err(|_| "option name must be UTF-8")?;
        if ![
            "--model",
            "--prompt-file",
            "--max-tokens",
            "--context-size",
            "--repeat",
            "--mode",
            "--stop",
        ]
        .contains(&flag.as_str())
        {
            return Err("unknown option".into());
        }
        let value = arguments.next().ok_or("option requires a value")?;
        if values.insert(flag, value).is_some() {
            return Err("duplicate option".into());
        }
    }
    let model = values
        .remove("--model")
        .ok_or("--model is required")?
        .into();
    let prompt_file = values
        .remove("--prompt-file")
        .ok_or("--prompt-file is required")?;
    let mut prompt = String::new();
    File::open(prompt_file)
        .map_err(|_| "cannot open prompt fixture")?
        .take(runtime_types::MAX_MESSAGE_BYTES as u64 + 1)
        .read_to_string(&mut prompt)
        .map_err(|_| "cannot read UTF-8 prompt fixture")?;
    if prompt.is_empty() || prompt.len() > runtime_types::MAX_MESSAGE_BYTES {
        return Err("fixture is empty or exceeds input bound".into());
    }
    let max_tokens = number(&mut values, "--max-tokens", 64)?;
    let context_size = number(&mut values, "--context-size", 2048)?;
    let repeat = number(&mut values, "--repeat", 1)?;
    if !(1..=100).contains(&repeat) {
        return Err("repeat must be in 1..=100".into());
    }
    let mode = values
        .remove("--mode")
        .unwrap_or_else(|| "generate".into())
        .into_string()
        .map_err(|_| "mode must be UTF-8")?;
    if ![
        "generate",
        "budget",
        "cancel-before-load",
        "cancel-before-prepare",
        "cancel-before-generate",
        "cancel-during",
        "consumer-stop",
    ]
    .contains(&mode.as_str())
    {
        return Err("unsupported smoke mode".into());
    }
    let stops = values
        .remove("--stop")
        .map(|value| value.into_string().map_err(|_| "stop must be UTF-8"))
        .transpose()?
        .into_iter()
        .collect();
    Ok(Options {
        model,
        prompt,
        max_tokens,
        context_size,
        repeat,
        mode,
        stops,
    })
}
fn number(
    values: &mut HashMap<String, std::ffi::OsString>,
    key: &str,
    default: u32,
) -> Result<u32> {
    match values.remove(key) {
        Some(value) => Ok(value
            .to_str()
            .ok_or("numeric option must be UTF-8")?
            .parse()?),
        None => Ok(default),
    }
}
fn require(condition: bool, message: &'static str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}
fn observe(iteration: u32, mode: &str, usage: Usage, finish: &str, text: &str, callbacks: u64) {
    println!(
        "{}",
        json!({"iteration":iteration,"mode":mode,"prompt_tokens":usage.prompt_tokens,
        "completion_tokens":usage.completion_tokens,"finish_reason":finish,"text_bytes":text.len(),
        "callbacks":callbacks,"thinking_markup_detected":text.contains("<think>") || text.contains("</think>"),"text_sha256":format!("{:x}",Sha256::digest(text.as_bytes()))})
    );
}

fn run(options: Options) -> Result<()> {
    let info: Value = serde_json::from_str(&llama_adapter::build_info()?)?;
    require(
        info["shim_version"] == 1 && info["backend"] == "cpu",
        "unexpected native ABI/backend",
    )?;
    let load = LoadOptions {
        context_size: options.context_size,
        batch_size: 512.min(options.context_size),
        threads: 4,
    };
    load.validate()?;
    let sampling = GenerationOptions {
        max_tokens: if options.mode == "budget" {
            1
        } else {
            options.max_tokens
        },
        temperature: 0.0,
        top_p: 0.9,
        seed: 42,
        stops: options.stops,
    };
    sampling.validate()?;
    let messages = [Message::new(Role::User, options.prompt)];
    let mut template_sha256: Option<String> = None;
    // One engine survives all repeats. Each loop loads and drops a fresh model,
    // catching resource release bugs rather than restarting the whole process.
    let mut engine = Engine::new()?;
    for iteration in 1..=options.repeat {
        let load_cancel = CancelHandle::new()?;
        if options.mode == "cancel-before-load" {
            load_cancel.cancel();
            let error = match engine.load(&options.model, load, &load_cancel) {
                Err(error) => error,
                Ok(_) => return Err("pre-cancelled load unexpectedly succeeded".into()),
            };
            require(
                error.code == ErrorCode::RequestCancelled,
                "pre-cancelled load had wrong error",
            )?;
            observe(
                iteration,
                &options.mode,
                Usage::default(),
                "cancelled",
                "",
                0,
            );
            continue;
        }
        let mut model = engine.load(&options.model, load, &load_cancel)?;
        let hash = format!("{:x}", Sha256::digest(model.chat_template()?.as_bytes()));
        if let Some(previous) = &template_sha256 {
            require(previous == &hash, "template changed across reloads")?;
        }
        template_sha256 = Some(hash);
        // This handle is independent of the completed load operation.
        let cancel = CancelHandle::new()?;
        if options.mode == "cancel-before-prepare" {
            cancel.cancel();
            let error = match model.prepare(&messages, &sampling, &cancel) {
                Err(error) => error,
                Ok(_) => return Err("pre-cancelled prepare unexpectedly succeeded".into()),
            };
            require(
                error.code == ErrorCode::RequestCancelled,
                "pre-cancelled prepare had wrong error",
            )?;
            observe(
                iteration,
                &options.mode,
                Usage::default(),
                "cancelled",
                "",
                0,
            );
            continue;
        }
        // Explicitly exercise abandoning a prepared request before execution.
        let unused = model.prepare(&messages, &sampling, &cancel)?;
        let expected_prompt_tokens = unused.prompt_tokens();
        drop(unused);
        let prepared = model.prepare(&messages, &sampling, &cancel)?;
        require(
            prepared.prompt_tokens() == expected_prompt_tokens && expected_prompt_tokens > 0,
            "preparation token count is empty or unstable",
        )?;
        if options.mode == "cancel-before-generate" {
            cancel.cancel();
        }
        let (trigger, receive) = mpsc::sync_channel(1);
        let controller = if options.mode == "cancel-during" {
            let cancel = cancel.clone();
            Some(std::thread::spawn(move || {
                if receive.recv().is_ok() {
                    cancel.cancel();
                }
            }))
        } else {
            None
        };
        let mut text = String::new();
        let mut callbacks = 0;
        let result = prepared.generate(&cancel, |piece| {
            callbacks += 1;
            text.push_str(piece);
            if callbacks == 1 && options.mode == "cancel-during" {
                let _ = trigger.try_send(());
            }
            if options.mode == "consumer-stop" {
                StreamControl::Stop
            } else {
                StreamControl::Continue
            }
        });
        drop(trigger); // Also releases a controller if there was no text callback.
        if let Some(controller) = controller {
            controller
                .join()
                .map_err(|_| "cancel controller panicked")?;
        }
        let (usage, finish) = match result {
            Ok(result) => {
                require(
                    options.mode == "generate" || options.mode == "budget",
                    "expected cancellation but generation completed",
                )?;
                require(
                    (!text.is_empty() && callbacks > 0)
                        || (!sampling.stops.is_empty()
                            && result.finish_reason == FinishReason::Stop),
                    "generation emitted no text without a configured stop",
                )?;
                if options.mode == "budget" {
                    require(
                        result.finish_reason == FinishReason::Length
                            && result.usage.completion_tokens == 1,
                        "one-token budget did not terminate by length",
                    )?;
                }
                (result.usage, result.finish_reason.as_str())
            }
            Err(error) => {
                let expected = if options.mode == "consumer-stop" {
                    ErrorCode::ConsumerStopped
                } else {
                    ErrorCode::RequestCancelled
                };
                require(
                    options.mode.starts_with("cancel-") || options.mode == "consumer-stop",
                    "generation failed unexpectedly",
                )?;
                require(
                    error.error.code == expected,
                    "generation failed with unexpected error category",
                )?;
                if options.mode == "consumer-stop" {
                    require(
                        callbacks == 1,
                        "consumer stop emitted more than one callback",
                    )?;
                }
                if options.mode == "cancel-during" {
                    require(
                        callbacks > 0,
                        "in-flight cancellation emitted no initial text",
                    )?;
                }
                if options.mode == "cancel-before-generate" {
                    require(
                        callbacks == 0 && error.usage.completion_tokens == 0,
                        "pre-cancelled generation performed inference",
                    )?;
                }
                (
                    error.usage,
                    if expected == ErrorCode::ConsumerStopped {
                        "consumer_stopped"
                    } else {
                        "cancelled"
                    },
                )
            }
        };
        require(
            usage.prompt_tokens == expected_prompt_tokens,
            "usage prompt count differs from prepared tokenizer count",
        )?;
        require(
            usage.completion_tokens <= sampling.max_tokens
                && usage.total_tokens() <= u64::from(load.context_size),
            "usage exceeded token budget",
        )?;
        require(
            sampling.stops.iter().all(|stop| !text.contains(stop)),
            "stop sequence leaked into text",
        )?;
        require(
            !text.contains("<think>") && !text.contains("</think>"),
            "non-thinking fixture emitted thinking markup",
        )?;
        observe(iteration, &options.mode, usage, finish, &text, callbacks);
    }
    drop(engine);
    // A new engine must succeed after complete teardown in this same process.
    drop(Engine::new()?);
    println!(
        "{}",
        json!({"result":"pass","runs":options.repeat,"build_info":info,
        "template_sha256":template_sha256,
        "load_options":{"context_size":load.context_size,"threads":load.threads,"batch_size":load.batch_size},
        "generation_options":{"max_tokens":sampling.max_tokens,"temperature":sampling.temperature,
            "top_p":sampling.top_p,"seed":sampling.seed,"stop_count":sampling.stops.len()}})
    );
    Ok(())
}
fn main() -> ExitCode {
    if std::env::args_os().any(|argument| argument == "--help" || argument == "-h") {
        println!("{HELP}");
        return ExitCode::SUCCESS;
    }
    match parse().and_then(run) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // Native errors are category-only so a loader cannot leak local paths.
            if let Some(native) = error.downcast_ref::<runtime_types::RuntimeError>() {
                eprintln!("native-smoke failed: {}", native.code.as_str());
            } else {
                eprintln!("native-smoke failed: {error}");
            }
            ExitCode::FAILURE
        }
    }
}
