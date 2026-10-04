//! Private, hand-reviewed declarations of native/mnn-shim/include/nexa_mnn.h.
use std::ffi::c_void;

pub type Model = c_void;
pub type Prepared = c_void;
pub type Cancel = c_void;
pub type Progress = Option<unsafe extern "C" fn(*mut c_void, u32, u64)>;
pub type Text = Option<unsafe extern "C" fn(*mut c_void, Bytes) -> i32>;
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Bytes {
    pub data: *const u8,
    pub len: u64,
}
impl Bytes {
    pub fn new(s: &str) -> Self {
        Self {
            data: s.as_ptr(),
            len: s.len() as u64,
        }
    }
}
#[repr(C)]
pub struct Error {
    pub struct_size: u32,
    pub abi_version: u32,
    pub code: i32,
    pub message_len: u32,
    pub message: [u8; 512],
}
#[repr(C)]
pub struct Build {
    pub struct_size: u32,
    pub abi_version: u32,
    pub upstream_commit: [u8; 40],
    pub patch_sha256: [u8; 64],
    pub policy_sha256: [u8; 64],
    pub silent_logs: u32,
    pub reserved: u32,
}
#[repr(C)]
pub struct Load {
    pub struct_size: u32,
    pub abi_version: u32,
    pub runtime_config_path: Bytes,
    pub artifact_sha256: Bytes,
    pub policy_sha256: Bytes,
    pub expected_upstream_commit: Bytes,
    pub expected_patch_sha256: Bytes,
    pub logical_context: u32,
    pub threads: u32,
    pub prefill_chunk: u32,
    pub reserved: u32,
    pub progress: Progress,
    pub progress_user: *mut c_void,
}
#[repr(C)]
pub struct Message {
    pub struct_size: u32,
    pub abi_version: u32,
    pub role: i32,
    pub reserved: u32,
    pub content: Bytes,
}
#[repr(C)]
pub struct Request {
    pub struct_size: u32,
    pub abi_version: u32,
    pub messages: *const Message,
    pub message_count: u64,
    pub stops: *const Bytes,
    pub stop_count: u64,
    pub max_tokens: u32,
    pub temperature: f32,
    pub top_p: f32,
    pub seed: u32,
    pub progress: Progress,
    pub progress_user: *mut c_void,
    pub reserved: u64,
}
#[repr(C)]
pub struct PreparedInfo {
    pub struct_size: u32,
    pub abi_version: u32,
    pub prompt_tokens: u64,
    pub resolved_seed: u32,
    pub reserved: u32,
}
#[repr(C)]
pub struct Result {
    pub struct_size: u32,
    pub abi_version: u32,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub stop_reason: i32,
    pub resolved_seed: u32,
}
macro_rules! output {
 ($($ty:ty),*) => { $(impl Default for $ty { fn default() -> Self {
 // SAFETY: these C records contain only integers/arrays; all-zero is valid.
 let mut value: Self = unsafe { std::mem::zeroed() };
 value.struct_size = std::mem::size_of::<Self>() as u32; value.abi_version = 1; value
 } })* }
}
output!(Error, Build, PreparedInfo, Result);
unsafe extern "C" {
    pub fn nexa_mnn_v1_build_info(out: *mut Build) -> i32;
    pub fn nexa_mnn_v1_cancel_create(out: *mut *mut Cancel) -> i32;
    pub fn nexa_mnn_v1_cancel_request(flag: *mut Cancel);
    pub fn nexa_mnn_v1_cancel_destroy(flag: *mut Cancel);
    pub fn nexa_mnn_v1_load(
        options: *const Load,
        cancel: *mut Cancel,
        out: *mut *mut Model,
        error: *mut Error,
    ) -> i32;
    pub fn nexa_mnn_v1_prepare(
        model: *mut Model,
        request: *const Request,
        cancel: *mut Cancel,
        out: *mut *mut Prepared,
        info: *mut PreparedInfo,
        error: *mut Error,
    ) -> i32;
    pub fn nexa_mnn_v1_generate(
        prepared: *mut Prepared,
        cancel: *mut Cancel,
        text: Text,
        user: *mut c_void,
        progress: Progress,
        progress_user: *mut c_void,
        result: *mut Result,
        error: *mut Error,
    ) -> i32;
    pub fn nexa_mnn_v1_prepared_destroy(prepared: *mut Prepared, error: *mut Error) -> i32;
    pub fn nexa_mnn_v1_model_destroy(model: *mut Model, error: *mut Error) -> i32;
}
