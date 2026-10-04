//! Supervisor for Nexa's pinned, policy-constrained aria2 executable.
//!
//! aria2 owns HTTP, redirects, retries and Range. The caller owns binary trust,
//! protected staging, independent size/hash verification and atomic publication.
//! Exit zero means transfer finished, never that a model was saved or registered.
//! Production process containment is Windows-only; Unix support is test-only.
#[cfg(any(windows, all(unix, test)))]
mod output;
mod platform;
mod supervisor;
mod types;

pub use supervisor::transfer;
pub use types::*;
pub use url::Url;

pub mod identity;
