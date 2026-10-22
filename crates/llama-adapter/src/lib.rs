//! Safe, thread-affine access to Nexa's pinned llama.cpp shim.
//!
//! All model work runs on the creating inference thread. The borrow chain is
//! Engine → Model → Prepared; only [`CancelHandle`] may cross threads. A prepared
//! request is consumed by generation on every result, including cancellation.
//!
//! ```compile_fail
//! use llama_adapter::Engine;
//! let engine = Engine::new().unwrap();
//! std::thread::spawn(move || drop(engine)); // Engines cannot change threads.
//! ```
//! ```compile_fail
//! use llama_adapter::{CancelHandle, Engine};
//! use runtime_types::LoadOptions;
//! let mut engine = Engine::new().unwrap();
//! let cancel = CancelHandle::new().unwrap();
//! let model = engine.load("model.gguf", LoadOptions::default(), &cancel).unwrap();
//! drop(engine); // A model keeps its engine alive.
//! drop(model);
//! ```
//! ```compile_fail
//! use llama_adapter::Model;
//! fn require_send<T: Send>() {}
//! require_send::<Model<'static>>();
//! ```
//! ```compile_fail
//! use llama_adapter::Prepared;
//! fn require_send<T: Send>() {}
//! require_send::<Prepared<'static>>();
//! ```
//! ```compile_fail
//! use llama_adapter::Engine;
//! fn require_sync<T: Sync>() {}
//! require_sync::<Engine>();
//! ```
//! ```compile_fail
//! use llama_adapter::Model;
//! fn require_sync<T: Sync>() {}
//! require_sync::<Model<'static>>();
//! ```
//! ```compile_fail
//! use llama_adapter::Prepared;
//! fn require_sync<T: Sync>() {}
//! require_sync::<Prepared<'static>>();
//! ```

mod chat;
mod ffi;
pub use chat::GeneratedDelta;

use runtime_types::{
    ErrorCode, FinishReason, GenerationOptions, LoadOptions, Message, RuntimeError, Usage,
    validate_messages,
};
use std::{
    any::Any,
    ffi::c_void,
    fmt,
    marker::PhantomData,
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
    path::Path,
    ptr::NonNull,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

/// A native engine. At most one engine may be alive in a process.
pub struct Engine {
    raw: NonNull<ffi::AirEngine>,
    _thread: PhantomData<Rc<()>>,
}

impl Engine {
    pub fn new() -> Result<Self, RuntimeError> {
        // Every product entry point, including direct EngineHost users, rejects
        // stale AIR_NATIVE_DIR archives before creating any native resources.
        verify_build_identity(&build_info()?)?;
        let mut raw = std::ptr::null_mut();
        let mut error = ffi::AirError::default();
        // SAFETY: Outputs are initialized and writable for this synchronous call.
        let status = unsafe { ffi::air_engine_create(&mut raw, &mut error) };
        check_status(status, error)?;
        Ok(Self {
            raw: nonnull(raw)?,
            _thread: PhantomData,
        })
    }

    /// Loads one model. Non-UTF-8 paths and interior NULs are rejected, never
    /// converted lossily. The native loader implements Windows UTF-8 paths.
    pub fn load<'engine>(
        &'engine mut self,
        path: impl AsRef<Path>,
        options: LoadOptions,
        cancel: &CancelHandle,
    ) -> Result<Model<'engine>, RuntimeError> {
        self.load_inner(path.as_ref(), None, options, cancel)
    }

    /// Loads a text model and its verified image projector on this same thread.
    pub fn load_with_projector<'engine>(
        &'engine mut self,
        path: impl AsRef<Path>,
        projector_path: impl AsRef<Path>,
        options: LoadOptions,
        cancel: &CancelHandle,
    ) -> Result<Model<'engine>, RuntimeError> {
        self.load_inner(
            path.as_ref(),
            Some(projector_path.as_ref()),
            options,
            cancel,
        )
    }

    fn load_inner<'engine>(
        &'engine mut self,
        path: &Path,
        projector_path: Option<&Path>,
        options: LoadOptions,
        cancel: &CancelHandle,
    ) -> Result<Model<'engine>, RuntimeError> {
        options.validate()?;
        let projector_path = projector_path
            .map(|path| {
                let value = path
                    .to_str()
                    .ok_or_else(|| RuntimeError::invalid("projector path must be valid UTF-8"))?;
                if value.is_empty()
                    || value.contains('\0')
                    || value.len() > runtime_types::MAX_MESSAGE_BYTES
                {
                    return Err(RuntimeError::invalid("invalid projector path"));
                }
                Ok(value)
            })
            .transpose()?;
        let path = path
            .to_str()
            .ok_or_else(|| RuntimeError::invalid("model path must be valid UTF-8"))?;
        if path.is_empty() || path.contains('\0') || path.len() > runtime_types::MAX_MESSAGE_BYTES {
            return Err(RuntimeError::invalid(
                "model path is empty, contains NUL, or exceeds the byte bound",
            ));
        }
        let mut raw = std::ptr::null_mut();
        let mut error = ffi::AirError::default();
        let options = ffi::AirLoadOptions {
            context_size: options.context_size,
            threads: options.threads,
            batch_size: options.batch_size,
        };
        // SAFETY: Self is exclusively borrowed on its creating thread; path and
        // cancellation storage outlive the synchronous native call.
        let status = unsafe {
            if let Some(projector_path) = projector_path {
                ffi::air_model_load_with_projector(
                    self.raw.as_ptr(),
                    ffi::AirString::borrowed(path),
                    ffi::AirString::borrowed(projector_path),
                    options,
                    cancel.raw(),
                    &mut raw,
                    &mut error,
                )
            } else {
                ffi::air_model_load(
                    self.raw.as_ptr(),
                    ffi::AirString::borrowed(path),
                    options,
                    cancel.raw(),
                    &mut raw,
                    &mut error,
                )
            }
        };
        check_status(status, error)?;
        Ok(Model {
            raw: nonnull(raw)?,
            _engine: PhantomData,
        })
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        // SAFETY: The borrowing API prevents any model from outliving its engine.
        unsafe { ffi::air_engine_destroy(self.raw.as_ptr()) };
    }
}

