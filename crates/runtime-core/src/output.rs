use crate::executor::{Envelope, ExecutionEvents, ExecutorEvent};
use runtime_types::{ErrorCode, RequestEvent};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Condvar, Mutex,
        mpsc::{RecvTimeoutError, TrySendError},
    },
    time::{Duration, Instant},
};

pub const MAX_BUFFERED_TEXT_BYTES: usize = 256 * 1024;
pub const MAX_DELTA_BYTES: usize = 4096;
// Tiny deltas also consume a slot budget, bounding per-event allocation overhead.
const MIN_EVENT_CHARGE: usize = 256;
pub(crate) struct Output {
    state: Mutex<State>,
    changed: Condvar,
    timeout: Duration,
}
struct State {
    events: VecDeque<(RequestEvent, usize)>,
    reserved: usize,
    last_progress: Instant,
    closed: bool,
    disconnected: bool,
    cancel: Option<ErrorCode>,
}
pub struct EventReceiver {
    pub(crate) output: Arc<Output>,
}
impl Output {
    pub(crate) fn new(timeout: Duration) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State {
                events: VecDeque::new(),
                reserved: 0,
                last_progress: Instant::now(),
                closed: false,
                disconnected: false,
                cancel: None,
            }),
            changed: Condvar::new(),
            timeout,
        })
    }
    pub(crate) fn reason(&self) -> Option<ErrorCode> {
        self.state.lock().unwrap().cancel
    }
    pub(crate) fn cancel(&self, reason: ErrorCode) {
        let mut s = self.state.lock().unwrap();
        s.cancel.get_or_insert(reason);
        self.changed.notify_all();
    }
    pub(crate) fn publish(&self, event: RequestEvent, charge: usize) {
        let mut s = self.state.lock().unwrap();
        if s.disconnected || s.closed {
            s.reserved = s.reserved.saturating_sub(charge);
            self.changed.notify_all();
            return;
        }
        if event.kind.is_terminal() {
            s.closed = true;
        }
        s.events.push_back((event, charge));
        self.changed.notify_all();
    }
    fn reserve(&self, bytes: usize) -> Option<usize> {
        let charge = bytes.max(MIN_EVENT_CHARGE);
        let mut s = self.state.lock().unwrap();
        loop {
            if s.closed || s.disconnected || s.cancel.is_some() {
                return None;
            }
            if s.reserved + charge <= MAX_BUFFERED_TEXT_BYTES {
                if s.reserved == 0 {
                    s.last_progress = Instant::now();
                }
                s.reserved += charge;
                return Some(charge);
            }
            let remaining = self.timeout.saturating_sub(s.last_progress.elapsed());
            if remaining.is_zero() {
                s.cancel = Some(ErrorCode::SlowConsumer);
                self.changed.notify_all();
                return None;
            }
            s = self.changed.wait_timeout(s, remaining).unwrap().0;
        }
    }
    pub(crate) fn release(&self, charge: usize) {
        let mut s = self.state.lock().unwrap();
        s.reserved = s.reserved.saturating_sub(charge);
        self.changed.notify_all();
    }
}
impl EventReceiver {
    pub fn recv(&self) -> Option<RequestEvent> {
        let mut s = self.output.state.lock().unwrap();
        loop {
            if let Some((event, charge)) = s.events.pop_front() {
                s.reserved = s.reserved.saturating_sub(charge);
                s.last_progress = Instant::now();
                self.output.changed.notify_all();
                return Some(event);
            }
            if s.closed {
                return None;
            }
            s = self.output.changed.wait(s).unwrap();
        }
    }
    pub fn recv_timeout(&self, timeout: Duration) -> Result<RequestEvent, RecvTimeoutError> {
        let start = Instant::now();
        let mut s = self.output.state.lock().unwrap();
        loop {
            if let Some((event, charge)) = s.events.pop_front() {
                s.reserved = s.reserved.saturating_sub(charge);
                s.last_progress = Instant::now();
                self.output.changed.notify_all();
                return Ok(event);
            }
            if s.closed {
                return Err(RecvTimeoutError::Disconnected);
            }
            let remaining = timeout.saturating_sub(start.elapsed());
            if remaining.is_zero() {
                return Err(RecvTimeoutError::Timeout);
            }
            s = self.output.changed.wait_timeout(s, remaining).unwrap().0;
        }
    }
    /// Includes text still travelling from the executor to the actor.
    pub fn buffered_bytes(&self) -> usize {
        self.output.state.lock().unwrap().reserved
    }
}
impl Drop for EventReceiver {
    fn drop(&mut self) {
        let mut s = self.output.state.lock().unwrap();
        s.disconnected = true;
        s.cancel.get_or_insert(ErrorCode::ConsumerStopped);
        s.events.clear();
        s.reserved = 0;
        self.output.changed.notify_all();
    }
}
impl ExecutionEvents {
    pub(crate) fn emit_inner(&self, event: ExecutorEvent) -> bool {
        if let ExecutorEvent::TextDelta(text) = event {
            self.emit_text(&text)
        } else {
            self.send(event, 0)
        }
    }
    pub(crate) fn emit_text(&self, text: &str) -> bool {
        let Some(output) = &self.output else {
            return false;
        };
        let mut rest = text;
        while !rest.is_empty() {
            let mut end = rest.len().min(MAX_DELTA_BYTES);
            while !rest.is_char_boundary(end) {
                end -= 1;
            }
            let part = &rest[..end];
            let Some(charge) = output.reserve(part.len()) else {
                return false;
            };
            if !self.send(ExecutorEvent::TextDelta(part.to_owned()), charge) {
                output.release(charge);
                return false;
            }
            rest = &rest[end..];
        }
        true
    }
    fn send(&self, event: ExecutorEvent, charge: usize) -> bool {
        let mut envelope = Envelope {
            output: self.output.clone(),
            emitted_at: Instant::now(),
            operation: self.operation,
            event,
            charge,
        };
        loop {
            match self.sender.try_send(envelope) {
                Ok(()) => return true,
                Err(TrySendError::Disconnected(_)) => return false,
                Err(TrySendError::Full(value)) => {
                    envelope = value;
                    if charge > 0 && self.output.as_ref().is_some_and(|o| o.reason().is_some()) {
                        return false;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        }
    }
}
