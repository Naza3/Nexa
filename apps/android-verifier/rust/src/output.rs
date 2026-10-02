//! Bounded one-consumer poll/ack transport; retrying is not consumption progress.
use crate::host::{Result, failure};
use runtime_core::CancellationHandle;
use serde_json::{Value, json};
use std::{
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
pub struct Operation {
    pub id: String,
    pub kind: &'static str,
    pub suite: Option<String>,
    pub cancelled: AtomicBool,
    pub control: Mutex<Option<CancellationHandle>>,
    pub terminal: Mutex<Option<Value>>,
    pub cases: Mutex<Vec<Value>>,
    pub started: String,
    pub output: Mutex<Buffer>,
    pub changed: Condvar,
    polling: AtomicBool,
}
pub struct Buffer {
    pending: Option<Value>,
    delivered: Option<Value>,
    last_ack: u64,
    discarded_delivery: Option<u64>,
    next: u64,
    progress: Instant,
    pub reason: Option<&'static str>,
}
impl Operation {
    pub fn new(kind: &'static str, suite: Option<String>) -> Arc<Self> {
        Arc::new(Self {
            id: uuid::Uuid::new_v4().to_string(),
            kind,
            suite,
            cancelled: AtomicBool::new(false),
            control: Mutex::new(None),
            terminal: Mutex::new(None),
            cases: Mutex::new(vec![]),
            started: crate::host::timestamp(),
            output: Mutex::new(Buffer {
                pending: None,
                delivered: None,
                last_ack: 0,
                discarded_delivery: None,
                next: 1,
                progress: Instant::now(),
                reason: None,
            }),
            changed: Condvar::new(),
            polling: AtomicBool::new(false),
        })
    }
    pub fn stop(&self, reason: &'static str) {
        self.cancelled.store(true, Ordering::Release);
        {
            let mut b = self.output.lock().unwrap();
            b.reason.get_or_insert(reason);
            b.pending = None;
            b.discarded_delivery = b
                .delivered
                .as_ref()
                .and_then(|v| v["sequence"].as_u64())
                .or(b.discarded_delivery);
            b.delivered = None;
        }
        if let Some(c) = self.control.lock().unwrap().as_ref() {
            c.cancel();
        }
        self.changed.notify_all();
    }
    pub fn stopped(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
    pub fn bind(&self, c: CancellationHandle) {
        let mut active = self.control.lock().unwrap();
        if self.stopped() {
            c.cancel();
        }
        *active = Some(c);
    }
    pub fn clear_control(&self) {
        self.control.lock().unwrap().take();
    }
    pub fn emit(&self, kind: &'static str, payload: Value) -> bool {
        if self.stopped() {
            return false;
        }
        let mut b = self.output.lock().unwrap();
        while b.pending.is_some() && !self.stopped() {
            if b.progress.elapsed() >= Duration::from_secs(10) {
                drop(b);
                self.stop("slow_consumer");
                return false;
            }
            b = self
                .changed
                .wait_timeout(b, Duration::from_millis(100))
                .unwrap()
                .0;
        }
        if self.stopped() {
            return false;
        }
        let event = json!({"sequence":b.next,"kind":kind,"payload":payload});
        if serde_json::to_vec(&event).unwrap().len() > 16384 || b.next > 4096 {
            drop(b);
            self.stop("output_limit");
            return false;
        }
        b.next += 1;
        b.pending = Some(event);
        self.changed.notify_all();
        true
    }
    pub fn text(&self, text: &str) -> bool {
        if text.is_empty() || text.len() > 4096 {
            self.stop("native_protocol");
            return false;
        }
        self.emit("text_delta", json!({"text":text}))
    }
    pub fn next(&self, ack: Option<u64>) -> Result<Value> {
        if self
            .polling
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(failure("busy"));
        }
        struct Guard<'a>(&'a AtomicBool);
        impl Drop for Guard<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        let _guard = Guard(&self.polling);
        let mut b = self.output.lock().unwrap();
        if let Some(ack) = ack {
            if ack == b.last_ack && ack != 0 {
            } else if b.discarded_delivery == Some(ack) {
                b.discarded_delivery = None;
                b.last_ack = ack;
            } else if b.delivered.as_ref().and_then(|v| v["sequence"].as_u64()) == Some(ack) {
                b.delivered.take();
                b.last_ack = ack;
                b.progress = Instant::now();
                self.changed.notify_all();
            } else {
                return Err(failure("invalid_ack"));
            }
        }
        if b.delivered.is_none() && b.pending.is_none() && self.terminal.lock().unwrap().is_none() {
            b = self
                .changed
                .wait_timeout(b, Duration::from_secs(1))
                .unwrap()
                .0;
        }
        if b.delivered.is_none() {
            b.delivered = b.pending.take();
            self.changed.notify_all();
        }
        let terminal = if b.delivered.is_none() && b.pending.is_none() {
            self.terminal.lock().unwrap().clone()
        } else {
            None
        };
        Ok(json!({"operation_id":self.id,"event":b.delivered,"terminal":terminal}))
    }
    pub fn watchdog(self: &Arc<Self>) {
        let op = self.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(250));
                if op.terminal.lock().unwrap().is_some() {
                    break;
                }
                if op.output.lock().unwrap().progress.elapsed() >= Duration::from_secs(10) {
                    op.stop("slow_consumer");
                    break;
                }
            }
        });
    }
    pub fn heartbeat(&self) {
        let mut b = self.output.lock().unwrap();
        if b.delivered.is_none() && b.pending.is_none() {
            b.progress = Instant::now();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_one_poll_can_wait() {
        let op = Operation::new("suite", None);
        let other = op.clone();
        let waiter = std::thread::spawn(move || other.next(None));
        let start = Instant::now();
        while !op.polling.load(Ordering::Acquire) {
            assert!(start.elapsed() < Duration::from_secs(2));
            std::thread::yield_now();
        }
        assert_eq!(op.next(None).unwrap_err().code, "busy");
        op.stop("request_cancelled");
        assert!(waiter.join().unwrap().is_ok());
    }
    #[test]
    fn no_consumption_triggers_real_ten_second_deadline() {
        let op = Operation::new("suite", None);
        op.text("delivered");
        op.next(None).unwrap();
        op.text("pending");
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = cancelled.clone();
        op.bind(CancellationHandle::new(move || {
            flag.store(true, Ordering::Release)
        }));
        let start = Instant::now();
        assert!(!op.text("blocked"));
        assert!(start.elapsed() >= Duration::from_millis(9900));
        assert!(cancelled.load(Ordering::Acquire));
        assert_eq!(op.output.lock().unwrap().reason, Some("slow_consumer"));
    }
    #[test]
    fn replay_ack_and_budget() {
        let op = Operation::new("suite", None);
        assert!(op.text("你好"));
        let first = op.next(None).unwrap();
        assert_eq!(first, op.next(None).unwrap());
        assert!(op.next(Some(2)).is_err());
        assert!(op.text("next"));
        assert_eq!(op.next(Some(1)).unwrap()["event"]["sequence"], 2);
        assert_eq!(op.next(Some(1)).unwrap()["event"]["sequence"], 2);
        assert!(!op.text(&"a".repeat(4097)));
        assert!(op.stopped());
    }
    #[test]
    fn cancellation_accepts_the_in_flight_ack() {
        let op = Operation::new("suite", None);
        op.text("hello");
        assert_eq!(op.next(None).unwrap()["event"]["sequence"], 1);
        op.stop("backgrounded");
        *op.terminal.lock().unwrap() = Some(json!({"outcome":"cancelled"}));
        assert!(op.next(Some(1)).is_ok());
        assert!(op.next(Some(1)).is_ok());
        assert!(op.next(Some(2)).is_err());
    }
    #[test]
    fn cancellation_does_not_wait_for_full_slot() {
        let op = Operation::new("suite", None);
        op.text("first");
        let other = op.clone();
        let producer = std::thread::spawn(move || other.text("blocked"));
        op.stop("request_cancelled");
        assert!(!producer.join().unwrap());
    }
}
