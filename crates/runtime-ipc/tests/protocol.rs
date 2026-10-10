use runtime_core::ExecutorEvent;
use runtime_ipc::*;
use runtime_types::{
    FinishReason, GenerationOptions, GenerationRequest, Message as ChatMessage, ModelId, RequestId,
    Role, Usage,
};
use std::io::Cursor;
fn request() -> GenerationRequest {
    GenerationRequest {
        tools: runtime_types::ToolConfig::default(),
        request_id: RequestId::new(),
        model: ModelId::new("qa-small").unwrap(),
        messages: vec![ChatMessage::new(Role::User, "你好\n🙂")],
        options: GenerationOptions::default(),
    }
}
#[test]
fn image_request_roundtrip_preserves_order_and_remains_bounded() {
    let mut req = request();
    let mut image = runtime_types::ImageInput::from_data_url("data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/lX8AAAAASUVORK5CYII=").unwrap();
    image.after_text = true;
    req.messages[0].image = Some(image.clone());
    let mut frame = Frame::command(
        SessionId::new_v4(),
        1,
        Some(req.request_id),
        Message::Generate { request: req },
    );
    let encoded = encode_frame(&frame, MAX_REQUEST_FRAME_BYTES).unwrap();
    let decoded = read_frame(&mut Cursor::new(encoded), MAX_REQUEST_FRAME_BYTES)
        .unwrap()
        .unwrap();
    let Message::Generate { request } = decoded.message else {
        panic!()
    };
    assert_eq!(request.messages[0].image, Some(image));
    if let Message::Generate { request } = &mut frame.message {
        request.messages[0].image.as_mut().unwrap().data = "A".repeat(6 * 1024 * 1024);
    }
    assert!(encode_frame(&frame, MAX_REQUEST_FRAME_BYTES).is_err());
}
fn ready() -> (SessionId, RequestId, EventValidator) {
    let session = SessionId::new_v4();
    let mut validator = EventValidator::new(session);
    validator
        .accept_hello(&Frame::hello(session, Hello::expected()))
        .unwrap();
    let request = request();
    let id = request.request_id;
    validator
        .begin(&Frame::command(
            session,
            1,
            Some(id),
            Message::Generate { request },
        ))
        .unwrap();
    (session, id, validator)
}
#[test]
fn codec_utf8_newline_roundtrip_and_exact_limits_include_lf() {
    let frame = Frame::hello(SessionId::new_v4(), Hello::expected());
    let encoded = encode_frame(&frame, MAX_REQUEST_FRAME_BYTES).unwrap();
    assert_eq!(encoded.last(), Some(&b'\n'));
    assert!(encode_frame(&frame, encoded.len() - 1).is_err());
    assert!(encode_frame(&frame, encoded.len()).is_ok());
    assert!(
        read_frame(&mut Cursor::new(&encoded), encoded.len())
            .unwrap()
            .is_some()
    );
    assert!(read_frame(&mut Cursor::new(&encoded), encoded.len() - 1).is_err());
    let frame = Frame::event(
        frame.session_id,
        1,
        Some(RequestId::new()),
        1,
        ExecutorEvent::TextDelta("你\n🙂\0".into()),
        Some(1),
    );
    let encoded = encode_frame(&frame, MAX_EVENT_FRAME_BYTES).unwrap();
    assert_eq!(encoded.iter().filter(|b| **b == b'\n').count(), 1);
    let decoded = read_frame(&mut Cursor::new(encoded), MAX_EVENT_FRAME_BYTES)
        .unwrap()
        .unwrap();
    let Message::Event {
        event: ExecutorEvent::TextDelta(text),
        ..
    } = decoded.message
    else {
        panic!()
    };
    assert_eq!(text, "你\n🙂\0");
}
#[test]
fn malformed_frames_fail_without_unbounded_reads_or_partial_writes() {
    for bytes in [b"\n".as_slice(), b"{}\n", b"\xff\n", b"{", b"null\n", b"{}"] {
        assert!(read_frame(&mut Cursor::new(bytes), MAX_EVENT_FRAME_BYTES).is_err());
    }
    let mut too_large = Cursor::new(vec![b'x'; 128]);
    assert!(read_frame(&mut too_large, 64).is_err());
    assert!(too_large.position() <= 64);
    assert!(read_frame(&mut Cursor::new([]), 64).unwrap().is_none());
    let frame = Frame::hello(SessionId::new_v4(), Hello::expected());
    let mut writer = Vec::new();
    assert!(write_frame(&mut writer, &frame, 1).is_err());
    assert!(writer.is_empty());
    let raw = String::from_utf8(encode_frame(&frame, 4096).unwrap()).unwrap();
    for invalid in [
        raw.replace("\"hello\"", "\"unknown\""),
        raw.replacen("\"protocol_version\":5", "\"protocol_version\":1", 1),
        raw.replace(LLAMA_COMMIT, "wrong"),
        raw.replace("\"shim_version\":5", "\"shim_version\":1"),
    ] {
        assert!(read_frame(&mut Cursor::new(invalid), 4096).is_err());
    }
}
#[test]
fn escaped_input_must_fit_actual_encoded_request_and_text_has_tighter_bound() {
    let mut request = request();
    request.messages[0].content = Some("\0".repeat(1024 * 1024));
    let frame = Frame::command(
        SessionId::new_v4(),
        1,
        Some(request.request_id),
        Message::Generate { request },
    );
    assert!(encode_frame(&frame, MAX_REQUEST_FRAME_BYTES).is_err());
    let mut destination = Vec::new();
    assert!(write_frame(&mut destination, &frame, MAX_REQUEST_FRAME_BYTES).is_err());
    assert!(destination.is_empty());
    let frame = Frame::event(
        SessionId::new_v4(),
        u64::MAX,
        Some(RequestId::new()),
        u64::MAX,
        ExecutorEvent::TextDelta("\0".repeat(MAX_TEXT_BYTES)),
        Some(u64::MAX),
    );
    let encoded = encode_frame(&frame, MAX_EVENT_FRAME_BYTES).unwrap();
    assert!(encoded.len() <= MAX_TEXT_FRAME_BYTES);
    let mut oversized = frame;
    oversized.message = Message::Event {
        event: ExecutorEvent::TextDelta("x".repeat(MAX_TEXT_BYTES + 1)),
        credit_id: Some(1),
    };
    assert!(encode_frame(&oversized, MAX_EVENT_FRAME_BYTES).is_err());
}
#[test]
fn validator_rejects_prehandshake_wrong_ids_sequence_and_phase() {
    let session = SessionId::new_v4();
    let mut validator = EventValidator::new(session);
    let req = request();
    assert!(
        validator
            .begin(&Frame::command(
                session,
                1,
                Some(req.request_id),
                Message::Generate { request: req }
            ))
            .is_err()
    );
    assert!(
        validator
            .accept_hello(&Frame::hello(SessionId::new_v4(), Hello::expected()))
            .is_err()
    );
    let (session, id, mut validator) = ready();
    assert!(
        validator
            .accept_hello(&Frame::hello(session, Hello::expected()))
            .is_err()
    );
    let valid = Frame::event(
        session,
        1,
        Some(id),
        1,
        ExecutorEvent::Prepared { prompt_tokens: 2 },
        None,
    );
    for invalid in [
        Frame {
            session_id: SessionId::new_v4(),
            ..valid.clone()
        },
        Frame {
            operation_id: 2,
            ..valid.clone()
        },
        Frame {
            request_id: Some(RequestId::new()),
            ..valid.clone()
        },
        Frame {
            seq: Some(2),
            ..valid.clone()
        },
    ] {
        assert!(validator.accept(&invalid).is_err());
    }
    assert!(
        validator
            .accept(&Frame::event(
                session,
                1,
                Some(id),
                1,
                ExecutorEvent::TextDelta("early".into()),
                Some(1)
            ))
            .is_err()
    );
    validator.accept(&valid).unwrap();
    assert!(validator.accept(&valid).is_err());
    assert!(
        validator
            .accept(&Frame {
                seq: Some(2),
                ..valid
            })
            .is_err()
    );
}
#[test]
fn credits_once_only_max_two_and_terminal_once() {
    let (session, id, mut validator) = ready();
    assert!(validator.grant(0).is_err());
    validator.grant(1).unwrap();
    assert!(validator.grant(1).is_err());
    validator.grant(2).unwrap();
    assert!(validator.grant(3).is_err());
    validator
        .accept(&Frame::event(
            session,
            1,
            Some(id),
            1,
            ExecutorEvent::Prepared { prompt_tokens: 2 },
            None,
        ))
        .unwrap();
    let event = Frame::event(
        session,
        1,
        Some(id),
        2,
        ExecutorEvent::TextDelta("text".into()),
        Some(1),
    );
    validator.accept(&event).unwrap();
    assert!(
        validator
            .accept(&Frame {
                seq: Some(3),
                ..event
            })
            .is_err()
    );
    validator.grant(3).unwrap();
    let terminal = Frame::event(
        session,
        1,
        Some(id),
        3,
        ExecutorEvent::Completed {
            usage: Usage {
                prompt_tokens: 2,
                completion_tokens: 0,
            },
            finish_reason: FinishReason::Stop,
            timings: None,
        },
        None,
    );
    validator.accept(&terminal).unwrap();
    assert!(
        validator
            .accept(&Frame {
                seq: Some(4),
                ..terminal
            })
            .is_err()
    );
    assert!(validator.grant(4).is_err());
    let mut req = request();
    req.request_id = id; // Reused UUID does not revive the old operation/credits.
    validator
        .begin(&Frame::command(
            session,
            2,
            Some(id),
            Message::Generate { request: req },
        ))
        .unwrap();
    assert!(validator.grant(3).is_err());
    validator.grant(4).unwrap();
    assert!(
        validator
            .accept(&Frame::event(
                session,
                1,
                Some(id),
                4,
                ExecutorEvent::Prepared { prompt_tokens: 2 },
                None
            ))
            .is_err()
    );
    validator
        .accept(&Frame::event(
            session,
            2,
            Some(id),
            4,
            ExecutorEvent::Prepared { prompt_tokens: 2 },
            None,
        ))
        .unwrap();
    assert!(
        validator
            .accept(&Frame::event(
                session,
                2,
                Some(id),
                5,
                ExecutorEvent::TextDelta("old credit".into()),
                Some(2)
            ))
            .is_err()
    );
}
#[test]
fn unknown_fields_and_duplicate_json_keys_are_rejected_at_each_layer() {
    let hello = String::from_utf8(
        encode_frame(&Frame::hello(SessionId::new_v4(), Hello::expected()), 4096).unwrap(),
    )
    .unwrap();
    let req = request();
    let command = String::from_utf8(
        encode_frame(
            &Frame::command(
                SessionId::new_v4(),
                1,
                Some(req.request_id),
                Message::Generate { request: req },
            ),
            4096,
        )
        .unwrap(),
    )
    .unwrap();
    for invalid in [
        hello.replacen('{', "{\"unknown\":true,", 1),
        hello.replacen("\"payload\":{", "\"payload\":{\"unknown\":true,", 1),
        hello.replacen(
            "\"protocol_version\":5",
            "\"protocol_version\":5,\"protocol_version\":5",
            1,
        ),
        hello.replacen(
            "\"shim_version\":5",
            "\"shim_version\":5,\"shim_version\":5",
            1,
        ),
        hello.replacen(
            "\"kind\":\"hello\"",
            "\"kind\":\"hello\",\"kind\":\"hello\"",
            1,
        ),
        command.replacen("\"payload\":{", "\"payload\":{\"unknown\":true,", 1),
        command.replacen("\"request\":{", "\"request\":{\"unknown\":true,", 1),
        command.replacen("\"role\":\"user\"", "\"role\":\"user\",\"unknown\":true", 1),
        command.replacen(
            "\"max_tokens\":512",
            "\"max_tokens\":512,\"max_tokens\":1",
            1,
        ),
    ] {
        assert!(
            read_frame(&mut Cursor::new(&invalid), MAX_REQUEST_FRAME_BYTES).is_err(),
            "accepted malformed frame: {invalid}"
        );
    }
}
#[test]
fn usage_is_bound_to_prepared_prompt_and_requested_output_maximum() {
    let (session, id, mut validator) = ready();
    let failure = |seq, prompt_tokens, completion_tokens| {
        Frame::event(
            session,
            1,
            Some(id),
            seq,
            ExecutorEvent::GenerationFailed {
                error: runtime_types::RuntimeError::invalid("synthetic"),
                usage: Usage {
                    prompt_tokens,
                    completion_tokens,
                },
            },
            None,
        )
    };
    assert!(validator.accept(&failure(1, 1, 0)).is_err());
    assert!(validator.accept(&failure(1, 0, 1)).is_err());
    validator
        .accept(&Frame::event(
            session,
            1,
            Some(id),
            1,
            ExecutorEvent::Prepared { prompt_tokens: 7 },
            None,
        ))
        .unwrap();
    assert!(validator.accept(&failure(2, 8, 0)).is_err());
    assert!(validator.accept(&failure(2, 7, 513)).is_err());
    validator.accept(&failure(2, 7, 512)).unwrap();
    assert!(validator.operation_complete());
    let (session, id, mut validator) = ready();
    validator
        .accept(&Frame::event(
            session,
            1,
            Some(id),
            1,
            ExecutorEvent::GenerationFailed {
                error: runtime_types::RuntimeError::invalid("before preparation"),
                usage: Usage::default(),
            },
            None,
        ))
        .unwrap();
}