/// A model/context pair borrowed from its engine, used only on that thread.
pub struct Model<'engine> {
    raw: NonNull<ffi::AirModel>,
    _engine: PhantomData<&'engine mut Engine>,
}

impl Model<'_> {
    /// Copies the original GGUF template for reproducibility checks.
    pub fn chat_template(&self) -> Result<String, RuntimeError> {
        let mut buffer = ffi::AirBuffer::default();
        let mut error = ffi::AirError::default();
        // SAFETY: The model is live and thread-affine; outputs are writable.
        let status = unsafe { ffi::air_model_template(self.raw.as_ptr(), &mut buffer, &mut error) };
        let buffer = OwnedBuffer(buffer);
        check_status(status, error)?;
        buffer.to_string()
    }

    /// Applies the model's actual template and tokenizer and checks the complete
    /// prompt + output budget. It does not truncate input or perform inference.
    pub fn prepare<'model>(
        &'model mut self,
        messages: &[Message],
        options: &GenerationOptions,
        cancel: &CancelHandle,
    ) -> Result<Prepared<'model>, RuntimeError> {
        validate_messages(messages)?;
        options.validate()?;
        let image = messages.iter().find_map(|message| message.image.as_ref());
        let image_bytes = image.map(runtime_types::ImageInput::decode).transpose()?;
        let image_after_text = image.is_some_and(|image| image.after_text);
        let messages: Vec<_> = messages
            .iter()
            .map(|message| ffi::AirMessage {
                role: ffi::AirString::borrowed(message.role.as_str()),
                content: ffi::AirString::borrowed(message.content.as_deref().unwrap_or_default()),
            })
            .collect();
        let stops: Vec<_> = options
            .stops
            .iter()
            .map(|stop| ffi::AirString::borrowed(stop))
            .collect();
        let mut raw = std::ptr::null_mut();
        let mut prompt_tokens = 0;
        let mut error = ffi::AirError::default();
        let options = ffi::AirGenerateOptions {
            max_tokens: options.max_tokens,
            temperature: options.temperature,
            top_p: options.top_p,
            seed: options.seed,
        };
        // SAFETY: The model is exclusively borrowed; all borrowed input arrays
        // remain live for the call. The shim copies everything into Prepared.
        let status = unsafe {
            if let Some(image) = &image_bytes {
                ffi::air_prepare_image(
                    self.raw.as_ptr(),
                    messages[0].content,
                    image.as_ptr(),
                    image.len() as u64,
                    u32::from(image_after_text),
                    options,
                    stops.as_ptr(),
                    stops.len() as u64,
                    cancel.raw(),
                    &mut raw,
                    &mut prompt_tokens,
                    &mut error,
                )
            } else {
                ffi::air_prepare(
                    self.raw.as_ptr(),
                    messages.as_ptr(),
                    messages.len() as u64,
                    options,
                    stops.as_ptr(),
                    stops.len() as u64,
                    cancel.raw(),
                    &mut raw,
                    &mut prompt_tokens,
                    &mut error,
                )
            }
        };
        check_status(status, error)?;
        Ok(Prepared {
            raw: Some(nonnull(raw)?),
            prompt_tokens,
            chat: None,
            _model: PhantomData,
            _thread: PhantomData,
        })
    }
}

