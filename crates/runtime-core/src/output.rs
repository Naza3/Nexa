use crate::executor::{Envelope, EventDestination, ExecutionEvents, ExecutorEvent};
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
const MIN_EVENT_CHARGE: usize = 256;
/// The ledger is separate from Output. Its permits never own the queue that
/// contains them, so retaining or dropping an unread stream cannot form a cycle.
struct Budget {
    state: Mutex<BudgetState>,
    changed: Condvar,
    timeout: Duration,
}
struct BudgetState {
    reserved: usize,
    // Progress is completed lease consumption, not each partial socket write.
    // One delta taking longer than timeout can be conservatively cancelled.
    last_progress: Instant,
    blocked_since: Option<Instant>,
    closed: bool,
    disconnected: bool,
    cancel: Option<ErrorCode>,
}
pub(crate) struct Output {
    state: Mutex<State>,
    changed: Condvar,
    budget: Arc<Budget>,
}
struct State {
    events: VecDeque<EventLease>,
    closed: bool,
    disconnected: bool,
}
/// An affine reservation belonging to exactly one operation and original ledger.
/// Dropping an unsent credit or stale envelope releases only that reservation.
/// This type deliberately does not implement Clone.
pub struct TextPermit {
    budget: Arc<Budget>,
    operation: u64,
    charge: usize,
}
impl TextPermit {
    pub fn charged_bytes(&self) -> usize {
        self.charge
    }
    /// Reduce a live reservation after its covered transient allocations are
    /// destroyed. Never expands, changes ledgers/operations, or reports progress.
    pub fn shrink_to(&mut self, bytes: usize) -> bool {
        if bytes > self.charge {
            return false;
        }
        let released = self.charge - bytes;
        self.charge = bytes;
        let mut state = self.budget.state.lock().unwrap();
        state.reserved = state
            .reserved
            .checked_sub(released)
            .expect("permit ledger underflow");
        self.budget.changed.notify_all();
        true
    }
}
impl Drop for TextPermit {
    fn drop(&mut self) {
        let mut s = self.budget.state.lock().unwrap();
        s.reserved = s
            .reserved
            .checked_sub(self.charge)
            .expect("permit ledger underflow");
        self.budget.changed.notify_all();
    }
}
/// A dequeued event whose pending-output charge remains live until the consumer
/// has finished using/serializing/writing it. Keep this lease through SSE writes.
pub struct EventLease {
    event: Option<RequestEvent>,
    permit: Option<TextPermit>,
}
impl EventLease {
    pub fn event(&self) -> &RequestEvent {
        self.event.as_ref().unwrap()
    }
    /// Transfer the original affine reservation to a transport-owned aggregate.
    /// The original event/text is destroyed BEFORE reducing the reservation.
    /// A transfer is not consumer progress. No reservation can be manufactured,
    /// expanded, rebound to another operation, or released twice by this API.
    pub fn retain_permit(mut self, bytes: usize) -> Result<TextPermit, Self> {
        if !self.permit.as_ref().is_some_and(|p| p.charge >= bytes) {
            return Err(self);
        }
        drop(self.event.take());
        let mut permit = self.permit.take().unwrap();
        assert!(permit.shrink_to(bytes));
        Ok(permit)
    }
    pub fn charged_bytes(&self) -> usize {
        self.permit.as_ref().map_or(0, TextPermit::charged_bytes)
    }
    /// Compatibility boundary: taking the owned event counts as consumption.
    /// Transports should retain the lease instead, until their write completes.
    pub fn into_event(mut self) -> RequestEvent {
        self.event.take().unwrap()
    }
}
impl Drop for EventLease {
    fn drop(&mut self) {
        if let Some(permit) = &self.permit {
            permit.budget.state.lock().unwrap().last_progress = Instant::now();
        }
        // The permit then releases its own charge and wakes producers.
    }
}

