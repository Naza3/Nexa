//! Single-owner scheduler. Native pointers and platform transports stay outside.
mod executor;
mod output;
mod scheduler;
pub use executor::*;
pub use output::{EventReceiver, MAX_BUFFERED_TEXT_BYTES, MAX_DELTA_BYTES};
use runtime_types::{ModelId, ResolvedModel, RuntimeError};
pub use scheduler::{Runtime, RuntimeHandle};
/// A bounded metadata/path lookup. Perform imports and whole-file integrity
/// verification before starting the actor; never hash a model in this call.
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
