//! Correlates native close requests with frontend acknowledgments. A delayed
//! native dialog must recheck its request ID before acting on the result.
use std::sync::Mutex;
use uuid::Uuid;

#[derive(Default)]
pub(crate) struct CloseRequestGate {
    pending: Mutex<Option<(Uuid, bool)>>,
}

impl CloseRequestGate {
    /// Repeated native close events share the one outstanding request.
    pub(crate) fn begin(&self) -> Option<Uuid> {
        let mut pending = self.pending.lock().unwrap();
        if pending.is_some() {
            return None;
        }
        let id = Uuid::new_v4();
        *pending = Some((id, true));
        Some(id)
    }

    /// Only the frontend acknowledgment for the outstanding request can clear it.
    pub(crate) fn acknowledge(&self, id: Uuid) -> bool {
        let mut pending = self.pending.lock().unwrap();
        if *pending != Some((id, true)) {
            return false;
        }
        *pending = None;
        true
    }

    /// Stop accepting a delayed frontend acknowledgment before showing native UI.
    /// The request stays pending so repeated close events remain coalesced.
    pub(crate) fn expire(&self, id: Uuid) -> bool {
        let mut pending = self.pending.lock().unwrap();
        if *pending != Some((id, true)) {
            return false;
        }
        *pending = Some((id, false));
        true
    }

    pub(crate) fn is_pending(&self, id: Uuid) -> bool {
        self.pending
            .lock()
            .unwrap()
            .is_some_and(|(pending_id, _)| pending_id == id)
    }

    pub(crate) fn clear(&self) {
        *self.pending.lock().unwrap() = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_begin_coalesces_until_the_matching_acknowledgment() {
        let gate = CloseRequestGate::default();
        let first = gate.begin().unwrap();
        assert!(!first.is_nil());
        assert!(gate.begin().is_none());
        assert!(gate.is_pending(first));
        assert!(gate.acknowledge(first));
        assert!(!gate.is_pending(first));
        let second = gate.begin().unwrap();
        assert_ne!(first, second);
        assert!(gate.is_pending(second));
    }

    #[test]
    fn wrong_or_stale_acknowledgments_preserve_the_current_request() {
        let gate = CloseRequestGate::default();
        let first = gate.begin().unwrap();
        assert!(!gate.acknowledge(Uuid::new_v4()));
        assert!(gate.is_pending(first));
        assert!(gate.acknowledge(first));
        assert!(!gate.acknowledge(first));
        let second = gate.begin().unwrap();
        assert!(!gate.acknowledge(first));
        assert!(gate.is_pending(second));
    }

    #[test]
    fn clearing_invalidates_delayed_dialogs_and_old_acknowledgments() {
        let gate = CloseRequestGate::default();
        let old_dialog = gate.begin().unwrap();
        gate.clear();
        assert!(!gate.is_pending(old_dialog));
        assert!(!gate.acknowledge(old_dialog));
        let current = gate.begin().unwrap();
        assert!(!gate.is_pending(old_dialog));
        assert!(gate.is_pending(current));
        gate.clear();
        gate.clear();
        assert!(!gate.is_pending(current));
        assert!(gate.begin().is_some());
    }
    #[test]
    fn expired_request_rejects_late_ack_and_coalesces_close_until_cleared() {
        let gate = CloseRequestGate::default();
        let expired = gate.begin().unwrap();
        assert!(!gate.expire(Uuid::new_v4()));
        assert!(gate.is_pending(expired));
        assert!(gate.expire(expired));
        assert!(!gate.expire(expired));
        assert!(!gate.acknowledge(expired));
        assert!(gate.is_pending(expired));
        assert!(gate.begin().is_none());
        gate.clear();
        assert!(!gate.is_pending(expired));
        let retry = gate.begin().unwrap();
        assert_ne!(expired, retry);
        assert!(!gate.expire(expired));
        assert!(!gate.acknowledge(expired));
        assert!(gate.acknowledge(retry));
    }
}
