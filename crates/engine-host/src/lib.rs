//! Thread-affine native executor for Nexa's platform-neutral scheduler.
//!
//! Commands contain owned Rust data. Engine, model, prepared request, context,
//! and sampler are created and destroyed exclusively on `nexa-inference`.
//! Only the isolated one-shot native cancellation flag crosses that boundary.
//! The scheduler's shared byte budget is the sole pending-text buffer.

use llama_adapter::{
    CancelHandle, Engine, GeneratedDelta, GenerationPhase, GenerationProgress, Model, StreamControl,
};
use runtime_core::{CancellationHandle, ExecutionEvents, Executor, ExecutorCommand, ExecutorEvent};
use runtime_types::{ErrorCode, GenerationRequest, InferenceTimings, RuntimeError, Usage};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::mpsc::{self, Receiver, SyncSender, TrySendError},
    thread,
    time::{Duration, Instant},
};

/// Best-effort numeric diagnostics, never a second public lifecycle stream.
/// Reports omit paths, request messages, and generated text. The diagnostics
/// queue is bounded to 16 reports and never blocks inference when it is full.
#[derive(Clone, Debug)]
pub struct ExecutionReport {
    pub operation: Operation,
    pub load: Duration,
    pub prepare: Duration,
    pub prefill: Duration,
    pub decode: Duration,
    pub usage: Usage,
    pub error: Option<ErrorCode>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    Load,
    Generate,
    Unload,
}

struct Job {
    command: ExecutorCommand,
    events: ExecutionEvents,
    cancel: CancelHandle,
}

/// One bounded command mailbox and one native inference thread.
/// Dropping the host requests cancellation and closes the mailbox. A native
/// call must return before its thread releases resources; T02 never kills it.
/// Call runtime shutdown/unload for an acknowledged resource-release boundary.
pub struct EngineHost {
    sender: Option<SyncSender<Job>>,
    active_cancel: Option<CancelHandle>,
    diagnostics: Option<Receiver<ExecutionReport>>,
}

impl EngineHost {
    pub fn new() -> Result<Self, RuntimeError> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let (reports, diagnostics) = mpsc::sync_channel(16);
        thread::Builder::new()
            .name("nexa-inference".into())
            .spawn(move || inference_thread(receiver, reports))
            .map_err(|_| {
                RuntimeError::new(
                    ErrorCode::ExecutorUnavailable,
                    "cannot start inference thread",
                )
            })?;
        Ok(Self {
            sender: Some(sender),
            active_cancel: None,
            diagnostics: Some(diagnostics),
        })
    }

    /// Must be taken before passing the host to Runtime. Missing observations
    /// do not mean that an operation did not run: delivery is best effort.
    pub fn take_diagnostics(&mut self) -> Option<Receiver<ExecutionReport>> {
        self.diagnostics.take()
    }
}

impl Executor for EngineHost {
    fn start(
        &mut self,
        command: ExecutorCommand,
        events: ExecutionEvents,
    ) -> Result<CancellationHandle, RuntimeError> {
        let cancel = CancelHandle::new()?;
        let control = cancel.clone();
        let job = Job {
            command,
            events,
            cancel: cancel.clone(),
        };
        let sender = self
            .sender
            .as_ref()
            .ok_or_else(|| unavailable("inference mailbox is closed"))?;
        match sender.try_send(job) {
            Ok(()) => {
                self.active_cancel = Some(cancel);
                Ok(CancellationHandle::new(move || control.cancel()))
            }
            Err(TrySendError::Full(_)) => Err(RuntimeError::new(
                ErrorCode::RuntimeBusy,
                "inference command mailbox is full",
            )),
            Err(TrySendError::Disconnected(_)) => Err(unavailable("inference thread has stopped")),
        }
    }
}

impl Drop for EngineHost {
    fn drop(&mut self) {
        if let Some(cancel) = &self.active_cancel {
            cancel.cancel();
        }
        self.sender.take();
        // Do not join a potentially stuck native call or destroy it elsewhere.
        // The detached owner thread cleans up in order once cancellation returns.
    }
}

fn unavailable(message: &str) -> RuntimeError {
    RuntimeError::new(ErrorCode::ExecutorUnavailable, message)
}