#[test]
fn cleanup_unconfirmed_is_parent_only_and_cannot_be_forged_on_worker_wire() {
    let (session, id, mut validator) = ready();
    let error = runtime_types::RuntimeError::new(
        runtime_types::ErrorCode::ExecutorCleanupUnconfirmed,
        "unconfirmed",
    );
    let frame = Frame::event(
        session,
        1,
        Some(id),
        1,
        ExecutorEvent::CleanupUnconfirmed(error.clone()),
        None,
    );
    assert!(encode_frame(&frame, MAX_EVENT_FRAME_BYTES).is_err());
    assert!(validator.accept(&frame).is_err());
    let frame = Frame::event(session, 1, Some(id), 1, ExecutorEvent::Failed(error), None);
    assert!(encode_frame(&frame, MAX_EVENT_FRAME_BYTES).is_err());
    assert!(validator.accept(&frame).is_err());
    let valid_error = runtime_types::RuntimeError::invalid("synthetic");
    let valid = Frame::event(
        session,
        1,
        Some(id),
        1,
        ExecutorEvent::Failed(valid_error),
        None,
    );
    let raw = String::from_utf8(encode_frame(&valid, MAX_EVENT_FRAME_BYTES).unwrap()).unwrap();
    for forged in [
        raw.replace("\"failed\"", "\"cleanup_unconfirmed\""),
        raw.replace("\"invalid_argument\"", "\"executor_cleanup_unconfirmed\""),
    ] {
        assert!(read_frame(&mut Cursor::new(forged), MAX_EVENT_FRAME_BYTES).is_err());
    }
}

