use axum::{
    Json,
    http::{HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use runtime_types::{ErrorCode, RuntimeError};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct ErrorDetail {
    pub message: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub param: Option<String>,
    pub code: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct ApiError {
    #[serde(skip)]
    pub status: StatusCode,
    pub error: ErrorDetail,
}
impl ApiError {
    pub fn new(
        status: StatusCode,
        code: impl Into<String>,
        message: impl Into<String>,
        param: Option<&str>,
    ) -> Self {
        Self {
            status,
            error: ErrorDetail {
                message: message.into(),
                kind: if status.is_server_error() {
                    "server_error"
                } else {
                    "invalid_request_error"
                }
                .into(),
                param: param.map(str::to_owned),
                code: code.into(),
            },
        }
    }
    pub fn invalid(param: &str, message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            message,
            Some(param),
        )
    }
    pub fn unsupported(param: &str) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "unsupported_parameter",
            "This parameter or value is not supported by the text-only API.",
            Some(param),
        )
    }
    pub fn internal() -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "The runtime could not complete this operation.",
            None,
        )
    }
    pub fn from_generation(error: RuntimeError) -> Self {
        if error.code == ErrorCode::NativeFailure {
            Self::internal()
        } else {
            error.into()
        }
    }
    pub fn busy() -> Self {
        RuntimeError::new(ErrorCode::RuntimeBusy, "busy").into()
    }
    pub fn response_too_large() -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "response_too_large",
            "The complete JSON response exceeds 96 KiB. Use streaming or a smaller output budget.",
            Some("stream"),
        )
    }
}
impl From<RuntimeError> for ApiError {
    fn from(value: RuntimeError) -> Self {
        use ErrorCode::*;
        let (status, code, message, param) = match value.code {
            InvalidArgument | InvalidManifest | IntegrityFailure => (
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "The request or model registration is invalid.",
                None,
            ),
            UnsupportedModel => (
                StatusCode::BAD_REQUEST,
                "unsupported_model",
                "This model has no verified configuration in this build.",
                Some("model"),
            ),
            UnsupportedChatTemplate => (
                StatusCode::BAD_REQUEST,
                "unsupported_chat_template",
                "The model chat template is unsupported.",
                Some("messages"),
            ),
            ContextLengthExceeded => (
                StatusCode::BAD_REQUEST,
                "context_length_exceeded",
                "The requested context or token budget exceeds the verified model configuration.",
                Some("messages"),
            ),
            ModelNotFound => (
                StatusCode::NOT_FOUND,
                "model_not_found",
                "The registered model was not found.",
                Some("model"),
            ),
            RequestNotFound => (
                StatusCode::NOT_FOUND,
                "request_not_found",
                "The active request was not found.",
                None,
            ),
            ModelConflict => (
                StatusCode::CONFLICT,
                "model_conflict",
                "Unload or explicitly load the requested model while idle.",
                Some("model"),
            ),
            RuntimeBusy | AlreadyExists => (
                StatusCode::CONFLICT,
                value.code.as_str(),
                "The runtime or registry is busy, or the model ID already exists.",
                None,
            ),
            DuplicateRequestId => (
                StatusCode::CONFLICT,
                "duplicate_request_id",
                "This request ID is already active.",
                Some("X-Request-ID"),
            ),
            RequestCancelled | ConsumerStopped | SlowConsumer => (
                StatusCode::REQUEST_TIMEOUT,
                value.code.as_str(),
                "The request was cancelled.",
                None,
            ),
            QueueFull => (
                StatusCode::TOO_MANY_REQUESTS,
                "queue_full",
                "The bounded request queue is full. Retry later.",
                None,
            ),
            QueueTimeout | LoadTimeout | ExecutionTimeout => (
                StatusCode::GATEWAY_TIMEOUT,
                value.code.as_str(),
                "The operation exceeded its deadline.",
                None,
            ),
            RuntimeFaulted | ExecutorCleanupUnconfirmed => (
                StatusCode::SERVICE_UNAVAILABLE,
                "runtime_faulted",
                "The executor is faulted or its cleanup could not be confirmed.",
                None,
            ),
            ExecutorUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "worker_lost",
                "The worker is unavailable.",
                None,
            ),
            RuntimeShutdown => (
                StatusCode::SERVICE_UNAVAILABLE,
                "runtime_shutdown",
                "The runtime is shutting down.",
                None,
            ),
            NativeFailure => (
                StatusCode::SERVICE_UNAVAILABLE,
                "model_load_failed",
                "The model operation failed.",
                None,
            ),
            InsufficientSpace => (
                StatusCode::SERVICE_UNAVAILABLE,
                "insufficient_storage",
                "Insufficient storage for the private model copy.",
                None,
            ),
            ModelDirectoryRequired
            | ModelDirectoryUnavailable
            | ModelDirectoryUnsupported
            | ModelLibraryUnsupported
            | ModelLibraryLimit
            | ModelLibraryChanged
            | ModelListChanged
            | ModelScanTimeout
            | ModelScanCancelled
            | ModelFileChanged
            | ModelFileUnavailable
            | ModelFileInUse
            | ModelLibraryWriteFailed => (
                StatusCode::BAD_REQUEST,
                value.code.as_str(),
                "The selected model library could not be used safely. Refresh its state before retrying.",
                Some("model_library"),
            ),
            Io | WrongThread | NativeProtocol => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "The runtime could not complete this operation.",
                None,
            ),
        };
        Self::new(status, code, message, param)
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let retry = self.status == StatusCode::TOO_MANY_REQUESTS;
        let mut response = (self.status, Json(self)).into_response();
        if retry {
            response
                .headers_mut()
                .insert("retry-after", HeaderValue::from_static("1"));
        }
        response
    }
}
impl std::fmt::Display for ApiError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.error.code, self.error.message)
    }
}
impl std::error::Error for ApiError {}
