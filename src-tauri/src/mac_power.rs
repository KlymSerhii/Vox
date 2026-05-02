//! Power-source probe.
//!
//! Returns whether the Mac is currently running on its internal battery
//! (laptop unplugged) vs an external power supply (laptop on AC adapter,
//! UPS, or any desktop with no battery at all). Used to gate optional
//! CPU-heavy work like the speculative streaming Whisper decoder so we
//! don't drain a user's battery on a 1-second utterance.
//!
//! macOS implementation: raw `extern "C"` against
//! `IOPSCopyPowerSourcesInfo` and `IOPSGetProvidingPowerSourceType`,
//! identical pattern to [`mac_chord`]'s tap setup so we don't pull a
//! different `objc2-core-foundation` semver into our link surface for
//! this one call. The call is sub-millisecond on a warm system.
//!
//! Returns `false` (i.e. "not on battery, run the heavy path") on any
//! error, on UPS power, on AC, and on desktops — fail-open. The cost
//! of a false negative is one extra speculative decode we could have
//! skipped; the cost of a false positive is no streaming UI for an
//! entire dictation. False negatives are cheaper.
//!
//! Non-macOS targets get a stub that returns `false` so callers in
//! shared code (e.g. `session.rs`) don't need their own `cfg` gates.

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::{c_char, c_void, CStr};

    type CFTypeRef = *const c_void;
    type CFStringRef = *const c_void;
    type CFStringEncoding = u32;

    /// kCFStringEncodingUTF8 = 0x08000100. Stable since macOS 10.0.
    const K_CF_STRING_ENCODING_UTF8: CFStringEncoding = 0x0800_0100;

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        /// Returns a CFTypeRef "power sources blob" with +1 retain
        /// (Copy rule). Caller owns and must `CFRelease`. Can return
        /// NULL on failure.
        fn IOPSCopyPowerSourcesInfo() -> CFTypeRef;
        /// Returns one of the kIOPSPowerSourceType constants
        /// (`"AC Power"` / `"Battery Power"` / `"UPS Power"`) as a
        /// CFStringRef *borrowed* from the blob — do not release.
        /// Returns NULL if the blob is malformed.
        fn IOPSGetProvidingPowerSourceType(blob: CFTypeRef) -> CFStringRef;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(cf: CFTypeRef);
        /// Copies the CFString into a UTF-8 buffer. Returns false if
        /// the buffer is too small (we use a 32-byte stack buffer —
        /// every documented value is well under that).
        fn CFStringGetCString(
            s: CFStringRef,
            buffer: *mut c_char,
            buffer_size: isize,
            encoding: CFStringEncoding,
        ) -> bool;
    }

    pub fn on_battery() -> bool {
        // SAFETY: We follow the documented Copy / Get reference rules:
        //   - Copy returns +1 retain → we CFRelease at end.
        //   - Get returns a borrow that lives as long as the blob → we
        //     finish reading from it before releasing the blob.
        //   - We bail on every NULL.
        unsafe {
            let blob = IOPSCopyPowerSourcesInfo();
            if blob.is_null() {
                return false;
            }

            let ty = IOPSGetProvidingPowerSourceType(blob);
            let result = if ty.is_null() {
                false
            } else {
                // The constant value is the literal `"Battery Power"`
                // (i.e. `kIOPSBatteryPowerValue`). Pull bytes into a
                // tiny stack buffer — every documented value here is
                // short ASCII per Apple's public IOPSKeys.h.
                let mut buf = [0 as c_char; 32];
                let ok = CFStringGetCString(
                    ty,
                    buf.as_mut_ptr(),
                    buf.len() as isize,
                    K_CF_STRING_ENCODING_UTF8,
                );
                if ok {
                    match CStr::from_ptr(buf.as_ptr()).to_str() {
                        Ok(s) => s == "Battery Power",
                        Err(_) => false,
                    }
                } else {
                    false
                }
            };

            CFRelease(blob);
            result
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    pub fn on_battery() -> bool {
        // Non-macOS builds never run the speculative decoder gating
        // path, but the symbol must still exist so callers in shared
        // modules compile without their own `cfg` guards.
        false
    }
}

/// `true` iff the system is currently drawing from its internal
/// battery. Returns `false` for AC, UPS, desktops without a battery,
/// any error, and any non-macOS target.
///
/// Sub-millisecond. Safe to call on any thread.
pub fn on_battery() -> bool {
    imp::on_battery()
}