#[test]
fn load_eligibility_is_required_and_old_validation_claim_is_rejected() {
    let model = runtime_types::ResolvedModel {
        id: ModelId::new("candidate").unwrap(),
        path: "controlled.gguf".into(),
        projector_path: None,
        context_limit: 4096,
        default_context: 2048,
        loadable: true,
    };
    let frame = Frame::command(
        SessionId::new_v4(),
        1,
        None,
        Message::Load {
            model,
            options: runtime_types::LoadOptions::default(),
        },
    );
    let encoded = String::from_utf8(encode_frame(&frame, 4096).unwrap()).unwrap();
    assert!(read_frame(&mut Cursor::new(encoded.as_bytes()), 4096).is_ok());
    for old in [
        encoded.replace("\"loadable\":true", "\"validated\":true"),
        encoded.replace("\"loadable\":true", "\"unexpected\":true"),
    ] {
        assert!(read_frame(&mut Cursor::new(old), 4096).is_err());
    }
}

#[test]
fn completion_timings_are_bounded_and_only_accepted_after_prepared_once() {
    use runtime_types::InferenceTimings;
    let good = InferenceTimings {
        prepare_us: 1,
        prefill_us: 2,
        decode_us: 3,
        output_callback_us: 4,
    };
    for timings in [
        good,
        InferenceTimings {
            prepare_us: u64::MAX,
            ..good
        },
        InferenceTimings {
            prepare_us: 9_007_199_254_740_991,
            ..good
        },
    ] {
        let (session, id, mut validator) = ready();
        let completion = |seq| {
            Frame::event(
                session,
                1,
                Some(id),
                seq,
                ExecutorEvent::Completed {
                    usage: Usage {
                        prompt_tokens: 8,
                        completion_tokens: 1,
                    },
                    finish_reason: FinishReason::Stop,
                    timings: Some(timings),
                },
                None,
            )
        };
        assert!(validator.accept(&completion(1)).is_err());
        validator
            .accept(&Frame::event(
                session,
                1,
                Some(id),
                1,
                ExecutorEvent::Prepared { prompt_tokens: 8 },
                None,
            ))
            .unwrap();
        assert_eq!(validator.accept(&completion(2)).is_ok(), timings == good);
        if timings == good {
            assert!(validator.accept(&completion(3)).is_err());
        }
    }
}

