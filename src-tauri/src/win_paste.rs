//! Synthetic Ctrl+V poster used by the opt-in "Paste into focused field"
//! pipeline finishing step (`Settings::auto_paste_enabled`).
//!
//! Sends a four-event sequence (Ctrl down, V down, V up, Ctrl up) through
//! `SendInput`. The frontmost window receives it as if the user pressed
//! Ctrl+V manually. This is the Windows analogue of `mac_paste::post_cmd_v`.
//!
//! ## Permissions
//!
//! Unlike the macOS path, `SendInput` needs no Accessibility-style grant —
//! a normal-integrity process can inject input into another normal-integrity
//! foreground window. There is therefore no permission probe here. The one
//! failure mode worth surfacing is a short write: `SendInput` returns the
//! number of events it actually injected, and the OS silently drops the
//! sequence when the foreground window is higher-integrity (e.g. an elevated
//! console) than an unelevated Vox. The pipeline fail-opens to clipboard-only
//! in that case.

use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    VIRTUAL_KEY, VK_CONTROL, VK_V,
};

#[derive(Debug, thiserror::Error)]
pub enum PasteError {
    #[error("SendInput injected {sent} of {expected} events — the focused window may be elevated")]
    ShortWrite { sent: u32, expected: u32 },
}

fn key_event(vk: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// Post a synthetic Ctrl+V keystroke. The frontmost window receives it as
/// if the user pressed Ctrl+V manually.
///
/// Like the macOS poster, this returns as soon as the events are queued —
/// the paste itself is applied in the target window's message loop, so a
/// follow-up clipboard restore must wait long enough for that to happen.
pub fn post_ctrl_v() -> Result<(), PasteError> {
    // Virtual-key Ctrl+V (not scan codes): Windows routes Ctrl-shortcuts by
    // virtual key, so this is layout-correct on every keyboard the same way
    // the macOS path routes ⌘V by physical key.
    let inputs = [
        key_event(VK_CONTROL, KEYBD_EVENT_FLAGS(0)),
        key_event(VK_V, KEYBD_EVENT_FLAGS(0)),
        key_event(VK_V, KEYEVENTF_KEYUP),
        key_event(VK_CONTROL, KEYEVENTF_KEYUP),
    ];

    // SAFETY: `inputs` is a valid, fully-initialised array of INPUT and
    // `size_of::<INPUT>()` is the documented cbSize argument. SendInput
    // copies the events synchronously and does not retain the pointer.
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    let expected = inputs.len() as u32;
    if sent != expected {
        return Err(PasteError::ShortWrite { sent, expected });
    }
    Ok(())
}