fn inference_thread(receiver: Receiver<Job>, reports: SyncSender<ExecutionReport>) {
    while let Ok(job) = receiver.recv() {
        let mut current_events = job.events.clone();
        let result = catch_unwind(AssertUnwindSafe(|| match job.command {
            ExecutorCommand::Load { .. } => {
                loaded_session(job, &receiver, &reports, &mut current_events)
            }
            ExecutorCommand::Unload => {
                job.events.emit(ExecutorEvent::Unloaded);
            }
            ExecutorCommand::Generate { .. } => {
                job.events.emit(ExecutorEvent::Failed(RuntimeError::new(
                    ErrorCode::RuntimeFaulted,
                    "no model is loaded",
                )));
            }
        }));
        if result.is_err() {
            // Native RAII has already unwound on this owner thread. Never pass
            // panic contents (which can include user data) into diagnostics.
            current_events.emit(ExecutorEvent::Faulted(unavailable(
                "inference executor panicked after safe native cleanup",
            )));
        }
    }
}

fn loaded_session(
    job: Job,
    receiver: &Receiver<Job>,
    reports: &SyncSender<ExecutionReport>,
    current_events: &mut ExecutionEvents,
) {
    let ExecutorCommand::Load {
        model: resolved,
        options,
    } = job.command
    else {
        unreachable!()
    };
    let started = Instant::now();
    let mut engine = match Engine::new() {
        Ok(engine) => engine,
        Err(error) => {
            send_report(
                reports,
                Operation::Load,
                started.elapsed(),
                Duration::ZERO,
                Duration::ZERO,
                Duration::ZERO,
                Usage::default(),
                Some(error.code),
            );
            job.events.emit(ExecutorEvent::Failed(error));
            return;
        }
    };
    let loaded = if let Some(projector_path) = &resolved.projector_path {
        engine.load_with_projector(&resolved.path, projector_path, options, &job.cancel)
    } else {
        engine.load(&resolved.path, options, &job.cancel)
    };
    if let Some(error) = loaded.as_ref().err().cloned() {
        // A failed/cancelled Load is a cleanup acknowledgment too. Drop the
        // borrowing result and engine before another thread can see it.
        drop(loaded);
        drop(engine);
        send_report(
            reports,
            Operation::Load,
            started.elapsed(),
            Duration::ZERO,
            Duration::ZERO,
            Duration::ZERO,
            Usage::default(),
            Some(error.code),
        );
        job.events.emit(ExecutorEvent::Failed(error));
        return;
    }
    let mut model = loaded.expect("load error handled before cleanup acknowledgment");
    send_report(
        reports,
        Operation::Load,
        started.elapsed(),
        Duration::ZERO,
        Duration::ZERO,
        Duration::ZERO,
        Usage::default(),
        None,
    );
    if !job.events.emit(ExecutorEvent::Loaded) {
        return;
    }
    let unload_events = loop {
        let Ok(job) = receiver.recv() else {
            break None;
        };
        *current_events = job.events.clone();
        match job.command {
            ExecutorCommand::Generate { request } => {
                generate(&mut model, request, &job.cancel, &job.events, reports)
            }
            ExecutorCommand::Unload => break Some(job.events),
            ExecutorCommand::Load { .. } => {
                job.events.emit(ExecutorEvent::Failed(RuntimeError::new(
                    ErrorCode::RuntimeBusy,
                    "unload the existing model before changing load parameters",
                )));
            }
        }
    };
    // The public unload acknowledgment is strictly after all native drops.
    drop(model);
    drop(engine);
    if let Some(events) = unload_events {
        send_report(
            reports,
            Operation::Unload,
            Duration::ZERO,
            Duration::ZERO,
            Duration::ZERO,
            Duration::ZERO,
            Usage::default(),
            None,
        );
        events.emit(ExecutorEvent::Unloaded);
    }
}