#[test]
fn timing_nested_schema_and_old_private_version_are_rejected() {
    let (session, id, _) = ready();
    let frame = Frame::event(
        session,
        1,
        Some(id),
        2,
        ExecutorEvent::Completed {
            usage: Usage {
                prompt_tokens: 8,
                completion_tokens: 1,
            },
            finish_reason: FinishReason::Stop,
            timings: Some(runtime_types::InferenceTimings {
                prepare_us: 1,
                prefill_us: 2,
                decode_us: 3,
                output_callback_us: 4,
            }),
        },
        None,
    );
    let value = serde_json::to_value(&frame).unwrap();
    for bad in [
        serde_json::json!({"prepare_us":1,"prefill_us":2,"decode_us":3,"output_callback_us":4,"unknown":1}),
        serde_json::json!({"prepare_us":-1,"prefill_us":2,"decode_us":3,"output_callback_us":4}),
        serde_json::json!({"prepare_us":1,"prefill_us":2,"decode_us":3}),
    ] {
        let mut malformed = value.clone();
        malformed["payload"]["event"]["data"]["timings"] = bad;
        assert!(serde_json::from_value::<Frame>(malformed).is_err());
    }
    let mut old = Frame::hello(session, Hello::expected());
    old.protocol_version = 3;
    assert!(EventValidator::new(session).accept_hello(&old).is_err());
    let mut old_hello = Hello::expected();
    old_hello.protocol_version = 3;
    assert!(old_hello.validate().is_err());
}

