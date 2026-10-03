//! Private process transport around one thread-affine EngineHost.
//!
//! This is deliberately not a Runtime: the parent owns scheduling, deadlines,
//! public request events and the sole text-byte ledger. A credit is a one-use
//! permission to send one <=4 KiB UTF-8 delta, never an autonomous refill.
use engine_host::EngineHost;
use runtime_core::{
    CancellationHandle, ExecutionEventSink, ExecutionEvents, Executor, ExecutorCommand,
    ExecutorEvent,
};
use runtime_ipc::{
    Frame, Hello, MAX_EVENT_FRAME_BYTES, MAX_REQUEST_FRAME_BYTES, MAX_TEXT_BYTES, MAX_TEXT_CREDITS,
    Message, PROTOCOL_VERSION, SessionId, encode_frame, protocol_error, read_frame,
};
use runtime_types::{ModelId, RequestId, RuntimeError};
use std::{
    collections::VecDeque,
    io::{self, BufRead, BufReader, Write},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Condvar, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

// At most two credited text frames plus small lifecycle frames. The writer owns
// at most one additional encoded frame. Queue capacity is not a second text
// budget: every text frame, including the writer/pipe copy, requires a parent
// reservation that remains held until the final consumer drops its permit.
const OUTPUT_QUEUE_CAPACITY: usize = 4;
const SHUTDOWN_GRACE: Duration = Duration::from_secs(4);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Load,
    Generate,
    Unload,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Scope {
    operation: u64,
    request: Option<RequestId>,
}
struct Active {
    scope: Scope,
    kind: Kind,
    target_model: Option<ModelId>,
    prepared: bool,
    cancelled: bool,
    cancel: Option<CancellationHandle>,
    credits: VecDeque<u64>,
}
struct State {
    session: Option<SessionId>,
    last_operation: u64,
    last_credit: u64,
    next_seq: u64,
    model: Option<ModelId>,
    active: Option<Active>,
    retired: Option<(Scope, Kind)>,
    output: VecDeque<Vec<u8>>,
    stopping: bool,
    output_failed: bool,
    writer_closed: bool,
}
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}
impl Shared {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State {
                session: None,
                last_operation: 0,
                last_credit: 0,
                next_seq: 1,
                model: None,
                active: None,
                retired: None,
                output: VecDeque::with_capacity(OUTPUT_QUEUE_CAPACITY),
                stopping: false,
                output_failed: false,
                writer_closed: false,
            }),
            changed: Condvar::new(),
        })
    }
    fn stop(&self, output_failed: bool) {
        let cancel = {
            let mut state = self.state.lock().unwrap();
            state.stopping = true;
            state.output_failed |= output_failed;
            if let Some(active) = &mut state.active {
                active.cancelled = true;
                active.credits.clear();
                active.cancel.clone()
            } else {
                None
            }
        };
        // Never hold the transport mutex while calling the native flag setter.
        if let Some(cancel) = cancel {
            cancel.cancel();
        }
        self.changed.notify_all();
    }
    fn await_inactive(&self, deadline: Instant) -> bool {
        let mut state = self.state.lock().unwrap();
        while state.active.is_some() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            state = self.changed.wait_timeout(state, left).unwrap().0;
        }
        true
    }
}

/// Starts independent stdin control, stdout writer, and EngineHost inference
/// threads. There is no unbounded join: process exit is the final bounded
/// fallback for a blocked OS pipe or uncooperative native call. The parent's
/// containment/watchdog remains authoritative if this process itself stalls.
pub fn run() -> Result<(), RuntimeError> {
    let shared = Shared::new();
    let (finished, notifications) = mpsc::sync_channel(2);
    let output_shared = shared.clone();
    let output_finished = finished.clone();
    thread::Builder::new()
        .name("nexa-worker-output".into())
        .spawn(move || {
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                writer(io::stdout().lock(), &output_shared)
            }));
            if !matches!(outcome, Ok(Ok(()))) {
                output_shared.stop(true);
                let _ = output_finished.try_send(Notification::OutputFailed);
            }
        })
        .map_err(|_| protocol_error("cannot start worker output thread"))?;
    let input_shared = shared.clone();
    thread::Builder::new()
        .name("nexa-worker-control".into())
        .spawn(move || {
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                control(BufReader::new(io::stdin().lock()), &input_shared)
            }));
            let _ = finished.try_send(Notification::ControlFinished(matches!(outcome, Ok(Ok(())))));
        })
        .map_err(|_| protocol_error("cannot start worker control thread"))?;
    let success = match notifications.recv() {
        Ok(Notification::ControlFinished(success)) => success,
        _ => {
            // A writer failure may leave stdin blocked. Cancel independently,
            // wait for native return once, and never join the blocked reader.
            shared.stop(true);
            shared.await_inactive(Instant::now() + SHUTDOWN_GRACE);
            false
        }
    };
    shared.stop(!success);
    if success {
        Ok(())
    } else {
        Err(protocol_error("worker session failed"))
    }
}

