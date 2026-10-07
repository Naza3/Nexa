//! Opt-in real CPU image inference. No image weights or user data enter the tree.
use llama_adapter::{CancelHandle, Engine, GenerationPhase, StreamControl};
use runtime_types::{ErrorCode, GenerationOptions, ImageInput, LoadOptions, Message, Role};
use std::{sync::mpsc, time::Duration};

#[test]
#[ignore = "requires AIR_OCR_MODEL, AIR_OCR_PROJECTOR and AIR_OCR_IMAGE_DATA_URL_FILE"]
fn real_image_budget_cancel_decode_and_recovery() {
    let model_path = std::env::var("AIR_OCR_MODEL").unwrap();
    let projector = std::env::var("AIR_OCR_PROJECTOR").unwrap();
    let image_path = std::env::var("AIR_OCR_IMAGE_DATA_URL_FILE").unwrap();
    let image =
        ImageInput::from_data_url(std::fs::read_to_string(image_path).unwrap().trim()).unwrap();
    let mut message = Message::new(Role::User, "Text Recognition:");
    message.image = Some(image);
    let messages = [message];
    let mut engine = Engine::new().unwrap();
    let load_cancel = CancelHandle::new().unwrap();
    let mut model = engine
        .load_with_projector(
            model_path,
            projector,
            LoadOptions {
                context_size: 4096,
                threads: 4,
                batch_size: 512,
            },
            &load_cancel,
        )
        .unwrap();
    let options = GenerationOptions {
        max_tokens: 256,
        temperature: 0.0,
        ..Default::default()
    };
    let cancel = CancelHandle::new().unwrap();
    // Ordinary text remains an explicit failure on this dedicated OCR template.
    assert_eq!(
        model
            .prepare(&[Message::new(Role::User, "hello")], &options, &cancel)
            .err()
            .unwrap()
            .code,
        ErrorCode::UnsupportedChatTemplate
    );
    // Valid PNG dimensions alone are insufficient: native must reject a
    // truncated image body, then leave the resident model usable.
    if messages[0].image.as_ref().unwrap().mime_type == "image/png" {
        let mut damaged = messages.clone();
        damaged[0].image.as_mut().unwrap().data.truncate(64);
        assert!(damaged[0].image.as_ref().unwrap().decode().is_ok());
        assert!(model.prepare(&damaged, &options, &cancel).is_err());
    }
    let prompt_tokens = model
        .prepare(&messages, &options, &cancel)
        .unwrap()
        .prompt_tokens();
    assert!(prompt_tokens > 32, "must account for real image tokens");
    let oversized = GenerationOptions {
        max_tokens: 4096,
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
    // Cancellation originates on the independent control thread during visual prefill.
    let prepared = model.prepare(&messages, &options, &cancel).unwrap();
    let (send, receive) = mpsc::sync_channel(1);
    let controller_cancel = cancel.clone();
    let controller = std::thread::spawn(move || {
        if receive.recv().is_ok() {
            std::thread::sleep(Duration::from_millis(30));
            controller_cancel.cancel();
        }
    });
    let mut triggered = false;
    let result = prepared.generate_observed(
        &cancel,
        |_| StreamControl::Continue,
        |event| {
            if !triggered && event.phase == GenerationPhase::PrefillBatchCompleted {
                triggered = true;
                let _ = send.try_send(());
            }
            StreamControl::Continue
        },
    );
    drop(send);
    controller.join().unwrap();
    assert_eq!(result.unwrap_err().error.code, ErrorCode::RequestCancelled);
    // The same model must recover with fresh flags and cleared KV/image state.
    let recovery = CancelHandle::new().unwrap();
    let mut text = String::new();
    let result = model
        .prepare(&messages, &options, &recovery)
        .unwrap()
        .generate(&recovery, |piece| {
            text.push_str(piece);
            StreamControl::Continue
        })
        .unwrap();
    assert_eq!(result.usage.prompt_tokens, prompt_tokens);
    assert!(!text.trim().is_empty());
    if let Ok(anchor) = std::env::var("AIR_OCR_EXPECTED_TEXT") {
        assert!(text.contains(&anchor));
    }
    let mut after = messages.clone();
    after[0].image.as_mut().unwrap().after_text = true;
    assert!(
        model
            .prepare(&after, &options, &recovery)
            .unwrap()
            .prompt_tokens()
            > 32
    );
}
