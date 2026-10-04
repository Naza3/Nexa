//! Loopback HTTP management, strict text-only API, and bounded transport.
//! This crate and the CLI never link the native inference libraries.
mod config;
pub mod dto;
pub mod errors;
pub mod probe;
pub mod service;
pub mod state;
pub use config::Config;
pub use errors::ApiError;
pub use service::ServiceShutdown;
pub use state::ApiState;
pub mod chat;
pub mod proof;
pub mod routes;
pub mod security;
pub mod token;
pub mod transport;
pub use routes::router;
