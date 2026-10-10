use crate::{
    Frame, MAX_TEXT_BYTES, MAX_TEXT_CREDITS, Message, PROTOCOL_VERSION, SessionId, protocol_error,
};
use runtime_core::ExecutorEvent;
use runtime_types::{RequestId, RuntimeError};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationKind {
    Load,
    Generate,
    Unload,
}
struct Active {
    id: u64,
    request_id: Option<RequestId>,
    kind: OperationKind,
    prepared: Option<u32>,
    max_tokens: u32,
    terminal: bool,
    credits: BTreeSet<u64>,
    tools: Option<runtime_core::ToolStreamValidator>,
}
/// Receiver-side validation. Keep it with the reader so no uncredited text can
/// enter another queue. begin/grant must be called before the corresponding write.
pub struct EventValidator {
    session: SessionId,
    hello: bool,
    last_seq: u64,
    last_operation: u64,
    last_credit: u64,
    active: Option<Active>,
}
impl EventValidator {
    pub fn new(session: SessionId) -> Self {
        Self {
            session,
            hello: false,
            last_seq: 0,
            last_operation: 0,
            last_credit: 0,
            active: None,
        }
    }
    pub fn accept_hello(&mut self, frame: &Frame) -> Result<(), RuntimeError> {
        self.identity(frame)?;
        if self.hello
            || frame.operation_id != 0
            || frame.request_id.is_some()
            || frame.seq.is_some()
        {
            return Err(protocol_error("unexpected Hello"));
        }
        let Message::Hello(hello) = &frame.message else {
            return Err(protocol_error("expected worker Hello"));
        };
        hello.validate()?;
        self.hello = true;
        Ok(())
    }
    pub fn begin(&mut self, frame: &Frame) -> Result<(), RuntimeError> {
        self.identity(frame)?;
        if !self.hello
            || frame.operation_id <= self.last_operation
            || frame.seq.is_some()
            || self.active.as_ref().is_some_and(|a| !a.terminal)
        {
            return Err(protocol_error(
                "operation before handshake, stale, or overlapping",
            ));
        }
        let kind = match &frame.message {
            Message::Load { .. } if frame.request_id.is_none() => OperationKind::Load,
            Message::Generate { request } if frame.request_id == Some(request.request_id) => {
                OperationKind::Generate
            }
            Message::Unload {} if frame.request_id.is_none() => OperationKind::Unload,
            _ => return Err(protocol_error("not an operation command")),
        };
        self.last_operation = frame.operation_id;
        self.active = Some(Active {
            id: frame.operation_id,
            request_id: frame.request_id,
            kind,
            prepared: None,
            max_tokens: match &frame.message {
                Message::Generate { request } => request.options.max_tokens,
                _ => 0,
            },
            terminal: false,
            credits: BTreeSet::new(),
            tools: match &frame.message {
                Message::Generate { request } => {
                    Some(runtime_core::ToolStreamValidator::new(request))
                }
                _ => None,
            },
        });
        Ok(())
    }
    pub fn operation_complete(&self) -> bool {
        self.active.as_ref().is_some_and(|a| a.terminal)
    }
    pub fn grant(&mut self, credit_id: u64) -> Result<(), RuntimeError> {
        let active = self
            .active
            .as_mut()
            .ok_or_else(|| protocol_error("credit without operation"))?;
        if active.kind != OperationKind::Generate
            || active.terminal
            || credit_id <= self.last_credit
            || active.credits.len() >= MAX_TEXT_CREDITS
        {
            return Err(protocol_error(
                "stale, duplicate, excessive, or misplaced credit",
            ));
        }
        active.credits.insert(credit_id);
        self.last_credit = credit_id;
        Ok(())
    }
    pub fn accept(&mut self, frame: &Frame) -> Result<(), RuntimeError> {
        self.identity(frame)?;
        if !self.hello || frame.seq != self.last_seq.checked_add(1) {
            return Err(protocol_error("event before Hello or out of sequence"));
        }
        let active = self
            .active
            .as_mut()
            .ok_or_else(|| protocol_error("event without operation"))?;
        if frame.operation_id != active.id
            || frame.request_id != active.request_id
            || active.terminal
        {
            return Err(protocol_error(
                "wrong operation/request or duplicate terminal",
            ));
        }
        let Message::Event { event, credit_id } = &frame.message else {
            return Err(protocol_error("expected worker event"));
        };
        if let ExecutorEvent::Completed {
            timings: Some(timings),
            ..
        } = event
            && !timings.is_valid()
        {
            return Err(protocol_error("invalid inference timings"));
        }
        if let ExecutorEvent::Completed { usage, .. }
        | ExecutorEvent::GenerationFailed { usage, .. } = event
            && (usage.completion_tokens > active.max_tokens
                || match active.prepared {
                    Some(prompt) => usage.prompt_tokens != prompt,
                    None => usage.prompt_tokens != 0 || usage.completion_tokens != 0,
                })
        {
            return Err(protocol_error(
                "terminal usage disagrees with prepared/request budget",
            ));
        }
        if matches!(event, ExecutorEvent::Failed(error) | ExecutorEvent::Faulted(error) | ExecutorEvent::GenerationFailed { error, .. } if error.code == runtime_types::ErrorCode::ExecutorCleanupUnconfirmed)
        {
            return Err(protocol_error(
                "parent-only cleanup error is forbidden on wire",
            ));
        }
        let terminal = match (active.kind, event) {
            (_, ExecutorEvent::Failed(_) | ExecutorEvent::Faulted(_)) => true,
            (OperationKind::Load, ExecutorEvent::Loaded) => true,
            (OperationKind::Unload, ExecutorEvent::Unloaded) => true,
            (OperationKind::Generate, ExecutorEvent::Prepared { prompt_tokens })
                if active.prepared.is_none() =>
            {
                active.prepared = Some(*prompt_tokens);
                false
            }
            (OperationKind::Generate, ExecutorEvent::TextDelta(text))
                if active.prepared.is_some() =>
            {
                if active.tools.as_mut().is_none_or(|v| v.text(text).is_err())
                    || text.is_empty()
                    || text.len() > MAX_TEXT_BYTES
                    || !credit_id.is_some_and(|id| active.credits.remove(&id))
                {
                    return Err(protocol_error("text without valid once-only credit"));
                }
                false
            }
            (OperationKind::Generate, ExecutorEvent::ToolCallDelta(delta))
                if active.prepared.is_some() =>
            {
                if !credit_id.is_some_and(|id| active.credits.remove(&id))
                    || active
                        .tools
                        .as_mut()
                        .is_none_or(|v| v.delta(delta).is_err())
                {
                    return Err(protocol_error(
                        "tool payload without valid sequence or once-only credit",
                    ));
                }
                false
            }
            (OperationKind::Generate, ExecutorEvent::Completed { finish_reason, .. })
                if active.prepared.is_some() =>
            {
                if active
                    .tools
                    .as_ref()
                    .is_none_or(|v| v.complete(*finish_reason).is_err())
                {
                    return Err(protocol_error(
                        "tool payload disagrees with terminal outcome",
                    ));
                }
                true
            }
            (OperationKind::Generate, ExecutorEvent::GenerationFailed { .. }) => true,
            _ => return Err(protocol_error("event invalid for operation or phase")),
        };
        if !matches!(
            event,
            ExecutorEvent::TextDelta(_) | ExecutorEvent::ToolCallDelta(_)
        ) && credit_id.is_some()
        {
            return Err(protocol_error("non-payload event carries credit"));
        }
        active.terminal = terminal;
        if terminal {
            active.tools = None;
            active.credits.clear();
        }
        self.last_seq = frame.seq.unwrap();
        Ok(())
    }
    fn identity(&self, frame: &Frame) -> Result<(), RuntimeError> {
        if frame.protocol_version != PROTOCOL_VERSION
            || frame.session_id != self.session
            || frame.session_id.is_nil()
        {
            return Err(protocol_error("protocol/session mismatch"));
        }
        Ok(())
    }
}