impl Drop for Model<'_> {
    fn drop(&mut self) {
        // SAFETY: Prepared exclusively borrows its model; Engine outlives it.
        unsafe { ffi::air_model_unload(self.raw.as_ptr()) };
    }
}

/// Exactly one prepared request, borrowing its model until consumed or dropped.
pub struct Prepared<'model> {
    raw: Option<NonNull<ffi::AirPrepared>>,
    prompt_tokens: u32,
    chat: Option<chat::ChatContext>,
    _model: PhantomData<&'model mut ffi::AirModel>,
    _thread: PhantomData<Rc<()>>,
}

impl Prepared<'_> {
    pub const fn prompt_tokens(&self) -> u32 {
        self.prompt_tokens
    }

    /// Streams complete UTF-8 pieces, at most 4096 bytes per callback. The
    /// callback may use a bounded, cancellation-interruptible output budget
    /// wait between decode operations. It must never wait indefinitely or hold
    /// additional native locks. Returning Stop aborts as ConsumerStopped.
    ///
    /// A callback panic is caught at the C boundary. Native resources are cleaned
    /// up before the panic resumes on this Rust stack (with panic=unwind).
    pub fn generate<F>(
        self,
        cancel: &CancelHandle,
        on_text: F,
    ) -> Result<GenerationResult, GenerationError>
    where
        F: FnMut(&str) -> StreamControl,
    {
        self.generate_observed(cancel, on_text, |_| StreamControl::Continue)
    }

    /// Adds synchronous, numeric phase evidence. Progress callbacks must return
    /// promptly and are panic-isolated exactly like text callbacks. Batch
    /// completion means successful native evaluation, never a timing estimate.
    pub fn generate_observed<F, P>(
        mut self,
        cancel: &CancelHandle,
        on_text: F,
        on_progress: P,
    ) -> Result<GenerationResult, GenerationError>
    where
        F: FnMut(&str) -> StreamControl,
        P: FnMut(GenerationProgress) -> StreamControl,
    {
        let mut progress = ProgressCallbackState {
            on_progress,
            panic: None,
            protocol_error: None,
            stopped: false,
        };
        let mut callback = CallbackState {
            on_text,
            panic: None,
            protocol_error: None,
            stopped: false,
        };
        let mut usage = ffi::AirUsage::default();
        let mut error = ffi::AirError::default();
        // Transfer ownership before entering native code: air_generate consumes
        // the pointer on success, error, cancellation, and callback rejection.
        let raw = self
            .raw
            .take()
            .expect("prepared handle exists until generation");
        // SAFETY: All references stay live during this synchronous call. The
        // callback catches panics and never retains native borrowed text.
        let status = unsafe {
            ffi::air_generate_observed(
                raw.as_ptr(),
                cancel.raw(),
                text_callback::<F>,
                (&mut callback as *mut CallbackState<F>).cast(),
                progress_callback::<P>,
                (&mut progress as *mut ProgressCallbackState<P>).cast(),
                &mut usage,
                &mut error,
            )
        };
        let result = check_status(status, error);
        if let Some(panic) = callback.panic.take().or_else(|| progress.panic.take()) {
            resume_unwind(panic);
        }
        let usage_value = Usage {
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
        };
        if let Some(error) = callback.protocol_error.or(progress.protocol_error) {
            return Err(GenerationError {
                error,
                usage: usage_value,
            });
        }
        result.map_err(|error| GenerationError {
            error,
            usage: usage_value,
        })?;
        let finish_reason = match usage.finish_reason {
            0 => FinishReason::Stop,
            1 => FinishReason::Length,
            _ => {
                return Err(GenerationError {
                    error: protocol("native success has an invalid finish reason"),
                    usage: usage_value,
                });
            }
        };
        if callback.stopped || progress.stopped {
            return Err(GenerationError {
                error: protocol("native generation ignored consumer stop"),
                usage: usage_value,
            });
        }
        Ok(GenerationResult {
            usage: usage_value,
            finish_reason,
        })
    }
}