#[test]
fn duplicate_timing_fields_are_rejected_and_absent_measurement_is_not_zero() {
    let event = ExecutorEvent::Completed {
        usage: Usage {
            prompt_tokens: 8,
            completion_tokens: 1,
        },
        finish_reason: FinishReason::Stop,
        timings: Some(runtime_types::InferenceTimings {
            prepare_us: 1,
            prefill_us: 2,
            decode_us: 3,
            output_callback_us: 4,
        }),
    };
    let json = serde_json::to_string(&event).unwrap();
    let duplicate = json.replace("\"decode_us\":3", "\"decode_us\":3,\"decode_us\":4");
    assert_ne!(duplicate, json);
    assert!(serde_json::from_str::<ExecutorEvent>(&duplicate).is_err());
    let absent = ExecutorEvent::Completed {
        usage: Usage::default(),
        finish_reason: FinishReason::Stop,
        timings: None,
    };
    let json = serde_json::to_string(&absent).unwrap();
    assert!(!json.contains("timings"));
    assert!(matches!(
        serde_json::from_str::<ExecutorEvent>(&json).unwrap(),
        ExecutorEvent::Completed { timings: None, .. }
    ));
}

#[test]
fn tool_events_require_credit_order_and_matching_terminal() {
    use runtime_types::{ToolCallDelta, ToolChoice, ToolConfig, ToolDefinition};
    let session = SessionId::new_v4();
    let mut req = request();
    req.tools = ToolConfig {
        definitions: vec![ToolDefinition {
            name: "lookup".into(),
            description: None,
            parameters: serde_json::json!({"type":"object"}),
        }],
        choice: ToolChoice::Auto,
        parallel_tool_calls: false,
    };
    let id = req.request_id;
    let mut validator = EventValidator::new(session);
    validator
        .accept_hello(&Frame::hello(session, Hello::expected()))
        .unwrap();
    validator
        .begin(&Frame::command(
            session,
            1,
            Some(id),
            Message::Generate { request: req },
        ))
        .unwrap();
    validator
        .accept(&Frame::event(
            session,
            1,
            Some(id),
            1,
            ExecutorEvent::Prepared { prompt_tokens: 10 },
            None,
        ))
        .unwrap();
    let start = ExecutorEvent::ToolCallDelta(ToolCallDelta::Start {
        index: 0,
        id: "call_a".into(),
        name: "lookup".into(),
    });
    assert!(
        validator
            .accept(&Frame::event(session, 1, Some(id), 2, start.clone(), None))
            .is_err()
    );
    validator.grant(1).unwrap();
    validator
        .accept(&Frame::event(session, 1, Some(id), 2, start, Some(1)))
        .unwrap();
    validator.grant(2).unwrap();
    validator
        .accept(&Frame::event(
            session,
            1,
            Some(id),
            3,
            ExecutorEvent::ToolCallDelta(ToolCallDelta::Arguments {
                index: 0,
                arguments: "{}".into(),
            }),
            Some(2),
        ))
        .unwrap();
    let completed = |reason| ExecutorEvent::Completed {
        usage: Usage {
            prompt_tokens: 10,
            completion_tokens: 4,
        },
        finish_reason: reason,
        timings: None,
    };
    assert!(
        validator
            .accept(&Frame::event(
                session,
                1,
                Some(id),
                4,
                completed(FinishReason::Stop),
                None
            ))
            .is_err()
    );
    validator
        .accept(&Frame::event(
            session,
            1,
            Some(id),
            4,
            completed(FinishReason::ToolCalls),
            None,
        ))
        .unwrap();
    assert!(validator.operation_complete());
}

