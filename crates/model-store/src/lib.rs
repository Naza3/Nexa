//! Offline model import and a crash-recoverable, native-free registry.
//!
//! A model directory, containing both the GGUF and manifest, is the commit unit.
//! Managed identities are derived from these directories under the store lock.
//! The external library also holds the atomic, metadata-only visibility overlay;
//! unregistering never changes a model directory or its manifest.
//! The data directory must be private to the application. The process lock
//! coordinates cooperative clients, not hostile writers with the same OS account.
mod gguf;
pub mod inventory;
pub mod library;
pub mod local_validation;
mod manifest;
mod store;
pub mod unregister;

pub use manifest::{
    Capabilities, ImportRequest, ModelManifest, ModelSource, ModelStorage, ProjectorAsset,
    ProjectorImportRequest, ValidationEvidence,
};
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