impl Drop for Prepared<'_> {
    fn drop(&mut self) {
        if let Some(raw) = self.raw.take() {
            // SAFETY: This handle has not been transferred to air_generate.
            unsafe { ffi::air_prepared_free(raw.as_ptr()) };
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GenerationPhase {
    PrefillStarted,
    PrefillBatchCompleted,
    DecodeStarted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GenerationProgress {
    pub phase: GenerationPhase,
    pub completed_prompt_tokens: u32,
    pub total_prompt_tokens: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamControl {
    Continue,
    Stop,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GenerationResult {
    pub usage: Usage,
    pub finish_reason: FinishReason,
}

/// Failed/cancelled generations retain actual usage; partial text is never
/// silently retried. A host maps this result into exactly one terminal event.
#[derive(Debug)]
pub struct GenerationError {
    pub error: RuntimeError,
    pub usage: Usage,
}
impl fmt::Display for GenerationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(f)
    }
}
impl std::error::Error for GenerationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

/// One-shot, cloneable cancellation shared with the independent control thread.
/// A new request requires a new handle. Dropping a clone does not cancel work.
#[derive(Clone)]
pub struct CancelHandle(Arc<CancelInner>);
struct CancelInner {
    raw: NonNull<ffi::AirCancel>,
    cancelled: AtomicBool,
}
// SAFETY: The ABI promises this isolated object is an atomic cancellation flag.
// All operations are concurrent sets; Arc postpones its destroy until no Rust
// references or native calls using those references remain. No model is shared.
unsafe impl Send for CancelInner {}
// SAFETY: Same atomic flag guarantee; air_cancel_set never touches model state.
unsafe impl Sync for CancelInner {}
impl CancelHandle {
    pub fn new() -> Result<Self, RuntimeError> {
        let mut raw = std::ptr::null_mut();
        let mut error = ffi::AirError::default();
        // SAFETY: Valid output slots; no native model state is involved.
        let status = unsafe { ffi::air_cancel_create(&mut raw, &mut error) };
        check_status(status, error)?;
        Ok(Self(Arc::new(CancelInner {
            raw: nonnull(raw)?,
            cancelled: AtomicBool::new(false),
        })))
    }
    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::Release);
        // SAFETY: Arc keeps the thread-safe native flag alive for this call.
        unsafe { ffi::air_cancel_set(self.raw()) };
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::Acquire)
    }
    fn raw(&self) -> *mut ffi::AirCancel {
        self.0.raw.as_ptr()
    }
}
impl Drop for CancelInner {
    fn drop(&mut self) {
        // SAFETY: Arc has no remaining users, including in-flight native calls.
        unsafe { ffi::air_cancel_destroy(self.raw.as_ptr()) };
    }
}

/// JSON text from the actual native build, including commit/backend/shim version.
pub fn build_info() -> Result<String, RuntimeError> {
    let mut buffer = ffi::AirBuffer::default();
    let mut error = ffi::AirError::default();
    // SAFETY: This read-only ABI operation initializes the output slots.
    let status = unsafe { ffi::air_get_build_info(&mut buffer, &mut error) };
    let buffer = OwnedBuffer(buffer);
    check_status(status, error)?;
    buffer.to_string()
}

