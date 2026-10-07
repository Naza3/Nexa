//! CPU OCR smoke: model GGUF, projector GGUF, image data-URL file, optional mode.
use llama_adapter::{CancelHandle, Engine, GenerationPhase, StreamControl};
use runtime_types::{ErrorCode, GenerationOptions, ImageInput, LoadOptions, Message, Role};
use std::{error::Error, time::Instant};
fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() < 3 {
        return Err("usage: ocr-smoke MODEL PROJECTOR DATA_URL_FILE [generate|cancel-prefill|budget|after-text]".into());
    }
    let mode = args.get(3).map(String::as_str).unwrap_or("generate");
    let started = Instant::now();
    let cancel = CancelHandle::new()?;
    let mut engine = Engine::new()?;
    let mut model = engine.load_with_projector(
        &args[0],
        &args[1],
        LoadOptions {
            context_size: if mode == "budget" { 256 } else { 4096 },
            threads: 4,
            batch_size: if mode == "budget" { 256 } else { 512 },
        },
        &cancel,
    )?;
    let mut image = ImageInput::from_data_url(std::fs::read_to_string(&args[2])?.trim())?;
    image.after_text = mode == "after-text";
    let mut message = Message::new(Role::User, "Text Recognition:");
    message.image = Some(image);
    let options = GenerationOptions {
        max_tokens: 512,
        temperature: 0.0,
        ..Default::default()
    };
    if mode == "budget" {
        let error = match model.prepare(&[message], &options, &cancel) {
            Ok(_) => return Err("oversized image prompt was accepted".into()),
            Err(error) => error,
        };
        assert_eq!(error.code, ErrorCode::ContextLengthExceeded);
        println!("{{\"mode\":\"budget\",\"rejected\":true}}");
        return Ok(());
    }
    let prepared = model.prepare(&[message], &options, &cancel)?;
    let prompt_tokens = prepared.prompt_tokens();
    let mut text = String::new();
    let result = prepared.generate_observed(
        &cancel,
        |piece| {
            text.push_str(piece);
            StreamControl::Continue
        },
        |event| {
            if mode == "cancel-prefill" && event.phase == GenerationPhase::PrefillStarted {
                cancel.cancel();
            }
            StreamControl::Continue
        },
    );
    if mode == "cancel-prefill" {
        assert_eq!(result.unwrap_err().error.code, ErrorCode::RequestCancelled);
    } else {
        let usage = result?;
        assert_eq!(usage.usage.prompt_tokens, prompt_tokens);
        assert!(!text.is_empty());
    }
    println!(
        "{}",
        serde_json::json!({"mode":mode,"prompt_tokens":prompt_tokens,"elapsed_ms":started.elapsed().as_millis(),"text":text})
    );
    Ok(())
}
