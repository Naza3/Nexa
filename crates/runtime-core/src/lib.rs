//! Single-owner scheduler. Native pointers and platform transports stay outside.
mod executor;
mod output;
mod scheduler;
pub use executor::*;
pub use output::{
    DisconnectHandle, EventLease, EventReceiver, MAX_BUFFERED_TEXT_BYTES, MAX_DELTA_BYTES,
    TextPermit,
};
use runtime_types::{ModelId, ResolvedModel, RuntimeError};
pub use scheduler::{LoadControl, RegistryLease, Runtime, RuntimeHandle};
/// A bounded metadata/path lookup. Perform imports and whole-file integrity
/// verification before actor startup or outside it under a RegistryLease.
/// Never copy or hash a model in this scheduler-facing call.
pub trait ModelResolver: Send + Sync + 'static {
    fn resolve(&self, id: &ModelId) -> Result<ResolvedModel, RuntimeError>;
}
impl<F> ModelResolver for F
where
    F: Fn(&ModelId) -> Result<ResolvedModel, RuntimeError> + Send + Sync + 'static,
{
    fn resolve(&self, id: &ModelId) -> Result<ResolvedModel, RuntimeError> {
        self(id)
    }
}
