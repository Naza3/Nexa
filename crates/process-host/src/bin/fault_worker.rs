//! Deliberately faulty protocol peer, separate from the production worker.
//! It links no native engine and is used only by process-host contract tests.
use runtime_core::ExecutorEvent;
use runtime_ipc::{
    Frame, Hello, MAX_EVENT_FRAME_BYTES, MAX_REQUEST_FRAME_BYTES, Message, read_frame, write_frame,
};
use runtime_types::{ErrorCode, FinishReason, RuntimeError, Usage};
use std::{
    fs,
    io::{self, BufReader, Write},
    process::{Command, Stdio},
    thread,
    time::Duration,
};
fn sleep_forever() -> ! {
    loop {
        thread::sleep(Duration::from_secs(60));
    }
}
fn emit(
    output: &mut impl Write,
    scope: &Frame,
    seq: &mut u64,
    event: ExecutorEvent,
    credit: Option<u64>,
) {
    *seq += 1;
    write_frame(
        output,
        &Frame::event(
            scope.session_id,
            scope.operation_id,
            scope.request_id,
            *seq,
            event,
            credit,
        ),
        MAX_EVENT_FRAME_BYTES,
    )
    .unwrap();
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|s| s == "descendant") {
        if let Some(path) = args.get(1) {
            fs::write(path, std::process::id().to_string()).unwrap();
        }
        sleep_forever();
    }
    let requested_case = args.first().map(String::as_str).unwrap_or("normal");
    let recovery_marker = args.get(1).map(|p| format!("{p}.failed"));
    let case = if requested_case == "crash_once" {
        if recovery_marker
            .as_ref()
            .is_some_and(|p| std::path::Path::new(p).exists())
        {
            "normal"
        } else {
            "text_then_crash"
        }
    } else {
        requested_case
    };
    if let Some(path) = args.get(1) {
        fs::write(path, std::process::id().to_string()).unwrap();
    }
    let mut input = BufReader::new(io::stdin());
    let mut output = io::stdout();
    let hello = read_frame(&mut input, MAX_REQUEST_FRAME_BYTES)
        .unwrap()
        .unwrap();
    match case {
        "silent_hello" => sleep_forever(),
        "crash_hello" => std::process::exit(42),
        "malformed_hello" => {
            output.write_all(b"{broken\n").unwrap();
            output.flush().unwrap();
            sleep_forever();
        }
        "oversized_hello" => {
            output
                .write_all(&vec![b'x'; MAX_EVENT_FRAME_BYTES + 1])
                .unwrap();
            output.flush().unwrap();
            sleep_forever();
        }
        _ => {}
    }
    if case == "delayed_hello" {
        thread::sleep(Duration::from_millis(250));
    }
    let mut identity = Hello::expected();
    if case == "wrong_hello" {
        identity.shim_version += 1;
    }
    if case == "wrong_hello" {
        serde_json::to_writer(&mut output, &Frame::hello(hello.session_id, identity)).unwrap();
        output.write_all(b"\n").unwrap();
        output.flush().unwrap();
    } else {
        write_frame(
            &mut output,
            &Frame::hello(hello.session_id, identity),
            MAX_EVENT_FRAME_BYTES,
        )
        .unwrap();
    }
    let mut seq = 0;
    let mut generate: Option<Frame> = None;
    let mut emitted = 0;
    while let Some(frame) = read_frame(&mut input, MAX_REQUEST_FRAME_BYTES).unwrap() {
        match &frame.message {
            Message::Load { .. } => {
                if let Some(path) = args.get(1) {
                    fs::write(format!("{path}.load"), b"executed").unwrap();
                }
                if case == "crash_load" {
                    std::process::exit(43);
                }
                if case == "hang_load" {
                    sleep_forever();
                }
                #[cfg(target_os = "linux")]
                if case == "escaped_pipe" {
                    use std::os::unix::process::CommandExt;
                    let child_path = format!("{}.descendant", args.get(1).unwrap());
                    let mut child = Command::new(std::env::current_exe().unwrap())
                        .args(["descendant", &child_path])
                        .process_group(0)
                        .stdin(Stdio::null())
                        .stdout(Stdio::inherit())
                        .stderr(Stdio::null())
                        .spawn()
                        .unwrap();
                    thread::spawn(move || {
                        let _ = child.wait();
                    });
                }
                if case == "descendants" {
                    let child_path = format!("{}.descendant", args.get(1).unwrap());
                    let mut child = Command::new(std::env::current_exe().unwrap())
                        .args(["descendant", &child_path])
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .spawn()
                        .unwrap();
                    // The separate fixture intentionally survives its worker;
                    // this wait thread reaps it if it exits before the fixture.
                    thread::spawn(move || {
                        let _ = child.wait();
                    });
                }
                emit(&mut output, &frame, &mut seq, ExecutorEvent::Loaded, None);
                if case == "idle_crash" {
                    thread::sleep(Duration::from_millis(120));
                    std::process::exit(44);
                }
                if case == "block_stdin" {
                    sleep_forever();
                }
            }
            Message::Generate { .. } => {
                if case == "crash_generate" {
                    std::process::exit(45);
                }
                if case == "alive_faulted" || case == "escaped_pipe" {
                    emit(
                        &mut output,
                        &frame,
                        &mut seq,
                        ExecutorEvent::Faulted(RuntimeError::new(
                            ErrorCode::NativeFailure,
                            "fixture native thread failed",
                        )),
                        None,
                    );
                    sleep_forever();
                }
                if case == "malformed_event" {
                    output.write_all(b"not-json\n").unwrap();
                    output.flush().unwrap();
                    sleep_forever();
                }
                if case == "oversized_event" || case == "blocked_stdout" {
                    let _ = output.write_all(&vec![b'x'; MAX_EVENT_FRAME_BYTES * 8]);
                    let _ = output.flush();
                    sleep_forever();
                }
                if case == "hang_generate" {
                    sleep_forever();
                }
                if case == "generation_failed" {
                    emit(
                        &mut output,
                        &frame,
                        &mut seq,
                        ExecutorEvent::GenerationFailed {
                            error: RuntimeError::new(
                                ErrorCode::ContextLengthExceeded,
                                "fixture prepare failure",
                            ),
                            usage: Usage::default(),
                        },
                        None,
                    );
                    continue;
                }
                let mut scope = frame.clone();
                if case == "stale_session" {
                    scope.session_id = runtime_ipc::SessionId::new_v4();
                }
                if case == "stale_operation" {
                    scope.operation_id = scope.operation_id.saturating_sub(1);
                }
                emit(
                    &mut output,
                    &scope,
                    &mut seq,
                    ExecutorEvent::Prepared { prompt_tokens: 4 },
                    None,
                );
                generate = Some(frame.clone());
                emitted = 0;
            }
            Message::Credit { credit_id } => {
                if let Some(scope) = &generate {
                    if case == "wait_cancel" || case == "ignore_cancel" {
                        continue;
                    }
                    emit(
                        &mut output,
                        scope,
                        &mut seq,
                        ExecutorEvent::TextDelta("fixture 文本🙂".into()),
                        Some(*credit_id),
                    );
                    emitted += 1;
                    if case == "duplicate_credit" {
                        emit(
                            &mut output,
                            scope,
                            &mut seq,
                            ExecutorEvent::TextDelta("duplicate".into()),
                            Some(*credit_id),
                        );
                    }
                    if case == "text_then_crash" {
                        if let Some(path) = &recovery_marker {
                            fs::write(path, b"failed").unwrap();
                        }
                        thread::sleep(Duration::from_millis(100));
                        std::process::exit(46);
                    }
                    if emitted == 2 && case != "stream" {
                        emit(
                            &mut output,
                            scope,
                            &mut seq,
                            ExecutorEvent::Completed {
                                usage: Usage {
                                    prompt_tokens: 4,
                                    completion_tokens: emitted,
                                },
                                finish_reason: FinishReason::Stop,
                            },
                            None,
                        );
                        if case == "duplicate_terminal" {
                            emit(
                                &mut output,
                                scope,
                                &mut seq,
                                ExecutorEvent::Completed {
                                    usage: Usage::default(),
                                    finish_reason: FinishReason::Stop,
                                },
                                None,
                            );
                        }
                        generate = None;
                    }
                }
            }
            Message::Cancel {} => {
                if case == "ignore_cancel" {
                    continue;
                }
                if let Some(scope) = generate.take() {
                    emit(
                        &mut output,
                        &scope,
                        &mut seq,
                        ExecutorEvent::GenerationFailed {
                            error: RuntimeError::new(
                                ErrorCode::RequestCancelled,
                                "fixture cancelled",
                            ),
                            usage: Usage {
                                prompt_tokens: 4,
                                completion_tokens: emitted,
                            },
                        },
                        None,
                    );
                }
            }
            Message::Unload {} => {
                if case == "crash_unload" {
                    std::process::exit(47);
                }
                if case == "hang_unload" {
                    sleep_forever();
                }
                emit(&mut output, &frame, &mut seq, ExecutorEvent::Unloaded, None);
            }
            Message::Shutdown {} => {
                if case == "hang_shutdown" {
                    sleep_forever();
                }
                return;
            }
            _ => std::process::exit(48),
        }
    }
}
