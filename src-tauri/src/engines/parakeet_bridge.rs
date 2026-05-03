//! Raw FFI surface for the Swift `ParakeetBridge` static library.
//!
//! Mirrors the structure of `apple_fm_bridge`: the unsafe boundary
//! lives here, callers in `parakeet_local` only see safe wrappers and
//! never need to remember to call `vox_parakeet_free_string`.
//!
//! Build availability is inverted between cfg branches: when the
//! Swift bridge is compiled in we expose `load` / `transcribe_blocking`
//! plus an `is_available() -> true`; otherwise we expose the same name
//! `is_available() -> false` so the engine impl can probe statically
//! without an `#[cfg]` ladder of its own.

#[cfg(all(target_os = "macos", target_arch = "aarch64", feature = "parakeet"))]
mod imp {
    use std::ffi::{CStr, CString};
    use std::os::raw::{c_char, c_int};
    use std::path::Path;
    use std::ptr;

    extern "C" {
        fn vox_parakeet_load(models_dir: *const c_char) -> c_int;
        fn vox_parakeet_transcribe_blocking(
            samples_ptr: *const f32,
            samples_len: usize,
            timeout_secs: c_int,
            out_text: *mut *mut c_char,
        ) -> c_int;
        fn vox_parakeet_take_last_error() -> *mut c_char;
        fn vox_parakeet_free_string(ptr: *mut c_char);
        fn vox_parakeet_evict();
    }

    pub fn is_available() -> bool {
        true
    }

    pub fn load(models_dir: &Path) -> Result<(), String> {
        let path_str = models_dir
            .to_str()
            .ok_or_else(|| "model dir path is not UTF-8".to_string())?;
        let c = CString::new(path_str).map_err(|e| format!("interior NUL in path: {e}"))?;

        let rc = unsafe { vox_parakeet_load(c.as_ptr()) };
        if rc == 0 {
            Ok(())
        } else {
            let msg = take_last_error()
                .unwrap_or_else(|| format!("parakeet load: rc={rc}"));
            Err(msg)
        }
    }

    pub fn transcribe_blocking(pcm: &[f32], timeout_secs: u32) -> Result<String, String> {
        if pcm.is_empty() {
            return Err("empty PCM buffer".into());
        }
        let mut out: *mut c_char = ptr::null_mut();
        let rc = unsafe {
            vox_parakeet_transcribe_blocking(
                pcm.as_ptr(),
                pcm.len(),
                timeout_secs as c_int,
                &mut out,
            )
        };

        if rc == 0 {
            if out.is_null() {
                return Err("parakeet returned null text on success".into());
            }
            // Round-trip the result + free in the same expression so a
            // panic in `to_str` can't leak the Swift-allocated buffer.
            let text = unsafe {
                let s = CStr::from_ptr(out).to_string_lossy().into_owned();
                vox_parakeet_free_string(out);
                s
            };
            Ok(text)
        } else {
            // Even if Swift wrote `out` before signalling error, free
            // it so we don't leak.
            if !out.is_null() {
                unsafe { vox_parakeet_free_string(out) };
            }
            let msg = take_last_error()
                .unwrap_or_else(|| format!("parakeet transcribe: rc={rc}"));
            Err(msg)
        }
    }

    /// Drop the Swift-side `ModelsCache` actor's cached `AsrManager`
    /// + loaded-dir pointer. Called from `Registry::drop_for_model`
    /// after a model delete so a re-download lands on a fresh load
    /// instead of the stale in-memory copy.
    pub fn evict() {
        unsafe { vox_parakeet_evict() };
    }

    fn take_last_error() -> Option<String> {
        let raw = unsafe { vox_parakeet_take_last_error() };
        if raw.is_null() {
            return None;
        }
        let msg = unsafe {
            let s = CStr::from_ptr(raw).to_string_lossy().into_owned();
            vox_parakeet_free_string(raw);
            s
        };
        if msg.is_empty() {
            None
        } else {
            Some(msg)
        }
    }
}

#[cfg(not(all(target_os = "macos", target_arch = "aarch64", feature = "parakeet")))]
mod imp {
    use std::path::Path;

    pub fn is_available() -> bool {
        false
    }

    pub fn load(_models_dir: &Path) -> Result<(), String> {
        Err("parakeet bridge not built into this binary".into())
    }

    pub fn transcribe_blocking(_pcm: &[f32], _timeout_secs: u32) -> Result<String, String> {
        Err("parakeet bridge not built into this binary".into())
    }

    pub fn evict() {}
}

pub use imp::{evict, is_available, load, transcribe_blocking};
