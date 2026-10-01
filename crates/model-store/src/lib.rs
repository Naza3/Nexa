//! Offline model import and a crash-recoverable, native-free registry.
//!
//! A model directory, containing both the GGUF and manifest, is the commit unit.
//! The registry index is derived from these directories while holding the store
//! lock; there is no second mutable index file that can disagree with a manifest.
//! The data directory must be private to the application. The process lock
//! coordinates cooperative clients, not hostile writers with the same OS account.
mod gguf;
mod manifest;
mod store;

pub use manifest::{Capabilities, ImportRequest, ModelManifest, ModelSource, ValidationEvidence};
pub use store::{ImportCancellation, ModelStore};

pub type Result<T> = std::result::Result<T, runtime_types::RuntimeError>;

fn invalid_manifest(message: &str) -> runtime_types::RuntimeError {
    runtime_types::RuntimeError::new(runtime_types::ErrorCode::InvalidManifest, message)
}
fn io_error(error: std::io::Error) -> runtime_types::RuntimeError {
    // Never leak a source/model path in diagnostics exposed by public methods.
    runtime_types::RuntimeError::new(
        runtime_types::ErrorCode::Io,
        format!("model storage I/O failed ({:?})", error.kind()),
    )
}
