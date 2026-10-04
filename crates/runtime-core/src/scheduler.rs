use crate::{
    CancellationHandle, EventReceiver, ExecutionEvents, Executor, ExecutorCommand, ExecutorEvent,
    ModelResolver, executor::Envelope, output::Output,
};
use runtime_types::*;
use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

type Reply<T> = mpsc::Sender<Result<T, RuntimeError>>;
#[derive(Clone)]
pub struct RuntimeHandle {
    sender: SyncSender<Command>,
}
/// Owns the scheduler thread. Drop requests graceful cancellation; it never kills
/// a native thread. Explicit shutdown waits for safe executor resource release.
pub struct Runtime {
    handle: RuntimeHandle,
    thread: Option<JoinHandle<()>>,
}
/// Exclusive registry mutation reservation granted atomically by the scheduler.
/// Keep this lease until copy/hash/registration AND partial-file cleanup finish.
/// Dropping it is nonblocking, so cancellation cannot deadlock on the mailbox.
pub struct RegistryLease {
    state: Arc<RegistryLeaseState>,
}
struct RegistryLeaseState {
    active: AtomicBool,
    cancelled: AtomicBool,
}
impl RegistryLease {
    pub fn cancellation_requested(&self) -> bool {
        self.state.cancelled.load(Ordering::Acquire)
    }
}
impl Drop for RegistryLease {
    fn drop(&mut self) {
        self.state.active.store(false, Ordering::Release);
    }
}
enum Command {
    Submit(GenerationRequest, Reply<EventReceiver>),
    Load(ModelId, LoadOptions, Reply<()>),
    LoadIfUnloaded(ModelId, LoadOptions, Reply<()>),
    SubmitIfIdle(GenerationRequest, LoadOptions, Reply<EventReceiver>),
    Unload(Reply<()>),
    Cancel(RequestId, Reply<()>),
    Status(Reply<RuntimeStatus>),
    ReserveRegistry(Reply<RegistryLease>),
    ReserveRegistryIfUnloaded(Reply<RegistryLease>),
    Shutdown(Option<Reply<()>>),
}
impl Runtime {
    pub fn spawn(
        config: RuntimeConfig,
        resolver: impl ModelResolver,
        executor: impl Executor,
    ) -> Result<Self, RuntimeError> {
        config.validate()?;
        let (sender, receiver) = mpsc::sync_channel(64);
        let (events, event_receiver) = mpsc::sync_channel(32);
        let handle = RuntimeHandle { sender };
        let thread = thread::Builder::new()
            .name("nexa-runtime".into())
            .spawn(move || {
                Actor::new(
                    config,
                    Box::new(resolver),
                    Box::new(executor),
                    receiver,
                    events,
                    event_receiver,
                )
                .run()
            })
            .map_err(|_| RuntimeError::new(ErrorCode::Io, "cannot create scheduler thread"))?;
        Ok(Self {
            handle,
            thread: Some(thread),
        })
    }
    pub fn handle(&self) -> RuntimeHandle {
        self.handle.clone()
    }
    /// Wait for cleanup and close the executor. Cleanup/close errors take
    /// precedence, then the first non-control failure observed during shutdown.
    /// An error therefore does not by itself mean cleanup was unconfirmed.
    pub fn shutdown(mut self) -> Result<(), RuntimeError> {
        let result = self.handle.shutdown();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        result
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        if self.thread.is_some() {
            let _ = self.handle.sender.send(Command::Shutdown(None));
        }
    }
}
impl RuntimeHandle {
    fn ask<T>(&self, build: impl FnOnce(Reply<T>) -> Command) -> Result<T, RuntimeError> {
        let (tx, rx) = mpsc::channel();
        self.sender.send(build(tx)).map_err(|_| stopped())?;
        rx.recv().map_err(|_| stopped())?
    }
    pub fn submit(&self, request: GenerationRequest) -> Result<EventReceiver, RuntimeError> {
        request.validate()?;
        self.ask(|reply| Command::Submit(request, reply))
    }
    pub fn load(&self, model: ModelId, options: LoadOptions) -> Result<(), RuntimeError> {
        options.validate()?;
        self.ask(|reply| Command::Load(model, options, reply))
    }
    /// Automatic onboarding must never evict a selection or wait behind work.
    pub fn load_if_unloaded(
        &self,
        model: ModelId,
        options: LoadOptions,
    ) -> Result<(), RuntimeError> {
        options.validate()?;
        self.ask(|reply| Command::LoadIfUnloaded(model, options, reply))
    }
    /// Atomically admit a private probe only for this already loaded scope.
    pub fn submit_if_idle(
        &self,
        request: GenerationRequest,
        options: LoadOptions,
    ) -> Result<EventReceiver, RuntimeError> {
        request.validate()?;
        self.ask(|reply| Command::SubmitIfIdle(request, options, reply))
    }
    pub fn unload(&self) -> Result<(), RuntimeError> {
        self.ask(Command::Unload)
    }
    pub fn cancel(&self, id: RequestId) -> Result<(), RuntimeError> {
        self.ask(|reply| Command::Cancel(id, reply))
    }
    pub fn status(&self) -> Result<RuntimeStatus, RuntimeError> {
        self.ask(Command::Status)
    }
    /// Reserve an idle registry transaction without running I/O in the actor.
    pub fn reserve_registry(&self) -> Result<RegistryLease, RuntimeError> {
        self.ask(Command::ReserveRegistry)
    }
    pub fn reserve_registry_if_unloaded(&self) -> Result<RegistryLease, RuntimeError> {
        self.ask(Command::ReserveRegistryIfUnloaded)
    }
    /// Request the same cleanup and error-reporting semantics as [`Runtime::shutdown`].
    pub fn shutdown(&self) -> Result<(), RuntimeError> {
        self.ask(|reply| Command::Shutdown(Some(reply)))
    }
}
fn stopped() -> RuntimeError {
    RuntimeError::new(ErrorCode::RuntimeShutdown, "runtime has shut down")
}
fn error(code: ErrorCode) -> RuntimeError {
    RuntimeError::new(code, code.as_str())
}
fn is_cancellation(code: ErrorCode) -> bool {
    matches!(
        code,
        ErrorCode::RequestCancelled
            | ErrorCode::ConsumerStopped
            | ErrorCode::SlowConsumer
            | ErrorCode::RuntimeShutdown
    )
}
fn is_control_termination(code: ErrorCode) -> bool {
    is_cancellation(code)
        || matches!(
            code,
            ErrorCode::QueueTimeout | ErrorCode::LoadTimeout | ErrorCode::ExecutionTimeout
        )
}
struct Job {
    request: GenerationRequest,
    output: Arc<Output>,
    seq: u64,
    enqueued: Instant,
    load_started: Option<Instant>,
    execution_started: Option<Instant>,
    timings: RequestTimings,
    usage: Usage,
    started: bool,
    waiting: bool,
}
impl Job {
    fn event(&mut self, kind: RequestEventKind, permit: Option<crate::TextPermit>) {
        self.seq += 1;
        self.output.publish(
            RequestEvent {
                request_id: self.request.request_id,
                seq: self.seq,
                kind,
            },
            permit,
        );
    }
    fn timings(&self) -> RequestTimings {
        let mut t = self.timings;
        if self.waiting {
            t.queue_ms = millis(self.enqueued.elapsed());
        }
        if let Some(start) = self.load_started {
            t.load_ms = millis(start.elapsed());
        }
        if let Some(start) = self.execution_started {
            t.execution_ms = millis(start.elapsed());
        }
        t
    }
    fn terminate(mut self, result: Result<(Usage, FinishReason), RuntimeError>) {
        if let Ok((usage, _)) = result {
            self.usage = usage;
        }
        let timings = self.timings();
        // Cancellation intent must not hide an actual executor failure. Keep
        // existing deadline/protocol causes and control-only cleanup ACKs: a PC
        // force-kill can legitimately report Faulted(RequestCancelled).
        let failed = result
            .as_ref()
            .is_err_and(|err| !is_control_termination(err.code));
        let reason = self
            .output
            .reason()
            .filter(|reason| !is_cancellation(*reason) || !failed);
        let kind = if let Some(reason) = reason {
            if !is_cancellation(reason) {
                RequestEventKind::Failed {
                    error: error(reason),
                    usage: self.usage,
                    timings,
                }
            } else {
                RequestEventKind::Cancelled {
                    reason,
                    usage: self.usage,
                    timings,
                }
            }
        } else {
            match result {
                Ok((usage, finish_reason)) => RequestEventKind::Completed {
                    usage,
                    finish_reason,
                    timings,
                },
                Err(error) => RequestEventKind::Failed {
                    error,
                    usage: self.usage,
                    timings,
                },
            }
        };
        self.event(kind, None);
    }
}
fn millis(d: Duration) -> u64 {
    d.as_millis().min(u64::MAX as u128) as u64
}
enum Operation {
    Load {
        start: Instant,
        cancel: CancellationHandle,
        reply: Option<Reply<()>>,
        timeout: bool,
        abandoned: bool,
    },
    Generate {
        start: Instant,
        cancel: CancellationHandle,
    },
    Unload {
        reply: Option<Reply<()>>,
        next: Option<(ResolvedModel, LoadOptions, Reply<()>)>,
        faulted: bool,
    },
}
struct Actor {
    config: RuntimeConfig,
    resolver: Box<dyn ModelResolver>,
    executor: Box<dyn Executor>,
    commands: Receiver<Command>,
    events: SyncSender<Envelope>,
    event_receiver: Receiver<Envelope>,
    state: ModelState,
    selected: Option<ResolvedModel>,
    options: Option<LoadOptions>,
    active: Option<Job>,
    queue: VecDeque<Job>,
    operation: Option<Operation>,
    operation_id: u64,
    last_error: Option<RuntimeError>,
    poison: Option<RuntimeError>,
    cleanup_error: Option<RuntimeError>,
    shutdown_error: Option<RuntimeError>,
    idle_since: Instant,
    stopping: bool,
    registry: Option<Arc<RegistryLeaseState>>,
    shutdown_replies: Vec<Reply<()>>,
}
impl Actor {
    fn new(
        config: RuntimeConfig,
        resolver: Box<dyn ModelResolver>,
        executor: Box<dyn Executor>,
        commands: Receiver<Command>,
        events: SyncSender<Envelope>,
        event_receiver: Receiver<Envelope>,
    ) -> Self {
        Self {
            config,
            resolver,
            executor,
            commands,
            events,
            event_receiver,
            state: ModelState::Unloaded,
            selected: None,
            options: None,
            active: None,
            queue: VecDeque::new(),
            operation: None,
            operation_id: 0,
            last_error: None,
            poison: None,
            cleanup_error: None,
            shutdown_error: None,
            idle_since: Instant::now(),
            stopping: false,
            registry: None,
            shutdown_replies: Vec::new(),
        }
    }
    fn run(mut self) {
        loop {
            for _ in 0..32 {
                match self.commands.try_recv() {
                    Ok(command) => self.command(command),
                    Err(mpsc::TryRecvError::Disconnected) => {
                        self.begin_shutdown(None);
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                }
            }
            for _ in 0..32 {
                match self.event_receiver.try_recv() {
                    Ok(event) => self.executor_event(event),
                    Err(_) => break,
                }
            }
            self.tick();
            if self.stopping
                && !self.registry_busy()
                && self.operation.is_none()
                && self.active.is_none()
                && matches!(self.state, ModelState::Unloaded | ModelState::Faulted)
            {
                let closed = self.executor.close();
                let result = self.cleanup_error.clone().map_or_else(
                    || closed.and_then(|()| self.shutdown_error.clone().map_or(Ok(()), Err)),
                    Err,
                );
                for reply in self.shutdown_replies.drain(..) {
                    let _ = reply.send(result.clone());
                }
                break;
            }
            match self.commands.recv_timeout(Duration::from_millis(2)) {
                Ok(command) => self.command(command),
                Err(mpsc::RecvTimeoutError::Disconnected) => self.begin_shutdown(None),
                Err(_) => {}
            }
        }
    }
    fn status(&self) -> RuntimeStatus {
        RuntimeStatus {
            state: self.state,
            selected_model: self.selected.as_ref().map(|m| m.id.clone()),
            load_options: self.options,
            active_request: self.active.as_ref().map(|j| j.request.request_id),
            queued_jobs: self.queue.len(),
            stopping: self.stopping,
            registry_busy: self.registry_busy(),
            last_error: self.last_error.clone(),
        }
    }
    fn command(&mut self, command: Command) {
        match command {
            Command::ReserveRegistryIfUnloaded(reply) => {
                if self.state != ModelState::Unloaded
                    || self.selected.is_some()
                    || self.busy_error().is_some()
                {
                    let _ = reply.send(Err(error(ErrorCode::RuntimeBusy)));
                } else {
                    self.command(Command::ReserveRegistry(reply));
                }
            }
            Command::ReserveRegistry(reply) => {
                let result = if let Some(err) = self.busy_error() {
                    Err(err)
                } else {
                    let state = Arc::new(RegistryLeaseState {
                        active: AtomicBool::new(true),
                        cancelled: AtomicBool::new(false),
                    });
                    self.registry = Some(state.clone());
                    Ok(RegistryLease { state })
                };
                let _ = reply.send(result);
            }
            Command::Status(reply) => {
                let _ = reply.send(Ok(self.status()));
            }
            Command::Submit(request, reply) => {
                let result = self.submit(request);
                let _ = reply.send(result);
            }
            Command::Load(id, options, reply) => self.explicit_load(id, options, reply),
            Command::LoadIfUnloaded(id, options, reply) => {
                if self.state != ModelState::Unloaded
                    || self.selected.is_some()
                    || self.busy_error().is_some()
                {
                    let _ = reply.send(Err(error(ErrorCode::RuntimeBusy)));
                } else {
                    self.explicit_load(id, options, reply);
                }
            }
            Command::SubmitIfIdle(request, options, reply) => {
                let result = if self.state != ModelState::Ready
                    || self.busy_error().is_some()
                    || self.options != Some(options)
                    || self
                        .selected
                        .as_ref()
                        .is_none_or(|model| model.id != request.model)
                {
                    Err(error(ErrorCode::RuntimeBusy))
                } else {
                    self.submit(request)
                };
                let _ = reply.send(result);
            }
            Command::Unload(reply) => {
                if let Some(err) = self.busy_error() {
                    let _ = reply.send(Err(err));
                } else if self.state == ModelState::Unloaded {
                    let _ = reply.send(Ok(()));
                } else if self.state == ModelState::Faulted {
                    let _ = reply.send(Err(error(ErrorCode::RuntimeFaulted)));
                } else {
                    self.unload(Some(reply), None);
                }
            }
            Command::Cancel(id, reply) => {
                let result = self.cancel(id, ErrorCode::RequestCancelled);
                let _ = reply.send(result);
            }
            Command::Shutdown(reply) => self.begin_shutdown(reply),
        }
    }
    fn registry_busy(&self) -> bool {
        self.registry
            .as_ref()
            .is_some_and(|lease| lease.active.load(Ordering::Acquire))
    }
    fn busy_error(&self) -> Option<RuntimeError> {
        if self.stopping {
            Some(stopped())
        } else if self.active.is_some()
            || !self.queue.is_empty()
            || self.operation.is_some()
            || self.registry_busy()
        {
            Some(error(ErrorCode::RuntimeBusy))
        } else {
            None
        }
    }
    fn resolve(&self, id: &ModelId, options: LoadOptions) -> Result<ResolvedModel, RuntimeError> {
        let model = self.resolver.resolve(id)?;
        if model.id != *id || !model.loadable {
            return Err(error(ErrorCode::UnsupportedModel));
        }
        if options.context_size > model.context_limit {
            return Err(RuntimeError::new(
                ErrorCode::ContextLengthExceeded,
                "load context exceeds the model context limit",
            ));
        }
        Ok(model)
    }
    fn submit(&mut self, request: GenerationRequest) -> Result<EventReceiver, RuntimeError> {
        if self.stopping {
            return Err(stopped());
        }
        if self.registry_busy()
            || matches!(
                self.operation,
                Some(Operation::Unload { next: Some(_), .. })
            )
        {
            return Err(error(ErrorCode::RuntimeBusy));
        }
        if self.state == ModelState::Faulted
            || matches!(
                self.operation,
                Some(Operation::Unload { faulted: true, .. })
            )
        {
            return Err(error(ErrorCode::RuntimeFaulted));
        }
        if self
            .active
            .as_ref()
            .is_some_and(|j| j.request.request_id == request.request_id)
            || self
                .queue
                .iter()
                .any(|j| j.request.request_id == request.request_id)
        {
            return Err(error(ErrorCode::DuplicateRequestId));
        }
        if self
            .selected
            .as_ref()
            .is_some_and(|m| m.id != request.model)
        {
            return Err(error(ErrorCode::ModelConflict));
        }
        if self.active.is_some() && self.queue.len() >= self.config.max_queued_jobs {
            return Err(error(ErrorCode::QueueFull));
        }
        if self.selected.is_none() {
            let options = self.config.load_options;
            let model = self.resolve(&request.model, options)?;
            self.selected = Some(model);
            self.options = Some(options);
        }
        let output = Output::new(self.config.slow_consumer_timeout);
        let mut job = Job {
            request,
            output: output.clone(),
            seq: 0,
            enqueued: Instant::now(),
            load_started: None,
            execution_started: None,
            timings: RequestTimings::default(),
            usage: Usage::default(),
            started: false,
            waiting: false,
        };
        job.event(RequestEventKind::Accepted, None);
        if self.active.is_some() {
            job.waiting = true;
            job.event(RequestEventKind::Queued, None);
            self.queue.push_back(job);
        } else {
            if matches!(self.operation, Some(Operation::Load { .. })) {
                job.load_started = Some(Instant::now());
                job.event(RequestEventKind::Loading, None);
            }
            self.active = Some(job);
            self.start_active();
        }
        Ok(EventReceiver { output })
    }
    fn explicit_load(&mut self, id: ModelId, options: LoadOptions, reply: Reply<()>) {
        if let Some(error) = &self.cleanup_error {
            let _ = reply.send(Err(error.clone()));
            return;
        }
        if let Some(err) = self.busy_error() {
            let _ = reply.send(Err(err));
            return;
        }
        let model = match self.resolve(&id, options) {
            Ok(model) => model,
            Err(err) => {
                let _ = reply.send(Err(err));
                return;
            }
        };
        if self.state == ModelState::Ready
            && self.selected.as_ref().is_some_and(|m| m.id == id)
            && self.options == Some(options)
        {
            let _ = reply.send(Ok(()));
            return;
        }
        if matches!(self.state, ModelState::Ready | ModelState::Faulted) {
            self.unload(None, Some((model, options, reply)));
        } else {
            self.selected = Some(model);
            self.options = Some(options);
            self.start_load(Some(reply));
        }
    }
    fn sink(&mut self, output: Option<Arc<Output>>) -> ExecutionEvents {
        self.operation_id += 1;
        ExecutionEvents::for_actor(self.operation_id, self.events.clone(), output)
    }
    fn start_load(&mut self, reply: Option<Reply<()>>) {
        self.state = ModelState::Loading;
        self.last_error = None;
        if let Some(job) = &mut self.active {
            job.load_started = Some(Instant::now());
            job.event(RequestEventKind::Loading, None);
        }
        // Selection remembers identity/options, not an everlasting validation.
        // Re-resolve at EVERY real load, including idle unload/reload and the
        // post-Unload half of explicit recovery. The resolver only checks bounded
        // metadata/fingerprints here; full verification belongs outside the actor.
        let options = self.options.unwrap();
        let id = self.selected.as_ref().unwrap().id.clone();
        let model = match self.resolve(&id, options) {
            Ok(model) => model,
            Err(error) => {
                if let Some(reply) = reply {
                    let _ = reply.send(Err(error.clone()));
                }
                self.fault(error);
                return;
            }
        };
        self.selected = Some(model.clone());
        let command = ExecutorCommand::Load { model, options };
        let sink = self.sink(None);
        match self.executor.start(command, sink) {
            Ok(cancel) => {
                self.operation = Some(Operation::Load {
                    start: Instant::now(),
                    cancel,
                    reply,
                    timeout: false,
                    abandoned: false,
                })
            }
            Err(err) => {
                if let Some(reply) = reply {
                    let _ = reply.send(Err(err.clone()));
                }
                self.fault(err);
            }
        }
    }
    fn start_active(&mut self) {
        if self.operation.is_some() {
            return;
        }
        match self.state {
            ModelState::Unloaded => self.start_load(None),
            ModelState::Ready => {
                let job = self.active.as_mut().unwrap();
                job.execution_started = Some(Instant::now());
                let command = ExecutorCommand::Generate {
                    request: job.request.clone(),
                };
                let output = job.output.clone();
                let sink = self.sink(Some(output));
                self.state = ModelState::Generating;
                match self.executor.start(command, sink) {
                    Ok(cancel) => {
                        self.operation = Some(Operation::Generate {
                            start: Instant::now(),
                            cancel,
                        })
                    }
                    Err(err) => self.fault(err),
                }
            }
            _ => {}
        }
    }
    fn unload(
        &mut self,
        reply: Option<Reply<()>>,
        next: Option<(ResolvedModel, LoadOptions, Reply<()>)>,
    ) {
        let faulted = self.state == ModelState::Faulted;
        self.state = ModelState::Unloading;
        let sink = self.sink(None);
        match self.executor.start(ExecutorCommand::Unload, sink) {
            Ok(_) => {
                self.operation = Some(Operation::Unload {
                    reply,
                    next,
                    faulted,
                })
            }
            Err(err) => {
                if let Some(reply) = reply {
                    let _ = reply.send(Err(err.clone()));
                }
                if let Some((_, _, reply)) = next {
                    let _ = reply.send(Err(err.clone()));
                }
                self.fault(err);
            }
        }
    }
    fn cancel(&mut self, id: RequestId, reason: ErrorCode) -> Result<(), RuntimeError> {
        if let Some(index) = self.queue.iter().position(|j| j.request.request_id == id) {
            let job = self.queue.remove(index).unwrap();
            job.output.cancel(reason);
            job.terminate(Err(error(reason)));
            return Ok(());
        }
        if self
            .active
            .as_ref()
            .is_some_and(|j| j.request.request_id == id)
        {
            self.active.as_ref().unwrap().output.cancel(reason);
            self.cancel_active();
            return Ok(());
        }
        Err(error(ErrorCode::RequestNotFound))
    }
    fn cancel_active(&mut self) {
        if let Some(Operation::Generate { cancel, .. }) = &self.operation {
            cancel.cancel();
            return;
        }
        if matches!(self.operation, Some(Operation::Load { .. })) {
            if let Some(job) = self.active.take() {
                job.terminate(Err(error(ErrorCode::RequestCancelled)));
            }
            self.promote_during_load();
            if self.active.is_none()
                && let Some(Operation::Load {
                    cancel,
                    reply,
                    abandoned,
                    ..
                }) = &mut self.operation
                && reply.is_none()
            {
                *abandoned = true;
                cancel.cancel();
            }
            return;
        }
        if let Some(job) = self.active.take() {
            job.terminate(Err(error(ErrorCode::RequestCancelled)));
        }
        self.next_job();
    }
    fn pop_eligible(&mut self) -> Option<Job> {
        while let Some(job) = self.queue.pop_front() {
            if let Some(reason) = job.output.reason() {
                job.terminate(Err(error(reason)));
            } else if job.enqueued.elapsed() >= self.config.queue_timeout {
                job.terminate(Err(error(ErrorCode::QueueTimeout)));
            } else {
                return Some(job);
            }
        }
        None
    }
    fn promote_during_load(&mut self) {
        if let Some(mut job) = self.pop_eligible() {
            job.timings.queue_ms = millis(job.enqueued.elapsed());
            job.waiting = false;
            job.load_started = Some(Instant::now());
            job.event(RequestEventKind::Loading, None);
            self.active = Some(job);
        }
    }
    fn next_job(&mut self) {
        self.idle_since = Instant::now();
        if let Some(mut job) = self.pop_eligible() {
            job.timings.queue_ms = millis(job.enqueued.elapsed());
            job.waiting = false;
            self.active = Some(job);
            self.start_active();
        }
    }
    fn executor_event(&mut self, mut envelope: Envelope) {
        // A parent-local containment failure disables this executor even if its
        // last callback scope has just retired. Never treat it as a stale ACK.
        if let ExecutorEvent::CleanupUnconfirmed(error) = &envelope.event {
            self.cleanup_unconfirmed(error.clone());
            return;
        }
        if self.cleanup_error.is_some() {
            return;
        }
        if envelope.operation != self.operation_id {
            return;
        }
        if self.poison.is_some() {
            let ended = matches!(
                (&self.operation, &envelope.event),
                (
                    Some(Operation::Load { .. }),
                    ExecutorEvent::Loaded | ExecutorEvent::Failed(_)
                ) | (
                    Some(Operation::Generate { .. }),
                    ExecutorEvent::Completed { .. }
                        | ExecutorEvent::GenerationFailed { .. }
                        | ExecutorEvent::Failed(_)
                ) | (
                    Some(Operation::Unload { .. }),
                    ExecutorEvent::Unloaded | ExecutorEvent::Failed(_)
                )
            ) || matches!(envelope.event, ExecutorEvent::Faulted(_));
            if ended {
                let loaded = matches!(envelope.event, ExecutorEvent::Loaded);
                let err = self.poison.take().unwrap();
                self.fault(err);
                if loaded {
                    self.unload(None, None);
                }
            }
            return;
        }
        match &mut self.operation {
            Some(Operation::Load { start, timeout, .. })
                if envelope.emitted_at.saturating_duration_since(*start)
                    >= self.config.load_timeout =>
            {
                *timeout = true
            }
            Some(Operation::Generate { start, .. })
                if envelope.emitted_at.saturating_duration_since(*start)
                    >= self.config.execution_timeout =>
            {
                if let Some(job) = &self.active {
                    job.output.cancel(ErrorCode::ExecutionTimeout);
                }
            }
            _ => {}
        }
        match envelope.event {
            ExecutorEvent::Loaded => {
                let Some(Operation::Load {
                    reply,
                    timeout,
                    abandoned,
                    ..
                }) = self.operation.take()
                else {
                    self.protocol_fault();
                    return;
                };
                if timeout {
                    if let Some(reply) = reply {
                        let _ = reply.send(Err(error(ErrorCode::LoadTimeout)));
                    }
                    self.fault(error(ErrorCode::LoadTimeout));
                    self.unload(None, None);
                    return;
                }
                if let Some(reply) = reply {
                    let _ = reply.send(Ok(()));
                }
                self.state = ModelState::Ready;
                if let Some(job) = &mut self.active
                    && let Some(start) = job.load_started.take()
                {
                    job.timings.load_ms = millis(start.elapsed());
                }
                if self.active.is_some() {
                    self.start_active();
                } else {
                    self.idle_since = Instant::now();
                    if abandoned || self.stopping {
                        self.unload(None, None);
                    }
                }
            }
            ExecutorEvent::Prepared { prompt_tokens } => {
                if !matches!(self.operation, Some(Operation::Generate { .. })) {
                    self.protocol_fault();
                    return;
                }
                if let Some(job) = &mut self.active {
                    if job.started {
                        self.protocol_fault();
                        return;
                    }
                    job.started = true;
                    job.usage.prompt_tokens = prompt_tokens;
                    if job.output.reason().is_none() {
                        job.event(RequestEventKind::Started { prompt_tokens }, None);
                    }
                }
            }
            ExecutorEvent::TextDelta(text) => {
                if let Some(job) = &mut self.active {
                    if !job.started {
                        self.protocol_fault();
                        return;
                    }
                    if job.output.reason().is_none() {
                        job.event(RequestEventKind::TextDelta(text), envelope.permit.take());
                    }
                }
            }
            ExecutorEvent::Completed {
                usage,
                finish_reason,
            } => {
                if !matches!(self.operation, Some(Operation::Generate { .. }))
                    || self.active.as_ref().is_none_or(|j| !j.started)
                {
                    self.fault(error(ErrorCode::NativeProtocol));
                    return;
                }
                self.operation = None;
                if let Some(job) = self.active.take() {
                    job.terminate(Ok((usage, finish_reason)));
                }
                self.state = ModelState::Ready;
                self.next_job();
            }
            ExecutorEvent::Failed(err) => self.operation_failed(err),
            ExecutorEvent::GenerationFailed { error, usage } => {
                if let Some(job) = &mut self.active {
                    job.usage = usage;
                }
                self.operation_failed(error);
            }
            ExecutorEvent::Unloaded => {
                let Some(Operation::Unload {
                    reply,
                    next,
                    faulted,
                }) = self.operation.take()
                else {
                    self.protocol_fault();
                    return;
                };
                self.state = if faulted {
                    ModelState::Faulted
                } else {
                    ModelState::Unloaded
                };
                if let Some(reply) = reply {
                    let _ = reply.send(Ok(()));
                }
                if let Some((model, options, reply)) = next {
                    if self.stopping {
                        let _ = reply.send(Err(stopped()));
                        return;
                    }
                    self.selected = Some(model);
                    self.options = Some(options);
                    self.start_load(Some(reply));
                } else if self.active.is_some() {
                    self.start_active();
                }
            }
            ExecutorEvent::Faulted(err) => self.fault(err),
            ExecutorEvent::CleanupUnconfirmed(_) => {
                unreachable!("handled before operation matching")
            }
        }
    }
    fn cleanup_unconfirmed(&mut self, _diagnostic: RuntimeError) {
        if self.cleanup_error.is_some() {
            return;
        }
        let cause = self
            .active
            .as_ref()
            .and_then(|job| job.output.reason())
            .or({
                if self.stopping {
                    Some(ErrorCode::RuntimeShutdown)
                } else if matches!(self.operation, Some(Operation::Load { timeout: true, .. })) {
                    Some(ErrorCode::LoadTimeout)
                } else {
                    None
                }
            });
        let message = match cause {
            Some(cause) => format!(
                "executor cleanup could not be confirmed; original cause: {}",
                cause.as_str()
            ),
            None => "executor cleanup could not be confirmed".into(),
        };
        let error = RuntimeError::new(ErrorCode::ExecutorCleanupUnconfirmed, message);
        self.cleanup_error = Some(error.clone());
        self.poison = None;
        if let Some(operation) = self.operation.take() {
            match operation {
                Operation::Load { reply, .. } => {
                    if let Some(reply) = reply {
                        let _ = reply.send(Err(error.clone()));
                    }
                }
                Operation::Unload { reply, next, .. } => {
                    if let Some(reply) = reply {
                        let _ = reply.send(Err(error.clone()));
                    }
                    if let Some((_, _, reply)) = next {
                        let _ = reply.send(Err(error.clone()));
                    }
                }
                Operation::Generate { .. } => {}
            }
        }
        self.state = ModelState::Faulted;
        self.last_error = Some(error.clone());
        // Unconfirmed cleanup outranks all cancellation/deadline causes and
        // earlier failures, independently of the ordinary termination policy.
        for mut job in self.active.take().into_iter().chain(self.queue.drain(..)) {
            let timings = job.timings();
            let usage = job.usage;
            job.event(
                RequestEventKind::Failed {
                    error: error.clone(),
                    usage,
                    timings,
                },
                None,
            );
        }
    }
    fn operation_failed(&mut self, err: RuntimeError) {
        match self.operation.take() {
            Some(Operation::Load {
                reply,
                timeout,
                abandoned,
                ..
            }) => {
                let err = if timeout {
                    error(ErrorCode::LoadTimeout)
                } else {
                    err
                };
                if let Some(reply) = reply {
                    let _ = reply.send(Err(err.clone()));
                }
                if abandoned && !timeout && is_control_termination(err.code) {
                    self.state = ModelState::Unloaded;
                    if self.active.is_some() {
                        self.start_active();
                    } else {
                        self.next_job();
                    }
                } else {
                    self.fault(err);
                }
            }
            Some(Operation::Generate { .. }) => {
                self.remember_shutdown_error(&err);
                if let Some(job) = self.active.take() {
                    job.terminate(Err(err));
                }
                self.state = ModelState::Ready;
                self.next_job();
            }
            Some(Operation::Unload { reply, next, .. }) => {
                if let Some(reply) = reply {
                    let _ = reply.send(Err(err.clone()));
                }
                if let Some((_, _, reply)) = next {
                    let _ = reply.send(Err(err.clone()));
                }
                self.fault(err);
            }
            None => self.fault(err),
        }
    }
    fn protocol_fault(&mut self) {
        let err = error(ErrorCode::NativeProtocol);
        if self.operation.is_none() {
            self.fault(err);
            return;
        }
        self.state = ModelState::Faulted;
        self.last_error = Some(err.clone());
        self.poison = Some(err.clone());
        if let Some(job) = &self.active {
            job.output.cancel(ErrorCode::NativeProtocol);
        }
        for job in self.queue.drain(..) {
            job.terminate(Err(err.clone()));
        }
        if let Some(Operation::Load { cancel, .. } | Operation::Generate { cancel, .. }) =
            &self.operation
        {
            cancel.cancel();
        }
    }
    fn remember_shutdown_error(&mut self, err: &RuntimeError) {
        // This shutdown may already have ended a loading request, leaving no
        // stream on which to report a later real failure. Do not pull in an
        // unrelated historical fault or treat a cancellation ACK as a failure.
        if self.stopping && !is_control_termination(err.code) {
            self.shutdown_error.get_or_insert_with(|| err.clone());
        }
    }
    fn fault(&mut self, mut err: RuntimeError) {
        // A transport sees only the cancellation flag for Load. Preserve the
        // actor's original deadline cause after the child has been reaped.
        if matches!(self.operation, Some(Operation::Load { timeout: true, .. })) {
            err = error(ErrorCode::LoadTimeout);
        }
        self.remember_shutdown_error(&err);
        self.poison = None;
        if let Some(operation) = self.operation.take() {
            match operation {
                Operation::Load { cancel, reply, .. } => {
                    cancel.cancel();
                    if let Some(reply) = reply {
                        let _ = reply.send(Err(err.clone()));
                    }
                }
                Operation::Generate { cancel, .. } => cancel.cancel(),
                Operation::Unload { reply, next, .. } => {
                    if let Some(reply) = reply {
                        let _ = reply.send(Err(err.clone()));
                    }
                    if let Some((_, _, reply)) = next {
                        let _ = reply.send(Err(err.clone()));
                    }
                }
            }
        }
        self.state = ModelState::Faulted;
        self.last_error = Some(err.clone());
        if let Some(job) = self.active.take() {
            job.terminate(Err(err.clone()));
        }
        for job in self.queue.drain(..) {
            job.terminate(Err(err.clone()));
        }
    }
    fn begin_shutdown(&mut self, reply: Option<Reply<()>>) {
        if let Some(reply) = reply {
            self.shutdown_replies.push(reply);
        }
        if self.stopping {
            return;
        }
        self.stopping = true;
        if let Some(lease) = &self.registry {
            lease.cancelled.store(true, Ordering::Release);
        }
        for job in self.queue.drain(..) {
            job.output.cancel(ErrorCode::RuntimeShutdown);
            job.terminate(Err(stopped()));
        }
        if let Some(job) = &self.active {
            job.output.cancel(ErrorCode::RuntimeShutdown);
            self.cancel_active();
        }
        if let Some(Operation::Load { cancel, .. }) = &self.operation {
            cancel.cancel();
        }
    }
    fn tick(&mut self) {
        if !self.registry_busy() {
            self.registry = None;
        }
        let mut retained = VecDeque::new();
        while let Some(job) = self.queue.pop_front() {
            if let Some(reason) = job.output.reason() {
                job.terminate(Err(error(reason)));
            } else if job.enqueued.elapsed() >= self.config.queue_timeout {
                job.terminate(Err(error(ErrorCode::QueueTimeout)));
            } else {
                retained.push_back(job);
            }
        }
        self.queue = retained;
        if self
            .active
            .as_ref()
            .is_some_and(|j| j.output.reason().is_some())
        {
            self.cancel_active();
        }
        match &mut self.operation {
            Some(Operation::Load {
                start,
                cancel,
                timeout,
                ..
            }) if start.elapsed() >= self.config.load_timeout && !*timeout => {
                *timeout = true;
                cancel.cancel();
            }
            Some(Operation::Generate { start, cancel, .. })
                if start.elapsed() >= self.config.execution_timeout =>
            {
                if let Some(job) = &self.active {
                    job.output.cancel(ErrorCode::ExecutionTimeout);
                }
                cancel.cancel();
            }
            _ => {}
        }
        if self.operation.is_none()
            && !self.registry_busy()
            && self.active.is_none()
            && self.queue.is_empty()
            && self.state == ModelState::Ready
            && (self.stopping || self.idle_since.elapsed() >= self.config.idle_unload)
        {
            self.unload(None, None);
        }
    }
}

#[cfg(test)]
mod ledger_tests {
    use super::*;
    struct Noop;
    impl Executor for Noop {
        fn start(
            &mut self,
            _: ExecutorCommand,
            _: ExecutionEvents,
        ) -> Result<CancellationHandle, RuntimeError> {
            Ok(CancellationHandle::noop())
        }
    }
    #[test]
    fn termination_priority_distinguishes_real_errors_from_control_acknowledgments() {
        let cancellations = [
            ErrorCode::RequestCancelled,
            ErrorCode::ConsumerStopped,
            ErrorCode::SlowConsumer,
            ErrorCode::RuntimeShutdown,
        ];
        let deadlines = [
            ErrorCode::QueueTimeout,
            ErrorCode::LoadTimeout,
            ErrorCode::ExecutionTimeout,
        ];
        let real_errors = [
            ErrorCode::NativeFailure,
            ErrorCode::NativeProtocol,
            ErrorCode::ExecutorUnavailable,
            ErrorCode::ContextLengthExceeded,
        ];
        for reason in cancellations
            .into_iter()
            .chain(deadlines)
            .chain([ErrorCode::NativeProtocol])
        {
            for failure in cancellations
                .into_iter()
                .chain(deadlines)
                .chain(real_errors)
                .map(Some)
                .chain([None])
            {
                let output = Output::new(Duration::from_secs(10));
                output.cancel(reason);
                let receiver = EventReceiver {
                    output: output.clone(),
                };
                let job = Job {
                    request: GenerationRequest {
                        request_id: RequestId::new(),
                        model: ModelId::new("qa-small").unwrap(),
                        messages: vec![Message::new(Role::User, "synthetic")],
                        options: GenerationOptions::default(),
                    },
                    output,
                    seq: 0,
                    enqueued: Instant::now(),
                    load_started: None,
                    execution_started: None,
                    timings: RequestTimings::default(),
                    usage: Usage::default(),
                    started: false,
                    waiting: false,
                };
                job.terminate(
                    failure.map_or(Ok((Usage::default(), FinishReason::Stop)), |code| {
                        Err(error(code))
                    }),
                );
                let terminal = receiver.recv().unwrap();
                let real_failure = failure.filter(|code| real_errors.contains(code));
                if cancellations.contains(&reason) && real_failure.is_some() {
                    assert!(
                        matches!(terminal.kind, RequestEventKind::Failed { error, .. }
                        if Some(error.code) == real_failure)
                    );
                } else if cancellations.contains(&reason) {
                    assert!(
                        matches!(terminal.kind, RequestEventKind::Cancelled { reason: actual, .. }
                        if actual == reason)
                    );
                } else {
                    assert!(
                        matches!(terminal.kind, RequestEventKind::Failed { error, .. }
                        if error.code == reason)
                    );
                }
                assert!(receiver.recv().is_none());
            }
        }
    }
    #[test]
    fn vanished_reservation_receiver_releases_its_grant() {
        let (_tx, rx) = mpsc::sync_channel(1);
        let (events, event_receiver) = mpsc::sync_channel(32);
        let resolver = |_id: &ModelId| Err(error(ErrorCode::ModelNotFound));
        let mut actor = Actor::new(
            RuntimeConfig::default(),
            Box::new(resolver),
            Box::new(Noop),
            rx,
            events,
            event_receiver,
        );
        let (reply, receiver) = mpsc::channel();
        drop(receiver);
        actor.command(Command::ReserveRegistry(reply));
        assert!(!actor.registry_busy());
        actor.tick();
        assert!(actor.registry.is_none());
    }
    #[test]
    fn stale_delta_releases_its_own_ledger_not_new_request() {
        let (_tx, rx) = mpsc::sync_channel(1);
        let (events, event_receiver) = mpsc::sync_channel(32);
        let resolver = |id: &ModelId| {
            Ok(ResolvedModel {
                id: id.clone(),
                path: "fake.gguf".into(),
                context_limit: 4096,
                default_context: 4096,
                loadable: true,
            })
        };
        let mut actor = Actor::new(
            RuntimeConfig::default(),
            Box::new(resolver),
            Box::new(Noop),
            rx,
            events,
            event_receiver,
        );
        let old = Output::new(Duration::from_secs(10));
        let old_sink = actor.sink(Some(old.clone()));
        assert!(old_sink.text_delta("old"));
        let stale = actor.event_receiver.recv().unwrap();
        let new = Output::new(Duration::from_secs(10));
        let new_receiver = EventReceiver {
            output: new.clone(),
        };
        let old_receiver = EventReceiver { output: old };
        let new_sink = actor.sink(Some(new));
        assert!(new_sink.text_delta("new"));
        let before = new_receiver.buffered_bytes();
        assert!(before > 0);
        assert!(old_receiver.buffered_bytes() > 0);
        actor.executor_event(stale);
        assert_eq!(old_receiver.buffered_bytes(), 0);
        assert_eq!(new_receiver.buffered_bytes(), before);
    }
    #[test]
    fn a11_new_arrival_during_idle_unload_waits_and_reloads() {
        let (_tx, rx) = mpsc::sync_channel(1);
        let (events, event_receiver) = mpsc::sync_channel(32);
        let resolver = |id: &ModelId| {
            Ok(ResolvedModel {
                id: id.clone(),
                path: "fake.gguf".into(),
                context_limit: 4096,
                default_context: 4096,
                loadable: true,
            })
        };
        let config = RuntimeConfig {
            idle_unload: Duration::from_millis(1),
            ..RuntimeConfig::default()
        };
        let mut actor = Actor::new(
            config,
            Box::new(resolver),
            Box::new(Noop),
            rx,
            events,
            event_receiver,
        );
        let request = || GenerationRequest {
            request_id: RequestId::new(),
            model: ModelId::new("qa-small").unwrap(),
            messages: vec![Message::new(Role::User, "synthetic")],
            options: GenerationOptions::default(),
        };
        fn feed(actor: &mut Actor, event: ExecutorEvent) {
            actor.executor_event(Envelope {
                operation: actor.operation_id,
                event,
                permit: None,
                emitted_at: Instant::now(),
            });
        }
        let _first = actor.submit(request()).unwrap();
        feed(&mut actor, ExecutorEvent::Loaded);
        feed(&mut actor, ExecutorEvent::Prepared { prompt_tokens: 8 });
        feed(
            &mut actor,
            ExecutorEvent::Completed {
                usage: Usage::default(),
                finish_reason: FinishReason::Stop,
            },
        );
        actor.idle_since = Instant::now() - Duration::from_millis(2);
        actor.tick();
        assert_eq!(actor.state, ModelState::Unloading);
        let _second = actor.submit(request()).unwrap();
        assert_eq!(actor.state, ModelState::Unloading);
        assert!(actor.active.is_some());
        feed(&mut actor, ExecutorEvent::Unloaded);
        assert_eq!(actor.state, ModelState::Loading);
        feed(&mut actor, ExecutorEvent::Loaded);
        assert_eq!(actor.state, ModelState::Generating);
        feed(&mut actor, ExecutorEvent::Prepared { prompt_tokens: 8 });
        feed(
            &mut actor,
            ExecutorEvent::Completed {
                usage: Usage::default(),
                finish_reason: FinishReason::Stop,
            },
        );
        assert_eq!(actor.state, ModelState::Ready);
    }
}