enum Notification {
    ControlFinished(bool),
    OutputFailed,
}

fn native_hello() -> Result<Hello, RuntimeError> {
    let value: serde_json::Value = serde_json::from_str(&llama_adapter::build_info()?)
        .map_err(|_| protocol_error("invalid native build identity"))?;
    let shim_version = value["shim_version"]
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| protocol_error("missing native shim version"))?;
    let llama_commit = value["llama_commit"]
        .as_str()
        .ok_or_else(|| protocol_error("missing native commit"))?
        .to_owned();
    let hello = Hello {
        protocol_version: PROTOCOL_VERSION,
        shim_version,
        llama_commit,
    };
    hello.validate()?;
    Ok(hello)
}

fn control(mut reader: impl BufRead, shared: &Arc<Shared>) -> Result<(), RuntimeError> {
    let mut host = EngineHost::new()?;
    let outcome = (|| {
        while let Some(frame) = read_frame(&mut reader, MAX_REQUEST_FRAME_BYTES)? {
            let action = accept(frame, shared)?;
            match action {
                Action::None => {}
                Action::Stop => return Ok(()),
                Action::Start { scope, command } => {
                    let events = ExecutionEvents::from_sink(
                        scope.operation,
                        Arc::new(WireSink {
                            shared: shared.clone(),
                            scope,
                        }),
                    );
                    match host.start(command, events.clone()) {
                        Ok(cancel) => {
                            let cancelled = {
                                let mut state = shared.state.lock().unwrap();
                                if let Some(active) =
                                    state.active.as_mut().filter(|active| active.scope == scope)
                                {
                                    active.cancel = Some(cancel.clone());
                                    active.cancelled
                                } else {
                                    false
                                }
                            };
                            if cancelled {
                                cancel.cancel();
                            }
                        }
                        Err(error) => {
                            events.emit(ExecutorEvent::Failed(error));
                        }
                    }
                }
            }
        }
        Ok(())
    })();
    let deadline = Instant::now() + SHUTDOWN_GRACE;
    shared.stop(outcome.is_err());
    let native_cleaned = cleanup(&mut host, shared, deadline);
    // Close the mailbox only after the native-thread unload acknowledgment or
    // the bounded grace. EngineHost never drops native objects off their owner.
    drop(host);
    {
        let mut state = shared.state.lock().unwrap();
        state.writer_closed = true;
    }
    shared.changed.notify_all();
    outcome?;
    if native_cleaned {
        Ok(())
    } else {
        Err(protocol_error("worker cleanup grace expired"))
    }
}

struct CleanupSink(mpsc::SyncSender<()>);
impl ExecutionEventSink for CleanupSink {
    fn emit(&self, event: ExecutorEvent) -> bool {
        if matches!(event, ExecutorEvent::Unloaded) {
            let _ = self.0.try_send(());
        }
        true
    }
}
fn cleanup(host: &mut EngineHost, shared: &Shared, deadline: Instant) -> bool {
    if !shared.await_inactive(deadline) {
        return false;
    }
    let (done, receiver) = mpsc::sync_channel(1);
    if host
        .start(
            ExecutorCommand::Unload,
            ExecutionEvents::from_sink(0, Arc::new(CleanupSink(done))),
        )
        .is_err()
    {
        return false;
    }
    receiver
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .is_ok()
}