fn generate(
    model: &mut Model<'_>,
    request: GenerationRequest,
    cancel: &CancelHandle,
    events: &ExecutionEvents,
    reports: &SyncSender<ExecutionReport>,
) {
    let started = Instant::now();
    let mut tool_output_reservation = if request.uses_tools() {
        match events.reserve_tool_output() {
            Ok(permit) => permit,
            Err(error) => {
                events.emit(ExecutorEvent::GenerationFailed {
                    error,
                    usage: Usage::default(),
                });
                return;
            }
        }
    } else {
        None
    };
    let prepared = match model.prepare_request(&request, cancel) {
        Ok(prepared) => prepared,
        Err(error) => {
            send_report(
                reports,
                Operation::Generate,
                Duration::ZERO,
                started.elapsed(),
                Duration::ZERO,
                Duration::ZERO,
                Usage::default(),
                Some(error.code),
            );
            events.emit_with_tool_reservation(
                ExecutorEvent::GenerationFailed {
                    error,
                    usage: Usage::default(),
                },
                tool_output_reservation.take(),
            );
            return;
        }
    };
    let prepare = started.elapsed();
    let prompt_tokens = prepared.prompt_tokens();
    // Exact native template/token budget has passed. The shared FIFO guarantees
    // the scheduler publishes Started before any TextDelta can be observed.
    if !events.emit(ExecutorEvent::Prepared { prompt_tokens }) {
        drop(prepared);
        events.emit_with_tool_reservation(
            ExecutorEvent::GenerationFailed {
                error: RuntimeError::new(
                    ErrorCode::ConsumerStopped,
                    "generation consumer stopped during preparation",
                ),
                usage: Usage {
                    prompt_tokens,
                    completion_tokens: 0,
                },
            },
            tool_output_reservation.take(),
        );
        return;
    }
    let mut phases = PhaseTimer::new(prompt_tokens);
    let mut output_callback = Duration::ZERO;
    let result = prepared.generate_chat_observed(
        cancel,
        |delta| {
            let started = Instant::now();
            let delivered = match delta {
                GeneratedDelta::Text(text) => events.text_delta(text),
                GeneratedDelta::ToolCall(delta) => events.tool_call_delta(delta),
            };
            output_callback = output_callback.saturating_add(started.elapsed());
            if delivered {
                StreamControl::Continue
            } else {
                StreamControl::Stop
            }
        },
        |progress| {
            phases.observe(progress, Instant::now());
            StreamControl::Continue
        },
    );
    let returned = Instant::now();
    // On cancellation the current phase includes native cleanup until return.
    let prefill = phases
        .prefill_started
        .map(|start| {
            phases
                .decode_started
                .unwrap_or(returned)
                .saturating_duration_since(start)
        })
        .unwrap_or_default();
    let decode = phases
        .decode_started
        .map(|start| {
            returned
                .saturating_duration_since(start)
                .saturating_sub(output_callback)
        })
        .unwrap_or_default();
    let timings = phases.finish(prepare, output_callback, returned);
    match result {
        Ok(result) => {
            send_report(
                reports,
                Operation::Generate,
                Duration::ZERO,
                prepare,
                prefill,
                decode,
                result.usage,
                None,
            );
            events.emit_with_tool_reservation(
                ExecutorEvent::Completed {
                    usage: result.usage,
                    finish_reason: result.finish_reason,
                    timings,
                },
                tool_output_reservation.take(),
            );
        }
        Err(failure) => {
            send_report(
                reports,
                Operation::Generate,
                Duration::ZERO,
                prepare,
                prefill,
                decode,
                failure.usage,
                Some(failure.error.code),
            );
            events.emit_with_tool_reservation(
                ExecutorEvent::GenerationFailed {
                    error: failure.error,
                    usage: failure.usage,
                },
                tool_output_reservation.take(),
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn send_report(
    reports: &SyncSender<ExecutionReport>,
    operation: Operation,
    load: Duration,
    prepare: Duration,
    prefill: Duration,
    decode: Duration,
    usage: Usage,
    error: Option<ErrorCode>,
) {
    let _ = reports.try_send(ExecutionReport {
        operation,
        load,
        prepare,
        prefill,
        decode,
        usage,
        error,
    });
}

/// Observes native boundaries without inventing lifecycle events or zero samples.
struct PhaseTimer {
    prompt_tokens: u32,
    completed: u32,
    prefill_started: Option<Instant>,
    decode_started: Option<Instant>,
    invalid: bool,
}
impl PhaseTimer {
    fn new(prompt_tokens: u32) -> Self {
        Self {
            prompt_tokens,
            completed: 0,
            prefill_started: None,
            decode_started: None,
            invalid: false,
        }
    }
    fn observe(&mut self, progress: GenerationProgress, now: Instant) {
        if progress.total_prompt_tokens != self.prompt_tokens
            || progress.completed_prompt_tokens > self.prompt_tokens
        {
            self.invalid = true;
            return;
        }
        let valid = match progress.phase {
            GenerationPhase::PrefillStarted => {
                let valid = self.prefill_started.is_none()
                    && self.decode_started.is_none()
                    && progress.completed_prompt_tokens == 0;
                if valid {
                    self.prefill_started = Some(now);
                }
                valid
            }
            GenerationPhase::PrefillBatchCompleted => {
                let valid = self.prefill_started.is_some()
                    && self.decode_started.is_none()
                    && progress.completed_prompt_tokens > self.completed;
                if valid {
                    self.completed = progress.completed_prompt_tokens;
                }
                valid
            }
            GenerationPhase::DecodeStarted => {
                let valid = self.prefill_started.is_some()
                    && self.decode_started.is_none()
                    && progress.completed_prompt_tokens == self.prompt_tokens
                    && self.completed == self.prompt_tokens;
                if valid {
                    self.decode_started = Some(now);
                }
                valid
            }
        };
        self.invalid |= !valid;
    }
    fn finish(
        &self,
        prepare: Duration,
        output_callback: Duration,
        returned: Instant,
    ) -> Option<InferenceTimings> {
        if self.invalid {
            return None;
        }
        let prefill = self
            .decode_started?
            .checked_duration_since(self.prefill_started?)?;
        let decode = returned
            .checked_duration_since(self.decode_started?)?
            .checked_sub(output_callback)?;
        let timings = InferenceTimings {
            prepare_us: prepare.as_micros().try_into().ok()?,
            prefill_us: prefill.as_micros().try_into().ok()?,
            decode_us: decode.as_micros().try_into().ok()?,
            output_callback_us: output_callback.as_micros().try_into().ok()?,
        };
        timings.is_valid().then_some(timings)
    }
}

#[cfg(test)]
mod timing_tests {
    use super::*;
    fn progress(phase: GenerationPhase, completed: u32) -> GenerationProgress {
        GenerationProgress {
            phase,
            completed_prompt_tokens: completed,
            total_prompt_tokens: 8,
        }
    }
    #[test]
    fn phases_include_prefill_and_exclude_callback_before_microsecond_conversion() {
        let start = Instant::now();
        let mut timer = PhaseTimer::new(8);
        timer.observe(progress(GenerationPhase::PrefillStarted, 0), start);
        timer.observe(progress(GenerationPhase::PrefillBatchCompleted, 4), start);
        timer.observe(progress(GenerationPhase::PrefillBatchCompleted, 8), start);
        timer.observe(
            progress(GenerationPhase::DecodeStarted, 8),
            start + Duration::from_micros(20),
        );
        let measured = timer
            .finish(
                Duration::from_micros(10),
                Duration::from_nanos(1500),
                start + Duration::from_nanos(23500),
            )
            .unwrap();
        assert_eq!(
            measured,
            InferenceTimings {
                prepare_us: 10,
                prefill_us: 20,
                decode_us: 2,
                output_callback_us: 1
            }
        );
        assert!(
            timer
                .finish(
                    Duration::ZERO,
                    Duration::from_secs(1),
                    start + Duration::from_micros(30)
                )
                .is_none()
        );
    }
    #[test]
    fn absent_duplicate_reordered_and_inconsistent_phases_are_not_measurements() {
        let start = Instant::now();
        let sequences = [
            vec![],
            vec![progress(GenerationPhase::PrefillStarted, 0)],
            vec![progress(GenerationPhase::DecodeStarted, 8)],
            vec![
                progress(GenerationPhase::PrefillStarted, 0),
                progress(GenerationPhase::DecodeStarted, 8),
            ],
            vec![
                progress(GenerationPhase::PrefillStarted, 0),
                progress(GenerationPhase::PrefillBatchCompleted, 4),
                progress(GenerationPhase::DecodeStarted, 8),
            ],
            vec![
                progress(GenerationPhase::PrefillStarted, 0),
                progress(GenerationPhase::PrefillStarted, 0),
                progress(GenerationPhase::DecodeStarted, 8),
            ],
            vec![
                progress(GenerationPhase::PrefillStarted, 0),
                progress(GenerationPhase::DecodeStarted, 8),
                progress(GenerationPhase::DecodeStarted, 8),
            ],
            vec![
                progress(GenerationPhase::PrefillStarted, 0),
                progress(GenerationPhase::PrefillBatchCompleted, 4),
                progress(GenerationPhase::PrefillBatchCompleted, 3),
                progress(GenerationPhase::DecodeStarted, 8),
            ],
            vec![
                GenerationProgress {
                    total_prompt_tokens: 9,
                    ..progress(GenerationPhase::PrefillStarted, 0)
                },
                progress(GenerationPhase::DecodeStarted, 8),
            ],
        ];
        for sequence in sequences {
            let mut timer = PhaseTimer::new(8);
            for event in sequence {
                timer.observe(event, start);
            }
            assert!(
                timer
                    .finish(Duration::ZERO, Duration::ZERO, start)
                    .is_none()
            );
        }
    }
}