const EXPECTED_BUILD_INFO: &str = concat!(
    "{\"shim_version\":5,\"backend\":\"cpu\",\"llama_commit\":\"",
    "2149c00f4442dc59302e134a02e4c99d5f7ed9fc\"}"
);
fn verify_build_identity(info: &str) -> Result<(), RuntimeError> {
    // The locked shim owns this exact bounded JSON encoding. Strict equality
    // also rejects duplicate keys; substring version checks are insufficient.
    if info != EXPECTED_BUILD_INFO {
        return Err(protocol(
            "native behavior identity mismatch; rebuild the pinned shim",
        ));
    }
    Ok(())
}

fn nonnull<T>(raw: *mut T) -> Result<NonNull<T>, RuntimeError> {
    NonNull::new(raw).ok_or_else(|| protocol("native success returned a null handle"))
}
fn protocol(message: &str) -> RuntimeError {
    RuntimeError::new(ErrorCode::NativeProtocol, message)
}

struct OwnedBuffer(ffi::AirBuffer);
impl OwnedBuffer {
    fn to_string(&self) -> Result<String, RuntimeError> {
        if self.0.len == 0 {
            return Ok(String::new());
        }
        if self.0.data.is_null() || self.0.len > isize::MAX as u64 {
            return Err(protocol("invalid native output buffer"));
        }
        // SAFETY: Shim-owned memory is valid for len bytes until Drop. The ABI
        // contract guarantees provenance; size/null are checked before slicing.
        let bytes = unsafe { std::slice::from_raw_parts(self.0.data, self.0.len as usize) };
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| protocol("native output buffer is not UTF-8"))
    }
}
impl Drop for OwnedBuffer {
    fn drop(&mut self) {
        // SAFETY: The shim is the allocator; ownership is transferred only once.
        unsafe { ffi::air_buffer_free(self.0) };
    }
}
fn check_status(status: i32, error: ffi::AirError) -> Result<(), RuntimeError> {
    let buffer = OwnedBuffer(error.message);
    if status == 0 {
        if error.code != 0 {
            return Err(protocol("native success carried an error code"));
        }
        return Ok(());
    }
    if error.code != status {
        return Err(protocol("native status/error code mismatch"));
    }
    let code = match status {
        1 => ErrorCode::InvalidArgument,
        2 => ErrorCode::RequestCancelled,
        3 => ErrorCode::UnsupportedModel,
        4 => ErrorCode::UnsupportedChatTemplate,
        5 => ErrorCode::ContextLengthExceeded,
        6 => ErrorCode::NativeFailure,
        7 => ErrorCode::WrongThread,
        8 => ErrorCode::ConsumerStopped,
        9 => ErrorCode::InvalidToolOutput,
        10 => ErrorCode::IncompleteGeneration,
        11 => ErrorCode::ToolOutputLimitExceeded,
        _ => ErrorCode::NativeProtocol,
    };
    let message = buffer.to_string()?;
    Err(RuntimeError::new(
        code,
        if message.is_empty() {
            code.as_str().to_owned()
        } else {
            message
        },
    ))
}

struct CallbackState<F> {
    on_text: F,
    panic: Option<Box<dyn Any + Send>>,
    protocol_error: Option<RuntimeError>,
    stopped: bool,
}
unsafe extern "C" fn text_callback<F>(user: *mut c_void, text: ffi::AirString) -> i32
where
    F: FnMut(&str) -> StreamControl,
{
    // SAFETY: air_generate only calls with our unique stack state, synchronously.
    let state = unsafe { &mut *user.cast::<CallbackState<F>>() };
    if state.panic.is_some() || state.protocol_error.is_some() || state.stopped {
        return 1;
    }
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        if text.len == 0 || text.len > 4096 || text.data.is_null() {
            state.protocol_error = Some(protocol("invalid text callback buffer"));
            return StreamControl::Stop;
        }
        // SAFETY: Shim promises valid borrowed bytes until this callback returns.
        let bytes = unsafe { std::slice::from_raw_parts(text.data, text.len as usize) };
        match std::str::from_utf8(bytes) {
            Ok(text) => (state.on_text)(text),
            Err(_) => {
                state.protocol_error = Some(protocol("text callback contains invalid UTF-8"));
                StreamControl::Stop
            }
        }
    }));
    match outcome {
        Ok(StreamControl::Continue) => 0,
        Ok(StreamControl::Stop) => {
            state.stopped = true;
            1
        }
        Err(panic) => {
            state.panic = Some(panic);
            1
        }
    }
}