enum Action {
    None,
    Stop,
    Start {
        scope: Scope,
        command: ExecutorCommand,
    },
}
fn accept(frame: Frame, shared: &Shared) -> Result<Action, RuntimeError> {
    let mut state = shared.state.lock().unwrap();
    if state.stopping || frame.protocol_version != PROTOCOL_VERSION || frame.seq.is_some() {
        return Err(protocol_error("invalid command envelope"));
    }
    if state.session.is_none() {
        let Message::Hello(hello) = frame.message else {
            return Err(protocol_error("command before handshake"));
        };
        if frame.operation_id != 0 || frame.request_id.is_some() || frame.session_id.is_nil() {
            return Err(protocol_error("invalid handshake envelope"));
        }
        hello.validate()?;
        let response = Frame::hello(frame.session_id, native_hello()?);
        state
            .output
            .push_back(encode_frame(&response, MAX_EVENT_FRAME_BYTES)?);
        state.session = Some(frame.session_id);
        shared.changed.notify_all();
        return Ok(Action::None);
    }
    if state.session != Some(frame.session_id) {
        return Err(protocol_error("command session mismatch"));
    }
    if matches!(frame.message, Message::Shutdown {}) {
        return if frame.operation_id == 0 && frame.request_id.is_none() {
            Ok(Action::Stop)
        } else {
            Err(protocol_error("invalid shutdown identity"))
        };
    }
    if frame.operation_id == 0 {
        return Err(protocol_error("invalid command operation"));
    }
    let scope = Scope {
        operation: frame.operation_id,
        request: frame.request_id,
    };
    match frame.message {
        Message::Hello(_) | Message::Event { .. } => Err(protocol_error("unexpected command kind")),
        Message::Cancel {} => {
            let Some(active) = &mut state.active else {
                return if state.retired.is_some_and(|(retired, _)| retired == scope) {
                    Ok(Action::None)
                } else {
                    Err(protocol_error("cancel operation mismatch"))
                };
            };
            if active.scope != scope {
                return if state.retired.is_some_and(|(retired, _)| retired == scope) {
                    Ok(Action::None)
                } else {
                    Err(protocol_error("cancel operation mismatch"))
                };
            }
            active.cancelled = true;
            active.credits.clear();
            let cancel = active.cancel.clone();
            drop(state);
            if let Some(cancel) = cancel {
                cancel.cancel();
            }
            shared.changed.notify_all();
            Ok(Action::None)
        }
        Message::Credit { credit_id } => {
            if credit_id == 0 || credit_id <= state.last_credit {
                return Err(protocol_error("duplicate or out-of-order credit"));
            }
            let retired = state.retired == Some((scope, Kind::Generate));
            let active = state.active.as_mut().filter(|active| active.scope == scope);
            if let Some(active) = active {
                if active.kind != Kind::Generate || active.credits.len() >= MAX_TEXT_CREDITS {
                    return Err(protocol_error("credit window or operation mismatch"));
                }
                if !active.cancelled {
                    active.credits.push_back(credit_id);
                }
            } else if !retired {
                return Err(protocol_error("credit operation mismatch"));
            }
            // A previously emitted delta may be consumed just after terminal.
            // Such a new credit is retired, never applied to a later request.
            state.last_credit = credit_id;
            shared.changed.notify_all();
            Ok(Action::None)
        }
        message => {
            if scope.operation <= state.last_operation {
                return Err(protocol_error("duplicate or out-of-order operation"));
            }
            if state.active.is_some() {
                return Err(protocol_error("concurrent worker operation"));
            }
            let (kind, target_model, command) = match message {
                Message::Load { model, options } => {
                    if scope.request.is_some() || state.model.is_some() {
                        return Err(protocol_error("invalid load state"));
                    }
                    options.validate()?;
                    if !model.loadable || options.context_size > model.context_limit {
                        return Err(protocol_error("ineligible model or invalid context"));
                    }
                    (
                        Kind::Load,
                        Some(model.id.clone()),
                        ExecutorCommand::Load { model, options },
                    )
                }
                Message::Generate { request } => {
                    if scope.request != Some(request.request_id)
                        || state.model.as_ref() != Some(&request.model)
                    {
                        return Err(protocol_error("invalid generation state or identity"));
                    }
                    request.validate()?;
                    (Kind::Generate, None, ExecutorCommand::Generate { request })
                }
                Message::Unload {} => {
                    if scope.request.is_some() {
                        return Err(protocol_error("unload carries request"));
                    }
                    (Kind::Unload, None, ExecutorCommand::Unload)
                }
                _ => unreachable!(),
            };
            state.last_operation = scope.operation;
            state.active = Some(Active {
                scope,
                kind,
                target_model,
                prepared: false,
                cancelled: false,
                cancel: None,
                credits: VecDeque::with_capacity(MAX_TEXT_CREDITS),
            });
            Ok(Action::Start { scope, command })
        }
    }
}

