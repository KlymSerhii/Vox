//! Raw FFI surface for the Swift `AppleFmBridge` static library.
//!
//! Three knobs the rest of the codebase actually uses live in
//! `apple_fm.rs`; this file is just the unsafe boundary. Every function
//! here is a thin wrapper that owns the pointer-lifetime story so
//! callers don't have to remember to `vox_apple_fm_free` on every path.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_uint};

extern "C" {
    fn vox_apple_fm_availability() -> c_int;

    fn vox_apple_fm_polish_blocking(
        system_prompt: *const c_char,
        transcript: *const c_char,
        max_tokens: c_int,
        timeout_secs: c_uint,
    ) -> *mut c_char;

    fn vox_apple_fm_take_last_error() -> *mut c_char;

    fn vox_apple_fm_free(ptr: *mut c_char);
}

/// Mirrors the integer codes the Swift bridge returns. Must stay in
/// lockstep with the `k*` constants in `AppleFmBridge.swift`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvailabilityCode {
    Available,
    DeviceNotEligible,
    IntelligenceNotEnabled,
    ModelNotReady,
    OsTooOld,
    InternalError,
    FrameworkMissing,
}

impl AvailabilityCode {
    fn from_raw(raw: c_int) -> Self {
        match raw {
            0 => Self::Available,
            1 => Self::DeviceNotEligible,
            2 => Self::IntelligenceNotEnabled,
            3 => Self::ModelNotReady,
            4 => Self::OsTooOld,
            5 => Self::InternalError,
            6 => Self::FrameworkMissing,
            _ => Self::InternalError,
        }
    }
}

pub fn availability() -> AvailabilityCode {
    AvailabilityCode::from_raw(unsafe { vox_apple_fm_availability() })
}

/// Synchronously call the Foundation Models polish path. Blocks the
/// current thread until the Swift `Task` resolves or `timeout_secs`
/// fires — call from `tokio::task::spawn_blocking` to keep the async
/// runtime healthy.
pub fn polish_blocking(
    system_prompt: &str,
    transcript: &str,
    max_tokens: i32,
    timeout_secs: u32,
) -> Result<String, String> {
    let prompt_c = CString::new(system_prompt)
        .map_err(|e| format!("system_prompt has interior NUL: {e}"))?;
    let transcript_c = CString::new(transcript)
        .map_err(|e| format!("transcript has interior NUL: {e}"))?;

    let raw = unsafe {
        vox_apple_fm_polish_blocking(
            prompt_c.as_ptr(),
            transcript_c.as_ptr(),
            max_tokens as c_int,
            timeout_secs as c_uint,
        )
    };

    if raw.is_null() {
        let msg = take_last_error()
            .unwrap_or_else(|| "Apple Intelligence: unknown error".to_string());
        return Err(msg);
    }

    // Round-trip the C string into a Rust `String` and release the
    // Swift-allocated buffer in the same call so a panic in `to_str`
    // can't leak it.
    let result = unsafe {
        let s = CStr::from_ptr(raw).to_string_lossy().into_owned();
        vox_apple_fm_free(raw);
        s
    };

    Ok(result)
}

fn take_last_error() -> Option<String> {
    let raw = unsafe { vox_apple_fm_take_last_error() };
    if raw.is_null() {
        return None;
    }
    let msg = unsafe {
        let s = CStr::from_ptr(raw).to_string_lossy().into_owned();
        vox_apple_fm_free(raw);
        s
    };
    if msg.is_empty() {
        None
    } else {
        Some(msg)
    }
}
