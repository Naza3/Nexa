//! Strict, bounded private worker protocol. Public request sequencing belongs to
//! runtime-core; `seq` here orders only worker frames within one process session.
use runtime_core::ExecutorEvent;
use runtime_types::{GenerationRequest, LoadOptions, RequestId, ResolvedModel, RuntimeError};
use serde::{Deserialize, Serialize};
pub use uuid::Uuid as SessionId;

pub const PROTOCOL_VERSION: u32 = 1;
pub const SHIM_VERSION: u32 = 2;
pub const LLAMA_COMMIT: &str = "2149c00f4442dc59302e134a02e4c99d5f7ed9fc";
/// Encoded UTF-8 bytes INCLUDING the final LF.
pub const MAX_REQUEST_FRAME_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_EVENT_FRAME_BYTES: usize = 64 * 1024;
pub const MAX_TEXT_BYTES: usize = 4096;
/// Two conservative slots account for worker/pipe/decoder/actor/consumer copies.
pub const TEXT_CREDIT_CHARGE: usize = 120 * 1024;
pub const TRANSPORT_SCRATCH_CHARGE: usize = 16 * 1024;
pub const MAX_TEXT_CREDITS: usize = 2;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hello {
    pub protocol_version: u32,
    pub shim_version: u32,
    pub llama_commit: String,
}
impl Hello {
    pub fn expected() -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            shim_version: SHIM_VERSION,
            llama_commit: LLAMA_COMMIT.into(),
        }
    }
    pub fn validate(&self) -> Result<(), RuntimeError> {
        if *self != Self::expected() {
            return Err(protocol_error("worker build identity mismatch"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "payload",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Message {
    Hello(Hello),
    Load {
        model: ResolvedModel,
        options: LoadOptions,
    },
    Generate {
        request: GenerationRequest,
    },
    Unload {},
    Cancel {},
    Credit {
        credit_id: u64,
    },
    Shutdown {},
    Event {
        event: ExecutorEvent,
        credit_id: Option<u64>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Frame {
    pub protocol_version: u32,
    pub session_id: SessionId,
    pub operation_id: u64,
    pub request_id: Option<RequestId>,
    /// Hello and commands have None. Worker events start at 1, never reset.
    pub seq: Option<u64>,
    #[serde(flatten)]
    pub message: Message,
}
impl Frame {
    pub fn hello(session_id: SessionId, hello: Hello) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            session_id,
            operation_id: 0,
            request_id: None,
            seq: None,
            message: Message::Hello(hello),
        }
    }
    pub fn command(
        session_id: SessionId,
        operation_id: u64,
        request_id: Option<RequestId>,
        message: Message,
    ) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            session_id,
            operation_id,
            request_id,
            seq: None,
            message,
        }
    }
    pub fn event(
        session_id: SessionId,
        operation_id: u64,
        request_id: Option<RequestId>,
        seq: u64,
        event: ExecutorEvent,
        credit_id: Option<u64>,
    ) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            session_id,
            operation_id,
            request_id,
            seq: Some(seq),
            message: Message::Event { event, credit_id },
        }
    }
}
pub fn protocol_error(message: &str) -> RuntimeError {
    RuntimeError::new(runtime_types::ErrorCode::NativeProtocol, message)
}

mod codec;
pub use codec::{MAX_TEXT_FRAME_BYTES, encode_frame, read_frame, write_frame};
mod validate;
pub use validate::{EventValidator, OperationKind};
