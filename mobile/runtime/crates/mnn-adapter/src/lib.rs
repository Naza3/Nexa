//! Safe, owner-thread bindings to the real Nexa MNN CPU ABI v1.
//! This crate does not grant model admission or verify a model package's files.
//! Callers must keep validated assets and the controlled runtime config immutable
//! throughout the model lifetime. Native artifacts are verified at build time.
//! `catch_unwind` prevents FFI unwinding but still invokes the embedding app's
//! panic hook. Callbacks should return controlled failures instead of panicking;
//! this crate never installs or replaces a process-global panic/logging hook.
//!
//! Models cannot move across threads:
//! ```compile_fail
//! fn requires_send<T: Send>() {}
//! requires_send::<mnn_adapter::Model>();
//! ```
//! Models cannot be shared across threads:
//! ```compile_fail
//! fn requires_sync<T: Sync>() {}
//! requires_sync::<mnn_adapter::Model>();
//! ```
//! A prepared request exclusively borrows its model:
//! ```compile_fail
//! use mnn_adapter::*;
//! fn overlap(model: &mut Model, request: &Request<'_>, cancel: &Cancellation) {
//!     let prepared = model.prepare(request, cancel, |_| {}).unwrap();
//!     model.close().unwrap();
//!     drop(prepared);
//! }
//! ```
//! Generation consumes the prepared request:
//! ```compile_fail
//! use mnn_adapter::*;
//! fn twice(prepared: Prepared<'_>, cancel: &Cancellation) {
//!     let _ = prepared.generate(cancel, |_| TextAction::Continue, |_| {});
//!     let _ = prepared.generate(cancel, |_| TextAction::Continue, |_| {});
//! }
//! ```
#![deny(unsafe_op_in_unsafe_fn)]
mod ffi;
use std::{
    ffi::c_void,
    marker::PhantomData,
    panic::{AssertUnwindSafe, catch_unwind},
    ptr::NonNull,
    rc::Rc,
    sync::Arc,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Invalid,
    Abi,
    WrongThread,
    Busy,
    Cancelled,
    Native,
    Budget,
    Consumed,
    Callback,
    Identity,
    CallbackPanic,
    Protocol,
    Closed,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub kind: ErrorKind,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MNN adapter {:?}", self.kind)
    }
}
impl std::error::Error for Error {}
fn error(kind: ErrorKind) -> Error {
    Error { kind }
}
fn status(code: i32) -> Result<(), Error> {
    if code == 0 {
        return Ok(());
    }
    Err(error(match code {
        1 => ErrorKind::Invalid,
        2 => ErrorKind::Abi,
        3 => ErrorKind::WrongThread,
        4 => ErrorKind::Busy,
        5 => ErrorKind::Cancelled,
        6 => ErrorKind::Native,
        7 => ErrorKind::Budget,
        8 => ErrorKind::Consumed,
        9 => ErrorKind::Callback,
        10 => ErrorKind::Identity,
        _ => ErrorKind::Protocol,
    }))
}

