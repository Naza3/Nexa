//! Private ABI v2 bindings. Keep layout synchronized with air_llama.h.
use std::ffi::c_void;

#[repr(C)]
pub struct AirEngine {
    _private: [u8; 0],
}
#[repr(C)]
pub struct AirModel {
    _private: [u8; 0],
}
#[repr(C)]
pub struct AirPrepared {
    _private: [u8; 0],
}
#[repr(C)]
pub struct AirCancel {
    _private: [u8; 0],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AirString {
    pub data: *const u8,
    pub len: u64,
}
impl AirString {
    pub fn borrowed(value: &str) -> Self {
        Self {
            data: value.as_ptr(),
            len: value.len() as u64,
        }
    }
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct AirBuffer {
    pub data: *mut u8,
    pub len: u64,
}
impl Default for AirBuffer {
    fn default() -> Self {
        Self {
            data: std::ptr::null_mut(),
            len: 0,
        }
    }
}
#[repr(C)]
#[derive(Default)]
pub struct AirError {
    pub code: i32,
    pub message: AirBuffer,
}
#[repr(C)]
pub struct AirMessage {
    pub role: AirString,
    pub content: AirString,
}
#[repr(C)]
pub struct AirLoadOptions {
    pub context_size: u32,
    pub threads: u32,
    pub batch_size: u32,
}
#[repr(C)]
pub struct AirGenerateOptions {
    pub max_tokens: u32,
    pub temperature: f32,
    pub top_p: f32,
    pub seed: u32,
}
#[repr(C)]
#[derive(Default)]
pub struct AirUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub finish_reason: i32,
}

pub type ProgressCallback = unsafe extern "C" fn(*mut c_void, u32, u32, u32) -> i32;

pub type TextCallback = unsafe extern "C" fn(*mut c_void, AirString) -> i32;

unsafe extern "C" {
    pub fn air_engine_create(out: *mut *mut AirEngine, error: *mut AirError) -> i32;
    pub fn air_engine_destroy(engine: *mut AirEngine);
    pub fn air_model_load(
        engine: *mut AirEngine,
        path: AirString,
        options: AirLoadOptions,
        cancel: *const AirCancel,
        out: *mut *mut AirModel,
        error: *mut AirError,
    ) -> i32;
    pub fn air_model_load_with_projector(
        engine: *mut AirEngine,
        path: AirString,
        projector_path: AirString,
        options: AirLoadOptions,
        cancel: *const AirCancel,
        out: *mut *mut AirModel,
        error: *mut AirError,
    ) -> i32;
    pub fn air_prepare_image(
        model: *mut AirModel,
        prompt: AirString,
        image: *const u8,
        image_len: u64,
        image_after_text: u32,
        options: AirGenerateOptions,
        stops: *const AirString,
        stop_count: u64,
        cancel: *const AirCancel,
        out: *mut *mut AirPrepared,
        prompt_tokens: *mut u32,
        error: *mut AirError,
    ) -> i32;
    pub fn air_model_unload(model: *mut AirModel);
    pub fn air_prepare(
        model: *mut AirModel,
        messages: *const AirMessage,
        count: u64,
        options: AirGenerateOptions,
        stops: *const AirString,
        stop_count: u64,
        cancel: *const AirCancel,
        out: *mut *mut AirPrepared,
        prompt_tokens: *mut u32,
        error: *mut AirError,
    ) -> i32;
    pub fn air_prepared_free(prepared: *mut AirPrepared);
    pub fn air_generate_observed(
        prepared: *mut AirPrepared,
        cancel: *const AirCancel,
        callback: TextCallback,
        user: *mut c_void,
        progress: ProgressCallback,
        progress_user: *mut c_void,
        usage: *mut AirUsage,
        error: *mut AirError,
    ) -> i32;
    pub fn air_cancel_create(out: *mut *mut AirCancel, error: *mut AirError) -> i32;
    pub fn air_cancel_set(cancel: *mut AirCancel);
    pub fn air_cancel_destroy(cancel: *mut AirCancel);
    pub fn air_buffer_free(buffer: AirBuffer);
    pub fn air_get_build_info(out: *mut AirBuffer, error: *mut AirError) -> i32;
    pub fn air_model_template(
        model: *mut AirModel,
        out: *mut AirBuffer,
        error: *mut AirError,
    ) -> i32;
}
