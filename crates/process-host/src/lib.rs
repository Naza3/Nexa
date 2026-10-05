//! Parent-only process executor. This crate never links engine-host or llama.
//! The scheduler calls a bounded mailbox; only the supervisor/writer/reader
//! touch process and pipe APIs. Cancellation is a timestamped atomic store.
use runtime_core::{
    CancellationHandle, ExecutionEvents, Executor, ExecutorCommand, ExecutorEvent, TextPermit,
};
use runtime_ipc::{
    EventValidator, Frame, Hello, MAX_EVENT_FRAME_BYTES, MAX_REQUEST_FRAME_BYTES, MAX_TEXT_CREDITS,
    Message, SessionId, TEXT_CREDIT_CHARGE, TRANSPORT_SCRATCH_CHARGE, encode_frame, read_frame,
};
use runtime_types::{ErrorCode, RuntimeError};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    io::{BufReader, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
mod platform;
const POLL: Duration = Duration::from_millis(5);
pub const CANCEL_GRACE: Duration = Duration::from_secs(5);

#[derive(Clone, Debug)]
pub struct ProcessHostConfig {
    pub worker_path: PathBuf,
    /// Used for separate external test fixtures; production worker has no args.
    pub worker_args: Vec<OsString>,
    pub handshake_timeout: Duration,
    pub unload_timeout: Duration,
    pub shutdown_timeout: Duration,
}
impl ProcessHostConfig {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            worker_path: path.into(),
            worker_args: Vec::new(),
            handshake_timeout: Duration::from_secs(5),
            unload_timeout: Duration::from_secs(5),
            shutdown_timeout: Duration::from_secs(5),
        }
    }
    fn validate(&self) -> Result<(), RuntimeError> {
        if self.worker_path.as_os_str().is_empty()
            || [
                self.handshake_timeout,
                self.unload_timeout,
                self.shutdown_timeout,
            ]
            .iter()
            .any(|d| d.is_zero() || *d > Duration::from_secs(300))
        {
            return Err(RuntimeError::invalid(
                "worker path and positive finite deadlines are required",
            ));
        }
        Ok(())
    }
}
struct Cancel {
    origin: Instant,
    first: AtomicU64,
}
impl Cancel {
    fn new() -> Self {
        Self {
            origin: Instant::now(),
            first: AtomicU64::new(0),
        }
    }
    fn set(&self) {
        let timestamp = self.origin.elapsed().as_millis().min(u64::MAX as u128 - 1) as u64 + 1;
        let _ = self
            .first
            .compare_exchange(0, timestamp, Ordering::AcqRel, Ordering::Acquire);
    }
    fn since(&self) -> Option<Duration> {
        let timestamp = self.first.load(Ordering::Acquire);
        (timestamp != 0).then(|| {
            self.origin
                .elapsed()
                .saturating_sub(Duration::from_millis(timestamp.saturating_sub(1)))
        })
    }
}
struct Job {
    command: ExecutorCommand,
    events: ExecutionEvents,
    cancel: Arc<Cancel>,
}
/// Best-effort numeric diagnostics; contains no path, prompt, or generated text.
#[derive(Clone)]
pub struct ProcessDiagnostics {
    pid: Arc<AtomicU32>,
    sessions: Arc<AtomicU64>,
    reaped: Arc<AtomicU64>,
}
impl ProcessDiagnostics {
    pub fn worker_pid(&self) -> Option<u32> {
        let pid = self.pid.load(Ordering::Acquire);
        (pid != 0).then_some(pid)
    }
    pub fn sessions_started(&self) -> u64 {
        self.sessions.load(Ordering::Acquire)
    }
    pub fn sessions_reaped(&self) -> u64 {
        self.reaped.load(Ordering::Acquire)
    }
}
pub struct ProcessHost {
    sender: Option<SyncSender<Job>>,
    stopping: Arc<AtomicBool>,
    thread: Option<JoinHandle<Result<(), RuntimeError>>>,
    diagnostics: ProcessDiagnostics,
}
impl ProcessHost {
    pub fn new(config: ProcessHostConfig) -> Result<Self, RuntimeError> {
        config.validate()?;
        let (sender, receiver) = mpsc::sync_channel(1);
        let stopping = Arc::new(AtomicBool::new(false));
        let stop = stopping.clone();
        let diagnostics = ProcessDiagnostics {
            pid: Arc::new(AtomicU32::new(0)),
            sessions: Arc::new(AtomicU64::new(0)),
            reaped: Arc::new(AtomicU64::new(0)),
        };
        let worker_diagnostics = diagnostics.clone();
        let thread = thread::Builder::new()
            .name("nexa-process-supervisor".into())
            .spawn(move || Supervisor::new(config, receiver, stop, worker_diagnostics).run())
            .map_err(|_| unavailable("cannot start process supervisor"))?;
        Ok(Self {
            sender: Some(sender),
            stopping,
            thread: Some(thread),
            diagnostics,
        })
    }
    pub fn diagnostics(&self) -> ProcessDiagnostics {
        self.diagnostics.clone()
    }
}
impl Executor for ProcessHost {
    fn start(
        &mut self,
        command: ExecutorCommand,
        events: ExecutionEvents,
    ) -> Result<CancellationHandle, RuntimeError> {
        if self.stopping.load(Ordering::Acquire) {
            return Err(unavailable("process executor is closed"));
        }
        let cancel = Arc::new(Cancel::new());
        let handle = cancel.clone();
        let sender = self
            .sender
            .as_ref()
            .ok_or_else(|| unavailable("process executor is closed"))?;
        sender
            .try_send(Job {
                command,
                events,
                cancel,
            })
            .map_err(|error| match error {
                TrySendError::Full(_) => {
                    RuntimeError::new(ErrorCode::RuntimeBusy, "process mailbox is full")
                }
                TrySendError::Disconnected(_) => unavailable("process supervisor exited"),
            })?;
        Ok(CancellationHandle::new(move || handle.set()))
    }
    fn close(&mut self) -> Result<(), RuntimeError> {
        self.stopping.store(true, Ordering::Release);
        self.sender.take();
        match self.thread.take() {
            Some(thread) => thread
                .join()
                .map_err(|_| unavailable("process supervisor panicked"))?,
            None => Ok(()),
        }
    }
}
impl Drop for ProcessHost {
    fn drop(&mut self) {
        let _ = self.close();
    }
}
fn unavailable(message: &str) -> RuntimeError {
    RuntimeError::new(ErrorCode::ExecutorUnavailable, message)
}
fn protocol(message: &str) -> RuntimeError {
    runtime_ipc::protocol_error(message)
}

