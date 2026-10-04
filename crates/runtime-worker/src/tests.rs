use super::*;
use runtime_types::{
    ErrorCode, FinishReason, GenerationOptions, GenerationRequest, Message as ChatMessage, Role,
    Usage,
};

fn handshaken() -> (Arc<Shared>, SessionId) {
    let shared = Shared::new();
    let session = SessionId::new_v4();
    assert!(matches!(
        accept(Frame::hello(session, Hello::expected()), &shared).unwrap(),
        Action::None
    ));
    let response = shared.state.lock().unwrap().output.pop_front().unwrap();
    let frame = read_frame(&mut response.as_slice(), MAX_EVENT_FRAME_BYTES)
        .unwrap()
        .unwrap();
    assert!(matches!(frame.message, Message::Hello(hello) if hello == Hello::expected()));
    (shared, session)
}
fn generation(shared: &Shared, session: SessionId, operation: u64) -> Scope {
    let model = ModelId::new("qwen").unwrap();
    shared.state.lock().unwrap().model = Some(model.clone());
    let request = GenerationRequest {
        request_id: RequestId::new(),
        model,
        messages: vec![ChatMessage::new(Role::User, "测试")],
        options: GenerationOptions::default(),
    };
    let scope = Scope {
        operation,
        request: Some(request.request_id),
    };
    assert!(matches!(
        accept(
            Frame::command(
                session,
                operation,
                scope.request,
                Message::Generate { request }
            ),
            shared
        )
        .unwrap(),
        Action::Start { .. }
    ));
    scope
}
fn command(session: SessionId, scope: Scope, message: Message) -> Frame {
    Frame::command(session, scope.operation, scope.request, message)
}
fn sink(shared: &Arc<Shared>, scope: Scope) -> WireSink {
    WireSink {
        shared: shared.clone(),
        scope,
    }
}
fn complete() -> ExecutorEvent {
    ExecutorEvent::Completed {
        usage: Usage {
            prompt_tokens: 1,
            completion_tokens: 1,
        },
        finish_reason: FinishReason::Length,
    }
}
#[test]
fn real_build_identity_and_handshake_reject_wrong_envelopes() {
    let shared = Shared::new();
    let session = SessionId::new_v4();
    assert!(
        accept(
            Frame::command(session, 1, None, Message::Unload {}),
            &shared
        )
        .is_err()
    );
    let mut hello = Frame::hello(session, Hello::expected());
    hello.operation_id = 1;
    assert!(accept(hello, &shared).is_err());
    let mut wrong = Hello::expected();
    wrong.shim_version += 1;
    assert!(accept(Frame::hello(session, wrong), &shared).is_err());
    assert!(accept(Frame::hello(SessionId::nil(), Hello::expected()), &shared).is_err());
    let (shared, session) = handshaken();
    assert!(accept(Frame::hello(session, Hello::expected()), &shared).is_err());
}
#[test]
fn command_state_rejects_duplicate_active_wrong_request_session_and_direction() {
    let (shared, session) = handshaken();
    let scope = generation(&shared, session, 1);
    assert!(accept(command(session, scope, Message::Unload {}), &shared).is_err());
    assert!(
        accept(
            Frame::command(session, 2, None, Message::Unload {}),
            &shared
        )
        .is_err()
    );
    assert!(
        accept(
            command(SessionId::new_v4(), scope, Message::Cancel {}),
            &shared
        )
        .is_err()
    );
    let wrong = Scope {
        operation: scope.operation,
        request: Some(RequestId::new()),
    };
    assert!(
        accept(
            command(session, wrong, Message::Credit { credit_id: 1 }),
            &shared
        )
        .is_err()
    );
    assert!(
        accept(
            command(
                session,
                scope,
                Message::Event {
                    event: complete(),
                    credit_id: None
                }
            ),
            &shared
        )
        .is_err()
    );
    let mut sequenced = command(session, scope, Message::Cancel {});
    sequenced.seq = Some(1);
    assert!(accept(sequenced, &shared).is_err());
}
#[test]
fn credits_are_one_use_bounded_and_never_refilled_by_worker() {
    let (shared, session) = handshaken();
    let scope = generation(&shared, session, 1);
    let sink = sink(&shared, scope);
    assert!(sink.emit(ExecutorEvent::Prepared { prompt_tokens: 5 }));
    for id in 1..=2 {
        assert!(
            accept(
                command(session, scope, Message::Credit { credit_id: id }),
                &shared
            )
            .is_ok()
        );
    }
    assert!(
        accept(
            command(session, scope, Message::Credit { credit_id: 3 }),
            &shared
        )
        .is_err()
    );
    assert!(sink.emit(ExecutorEvent::TextDelta("你好".into())));
    assert!(
        accept(
            command(session, scope, Message::Credit { credit_id: 1 }),
            &shared
        )
        .is_err()
    );
    assert!(sink.emit(ExecutorEvent::TextDelta("world".into())));
    let state = shared.state.lock().unwrap();
    assert!(state.active.as_ref().unwrap().credits.is_empty());
    let frames: Vec<_> = state
        .output
        .iter()
        .map(|bytes| {
            read_frame(&mut bytes.as_slice(), MAX_EVENT_FRAME_BYTES)
                .unwrap()
                .unwrap()
        })
        .collect();
    assert_eq!(
        frames
            .iter()
            .map(|frame| frame.seq.unwrap())
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert!(matches!(
        frames[1].message,
        Message::Event {
            credit_id: Some(1),
            ..
        }
    ));
    assert!(matches!(
        frames[2].message,
        Message::Event {
            credit_id: Some(2),
            ..
        }
    ));
}
#[test]
fn cancel_calls_native_handle_and_interrupts_no_credit_wait() {
    let (shared, session) = handshaken();
    let scope = generation(&shared, session, 1);
    let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = cancelled.clone();
    shared.state.lock().unwrap().active.as_mut().unwrap().cancel =
        Some(CancellationHandle::new(move || {
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        }));
    assert!(sink(&shared, scope).emit(ExecutorEvent::Prepared { prompt_tokens: 1 }));
    let blocked = sink(&shared, scope);
    let (done, receiver) = mpsc::channel();
    thread::spawn(move || {
        done.send(blocked.emit(ExecutorEvent::TextDelta("blocked".into())))
            .unwrap();
    });
    assert!(receiver.recv_timeout(Duration::from_millis(30)).is_err());
    assert!(accept(command(session, scope, Message::Cancel {}), &shared).is_ok());
    assert!(!receiver.recv_timeout(Duration::from_secs(1)).unwrap());
    assert!(cancelled.load(std::sync::atomic::Ordering::SeqCst));
    assert!(accept(command(session, scope, Message::Cancel {}), &shared).is_ok());
}
#[test]
fn cancel_interrupts_full_output_queue_without_writer_progress() {
    let (shared, session) = handshaken();
    let scope = generation(&shared, session, 1);
    {
        let mut state = shared.state.lock().unwrap();
        state.output = (0..OUTPUT_QUEUE_CAPACITY).map(|_| vec![b'\n']).collect();
    }
    let blocked = sink(&shared, scope);
    let (done, receiver) = mpsc::channel();
    thread::spawn(move || {
        done.send(blocked.emit(ExecutorEvent::Prepared { prompt_tokens: 1 }))
            .unwrap();
    });
    assert!(receiver.recv_timeout(Duration::from_millis(30)).is_err());
    assert!(accept(command(session, scope, Message::Cancel {}), &shared).is_ok());
    assert!(!receiver.recv_timeout(Duration::from_secs(1)).unwrap());
    shared.stop(false);
    assert!(!sink(&shared, scope).emit(ExecutorEvent::GenerationFailed {
        error: RuntimeError::new(ErrorCode::RequestCancelled, "cancelled"),
        usage: Usage::default(),
    }));
    assert!(shared.await_inactive(Instant::now() + Duration::from_secs(1)));
}
#[test]
fn terminal_retires_credits_and_late_credit_never_funds_next_request() {
    let (shared, session) = handshaken();
    let scope = generation(&shared, session, 1);
    assert!(
        accept(
            command(session, scope, Message::Credit { credit_id: 1 }),
            &shared
        )
        .is_ok()
    );
    assert!(sink(&shared, scope).emit(complete()));
    assert!(shared.state.lock().unwrap().active.is_none());
    assert!(
        accept(
            command(session, scope, Message::Credit { credit_id: 2 }),
            &shared
        )
        .is_ok()
    );
    let next = generation(&shared, session, 2);
    assert!(
        accept(
            command(session, scope, Message::Credit { credit_id: 3 }),
            &shared
        )
        .is_ok()
    );
    assert!(
        shared
            .state
            .lock()
            .unwrap()
            .active
            .as_ref()
            .unwrap()
            .credits
            .is_empty()
    );
    assert!(
        accept(
            command(session, next, Message::Credit { credit_id: 4 }),
            &shared
        )
        .is_ok()
    );
    assert!(sink(&shared, next).emit(complete()));
    assert!(
        accept(
            command(session, scope, Message::Credit { credit_id: 5 }),
            &shared
        )
        .is_err()
    );
}
#[test]
fn shutdown_and_eof_cleanup_release_engine_on_its_own_thread() {
    let shared = Shared::new();
    let session = SessionId::new_v4();
    let hello = encode_frame(
        &Frame::hello(session, Hello::expected()),
        MAX_REQUEST_FRAME_BYTES,
    )
    .unwrap();
    assert!(control(hello.as_slice(), &shared).is_ok());
    assert!(shared.state.lock().unwrap().writer_closed);
    let shared = Shared::new();
    let mut input = hello;
    input.extend(
        encode_frame(
            &Frame::command(session, 0, None, Message::Shutdown {}),
            MAX_REQUEST_FRAME_BYTES,
        )
        .unwrap(),
    );
    assert!(control(input.as_slice(), &shared).is_ok());
}

#[test]
fn blocked_writer_does_not_block_control_shutdown_or_native_cleanup() {
    struct BlockedWriter {
        started: Option<mpsc::Sender<()>>,
        release: mpsc::Receiver<()>,
    }
    impl Write for BlockedWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if let Some(started) = self.started.take() {
                started.send(()).unwrap();
                self.release.recv().unwrap();
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let (shared, session) = handshaken();
    shared.state.lock().unwrap().output.push_back(vec![b'\n']);
    let (started, on_start) = mpsc::channel();
    let (release, on_release) = mpsc::channel();
    let writer_shared = shared.clone();
    let writer_thread = thread::spawn(move || {
        writer(
            BlockedWriter {
                started: Some(started),
                release: on_release,
            },
            &writer_shared,
        )
    });
    on_start.recv_timeout(Duration::from_secs(1)).unwrap();
    let shutdown = encode_frame(
        &Frame::command(session, 0, None, Message::Shutdown {}),
        MAX_REQUEST_FRAME_BYTES,
    )
    .unwrap();
    let before = Instant::now();
    assert!(control(shutdown.as_slice(), &shared).is_ok());
    assert!(before.elapsed() < Duration::from_secs(1));
    assert!(
        !writer_thread.is_finished(),
        "test must retain a blocked writer across cleanup"
    );
    release.send(()).unwrap();
    assert!(writer_thread.join().unwrap().is_ok());
}

#[test]
fn cancellation_racing_prepare_preserves_prompt_usage_protocol() {
    let (shared, session) = handshaken();
    let scope = generation(&shared, session, 1);
    assert!(accept(command(session, scope, Message::Cancel {}), &shared).is_ok());
    // Native prepare may have just succeeded before the cancel flag changed.
    // Keep its numeric metadata so a following failure's prompt usage agrees.
    assert!(sink(&shared, scope).emit(ExecutorEvent::Prepared { prompt_tokens: 42 }));
    assert!(!sink(&shared, scope).emit(ExecutorEvent::TextDelta("must not leak".into())));
    assert!(sink(&shared, scope).emit(ExecutorEvent::GenerationFailed {
        error: RuntimeError::new(ErrorCode::RequestCancelled, "cancelled"),
        usage: Usage {
            prompt_tokens: 42,
            completion_tokens: 0
        },
    }));
    let state = shared.state.lock().unwrap();
    assert_eq!(state.output.len(), 2);
    assert!(state.active.is_none());
}

#[test]
fn shutdown_racing_queued_credit_and_cancel_never_reuses_them() {
    for controls_before_shutdown in [false, true] {
        let (shared, session) = handshaken();
        let scope = generation(&shared, session, 1);
        assert!(sink(&shared, scope).emit(ExecutorEvent::Prepared { prompt_tokens: 1 }));
        let credit = encode_frame(
            &command(session, scope, Message::Credit { credit_id: 1 }),
            MAX_REQUEST_FRAME_BYTES,
        )
        .unwrap();
        let cancel = encode_frame(
            &command(session, scope, Message::Cancel {}),
            MAX_REQUEST_FRAME_BYTES,
        )
        .unwrap();
        let shutdown = encode_frame(
            &Frame::command(session, 0, None, Message::Shutdown {}),
            MAX_REQUEST_FRAME_BYTES,
        )
        .unwrap();
        let frames = if controls_before_shutdown {
            vec![credit, cancel, shutdown]
        } else {
            vec![shutdown, credit, cancel]
        };
        let input: Vec<u8> = frames.into_iter().flatten().collect();
        let completion_shared = shared.clone();
        let completion = thread::spawn(move || {
            let mut state = completion_shared.state.lock().unwrap();
            while !state.stopping {
                state = completion_shared.changed.wait(state).unwrap();
            }
            drop(state);
            assert!(
                !sink(&completion_shared, scope).emit(ExecutorEvent::GenerationFailed {
                    error: RuntimeError::new(ErrorCode::RequestCancelled, "cancelled"),
                    usage: Usage {
                        prompt_tokens: 1,
                        completion_tokens: 0
                    },
                })
            );
        });
        let started = Instant::now();
        assert!(control(input.as_slice(), &shared).is_ok());
        assert!(started.elapsed() < Duration::from_secs(1));
        completion.join().unwrap();
        let state = shared.state.lock().unwrap();
        assert!(state.active.is_none() && state.stopping && state.writer_closed);
        assert_eq!(state.last_credit, u64::from(controls_before_shutdown));
    }
}