thread_local! { static IN_NATIVE_CALL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
struct NativeCall;
impl NativeCall {
    fn enter() -> Result<Self, Error> {
        IN_NATIVE_CALL.with(|active| {
            if active.replace(true) {
                Err(error(ErrorKind::Busy))
            } else {
                Ok(Self)
            }
        })
    }
}
impl Drop for NativeCall {
    fn drop(&mut self) {
        IN_NATIVE_CALL.with(|active| active.set(false));
    }
}

struct CancelInner(NonNull<ffi::Cancel>);
// SAFETY: ABI v1 cancel is only an independent atomic<bool>. Request may run on
// any thread; Arc keeps its allocation alive until every operation and controller
// releases its reference. Destruction cannot overlap a request or operation.
unsafe impl Send for CancelInner {}
unsafe impl Sync for CancelInner {}
impl Drop for CancelInner {
    fn drop(&mut self) {
        unsafe { ffi::nexa_mnn_v1_cancel_destroy(self.0.as_ptr()) }
    }
}
/// One-shot cancellation. Clone a controller for another thread; never reset it.
#[derive(Clone)]
pub struct Cancellation(Arc<CancelInner>);
impl Cancellation {
    pub fn new() -> Result<Self, Error> {
        let mut raw = std::ptr::null_mut();
        status(unsafe { ffi::nexa_mnn_v1_cancel_create(&mut raw) })?;
        Ok(Self(Arc::new(CancelInner(
            NonNull::new(raw).ok_or_else(|| error(ErrorKind::Protocol))?,
        ))))
    }
    pub fn cancel(&self) {
        unsafe { ffi::nexa_mnn_v1_cancel_request(self.0.0.as_ptr()) }
    }
    fn raw(&self) -> *mut ffi::Cancel {
        self.0.0.as_ptr()
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildIdentity {
    pub upstream_commit: String,
    pub patch_sha256: String,
    pub policy_sha256: String,
    /// SHA256 of exact verified artifact.json bytes, binding every archive hash,
    /// header, compiler, target and build field. This is a native build identity,
    /// not model-asset validation or a production admission certificate.
    pub artifact_manifest_sha256: String,
    pub target: String,
    pub compiler: String,
}
pub fn build_identity() -> Result<BuildIdentity, Error> {
    let mut out = ffi::Build::default();
    status(unsafe { ffi::nexa_mnn_v1_build_info(&mut out) })?;
    if out.abi_version != 1
        || out.struct_size as usize != std::mem::size_of_val(&out)
        || out.reserved != 0
        || out.silent_logs != 1
        || out.upstream_commit != env!("NEXA_MNN_COMMIT").as_bytes()
        || out.patch_sha256 != env!("NEXA_MNN_PATCH").as_bytes()
        || out.policy_sha256 != env!("NEXA_MNN_POLICY").as_bytes()
    {
        return Err(error(ErrorKind::Identity));
    }
    Ok(BuildIdentity {
        upstream_commit: env!("NEXA_MNN_COMMIT").into(),
        patch_sha256: env!("NEXA_MNN_PATCH").into(),
        policy_sha256: env!("NEXA_MNN_POLICY").into(),
        artifact_manifest_sha256: env!("NEXA_MNN_ARTIFACT_MANIFEST_SHA256").into(),
        target: env!("NEXA_MNN_TARGET").into(),
        compiler: env!("NEXA_MNN_COMPILER").into(),
    })
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Load,
    Template,
    Tokenize,
    Prefill,
    Decode,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub phase: Phase,
    pub count: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAction {
    Continue,
    Cancel,
    Fail,
}
struct Callbacks<'a> {
    text: &'a mut dyn FnMut(&str) -> TextAction,
    progress: &'a mut dyn FnMut(Progress),
    cancel: Cancellation,
    failure: Option<ErrorKind>,
}
impl Callbacks<'_> {
    fn fail(&mut self, kind: ErrorKind) {
        if self.failure.is_none() {
            self.failure = Some(kind);
        }
        self.cancel.cancel();
    }
}
unsafe extern "C" fn progress_callback(user: *mut c_void, phase: u32, count: u64) {
    // SAFETY: every synchronous native call receives its own live exclusive state;
    // native ABI forbids retained userdata, concurrent callbacks and reentry.
    let state = unsafe { &mut *user.cast::<Callbacks<'_>>() };
    if state.failure.is_some() {
        return;
    }
    let phase = match phase {
        1 => Phase::Load,
        2 => Phase::Template,
        3 => Phase::Tokenize,
        4 => Phase::Prefill,
        5 => Phase::Decode,
        _ => {
            state.fail(ErrorKind::Protocol);
            return;
        }
    };
    if let Err(payload) = catch_unwind(AssertUnwindSafe(|| {
        (state.progress)(Progress { phase, count })
    })) {
        // A panic payload can have a panicking Drop; never unwind it through C.
        std::mem::forget(payload);
        state.fail(ErrorKind::CallbackPanic);
    }
}
unsafe extern "C" fn text_callback(user: *mut c_void, bytes: ffi::Bytes) -> i32 {
    let state = unsafe { &mut *user.cast::<Callbacks<'_>>() };
    if state.failure.is_some() {
        return 2;
    }
    if bytes.data.is_null() || bytes.len == 0 || bytes.len > 4096 {
        state.fail(ErrorKind::Protocol);
        return 2;
    }
    // SAFETY: native owns a live readable byte slice for this callback only;
    // checked length is <=4096 and cannot exceed Rust's slice size bound.
    let text = match std::str::from_utf8(unsafe {
        std::slice::from_raw_parts(bytes.data, bytes.len as usize)
    }) {
        Ok(s) => s,
        Err(_) => {
            state.fail(ErrorKind::Protocol);
            return 2;
        }
    };
    match catch_unwind(AssertUnwindSafe(|| (state.text)(text))) {
        Ok(TextAction::Continue) => 0,
        Ok(TextAction::Cancel) => {
            state.cancel.cancel();
            1
        }
        Ok(TextAction::Fail) => {
            state.fail(ErrorKind::Callback);
            2
        }
        Err(payload) => {
            std::mem::forget(payload);
            state.fail(ErrorKind::CallbackPanic);
            2
        }
    }
}

#[derive(Debug, Clone)]
pub struct LoadOptions<'a> {
    pub runtime_config_path: &'a str,
    pub artifact_sha256: &'a str,
    pub logical_context: u32,
    pub threads: u32,
    pub prefill_chunk: u32,
}
impl LoadOptions<'_> {
    pub fn validate(&self) -> Result<(), Error> {
        if self.runtime_config_path.is_empty()
            || self.runtime_config_path.len() > 4096
            || self.runtime_config_path.contains('\0')
            || !std::path::Path::new(self.runtime_config_path).is_absolute()
            || self.artifact_sha256.len() != 64
            || !self
                .artifact_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || !(1..=2048).contains(&self.logical_context)
            || !(1..=2).contains(&self.threads)
            || !(1..=128).contains(&self.prefill_chunk)
        {
            return Err(error(ErrorKind::Invalid));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy)]
pub enum Role {
    System,
    User,
    Assistant,
}
#[derive(Debug, Clone, Copy)]
pub struct Message<'a> {
    pub role: Role,
    pub content: &'a str,
}
/// Fixed seeds include zero. UINT32_MAX is reserved for fresh native entropy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seed {
    Fixed(u32),
    Random,
}
#[derive(Debug, Clone)]
pub struct Request<'a> {
    pub messages: &'a [Message<'a>],
    pub stops: &'a [&'a str],
    pub max_tokens: u32,
    pub temperature: f32,
    pub top_p: f32,
    pub seed: Seed,
}
impl Request<'_> {
    pub fn validate(&self, logical_context: u32) -> Result<(), Error> {
        let total = self
            .messages
            .iter()
            .try_fold(0usize, |n, m| n.checked_add(m.content.len()));
        if self.messages.is_empty()
            || self.messages.len() > 4096
            || total.is_none_or(|n| n > 1048576)
            || self.stops.len() > 4
            || self.stops.iter().any(|s| s.is_empty() || s.len() > 128)
            || self.max_tokens == 0
            || self.max_tokens > logical_context
            || !self.temperature.is_finite()
            || !(0.0..=2.0).contains(&self.temperature)
            || !self.top_p.is_finite()
            || self.top_p <= 0.0
            || self.top_p > 1.0
            || self.seed == Seed::Fixed(u32::MAX)
        {
            return Err(error(ErrorKind::Invalid));
        }
        Ok(())
    }
}
/// Thread-confined native model. Explicit close reports destruction errors.
/// Drop makes one owner-thread cleanup attempt; an unexpected native rejection
/// leaks the still-owned handle rather than risking unsafe destruction. Dropping
/// another model/prepared from a native callback also fails closed this way;
/// retain handles until the callback returns and explicitly close them instead.
pub struct Model {
    raw: Option<NonNull<ffi::Model>>,
    context: u32,
    _thread: PhantomData<Rc<()>>,
}
impl Model {
    pub fn load(
        options: &LoadOptions<'_>,
        cancel: &Cancellation,
        mut progress: impl FnMut(Progress),
    ) -> Result<Self, Error> {
        options.validate()?;
        let call = NativeCall::enter()?;
        build_identity()?;
        let mut noop = |_: &str| TextAction::Continue;
        let mut callbacks = Callbacks {
            text: &mut noop,
            progress: &mut progress,
            cancel: cancel.clone(),
            failure: None,
        };
        let raw_options = ffi::Load {
            struct_size: std::mem::size_of::<ffi::Load>() as u32,
            abi_version: 1,
            runtime_config_path: ffi::Bytes::new(options.runtime_config_path),
            artifact_sha256: ffi::Bytes::new(options.artifact_sha256),
            policy_sha256: ffi::Bytes::new(env!("NEXA_MNN_POLICY")),
            expected_upstream_commit: ffi::Bytes::new(env!("NEXA_MNN_COMMIT")),
            expected_patch_sha256: ffi::Bytes::new(env!("NEXA_MNN_PATCH")),
            logical_context: options.logical_context,
            threads: options.threads,
            prefill_chunk: options.prefill_chunk,
            reserved: 0,
            progress: Some(progress_callback),
            progress_user: (&mut callbacks as *mut Callbacks<'_>).cast(),
        };
        let mut raw = std::ptr::null_mut();
        let mut native_error = ffi::Error::default();
        let code = unsafe {
            ffi::nexa_mnn_v1_load(
                &raw_options,
                callbacks.cancel.raw(),
                &mut raw,
                &mut native_error,
            )
        };
        drop(call);
        let mut model = Self {
            raw: NonNull::new(raw),
            context: options.logical_context,
            _thread: PhantomData,
        };
        if let Some(kind) = callbacks.failure {
            let _ = model.close();
            return Err(error(kind));
        }
        status(code)?;
        model.pointer()?;
        Ok(model)
    }
    fn pointer(&self) -> Result<*mut ffi::Model, Error> {
        self.raw
            .map(NonNull::as_ptr)
            .ok_or_else(|| error(ErrorKind::Closed))
    }
    pub fn close(&mut self) -> Result<(), Error> {
        if let Some(raw) = self.raw {
            let _call = NativeCall::enter()?;
            let mut e = ffi::Error::default();
            status(unsafe { ffi::nexa_mnn_v1_model_destroy(raw.as_ptr(), &mut e) })?;
            self.raw = None;
        }
        Ok(())
    }
    pub fn prepare<'model>(
        &'model mut self,
        request: &Request<'_>,
        cancel: &Cancellation,
        mut progress: impl FnMut(Progress),
    ) -> Result<Prepared<'model>, Error> {
        request.validate(self.context)?;
        let call = NativeCall::enter()?;
        let model = self.pointer()?;
        let messages: Vec<_> = request
            .messages
            .iter()
            .map(|m| ffi::Message {
                struct_size: std::mem::size_of::<ffi::Message>() as u32,
                abi_version: 1,
                role: match m.role {
                    Role::System => 1,
                    Role::User => 2,
                    Role::Assistant => 3,
                },
                reserved: 0,
                content: ffi::Bytes::new(m.content),
            })
            .collect();
        let stops: Vec<_> = request.stops.iter().map(|s| ffi::Bytes::new(s)).collect();
        let mut noop = |_: &str| TextAction::Continue;
        let mut callbacks = Callbacks {
            text: &mut noop,
            progress: &mut progress,
            cancel: cancel.clone(),
            failure: None,
        };
        let q = ffi::Request {
            struct_size: std::mem::size_of::<ffi::Request>() as u32,
            abi_version: 1,
            messages: messages.as_ptr(),
            message_count: messages.len() as u64,
            stops: stops.as_ptr(),
            stop_count: stops.len() as u64,
            max_tokens: request.max_tokens,
            temperature: request.temperature,
            top_p: request.top_p,
            seed: match request.seed {
                Seed::Fixed(n) => n,
                Seed::Random => u32::MAX,
            },
            progress: Some(progress_callback),
            progress_user: (&mut callbacks as *mut Callbacks<'_>).cast(),
            reserved: 0,
        };
        let mut raw = std::ptr::null_mut();
        let mut info = ffi::PreparedInfo::default();
        let mut e = ffi::Error::default();
        let code = unsafe {
            ffi::nexa_mnn_v1_prepare(
                model,
                &q,
                callbacks.cancel.raw(),
                &mut raw,
                &mut info,
                &mut e,
            )
        };
        drop(call);
        let mut prepared = Prepared {
            raw: NonNull::new(raw),
            model: self,
            max_tokens: request.max_tokens,
            info: PreparedInfo {
                prompt_tokens: info.prompt_tokens,
                resolved_seed: info.resolved_seed,
            },
        };
        if let Some(kind) = callbacks.failure {
            let _ = prepared.close();
            return Err(error(kind));
        }
        status(code)?;
        if prepared.raw.is_none()
            || info.reserved != 0
            || matches!(request.seed, Seed::Fixed(seed) if seed != info.resolved_seed)
            || info.prompt_tokens == 0
            || info
                .prompt_tokens
                .checked_add(u64::from(request.max_tokens))
                .is_none_or(|n| n > u64::from(prepared.model.context))
        {
            return Err(error(ErrorKind::Protocol));
        }
        Ok(prepared)
    }
}
impl Drop for Model {
    fn drop(&mut self) {
        let _ = self.close();
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreparedInfo {
    pub prompt_tokens: u64,
    pub resolved_seed: u32,
}
pub struct Prepared<'model> {
    max_tokens: u32,
    raw: Option<NonNull<ffi::Prepared>>,
    model: &'model mut Model,
    info: PreparedInfo,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishReason {
    Eos,
    Length,
    Stop,
    Cancelled,
    Error,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Generation {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub resolved_seed: u32,
    pub finish_reason: FinishReason,
}
#[derive(Debug)]
pub struct GenerationFailure {
    pub error: Error,
    pub usage: Generation,
    pub cleanup_error: Option<Error>,
}
impl std::fmt::Display for GenerationFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(f)
    }
}
impl std::error::Error for GenerationFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}
impl Prepared<'_> {
    pub fn info(&self) -> PreparedInfo {
        self.info
    }
    pub fn close(&mut self) -> Result<(), Error> {
        if let Some(raw) = self.raw {
            let _call = NativeCall::enter()?;
            let mut e = ffi::Error::default();
            status(unsafe { ffi::nexa_mnn_v1_prepared_destroy(raw.as_ptr(), &mut e) })?;
            self.raw = None;
        }
        Ok(())
    }
    /// Consume exactly once. Text is borrowed for the callback only. A callback
    /// cancel is cancellation, not a successful stop-string match. Failures retain
    /// native token counts, including accepted EOS or undisplayed stop tokens.
    pub fn generate(
        mut self,
        cancel: &Cancellation,
        mut text: impl FnMut(&str) -> TextAction,
        mut progress: impl FnMut(Progress),
    ) -> Result<Generation, GenerationFailure> {
        let mut callbacks = Callbacks {
            text: &mut text,
            progress: &mut progress,
            cancel: cancel.clone(),
            failure: None,
        };
        let user = (&mut callbacks as *mut Callbacks<'_>).cast();
        let mut result = ffi::Result {
            prompt_tokens: self.info.prompt_tokens,
            resolved_seed: self.info.resolved_seed,
            stop_reason: 5,
            ..Default::default()
        };
        let mut e = ffi::Error::default();
        let call = match NativeCall::enter() {
            Ok(call) => call,
            Err(error) => {
                return Err(GenerationFailure {
                    cleanup_error: Some(error.clone()),
                    error,
                    usage: Generation {
                        prompt_tokens: self.info.prompt_tokens,
                        completion_tokens: 0,
                        resolved_seed: self.info.resolved_seed,
                        finish_reason: FinishReason::Error,
                    },
                });
            }
        };
        let code = match self.raw {
            Some(raw) => unsafe {
                ffi::nexa_mnn_v1_generate(
                    raw.as_ptr(),
                    callbacks.cancel.raw(),
                    Some(text_callback),
                    user,
                    Some(progress_callback),
                    user,
                    &mut result,
                    &mut e,
                )
            },
            None => 8,
        };
        drop(call);
        let finish = match result.stop_reason {
            1 => Some(FinishReason::Eos),
            2 => Some(FinishReason::Length),
            3 => Some(FinishReason::Stop),
            4 => Some(FinishReason::Cancelled),
            5 => Some(FinishReason::Error),
            _ => None,
        };
        let mut failure = callbacks.failure.map(error).or_else(|| status(code).err());
        if finish.is_none()
            || (code == 5 && finish != Some(FinishReason::Cancelled))
            || (code != 0
                && matches!(
                    finish,
                    Some(FinishReason::Eos | FinishReason::Length | FinishReason::Stop)
                ))
            || result.prompt_tokens != self.info.prompt_tokens
            || result.resolved_seed != self.info.resolved_seed
            || result.completion_tokens > u64::from(self.max_tokens)
            || (code == 0 && matches!(finish, Some(FinishReason::Cancelled | FinishReason::Error)))
        {
            failure = Some(error(ErrorKind::Protocol));
        }
        let mut usage = Generation {
            prompt_tokens: result.prompt_tokens,
            completion_tokens: result.completion_tokens,
            resolved_seed: result.resolved_seed,
            finish_reason: finish.unwrap_or(FinishReason::Error),
        };
        if callbacks.failure.is_some() {
            usage.finish_reason = FinishReason::Error;
        }
        let cleanup_error = self.close().err();
        if let Some(failure) = failure {
            return Err(GenerationFailure {
                error: failure,
                usage,
                cleanup_error,
            });
        }
        if let Some(error) = cleanup_error {
            return Err(GenerationFailure {
                cleanup_error: Some(error.clone()),
                error,
                usage,
            });
        }
        Ok(usage)
    }
}
impl Drop for Prepared<'_> {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_identity_and_abi_layout() {
        let identity = build_identity().unwrap();
        assert_eq!(identity.artifact_manifest_sha256.len(), 64);
        assert!(
            identity
                .artifact_manifest_sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
        );
        assert_eq!(identity.target, env!("NEXA_MNN_TARGET"));
        assert_eq!(identity.compiler, env!("NEXA_MNN_COMPILER"));
        assert_eq!(std::mem::size_of::<ffi::Bytes>(), 16);
        assert_eq!(std::mem::size_of::<ffi::Error>(), 528);
        assert_eq!(std::mem::size_of::<ffi::Build>(), 184);
        assert_eq!(std::mem::size_of::<ffi::Load>(), 120);
        assert_eq!(std::mem::size_of::<ffi::Message>(), 32);
        assert_eq!(std::mem::size_of::<ffi::Request>(), 80);
        assert_eq!(std::mem::size_of::<ffi::PreparedInfo>(), 24);
        assert_eq!(std::mem::size_of::<ffi::Result>(), 32);
        assert_eq!(std::mem::offset_of!(ffi::Request, progress), 56);
        assert_eq!(std::mem::offset_of!(ffi::Load, progress_user), 112);
    }
    #[test]
    fn request_validation_bounds_and_seeds() {
        let messages = [Message {
            role: Role::User,
            content: "中文\0🙂",
        }];
        let mut r = Request {
            messages: &messages,
            stops: &[],
            max_tokens: 1,
            temperature: 0.,
            top_p: 1.,
            seed: Seed::Fixed(0),
        };
        assert!(r.validate(2048).is_ok());
        r.seed = Seed::Fixed(u32::MAX);
        assert!(r.validate(2048).is_err());
        r.seed = Seed::Random;
        assert!(r.validate(2048).is_ok());
        for value in [f32::NAN, f32::INFINITY, -0.1, 2.1] {
            r.temperature = value;
            assert!(r.validate(2048).is_err());
        }
        r.temperature = 0.;
        for value in [f32::NAN, f32::INFINITY, 0., -0.1, 1.1] {
            r.top_p = value;
            assert!(r.validate(2048).is_err());
        }
        r.top_p = 1.;
        r.stops = &[""];
        assert!(r.validate(2048).is_err());
        r.stops = &[];
        r.max_tokens = 2049;
        assert!(r.validate(2048).is_err());
        r.max_tokens = 1;
        r.messages = &[];
        assert!(r.validate(2048).is_err());
    }
    #[test]
    fn owner_thread_guard_rejects_nested_native_operations() {
        let first = NativeCall::enter().unwrap();
        assert_eq!(NativeCall::enter().err().unwrap().kind, ErrorKind::Busy);
        assert_eq!(NativeCall::enter().err().unwrap().kind, ErrorKind::Busy);
        drop(first);
        assert!(NativeCall::enter().is_ok());
    }
    #[test]
    fn load_validation_bounds() {
        let mut l = LoadOptions {
            runtime_config_path: "/controlled/runtime.json",
            artifact_sha256: &"a".repeat(64),
            logical_context: 2048,
            threads: 2,
            prefill_chunk: 128,
        };
        assert!(l.validate().is_ok());
        l.threads = 3;
        assert!(l.validate().is_err());
        l.threads = 2;
        l.runtime_config_path = "relative.json";
        assert!(l.validate().is_err());
    }
    #[test]
    fn cancellation_is_send_sync_and_lives_through_clones() {
        fn bounds<T: Send + Sync>() {}
        bounds::<Cancellation>();
        let c = Cancellation::new().unwrap();
        let controller = c.clone();
        std::thread::spawn(move || {
            controller.cancel();
            controller.cancel();
        })
        .join()
        .unwrap();
        c.cancel();
    }
    #[test]
    fn callbacks_bound_utf8_and_contain_panics() {
        let cancel = Cancellation::new().unwrap();
        let mut text = |_: &str| -> TextAction { panic!("synthetic callback panic") };
        let mut progress = |_: Progress| {};
        let mut state = Callbacks {
            text: &mut text,
            progress: &mut progress,
            cancel: cancel.clone(),
            failure: None,
        };
        assert_eq!(
            unsafe {
                text_callback(
                    (&mut state as *mut Callbacks<'_>).cast(),
                    ffi::Bytes::new("hello"),
                )
            },
            2
        );
        assert_eq!(state.failure, Some(ErrorKind::CallbackPanic));
        let mut text = |_: &str| TextAction::Continue;
        let mut progress = |_: Progress| panic!("synthetic progress panic");
        let mut state = Callbacks {
            text: &mut text,
            progress: &mut progress,
            cancel: cancel.clone(),
            failure: None,
        };
        unsafe { progress_callback((&mut state as *mut Callbacks<'_>).cast(), 1, 0) };
        assert_eq!(state.failure, Some(ErrorKind::CallbackPanic));
        let mut progress = |_: Progress| {};
        let mut state = Callbacks {
            text: &mut text,
            progress: &mut progress,
            cancel,
            failure: None,
        };
        let invalid = [0xff];
        assert_eq!(
            unsafe {
                text_callback(
                    (&mut state as *mut Callbacks<'_>).cast(),
                    ffi::Bytes {
                        data: invalid.as_ptr(),
                        len: 1,
                    },
                )
            },
            2
        );
        assert_eq!(state.failure, Some(ErrorKind::Protocol));
        state.failure = None;
        assert_eq!(
            unsafe {
                text_callback(
                    (&mut state as *mut Callbacks<'_>).cast(),
                    ffi::Bytes {
                        data: std::ptr::null(),
                        len: 4097,
                    },
                )
            },
            2
        );
        assert_eq!(state.failure, Some(ErrorKind::Protocol));
    }
}