#[test]
fn tool_codec_roundtrip_and_piece_credit_bounds_are_enforced() {
    use runtime_types::ToolCallDelta;
    let delta = ToolCallDelta::Start {
        index: 0,
        id: "call_a".into(),
        name: "lookup".into(),
    };
    let mut frame = Frame::event(
        SessionId::new_v4(),
        1,
        Some(RequestId::new()),
        2,
        ExecutorEvent::ToolCallDelta(delta.clone()),
        Some(1),
    );
    let bytes = encode_frame(&frame, MAX_EVENT_FRAME_BYTES).unwrap();
    let decoded = read_frame(&mut Cursor::new(bytes), MAX_EVENT_FRAME_BYTES)
        .unwrap()
        .unwrap();
    assert!(
        matches!(decoded.message, Message::Event { event:ExecutorEvent::ToolCallDelta(found),credit_id:Some(1)} if found==delta)
    );
    for credit_id in [None, Some(0)] {
        frame.message = Message::Event {
            event: ExecutorEvent::ToolCallDelta(delta.clone()),
            credit_id,
        };
        assert!(encode_frame(&frame, MAX_EVENT_FRAME_BYTES).is_err());
    }
    frame.message = Message::Event {
        event: ExecutorEvent::ToolCallDelta(ToolCallDelta::Arguments {
            index: 0,
            arguments: "x".repeat(MAX_TEXT_BYTES + 1),
        }),
        credit_id: Some(2),
    };
    assert!(encode_frame(&frame, MAX_EVENT_FRAME_BYTES).is_err());
    let raw = serde_json::to_vec(&frame).unwrap();
    let mut encoded = raw;
    encoded.push(b'\n');
    assert!(read_frame(&mut Cursor::new(encoded), MAX_EVENT_FRAME_BYTES).is_err());
}