struct WireSink {
    shared: Arc<Shared>,
    scope: Scope,
}
impl ExecutionEventSink for WireSink {
    fn emit(&self, event: ExecutorEvent) -> bool {
        let terminal = matches!(
            event,
            ExecutorEvent::Loaded
                | ExecutorEvent::Unloaded
                | ExecutorEvent::Completed { .. }
                | ExecutorEvent::Failed(_)
                | ExecutorEvent::GenerationFailed { .. }
                | ExecutorEvent::Faulted(_)
        );
        let text = matches!(event, ExecutorEvent::TextDelta(_));
        let mut state = self.shared.state.lock().unwrap();
        loop {
            let Some(active) = state
                .active
                .as_ref()
                .filter(|active| active.scope == self.scope)
            else {
                return false;
            };
            if state.stopping
                || state.output_failed
                || (active.cancelled && (text || state.output.len() >= OUTPUT_QUEUE_CAPACITY))
            {
                if terminal {
                    finish(&mut state, &event);
                }
                self.shared.changed.notify_all();
                return false;
            }
            if text && (active.kind != Kind::Generate || !active.prepared) {
                return false;
            }
            if state.output.len() < OUTPUT_QUEUE_CAPACITY && (!text || !active.credits.is_empty()) {
                break;
            }
            // Both credit and bounded queue waits are woken by Cancel, EOF,
            // Shutdown and output failure, independently of the blocked writer.
            state = self.shared.changed.wait(state).unwrap();
        }
        if let ExecutorEvent::TextDelta(piece) = &event
            && (piece.is_empty() || piece.len() > MAX_TEXT_BYTES)
        {
            return false;
        }
        if let ExecutorEvent::Prepared { .. } = event {
            let active = state.active.as_mut().unwrap();
            if active.kind != Kind::Generate || active.prepared {
                return false;
            }
            active.prepared = true;
        }
        let credit = if text {
            state.active.as_mut().unwrap().credits.pop_front()
        } else {
            None
        };
        let frame = Frame::event(
            state.session.unwrap(),
            self.scope.operation,
            self.scope.request,
            state.next_seq,
            event,
            credit,
        );
        let Ok(encoded) = encode_frame(&frame, MAX_EVENT_FRAME_BYTES) else {
            drop(state);
            self.shared.stop(true);
            return false;
        };
        state.next_seq += 1;
        state.output.push_back(encoded);
        if terminal {
            let Message::Event { event, .. } = &frame.message else {
                unreachable!()
            };
            finish(&mut state, event);
        }
        self.shared.changed.notify_all();
        true
    }
}
fn finish(state: &mut State, event: &ExecutorEvent) {
    let active = state.active.take().unwrap();
    match event {
        ExecutorEvent::Loaded => state.model = active.target_model,
        ExecutorEvent::Unloaded | ExecutorEvent::Faulted(_) => state.model = None,
        _ => {}
    }
    // The parent supervisor serializes command/credit writes into one FIFO.
    // It never grants to completed operations, and issues the next operation
    // only after terminal delivery. Thus already-enqueued controls cannot lag
    // beyond the immediately completed scope. Keep that scope for the duplex
    // terminal race without an unbounded history of request identities.
    state.retired = Some((active.scope, active.kind));
}
fn writer(mut output: impl Write, shared: &Shared) -> io::Result<()> {
    loop {
        let encoded = {
            let mut state = shared.state.lock().unwrap();
            while state.output.is_empty() && !state.writer_closed && !state.output_failed {
                state = shared.changed.wait(state).unwrap();
            }
            if state.output_failed {
                return Err(io::ErrorKind::BrokenPipe.into());
            }
            let Some(encoded) = state.output.pop_front() else {
                return Ok(());
            };
            shared.changed.notify_all();
            encoded
        };
        output.write_all(&encoded)?;
        output.flush()?;
    }
}

#[cfg(test)]
mod tests;
