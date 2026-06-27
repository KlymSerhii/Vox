//! Raw FFI surface for the Swift `AppleFmBridge` static library.
//!
//! Three knobs the rest of the codebase actually uses live in
//! `apple_fm.rs`; this file is just the unsafe boundary. Every function
//! here is a thin wrapper that owns the pointer-lifetime story so
//! callers don't have to remember to `vox_apple_fm_free` on every path.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_uint, c_void};
use std::sync::Arc;

extern "C" {
    fn vox_apple_fm_availability() -> c_int;

    fn vox_apple_fm_polish_blocking(
        system_prompt: *const c_char,
        transcript: *const c_char,
        max_tokens: c_int,
        timeout_secs: c_uint,
    ) -> *mut c_char;

    fn vox_apple_fm_polish_stream(
        system_prompt: *const c_char,
        transcript: *const c_char,
        max_tokens: c_int,
        timeout_secs: c_uint,
        user_data: *mut c_void,
        on_chunk: extern "C" fn(*mut c_void, *const c_char),
        on_done: extern "C" fn(*mut c_void, *const c_char),
    ) -> c_int;

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

struct StreamCtx {
    cb: Box<dyn Fn(&str) + Send + Sync + 'static>,
}

// Ownership convention for the StreamCtx pointer crossing into Swift:
// the value is heap-allocated as `Arc<StreamCtx>` and passed in via
// `Arc::into_raw`. Each Rust trampoline invoked from Swift takes a
// non-owning view by calling `Arc::increment_strong_count(ptr)` and
// then `Arc::from_raw(ptr)` to materialise a strong handle without
// consuming the original count. The single explicit "release" path —
// `stream_done_trampoline` (always called by Swift, including the
// timeout drain) — does NOT increment first; it consumes one count via
// `Arc::from_raw` and lets the Drop run. This means even if Apple's
// `streamResponse` fires a chunk callback after the Rust caller has
// returned, the strong count keeps the `StreamCtx` alive until that
// last chunk callback unwinds — no use-after-free on the timeout path.

extern "C" fn stream_chunk_trampoline(user_data: *mut c_void, text: *const c_char) {
    if user_data.is_null() || text.is_null() {
        return;
    }
    // SAFETY: `user_data` was created via `Arc::into_raw` and the
    // explicit "release" callback (`stream_done_trampoline`) has not
    // yet consumed the original count when this fires. We bump the
    // strong count and materialise a temporary `Arc` so this borrow
    // is independent of the release path's eventual consume.
    let ctx: Arc<StreamCtx> = unsafe {
        Arc::increment_strong_count(user_data as *const StreamCtx);
        Arc::from_raw(user_data as *const StreamCtx)
    };
    let s = unsafe { CStr::from_ptr(text).to_string_lossy().into_owned() };
    // Panicking across the FFI boundary is undefined behaviour in
    // Swift; trap any unwind from user code here so we never propagate.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (ctx.cb)(&s)));
}

extern "C" fn stream_done_trampoline(user_data: *mut c_void, _text: *const c_char) {
    if user_data.is_null() {
        return;
    }
    // The "release" path. Consumes the original `Arc::into_raw` count
    // (no increment first). Any concurrent chunk-trampoline still
    // holding its bumped strong count keeps the StreamCtx alive until
    // its own scope ends.
    //
    // SAFETY: Swift guarantees this is invoked exactly once per call
    // to `vox_apple_fm_polish_stream` — including on the timeout
    // drain path.
    let ctx: Arc<StreamCtx> = unsafe { Arc::from_raw(user_data as *const StreamCtx) };
    drop(ctx);
}

/// Streaming polish — fires `on_chunk` with cumulative partial output as
/// the Foundation Models session generates. Blocks the calling thread
/// (Swift waits on a `DispatchSemaphore`); call from a `spawn_blocking`
/// worker. Returns Err on timeout / framework error / guardrail rejection.
pub fn polish_stream_blocking(
    system_prompt: &str,
    transcript: &str,
    max_tokens: i32,
    timeout_secs: u32,
    on_chunk: Box<dyn Fn(&str) + Send + Sync + 'static>,
) -> Result<(), String> {
    let prompt_c = CString::new(system_prompt)
        .map_err(|e| format!("system_prompt has interior NUL: {e}"))?;
    let transcript_c = CString::new(transcript)
        .map_err(|e| format!("transcript has interior NUL: {e}"))?;

    // Allocate the context as an Arc so that any chunk callback firing
    // concurrently with the release path (e.g. on the Swift timeout
    // drain) keeps its own strong reference for its lifetime. The
    // "original" count is consumed exactly once by the
    // `stream_done_trampoline` released callback below.
    let raw_ctx = Arc::into_raw(Arc::new(StreamCtx { cb: on_chunk })) as *mut StreamCtx;

    let rc = unsafe {
        vox_apple_fm_polish_stream(
            prompt_c.as_ptr(),
            transcript_c.as_ptr(),
            max_tokens as c_int,
            timeout_secs as c_uint,
            raw_ctx as *mut c_void,
            stream_chunk_trampoline,
            stream_done_trampoline,
        )
    };

    // Ownership: the Swift bridge guarantees `stream_done_trampoline`
    // fires exactly once per call (including the timeout-drain path);
    // that callback is responsible for consuming the original
    // `Arc::into_raw` count. Rust does NOT reclaim here. Any
    // chunk-trampoline still racing to fire will have already
    // incremented the strong count and is safe in its own right.

    if rc == 0 {
        Ok(())
    } else {
        let msg = take_last_error()
            .unwrap_or_else(|| format!("Apple Intelligence stream: rc={rc}"));
        Err(msg)
    }
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
