//! Versioned JSON replies; inputs are scalar capabilities, never paths or URIs.
pub fn verifier_open() -> String {
    crate::host::reply(crate::host::open())
}
pub fn verifier_snapshot(epoch: String) -> String {
    crate::host::reply(crate::host::snapshot(&epoch))
}
pub fn candidate_import(epoch: String, selection_token: String) -> String {
    crate::host::reply(crate::host::start_import(&epoch, &selection_token))
}
pub fn suite_start(epoch: String, suite_id: String) -> String {
    crate::host::reply(crate::host::start_suite(&epoch, &suite_id))
}
pub fn operation_next(epoch: String, operation_id: String, ack_sequence: Option<u64>) -> String {
    crate::host::reply(crate::host::poll(&epoch, &operation_id, ack_sequence))
}
pub fn operation_cancel(epoch: String, operation_id: String) -> String {
    crate::host::reply(crate::host::cancel(&epoch, &operation_id))
}
pub fn candidate_remove(epoch: String) -> String {
    crate::host::reply(crate::host::start_remove(&epoch))
}
pub fn report_prepare(epoch: String, operation_id: String) -> String {
    crate::host::reply(crate::host::report_prepare(&epoch, &operation_id))
}
