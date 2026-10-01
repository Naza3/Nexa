use runtime_core::ExecutorEvent;
use runtime_ipc::*;
use runtime_types::{
    FinishReason, GenerationOptions, GenerationRequest, Message as ChatMessage, ModelId, RequestId,
    Role, Usage,
};
use std::io::Cursor;
fn request() -> GenerationRequest {
    GenerationRequest {
        request_id: RequestId::new(),
        model: ModelId::new("qa-small").unwrap(),
        messages: vec![ChatMessage::new(Role::User, "你好\n🙂")],
        options: GenerationOptions::default(),
    }
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
        raw.replacen("\"protocol_version\":1", "\"protocol_version\":2", 1),
        raw.replace(LLAMA_COMMIT, "wrong"),
        raw.replace("\"shim_version\":2", "\"shim_version\":1"),
    ] {
        assert!(read_frame(&mut Cursor::new(invalid), 4096).is_err());
    }
}
#[test]
fn escaped_input_must_fit_actual_encoded_request_and_text_has_tighter_bound() {
    let mut request = request();
    request.messages[0].content = "\0".repeat(1024 * 1024);
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
            "\"protocol_version\":1",
            "\"protocol_version\":1,\"protocol_version\":1",
            1,
        ),
        hello.replacen(
            "\"shim_version\":2",
            "\"shim_version\":2,\"shim_version\":2",
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