impl std::ops::Deref for EventLease {
    type Target = RequestEvent;
    fn deref(&self) -> &Self::Target {
        self.event()
    }
}
pub struct EventReceiver {
    pub(crate) output: Arc<Output>,
}
impl Budget {
    fn reserve(
        self: &Arc<Self>,
        bytes: usize,
        operation: u64,
        blocking: bool,
    ) -> Option<TextPermit> {
        let charge = bytes.max(MIN_EVENT_CHARGE);
        if charge > MAX_BUFFERED_TEXT_BYTES {
            return None;
        }
        let mut s = self.state.lock().unwrap();
        loop {
            if s.closed || s.disconnected || s.cancel.is_some() {
                return None;
            }
            if s.reserved <= MAX_BUFFERED_TEXT_BYTES - charge {
                if s.reserved == 0 {
                    s.last_progress = Instant::now();
                }
                s.reserved += charge;
                s.blocked_since = None;
                return Some(TextPermit {
                    budget: self.clone(),
                    operation,
                    charge,
                });
            }
            let blocked = *s.blocked_since.get_or_insert_with(Instant::now);
            let remaining = self
                .timeout
                .saturating_sub(blocked.max(s.last_progress).elapsed());
            if remaining.is_zero() {
                s.cancel = Some(ErrorCode::SlowConsumer);
                self.changed.notify_all();
                return None;
            }
            if !blocking {
                return None;
            }
            s = self.changed.wait_timeout(s, remaining).unwrap().0;
        }
    }
}
impl Output {
    pub(crate) fn new(timeout: Duration) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State {
                events: VecDeque::new(),
                closed: false,
                disconnected: false,
            }),
            changed: Condvar::new(),
            budget: Arc::new(Budget {
                state: Mutex::new(BudgetState {
                    reserved: 0,
                    last_progress: Instant::now(),
                    blocked_since: None,
                    closed: false,
                    disconnected: false,
                    cancel: None,
                }),
                changed: Condvar::new(),
                timeout,
            }),
        })
    }
    pub(crate) fn reason(&self) -> Option<ErrorCode> {
        self.budget.state.lock().unwrap().cancel
    }
    pub(crate) fn cancel(&self, reason: ErrorCode) {
        let mut s = self.budget.state.lock().unwrap();
        s.cancel.get_or_insert(reason);
        self.budget.changed.notify_all();
    }
    pub(crate) fn publish(&self, event: RequestEvent, permit: Option<TextPermit>) {
        let mut s = self.state.lock().unwrap();
        if s.disconnected || s.closed {
            // No external callbacks; permit destruction occurs after queue unlock.
            drop(s);
            drop(permit);
            return;
        }
        if event.kind.is_terminal() {
            s.closed = true;
            self.budget.state.lock().unwrap().closed = true;
            self.budget.changed.notify_all();
        }
        s.events.push_back(EventLease {
            event: Some(event),
            permit,
        });
        self.changed.notify_all();
    }
}
/// A non-blocking cancellation signal for transports whose receiver is owned
/// by a blocking pump. It never calls Runtime::cancel or waits for the actor.
#[derive(Clone)]
pub struct DisconnectHandle {
    output: Arc<Output>,
}
impl DisconnectHandle {
    pub fn disconnect(&self) {
        let queued = {
            let mut state = self.output.state.lock().unwrap();
            state.disconnected = true;
            std::mem::take(&mut state.events)
        };
        {
            let mut state = self.output.budget.state.lock().unwrap();
            state.disconnected = true;
            state.cancel.get_or_insert(ErrorCode::ConsumerStopped);
            self.output.budget.changed.notify_all();
        }
        drop(queued);
        self.output.changed.notify_all();
    }
}
impl EventReceiver {
    pub fn disconnect_handle(&self) -> DisconnectHandle {
        DisconnectHandle {
            output: self.output.clone(),
        }
    }
    pub fn recv_leased(&self) -> Option<EventLease> {
        let mut s = self.output.state.lock().unwrap();
        loop {
            if let Some(event) = s.events.pop_front() {
                return Some(event);
            }
            if s.closed || s.disconnected {
                return None;
            }
            s = self.output.changed.wait(s).unwrap();
        }
    }
    pub fn recv_timeout_leased(&self, timeout: Duration) -> Result<EventLease, RecvTimeoutError> {
        let start = Instant::now();
        let mut s = self.output.state.lock().unwrap();
        loop {
            if let Some(event) = s.events.pop_front() {
                return Ok(event);
            }
            if s.closed || s.disconnected {
                return Err(RecvTimeoutError::Disconnected);
            }
            let remaining = timeout.saturating_sub(start.elapsed());
            if remaining.is_zero() {
                return Err(RecvTimeoutError::Timeout);
            }
            s = self.output.changed.wait_timeout(s, remaining).unwrap().0;
        }
    }
    pub fn recv(&self) -> Option<RequestEvent> {
        self.recv_leased().map(EventLease::into_event)
    }
    pub fn recv_timeout(&self, timeout: Duration) -> Result<RequestEvent, RecvTimeoutError> {
        self.recv_timeout_leased(timeout)
            .map(EventLease::into_event)
    }
    /// Includes reserved credits, transport copies, actor envelopes and live leases.
    pub fn buffered_bytes(&self) -> usize {
        self.output.budget.state.lock().unwrap().reserved
    }
}
impl Drop for EventReceiver {
    fn drop(&mut self) {
        // External leases/in-flight permits retain and release their own charge.
        self.disconnect_handle().disconnect();
    }
}
impl ExecutionEvents {
    fn output(&self) -> Option<&Arc<Output>> {
        match &self.destination {
            EventDestination::Actor { output, .. } => output.as_ref(),
            EventDestination::Sink(_) => None,
        }
    }
    pub fn cancellation_reason(&self) -> Option<ErrorCode> {
        self.output().and_then(|o| o.reason())
    }
    pub fn buffered_bytes(&self) -> usize {
        self.output()
            .map_or(0, |o| o.budget.state.lock().unwrap().reserved)
    }
    pub fn last_consumer_progress(&self) -> Option<Instant> {
        self.output()
            .map(|o| o.budget.state.lock().unwrap().last_progress)
    }
    pub fn try_reserve_text(&self, accounting_bytes: usize) -> Option<TextPermit> {
        self.output()?
            .budget
            .reserve(accounting_bytes, self.operation, false)
    }
    pub fn emit_reserved_text(&self, text: String, permit: TextPermit) -> bool {
        let Some(output) = self.output() else {
            return false;
        };
        if text.is_empty()
            || text.len() > MAX_DELTA_BYTES
            || text.len() > permit.charge
            || permit.operation != self.operation
            || !Arc::ptr_eq(&permit.budget, &output.budget)
            || output.reason().is_some()
        {
            return false;
        }
        self.send(ExecutorEvent::TextDelta(text), Some(permit))
    }
    pub(crate) fn emit_inner(&self, event: ExecutorEvent) -> bool {
        if let ExecutorEvent::TextDelta(text) = event {
            self.emit_text(&text)
        } else {
            self.send(event, None)
        }
    }
    pub(crate) fn emit_text(&self, text: &str) -> bool {
        let mut rest = text;
        while !rest.is_empty() {
            let mut end = rest.len().min(MAX_DELTA_BYTES);
            while !rest.is_char_boundary(end) {
                end -= 1;
            }
            let part = &rest[..end];
            let permit = match &self.destination {
                EventDestination::Actor {
                    output: Some(output),
                    ..
                } => {
                    let Some(permit) = output.budget.reserve(part.len(), self.operation, true)
                    else {
                        return false;
                    };
                    Some(permit)
                }
                EventDestination::Actor { output: None, .. } => return false,
                EventDestination::Sink(_) => None,
            };
            if !self.send(ExecutorEvent::TextDelta(part.to_owned()), permit) {
                return false;
            }
            rest = &rest[end..];
        }
        true
    }
    fn send(&self, event: ExecutorEvent, permit: Option<TextPermit>) -> bool {
        let EventDestination::Actor { sender, output } = &self.destination else {
            if let EventDestination::Sink(sink) = &self.destination {
                return sink.emit(event);
            }
            unreachable!();
        };
        let mut envelope = Envelope {
            emitted_at: Instant::now(),
            operation: self.operation,
            event,
            permit,
        };
        loop {
            match sender.try_send(envelope) {
                Ok(()) => return true,
                Err(TrySendError::Disconnected(_)) => return false,
                Err(TrySendError::Full(value)) => {
                    envelope = value;
                    if envelope.permit.is_some()
                        && output.as_ref().is_some_and(|o| o.reason().is_some())
                    {
                        return false;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use runtime_types::{RequestEventKind, RequestId};
    use std::sync::mpsc;
    fn stream(operation: u64) -> (ExecutionEvents, EventReceiver, mpsc::Receiver<Envelope>) {
        let output = Output::new(Duration::from_millis(15));
        let (tx, rx) = mpsc::sync_channel(4);
        (
            ExecutionEvents::for_actor(operation, tx, Some(output.clone())),
            EventReceiver { output },
            rx,
        )
    }
    fn publish(receiver: &EventReceiver, envelope: Envelope) {
        let ExecutorEvent::TextDelta(text) = envelope.event else {
            panic!()
        };
        receiver.output.publish(
            RequestEvent {
                request_id: RequestId::new(),
                seq: 1,
                kind: RequestEventKind::TextDelta(text),
            },
            envelope.permit,
        );
    }
    #[test]
    fn lease_keeps_charge_until_consumed_or_dropped() {
        let (sink, receiver, rx) = stream(1);
        let permit = sink.try_reserve_text(120 * 1024).unwrap();
        assert!(sink.emit_reserved_text("你好".into(), permit));
        publish(&receiver, rx.recv().unwrap());
        let lease = receiver.recv_leased().unwrap();
        assert_eq!(lease.event().seq, 1);
        assert_eq!(receiver.buffered_bytes(), 120 * 1024);
        drop(lease);
        assert_eq!(receiver.buffered_bytes(), 0);
        assert!(sink.text_delta("next"));
        publish(&receiver, rx.recv().unwrap());
        assert!(receiver.recv().is_some());
        assert_eq!(receiver.buffered_bytes(), 0);
    }
    #[test]
    fn conservative_credits_exhaust_shared_ledger_without_early_refund() {
        let (sink, receiver, rx) = stream(2);
        let scratch = sink.try_reserve_text(16 * 1024).unwrap();
        let a = sink.try_reserve_text(120 * 1024).unwrap();
        let b = sink.try_reserve_text(120 * 1024).unwrap();
        assert_eq!(receiver.buffered_bytes(), MAX_BUFFERED_TEXT_BYTES);
        assert!(sink.try_reserve_text(1).is_none());
        assert!(sink.emit_reserved_text("a".into(), a));
        publish(&receiver, rx.recv().unwrap());
        let lease = receiver.recv_leased().unwrap();
        assert!(sink.try_reserve_text(1).is_none());
        drop(lease);
        let c = sink.try_reserve_text(120 * 1024).unwrap();
        assert_eq!(receiver.buffered_bytes(), MAX_BUFFERED_TEXT_BYTES);
        drop((scratch, b, c));
        assert_eq!(receiver.buffered_bytes(), 0);
    }
    #[test]
    fn foreign_stale_and_oversized_permits_only_release_original_ledger() {
        let (one, receiver_one, _) = stream(1);
        let (two, receiver_two, _) = stream(2);
        let held = two.try_reserve_text(4096).unwrap();
        assert!(!two.emit_reserved_text("bad".into(), one.try_reserve_text(4096).unwrap()));
        assert_eq!(receiver_one.buffered_bytes(), 0);
        assert_eq!(receiver_two.buffered_bytes(), 4096);
        let (tx, _rx) = mpsc::sync_channel(1);
        let newer = ExecutionEvents::for_actor(2, tx, Some(receiver_one.output.clone()));
        assert!(!newer.emit_reserved_text("stale".into(), one.try_reserve_text(4096).unwrap()));
        assert_eq!(receiver_one.buffered_bytes(), 0);
        assert!(!two.emit_reserved_text("x".repeat(4097), held));
        assert_eq!(receiver_two.buffered_bytes(), 0);
        assert!(!one.emit_reserved_text(String::new(), one.try_reserve_text(4096).unwrap()));
        assert_eq!(receiver_one.buffered_bytes(), 0);
    }
    #[test]
    fn disconnect_preserves_inflight_charges_and_does_not_cycle() {
        let (sink, receiver, rx) = stream(1);
        let budget = Arc::downgrade(&receiver.output.budget);
        let scratch = sink.try_reserve_text(4096).unwrap();
        assert!(sink.text_delta("queued"));
        publish(&receiver, rx.recv().unwrap());
        assert!(sink.text_delta("leased"));
        publish(&receiver, rx.recv().unwrap());
        let lease = receiver.recv_leased().unwrap();
        assert!(sink.text_delta("inflight"));
        let envelope = rx.recv().unwrap();
        drop(receiver);
        assert_eq!(sink.cancellation_reason(), Some(ErrorCode::ConsumerStopped));
        assert_eq!(sink.buffered_bytes(), 4096 + 2 * MIN_EVENT_CHARGE);
        drop(envelope);
        drop(lease);
        assert_eq!(sink.buffered_bytes(), 4096);
        drop(scratch);
        assert_eq!(sink.buffered_bytes(), 0);
        drop(sink);
        assert!(budget.upgrade().is_none());
    }
    #[test]
    fn failed_send_closed_output_and_cancel_release_only_own_permits() {
        let (sink, receiver, rx) = stream(1);
        drop(rx);
        assert!(!sink.text_delta("pipe closed"));
        assert_eq!(receiver.buffered_bytes(), 0);
        let permit = sink.try_reserve_text(4096).unwrap();
        receiver.output.cancel(ErrorCode::RequestCancelled);
        assert!(!sink.emit_reserved_text("late".into(), permit));
        assert_eq!(receiver.buffered_bytes(), 0);
        assert!(sink.try_reserve_text(4096).is_none());
        let (sink, receiver, rx) = stream(2);
        assert!(sink.text_delta("held"));
        let envelope = rx.recv().unwrap();
        receiver.output.state.lock().unwrap().closed = true;
        publish(&receiver, envelope);
        assert_eq!(receiver.buffered_bytes(), 0);
    }
    #[test]
    fn slow_timeout_starts_at_budget_shortage_not_prefill_or_credit_issue() {
        let (sink, receiver, _) = stream(1);
        let permit = sink.try_reserve_text(MAX_BUFFERED_TEXT_BYTES).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        assert!(sink.try_reserve_text(1).is_none());
        assert_eq!(sink.cancellation_reason(), None);
        std::thread::sleep(Duration::from_millis(20));
        assert!(sink.try_reserve_text(1).is_none());
        assert_eq!(sink.cancellation_reason(), Some(ErrorCode::SlowConsumer));
        drop(permit);
        assert_eq!(receiver.buffered_bytes(), 0);
    }
    #[test]
    fn disconnect_handle_wakes_transferred_receiver_and_preserves_live_lease() {
        let (sink, receiver, rx) = stream(1);
        let control = receiver.disconnect_handle();
        assert!(sink.emit_reserved_text("held".into(), sink.try_reserve_text(120 * 1024).unwrap()));
        publish(&receiver, rx.recv().unwrap());
        let lease = receiver.recv_leased().unwrap();
        let worker = std::thread::spawn(move || receiver.recv_leased());
        control.disconnect();
        assert!(worker.join().unwrap().is_none());
        assert_eq!(sink.cancellation_reason(), Some(ErrorCode::ConsumerStopped));
        assert_eq!(sink.buffered_bytes(), 120 * 1024);
        drop(lease);
        assert_eq!(sink.buffered_bytes(), 0);
    }
    #[test]
    fn retained_permit_is_affine_shrink_only_and_not_fake_progress() {
        let (sink, receiver, rx) = stream(1);
        let scratch = sink.try_reserve_text(16 * 1024).unwrap();
        assert!(
            sink.emit_reserved_text("first".into(), sink.try_reserve_text(120 * 1024).unwrap())
        );
        let transit = sink.try_reserve_text(120 * 1024).unwrap();
        publish(&receiver, rx.recv().unwrap());
        let before = sink.last_consumer_progress().unwrap();
        let lease = receiver.recv_leased().unwrap();
        let mut retained = lease
            .retain_permit(96 * 1024)
            .unwrap_or_else(|_| panic!("valid transfer"));
        assert_eq!(sink.buffered_bytes(), 232 * 1024);
        assert_eq!(sink.last_consumer_progress().unwrap(), before);
        assert!(!retained.shrink_to(120 * 1024));
        assert_eq!(retained.charged_bytes(), 96 * 1024);
        drop(transit);
        assert_eq!(sink.buffered_bytes(), 112 * 1024);
        assert!(retained.shrink_to(32));
        assert_eq!(sink.buffered_bytes(), 16 * 1024 + 32);
        drop((scratch, retained));
        assert_eq!(sink.buffered_bytes(), 0);
    }
    #[test]
    fn retained_whole_delta_counts_as_unconsumed_even_if_transport_partially_writes() {
        let (sink, receiver, rx) = stream(1);
        assert!(sink.emit_reserved_text(
            "pending".into(),
            sink.try_reserve_text(MAX_BUFFERED_TEXT_BYTES).unwrap()
        ));
        publish(&receiver, rx.recv().unwrap());
        let lease = receiver.recv_leased().unwrap();
        assert!(sink.try_reserve_text(1).is_none());
        std::thread::sleep(Duration::from_millis(20));
        assert!(sink.try_reserve_text(1).is_none());
        assert_eq!(sink.cancellation_reason(), Some(ErrorCode::SlowConsumer));
        assert_eq!(sink.buffered_bytes(), MAX_BUFFERED_TEXT_BYTES);
        drop(lease);
        assert_eq!(sink.buffered_bytes(), 0);
    }
    #[test]
    fn sink_splits_utf8_without_creating_a_second_budget() {
        struct Sink(Mutex<Vec<String>>);
        impl crate::ExecutionEventSink for Sink {
            fn emit(&self, event: ExecutorEvent) -> bool {
                if let ExecutorEvent::TextDelta(text) = event {
                    self.0.lock().unwrap().push(text);
                }
                true
            }
        }
        let sink = Arc::new(Sink(Mutex::new(Vec::new())));
        let events = ExecutionEvents::from_sink(42, sink.clone());
        let original = "你🙂".repeat(1000);
        assert!(events.text_delta(&original));
        let parts = sink.0.lock().unwrap();
        assert!(parts.iter().all(|p| p.len() <= MAX_DELTA_BYTES));
        assert_eq!(parts.concat(), original);
        assert!(events.try_reserve_text(1).is_none());
    }
}
