//! macOS microphone permission via AVFoundation.
//!
//! `AVCaptureDevice.authorizationStatusForMediaType:` is the canonical
//! non-prompting status check — it returns the current TCC state without
//! triggering the consent dialog. `AVCaptureDevice.requestAccessForMediaType:`
//! is the explicit request API that *does* show the prompt.
//!
//! Why we need this instead of cpal: opening (or in some cpal/CoreAudio
//! versions even just querying `default_input_config()` on) a default input
//! device implicitly triggers the TCC microphone prompt on macOS 14+. We
//! were doing exactly that at startup from the tray icon's permission
//! probe, which fired the dialog before the user reached the onboarding
//! mic step. The non-prompting status check below is the fix.
//!
//! This module is `#[cfg(target_os = "macos")]`-gated at its `mod`
//! declaration in `lib.rs`, so the module body unconditionally targets
//! macOS APIs.

use std::sync::mpsc;
use std::sync::Mutex;
use std::time::Duration;

use block2::RcBlock;
use objc2::msg_send;
use objc2::runtime::{AnyClass, AnyObject, Bool};

/// `AVAuthorizationStatus` values from `AVCaptureDevice.h`.
/// <https://developer.apple.com/documentation/avfoundation/avauthorizationstatus>
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicAuthStatus {
    NotDetermined = 0,
    Restricted = 1,
    Denied = 2,
    Authorized = 3,
}

impl MicAuthStatus {
    fn from_raw(v: i64) -> Self {
        match v {
            1 => Self::Restricted,
            2 => Self::Denied,
            3 => Self::Authorized,
            _ => Self::NotDetermined,
        }
    }

    pub fn is_granted(self) -> bool {
        matches!(self, Self::Authorized)
    }
}

/// Resolve `AVMediaTypeAudio` (an exported `NSString *` constant from
/// `AVFoundation.framework`). We can't pull it in via a typed binding
/// without adding a heavy AVFoundation crate dependency, so we link the
/// symbol manually.
fn av_media_type_audio() -> *const AnyObject {
    // `AVFoundation` exports `AVMediaTypeAudio` as an `NSString *`. The
    // `#[link]` attribute forces rustc to add `-framework AVFoundation`
    // to the link line so the symbol resolves; without it the dev
    // toolchain hasn't been pulling AVFoundation in.
    #[link(name = "AVFoundation", kind = "framework")]
    extern "C" {
        static AVMediaTypeAudio: *const AnyObject;
    }
    // SAFETY: the symbol is provided by `AVFoundation.framework`, which
    // is part of every macOS we support (12+).
    unsafe { AVMediaTypeAudio }
}

fn av_capture_device_class() -> &'static AnyClass {
    AnyClass::get(c"AVCaptureDevice").expect("AVCaptureDevice class missing")
}

/// Non-prompting status check. Safe to call repeatedly — does not trigger
/// the TCC dialog regardless of the current state.
pub fn authorization_status() -> MicAuthStatus {
    let cls = av_capture_device_class();
    let media: *const AnyObject = av_media_type_audio();
    // SAFETY: `AVCaptureDevice +authorizationStatusForMediaType:` is a
    // class method that returns `AVAuthorizationStatus` (NSInteger).
    let raw: i64 =
        unsafe { msg_send![cls, authorizationStatusForMediaType: media] };
    MicAuthStatus::from_raw(raw)
}

/// Explicitly trigger the TCC microphone request prompt. Blocks the
/// calling thread until the user makes a decision, the system responds
/// from a cached prior decision, or a 60 s safety timeout elapses.
///
/// On a state of `Authorized` / `Denied` / `Restricted` the system invokes
/// the completion handler synchronously without showing UI, so this
/// returns near-instantly. Only `NotDetermined` shows the dialog.
///
/// Returns `true` if the user (or prior cached decision) granted access.
pub fn request_access_blocking() -> bool {
    log::info!(
        "mac_perm: about to call AVCaptureDevice requestAccessForMediaType: \
         (this WILL prompt if status is NotDetermined)"
    );

    // Channel + a holder so the closure can `take()` the sender out for a
    // single send without requiring `FnOnce` (the block is `Fn`).
    let (tx, rx) = mpsc::channel::<bool>();
    let tx_holder: Mutex<Option<mpsc::Sender<bool>>> = Mutex::new(Some(tx));

    let handler = RcBlock::new(move |granted: Bool| {
        if let Ok(mut slot) = tx_holder.lock() {
            if let Some(tx) = slot.take() {
                let _ = tx.send(granted.as_bool());
            }
        }
    });

    let cls = av_capture_device_class();
    let media: *const AnyObject = av_media_type_audio();
    // SAFETY: `+requestAccessForMediaType:completionHandler:` accepts an
    // `AVMediaType` (NSString *) and a `void (^)(BOOL)` block.
    unsafe {
        let _: () = msg_send![
            cls,
            requestAccessForMediaType: media,
            completionHandler: &*handler,
        ];
    }

    match rx.recv_timeout(Duration::from_secs(60)) {
        Ok(b) => {
            log::info!("mac_perm: requestAccessForMediaType returned granted={b}");
            b
        }
        Err(e) => {
            log::warn!(
                "mac_perm: completion handler did not fire within timeout: {e}"
            );
            false
        }
    }
}