enum Incoming {
    Frame(Frame),
    Error(RuntimeError),
    Eof,
}
struct Session {
    id: SessionId,
    child: platform::Child,
    outgoing: Option<SyncSender<Vec<u8>>>,
    incoming: Option<Receiver<Incoming>>,
    reader: Option<JoinHandle<()>>,
    writer: Option<JoinHandle<()>>,
    validator: Arc<Mutex<EventValidator>>,
    cleaned: bool,
    handshaken: bool,
    born: Instant,
    next_credit: u64,
}
impl Session {
    fn spawn(path: &Path, args: &[OsString]) -> Result<Self, RuntimeError> {
        let born = Instant::now();
        let mut child = platform::Child::spawn(path, args).map_err(|error| {
            RuntimeError::new(
                ErrorCode::ExecutorUnavailable,
                format!(
                    "cannot spawn contained worker (os_error={:?})",
                    error.raw_os_error()
                ),
            )
        })?;
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let id = SessionId::new_v4();
        let validator = Arc::new(Mutex::new(EventValidator::new(id)));
        let (outgoing, writes) = mpsc::sync_channel::<Vec<u8>>(4);
        let (incoming_tx, incoming) = mpsc::sync_channel(1);
        let errors = incoming_tx.clone();
        let writer = thread::Builder::new()
            .name("nexa-worker-stdin".into())
            .spawn(move || {
                let mut stdin = stdin;
                while let Ok(frame) = writes.recv() {
                    if stdin.write_all(&frame).and_then(|_| stdin.flush()).is_err() {
                        let _ =
                            errors.try_send(Incoming::Error(unavailable("worker stdin closed")));
                        break;
                    }
                }
            })
            .map_err(|_| unavailable("cannot start worker stdin thread"))?;
        let validation = validator.clone();
        let reader = match thread::Builder::new()
            .name("nexa-worker-stdout".into())
            .spawn(move || {
                let mut stdout = BufReader::with_capacity(8192, stdout);
                let mut hello = false;
                loop {
                    let message = match read_frame(&mut stdout, MAX_EVENT_FRAME_BYTES) {
                        Ok(Some(frame)) => {
                            let checked = if hello {
                                validation.lock().unwrap().accept(&frame)
                            } else {
                                validation.lock().unwrap().accept_hello(&frame)
                            };
                            match checked {
                                Ok(()) => {
                                    hello = true;
                                    Incoming::Frame(frame)
                                }
                                Err(error) => Incoming::Error(error),
                            }
                        }
                        Ok(None) => Incoming::Eof,
                        Err(error) => Incoming::Error(error),
                    };
                    let terminal = !matches!(message, Incoming::Frame(_));
                    if incoming_tx.send(message).is_err() || terminal {
                        break;
                    }
                }
            }) {
            Ok(reader) => reader,
            Err(_) => {
                drop(outgoing);
                let _ = child.kill_tree();
                let _ = child.wait();
                let _ = writer.join();
                return Err(unavailable("cannot start worker stdout thread"));
            }
        };
        let mut session = Self {
            id,
            child,
            outgoing: Some(outgoing),
            incoming: Some(incoming),
            reader: Some(reader),
            writer: Some(writer),
            validator,
            cleaned: false,
            handshaken: false,
            born,
            next_credit: 0,
        };
        session.send(Frame::hello(id, Hello::expected()))?;
        Ok(session)
    }
    fn send(&mut self, frame: Frame) -> Result<(), RuntimeError> {
        // Encoding + size validation happens before the writer sees the frame.
        let bytes = encode_frame(&frame, MAX_REQUEST_FRAME_BYTES)?;
        self.outgoing
            .as_ref()
            .ok_or_else(|| unavailable("worker transport is closed"))?
            .try_send(bytes)
            .map_err(|_| unavailable("bounded worker stdin queue stalled"))
    }
    fn reap(&mut self) -> Result<(), RuntimeError> {
        if self.cleaned {
            return Ok(());
        }
        self.cleaned = true;
        // Drop receive first so a reader blocked on its bounded queue wakes.
        self.incoming.take();
        self.outgoing.take();
        let killed = self.child.kill_tree();
        let waited = self.child.wait();
        let transport_deadline = Instant::now() + Duration::from_secs(5);
        if killed.is_ok() && waited.is_ok() {
            while self
                .reader
                .as_ref()
                .is_some_and(|thread| !thread.is_finished())
                || self
                    .writer
                    .as_ref()
                    .is_some_and(|thread| !thread.is_finished())
            {
                if Instant::now() >= transport_deadline {
                    return Err(unavailable(
                        "worker pipe thread cleanup could not be confirmed",
                    ));
                }
                thread::sleep(POLL);
            }
            if let Some(reader) = self.reader.take() {
                let _ = reader.join();
            }
            if let Some(writer) = self.writer.take() {
                let _ = writer.join();
            }
        }
        // If exit cannot be confirmed, do not hang joining blocked pipe calls.
        // Owned job/process handles still close; never claim this was reaped.
        killed
            .and(waited)
            .map_err(|_| unavailable("worker containment cleanup could not be confirmed"))
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.reap();
    }
}
struct Active {
    job: Job,
    sent: bool,
    cancel_sent: bool,
    begun: Instant,
    permits: BTreeMap<u64, TextPermit>,
    scratch: Option<TextPermit>,
}
struct Supervisor {
    config: ProcessHostConfig,
    receiver: Receiver<Job>,
    stopping: Arc<AtomicBool>,
    diagnostics: ProcessDiagnostics,
    session: Option<Session>,
    active: Option<Active>,
    last_events: Option<ExecutionEvents>,
}
impl Supervisor {
    fn new(
        config: ProcessHostConfig,
        receiver: Receiver<Job>,
        stopping: Arc<AtomicBool>,
        diagnostics: ProcessDiagnostics,
    ) -> Self {
        Self {
            config,
            receiver,
            stopping,
            diagnostics,
            session: None,
            active: None,
            last_events: None,
        }
    }
    fn run(mut self) -> Result<(), RuntimeError> {
        loop {
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.run_loop())) {
                Ok(result) => return result,
                Err(_) => {
                    // Never strand an actor operation after a supervisor panic.
                    // Own/reap the session before the ordinary fault ACK; only
                    // a future explicit Load may then establish a new session.
                    if let Err(error) = self.fault(unavailable("process supervisor panicked")) {
                        return self.cleanup_unconfirmed(error);
                    }
                }
            }
        }
    }
    fn run_loop(&mut self) -> Result<(), RuntimeError> {
        loop {
            if self.stopping.load(Ordering::Acquire) {
                return self.shutdown();
            }
            if let Err(error) = self.tick()
                && let Err(cleanup) = self.fault(error)
            {
                return self.cleanup_unconfirmed(cleanup);
            }
            thread::sleep(POLL);
        }
    }
    fn tick(&mut self) -> Result<(), RuntimeError> {
        // Drain protocol before polling process exit: the final ACK may already
        // be in the pipe when a child exits. Unexpected exit still faults idle.
        let incoming = self
            .session
            .as_mut()
            .and_then(|s| s.incoming.as_ref())
            .map(|r| r.try_recv());
        if let Some(result) = incoming {
            match result {
                Ok(Incoming::Frame(frame)) => self.received(frame)?,
                Ok(Incoming::Error(error)) => return Err(error),
                Ok(Incoming::Eof) | Err(TryRecvError::Disconnected) => {
                    return Err(unavailable("worker stdout ended"));
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        if let Some(session) = &mut self.session {
            if session
                .child
                .try_wait()
                .map_err(|_| unavailable("cannot observe worker status"))?
            {
                return Err(unavailable("worker exited unexpectedly"));
            }
            if !session.handshaken && session.born.elapsed() >= self.config.handshake_timeout {
                return Err(protocol("worker Hello timed out"));
            }
        }
        if self.active.is_none() {
            match self.receiver.try_recv() {
                Ok(job) => self.begin(job)?,
                Err(TryRecvError::Disconnected) => {
                    self.stopping.store(true, Ordering::Release);
                    return Ok(());
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        if let Some(active) = &self.active {
            if active.job.events.cancellation_reason().is_some() {
                active.job.cancel.set();
            }
            if active
                .job
                .cancel
                .since()
                .is_some_and(|elapsed| elapsed >= CANCEL_GRACE)
            {
                let code = active
                    .job
                    .events
                    .cancellation_reason()
                    .unwrap_or(ErrorCode::RequestCancelled);
                return Err(RuntimeError::new(
                    code,
                    "worker did not acknowledge cancellation before grace deadline",
                ));
            }
            if matches!(active.job.command, ExecutorCommand::Unload)
                && active.begun.elapsed() >= self.config.unload_timeout
            {
                return Err(unavailable("worker unload timed out"));
            }
        }
        self.dispatch()?;
        Ok(())
    }
    fn begin(&mut self, job: Job) -> Result<(), RuntimeError> {
        self.last_events = Some(job.events.clone());
        if self.session.is_none() && matches!(job.command, ExecutorCommand::Unload) {
            job.events.emit(ExecutorEvent::Unloaded);
            return Ok(());
        }
        if self.session.is_none() && !matches!(job.command, ExecutorCommand::Load { .. }) {
            job.events.emit(ExecutorEvent::Faulted(unavailable(
                "worker is gone; explicit load is required",
            )));
            return Ok(());
        }
        self.active = Some(Active {
            job,
            sent: false,
            cancel_sent: false,
            begun: Instant::now(),
            permits: BTreeMap::new(),
            scratch: None,
        });
        if self
            .active
            .as_ref()
            .is_some_and(|a| a.job.cancel.since().is_some())
        {
            let active = self.active.take().unwrap();
            active
                .job
                .events
                .emit(ExecutorEvent::Failed(RuntimeError::new(
                    ErrorCode::RequestCancelled,
                    "operation cancelled before dispatch",
                )));
            return Ok(());
        }
        if self.session.is_none() {
            let session = Session::spawn(&self.config.worker_path, &self.config.worker_args)?;
            self.diagnostics
                .pid
                .store(session.child.id(), Ordering::Release);
            self.diagnostics.sessions.fetch_add(1, Ordering::AcqRel);
            self.session = Some(session);
        }
        Ok(())
    }
    fn dispatch(&mut self) -> Result<(), RuntimeError> {
        // A command not yet committed to the writer cannot have changed native
        // state. Settle it locally without faulting a healthy worker or sending
        // a Cancel for an operation the worker has never seen.
        if self
            .active
            .as_ref()
            .is_some_and(|a| !a.sent && a.job.cancel.since().is_some())
        {
            let active = self.active.take().unwrap();
            let reason = active
                .job
                .events
                .cancellation_reason()
                .unwrap_or(ErrorCode::RequestCancelled);
            active
                .job
                .events
                .emit(ExecutorEvent::Failed(RuntimeError::new(
                    reason,
                    "operation cancelled before dispatch",
                )));
            return Ok(());
        }
        let (Some(session), Some(active)) = (&mut self.session, &mut self.active) else {
            return Ok(());
        };
        if !session.handshaken {
            return Ok(());
        }
        let operation = active.job.events.operation_id();
        let request_id = match &active.job.command {
            ExecutorCommand::Generate { request } => Some(request.request_id),
            _ => None,
        };
        if !active.sent {
            let message = match &active.job.command {
                ExecutorCommand::Load { model, options } => Message::Load {
                    model: model.clone(),
                    options: *options,
                },
                ExecutorCommand::Generate { request } => Message::Generate {
                    request: request.clone(),
                },
                ExecutorCommand::Unload => Message::Unload {},
            };
            let frame = Frame::command(session.id, operation, request_id, message);
            // Validate encoded bounds before mutating protocol state.
            encode_frame(&frame, MAX_REQUEST_FRAME_BYTES)?;
            session.validator.lock().unwrap().begin(&frame)?;
            session.send(frame)?;
            active.sent = true;
        }
        if active.job.cancel.since().is_some() {
            if !active.cancel_sent {
                session.send(Frame::command(
                    session.id,
                    operation,
                    request_id,
                    Message::Cancel {},
                ))?;
                active.cancel_sent = true;
            }
            return Ok(());
        }
        if matches!(active.job.command, ExecutorCommand::Generate { .. }) {
            if active.scratch.is_none() {
                active.scratch = active.job.events.try_reserve_text(TRANSPORT_SCRATCH_CHARGE);
                if active.scratch.is_none() {
                    return Ok(());
                }
            }
            while active.permits.len() < MAX_TEXT_CREDITS {
                let Some(permit) = active.job.events.try_reserve_text(TEXT_CREDIT_CHARGE) else {
                    break;
                };
                session.next_credit = session
                    .next_credit
                    .checked_add(1)
                    .ok_or_else(|| protocol("credit identifier exhausted"))?;
                let id = session.next_credit;
                {
                    let mut validator = session.validator.lock().unwrap();
                    if validator.operation_complete() {
                        break;
                    }
                    validator.grant(id)?;
                }
                active.permits.insert(id, permit);
                session.send(Frame::command(
                    session.id,
                    operation,
                    request_id,
                    Message::Credit { credit_id: id },
                ))?;
            }
        }
        Ok(())
    }
    fn received(&mut self, frame: Frame) -> Result<(), RuntimeError> {
        if let Message::Hello(_) = frame.message {
            self.session.as_mut().unwrap().handshaken = true;
            return Ok(());
        }
        let Message::Event { event, credit_id } = frame.message else {
            return Err(protocol("worker sent a command"));
        };
        if matches!(event, ExecutorEvent::Faulted(_)) {
            let ExecutorEvent::Faulted(error) = event else {
                unreachable!()
            };
            return Err(error);
        }
        let active = self
            .active
            .as_mut()
            .ok_or_else(|| protocol("worker event without active operation"))?;
        if let ExecutorEvent::TextDelta(text) = event {
            let permit = active
                .permits
                .remove(&credit_id.ok_or_else(|| protocol("text has no credit"))?)
                .ok_or_else(|| protocol("text credit is not reserved"))?;
            if !active.job.events.emit_reserved_text(text, permit) {
                active.job.cancel.set();
            }
        } else {
            let terminal = matches!(
                event,
                ExecutorEvent::Loaded
                    | ExecutorEvent::Completed { .. }
                    | ExecutorEvent::Failed(_)
                    | ExecutorEvent::GenerationFailed { .. }
                    | ExecutorEvent::Unloaded
            );
            if terminal {
                let active = self.active.take().unwrap();
                // Unused credit/scratch permits drop; consumed text remains
                // charged in actor/consumer leases, not in this transport.
                active.job.events.emit(event);
            } else {
                active.job.events.emit(event);
            }
        }
        Ok(())
    }
    fn reap(&mut self) -> Result<(), RuntimeError> {
        if let Some(mut session) = self.session.take() {
            let result = session.reap();
            if result.is_ok() {
                self.diagnostics.pid.store(0, Ordering::Release);
                self.diagnostics.reaped.fetch_add(1, Ordering::AcqRel);
            }
            result
        } else {
            Ok(())
        }
    }
    fn fault(&mut self, mut error: RuntimeError) -> Result<(), RuntimeError> {
        self.reap()?;
        if let Some(active) = self.active.take() {
            // Cancellation explains an expected worker exit or control ACK,
            // never malformed IPC or a real executor/native failure. Cleanup
            // has already been confirmed above; unconfirmed cleanup is separate.
            if matches!(
                error.code,
                ErrorCode::ExecutorUnavailable
                    | ErrorCode::RequestCancelled
                    | ErrorCode::ConsumerStopped
                    | ErrorCode::SlowConsumer
                    | ErrorCode::RuntimeShutdown
                    | ErrorCode::QueueTimeout
                    | ErrorCode::LoadTimeout
                    | ErrorCode::ExecutionTimeout
            ) {
                if let Some(reason) = active.job.events.cancellation_reason() {
                    error = RuntimeError::new(reason, "worker stopped during cancellation");
                } else if active.job.cancel.since().is_some() {
                    error = RuntimeError::new(
                        ErrorCode::RequestCancelled,
                        "worker stopped during cancellation",
                    );
                }
            }
            active.job.events.emit(ExecutorEvent::Faulted(error));
        } else if let Some(events) = self.last_events.take() {
            events.emit(ExecutorEvent::Faulted(error));
        }
        // Never replay. The scheduler will explicitly call Load to recover.
        Ok(())
    }
    fn cleanup_unconfirmed(&mut self, error: RuntimeError) -> Result<(), RuntimeError> {
        let error = RuntimeError::new(ErrorCode::ExecutorCleanupUnconfirmed, error.message);
        self.stopping.store(true, Ordering::Release);
        if let Some(active) = self.active.take() {
            active
                .job
                .events
                .emit(ExecutorEvent::CleanupUnconfirmed(error.clone()));
        } else if let Some(events) = self.last_events.take() {
            events.emit(ExecutorEvent::CleanupUnconfirmed(error.clone()));
        }
        Err(error)
    }
    fn shutdown(&mut self) -> Result<(), RuntimeError> {
        if let Some(active) = &self.active {
            active.job.cancel.set();
        }
        // Normal close is called only after resource cleanup ACK. Direct host
        // drop is also bounded: send shutdown behind already accepted writes.
        if let Some(session) = &mut self.session
            && session.handshaken
        {
            let _ = session.send(Frame::command(session.id, 0, None, Message::Shutdown {}));
            let deadline = Instant::now() + self.config.shutdown_timeout;
            while Instant::now() < deadline {
                if session.child.try_wait().unwrap_or(false) {
                    break;
                }
                thread::sleep(POLL);
            }
        }
        if let Err(error) = self.reap() {
            return self.cleanup_unconfirmed(error);
        }
        if let Some(active) = self.active.take() {
            active
                .job
                .events
                .emit(ExecutorEvent::Faulted(RuntimeError::new(
                    ErrorCode::RuntimeShutdown,
                    "process executor shut down",
                )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn process_deadlines_reject_zero_and_overflowing_values() {
        for duration in [Duration::ZERO, Duration::MAX, Duration::from_secs(301)] {
            let mut config = ProcessHostConfig::new("worker");
            config.handshake_timeout = duration;
            assert!(config.validate().is_err());
            let mut config = ProcessHostConfig::new("worker");
            config.unload_timeout = duration;
            assert!(config.validate().is_err());
            let mut config = ProcessHostConfig::new("worker");
            config.shutdown_timeout = duration;
            assert!(config.validate().is_err());
        }
        assert!(ProcessHostConfig::new("worker").validate().is_ok());
        let mut config = ProcessHostConfig::new("worker");
        config.shutdown_timeout = Duration::from_secs(300);
        assert!(config.validate().is_ok());
    }
    #[test]
    fn atomic_cancel_retains_the_first_timestamp() {
        let cancel = Cancel::new();
        cancel.set();
        let first = cancel.first.load(Ordering::Acquire);
        thread::sleep(Duration::from_millis(3));
        cancel.set();
        assert_eq!(cancel.first.load(Ordering::Acquire), first);
        assert!(cancel.since().unwrap() >= Duration::from_millis(2));
    }
}