struct ProgressCallbackState<P> {
    on_progress: P,
    panic: Option<Box<dyn Any + Send>>,
    protocol_error: Option<RuntimeError>,
    stopped: bool,
}
unsafe extern "C" fn progress_callback<P>(
    user: *mut c_void,
    phase: u32,
    completed: u32,
    total: u32,
) -> i32
where
    P: FnMut(GenerationProgress) -> StreamControl,
{
    // SAFETY: The shim invokes this unique borrowed stack state synchronously.
    let state = unsafe { &mut *user.cast::<ProgressCallbackState<P>>() };
    if state.panic.is_some() || state.protocol_error.is_some() || state.stopped {
        return 1;
    }
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let valid = total > 0 && completed <= total;
        let phase = match phase {
            0 if valid && completed == 0 => GenerationPhase::PrefillStarted,
            1 if valid && completed > 0 => GenerationPhase::PrefillBatchCompleted,
            2 if valid && completed == total => GenerationPhase::DecodeStarted,
            _ => {
                state.protocol_error = Some(protocol("invalid native progress observation"));
                return StreamControl::Stop;
            }
        };
        (state.on_progress)(GenerationProgress {
            phase,
            completed_prompt_tokens: completed,
            total_prompt_tokens: total,
        })
    }));
    match outcome {
        Ok(StreamControl::Continue) => 0,
        Ok(StreamControl::Stop) => {
            state.stopped = true;
            1
        }
        Err(panic) => {
            state.panic = Some(panic);
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invoke<F: FnMut(&str) -> StreamControl>(state: &mut CallbackState<F>, bytes: &[u8]) -> i32 {
        // SAFETY: Valid synchronous fixture buffers and state, just as in the ABI.
        unsafe {
            text_callback::<F>(
                (state as *mut CallbackState<F>).cast(),
                ffi::AirString {
                    data: bytes.as_ptr(),
                    len: bytes.len() as u64,
                },
            )
        }
    }
    fn state<F: FnMut(&str) -> StreamControl>(on_text: F) -> CallbackState<F> {
        CallbackState {
            on_text,
            panic: None,
            protocol_error: None,
            stopped: false,
        }
    }

    #[test]
    fn callback_copies_complete_unicode() {
        let mut result = String::new();
        let mut callback = state(|text: &str| {
            result.push_str(text);
            StreamControl::Continue
        });
        assert_eq!(invoke(&mut callback, "你好🙂".as_bytes()), 0);
        assert!(callback.protocol_error.is_none());
        drop(callback);
        assert_eq!(result, "你好🙂");
    }
    #[test]
    fn callback_rejects_split_utf8_and_oversized_chunks() {
        for bytes in [vec![0xe4, 0xbd], vec![b'a'; 4097], vec![]] {
            let mut callback = state(|_: &str| panic!("invalid text must not reach consumer"));
            assert_eq!(invoke(&mut callback, &bytes), 1);
            assert!(callback.protocol_error.is_some());
            assert!(callback.panic.is_none());
        }
    }
    #[test]
    fn callback_panic_does_not_cross_ffi() {
        let mut callback = state(|_: &str| panic!("consumer panic fixture"));
        assert_eq!(invoke(&mut callback, b"valid"), 1);
        assert!(callback.panic.is_some());
        assert_eq!(invoke(&mut callback, b"again"), 1);
    }
    #[test]
    fn callback_stop_is_sticky() {
        let mut calls = 0;
        let mut callback = state(|_: &str| {
            calls += 1;
            StreamControl::Stop
        });
        assert_eq!(invoke(&mut callback, b"one"), 1);
        assert_eq!(invoke(&mut callback, b"two"), 1);
        drop(callback);
        assert_eq!(calls, 1);
    }
    fn invoke_progress<P: FnMut(GenerationProgress) -> StreamControl>(
        state: &mut ProgressCallbackState<P>,
        phase: u32,
        completed: u32,
        total: u32,
    ) -> i32 {
        // SAFETY: Synchronous fixture has the exact ABI callback state/layout.
        unsafe {
            progress_callback::<P>(
                (state as *mut ProgressCallbackState<P>).cast(),
                phase,
                completed,
                total,
            )
        }
    }
    fn progress_state<P: FnMut(GenerationProgress) -> StreamControl>(
        on_progress: P,
    ) -> ProgressCallbackState<P> {
        ProgressCallbackState {
            on_progress,
            panic: None,
            protocol_error: None,
            stopped: false,
        }
    }
    #[test]
    fn progress_panic_and_invalid_observations_never_cross_ffi() {
        let mut callback = progress_state(|_| panic!("progress consumer panic fixture"));
        assert_eq!(invoke_progress(&mut callback, 1, 16, 128), 1);
        assert!(callback.panic.is_some());
        assert_eq!(invoke_progress(&mut callback, 2, 128, 128), 1);
        for (phase, completed, total) in [
            (8, 0, 1),
            (0, 1, 2),
            (1, 0, 2),
            (1, 3, 2),
            (2, 1, 2),
            (0, 0, 0),
        ] {
            let mut callback =
                progress_state(|_| panic!("invalid progress must not reach observer"));
            assert_eq!(invoke_progress(&mut callback, phase, completed, total), 1);
            assert!(callback.protocol_error.is_some());
            assert!(callback.panic.is_none());
        }
    }
    #[test]
    fn progress_stop_is_sticky_and_records_successful_batch() {
        let mut calls = 0;
        let mut callback = progress_state(|event| {
            assert_eq!(event.phase, GenerationPhase::PrefillBatchCompleted);
            assert_eq!(event.completed_prompt_tokens, 16);
            assert_eq!(event.total_prompt_tokens, 128);
            calls += 1;
            StreamControl::Stop
        });
        assert_eq!(invoke_progress(&mut callback, 1, 16, 128), 1);
        assert_eq!(invoke_progress(&mut callback, 1, 32, 128), 1);
        drop(callback);
        assert_eq!(calls, 1);
    }
    #[test]
    fn cancellation_can_cross_threads() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<CancelHandle>();
        let cancel = CancelHandle::new().expect("create native cancellation flag");
        let copy = cancel.clone();
        std::thread::spawn(move || copy.cancel()).join().unwrap();
        cancel.cancel();
    }
    #[test]
    fn native_build_reports_pinned_abi() {
        let info = build_info().unwrap();
        assert!(info.contains("\"shim_version\":5"));
        assert!(info.contains("\"backend\":\"cpu\""));
        assert!(info.contains("2149c00f4442dc59302e134a02e4c99d5f7ed9fc"));
    }
    #[test]
    fn unknown_and_mismatched_statuses_fail_closed() {
        assert_eq!(
            check_status(
                88,
                ffi::AirError {
                    code: 88,
                    ..Default::default()
                }
            )
            .unwrap_err()
            .code,
            ErrorCode::NativeProtocol
        );
        assert_eq!(
            check_status(1, ffi::AirError::default()).unwrap_err().code,
            ErrorCode::NativeProtocol
        );
    }
}

#[cfg(test)]
mod behavior_identity_tests {
    use super::*;
    #[test]
    fn product_entry_identity_rejects_previous_and_forged_shims() {
        assert!(verify_build_identity(EXPECTED_BUILD_INFO).is_ok());
        for invalid in [
            EXPECTED_BUILD_INFO.replace("\"shim_version\":5", "\"shim_version\":4"),
            EXPECTED_BUILD_INFO.replace("\"shim_version\":5", "\"shim_version\":3"),
            EXPECTED_BUILD_INFO.replace(
                "\"shim_version\":5",
                "\"shim_version\":3,\"shim_version\":5",
            ),
            EXPECTED_BUILD_INFO.replace("cpu", "gpu"),
            EXPECTED_BUILD_INFO.replace("2149c00", "0000000"),
        ] {
            assert_eq!(
                verify_build_identity(&invalid).unwrap_err().code,
                ErrorCode::NativeProtocol
            );
        }
    }
}
