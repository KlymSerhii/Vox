//! Synthetic ⌘V poster used by the opt-in "Paste into focused field"
//! pipeline finishing step (`Settings::auto_paste_enabled`).
//!
//! Builds and posts two CGEvents (key-down + key-up) for the V key with
//! the Command flag set, through the HID event tap. The frontmost app
//! receives them as if the user pressed ⌘V manually.
//!
//! ## Permissions
//!
//! `CGEventPost(kCGHIDEventTap, …)` requires the calling process to be
//! Accessibility-trusted (`AXIsProcessTrusted()`). Without the grant the
//! posted events are silently dropped by the OS — there's no error
//! return — so we probe up front and surface a typed error instead.
//!
//! The check is also why Vox v1's default pipeline doesn't need
//! Accessibility: that path ends at clipboard write and trusts the user
//! to press ⌘V themselves. This module exists to power the opt-in
//! `auto_paste_enabled` setting and is unreferenced when that toggle is
//! off.

use std::os::raw::c_void;

// CoreGraphics handle aliases — all opaque pointers from the framework's
// public C ABI.
type CGEventRef = *const c_void;
type CGEventSourceRef = *const c_void;
type CFTypeRef = *const c_void;

/// `kCGEventSourceStateHIDSystemState` — share the system event tap's
/// modifier state so a still-held modifier (e.g. the user releasing the
/// PTT hotkey late) doesn't leak into our posted ⌘V.
const KCG_EVENT_SOURCE_STATE_HID_SYSTEM_STATE: i32 = 1;

/// `kCGHIDEventTap` — bottom of the event tap stack. Events posted
/// here flow up through every other tap and finally to the focused app.
const KCG_HID_EVENT_TAP: u32 = 0;

/// Virtual keycode for the `V` key on a US ANSI keyboard. The keycode
/// is layout-independent for ⌘V — macOS routes Cmd-shortcuts by the
/// physical key, not the produced character, so V here works on Dvorak,
/// Colemak, AZERTY, etc.
const KVK_ANSI_V: u16 = 9;

/// `kCGEventFlagMaskCommand` — set on both events so the focused app
/// dispatches them as the ⌘V combo rather than a bare `V` keystroke.
const KCG_EVENT_FLAG_MASK_COMMAND: u64 = 1 << 20;

#[link(name = "CoreGraphics", kind = "framework")]
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CGEventSourceCreate(state_id: i32) -> CGEventSourceRef;
    fn CGEventCreateKeyboardEvent(
        source: CGEventSourceRef,
        virtual_key: u16,
        key_down: bool,
    ) -> CGEventRef;
    fn CGEventSetFlags(event: CGEventRef, flags: u64);
    fn CGEventPost(tap: u32, event: CGEventRef);
    fn CFRelease(cf: CFTypeRef);
    fn AXIsProcessTrusted() -> bool;
}

#[derive(Debug, thiserror::Error)]
pub enum PasteError {
    #[error("accessibility not granted — open System Settings → Privacy & Security → Accessibility and enable Vox")]
    AccessibilityDenied,
    #[error("CGEventSourceCreate returned null")]
    EventSourceFailed,
    #[error("CGEventCreateKeyboardEvent returned null")]
    EventCreateFailed,
}

/// Non-prompting Accessibility status probe. Mirrors the same FFI used
/// by `commands::check_permissions` so the toggle's gate matches what
/// the Settings UI sees.
fn is_accessibility_trusted() -> bool {
    // SAFETY: AXIsProcessTrusted is non-prompting and side-effect-free.
    unsafe { AXIsProcessTrusted() }
}

/// Post a synthetic ⌘V keystroke. The frontmost application receives
/// it as if the user pressed ⌘V manually.
///
/// Returns immediately after the events are posted — actually applying
/// the paste happens in the target app's run loop. Callers that follow
/// the paste with a clipboard restore should wait long enough for the
/// app to consume the events first.
pub fn post_cmd_v() -> Result<(), PasteError> {
    if !is_accessibility_trusted() {
        return Err(PasteError::AccessibilityDenied);
    }

    // SAFETY: all returned handles are released on every exit path.
    // CGEventPost is a one-shot copy — releasing the event after the
    // post is the documented contract.
    unsafe {
        let source = CGEventSourceCreate(KCG_EVENT_SOURCE_STATE_HID_SYSTEM_STATE);
        if source.is_null() {
            return Err(PasteError::EventSourceFailed);
        }

        let down = CGEventCreateKeyboardEvent(source, KVK_ANSI_V, true);
        if down.is_null() {
            CFRelease(source);
            return Err(PasteError::EventCreateFailed);
        }
        let up = CGEventCreateKeyboardEvent(source, KVK_ANSI_V, false);
        if up.is_null() {
            CFRelease(down);
            CFRelease(source);
            return Err(PasteError::EventCreateFailed);
        }

        CGEventSetFlags(down, KCG_EVENT_FLAG_MASK_COMMAND);
        CGEventSetFlags(up, KCG_EVENT_FLAG_MASK_COMMAND);

        CGEventPost(KCG_HID_EVENT_TAP, down);
        CGEventPost(KCG_HID_EVENT_TAP, up);

        CFRelease(down);
        CFRelease(up);
        CFRelease(source);
    }

    Ok(())
}
