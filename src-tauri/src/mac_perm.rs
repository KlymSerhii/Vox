//! macOS microphone permission via AVFoundation.
//!
//! Querying cpal's `default_input_config()` on macOS 14+ implicitly fires
//! the TCC mic prompt, so we use AVFoundation's
//! `authorizationStatusForMediaType:` (non-prompting) for status and
//! `requestAccessForMediaType:` (prompting) only when the onboarding wizard
//! explicitly asks for it.

use std::sync::mpsc;
use std::sync::Mutex;
use std::time::Duration;

use block2::RcBlock;
use objc2::msg_send;
use objc2::runtime::{AnyClass, AnyObject, Bool};

/// `AVAuthorizationStatus` values from `AVCaptureDevice.h`.
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

fn av_media_type_audio() -> *const AnyObject {
    // The `#[link]` attribute forces rustc to add `-framework AVFoundation`
    // to the link line so this `NSString *` constant resolves.
    #[link(name = "AVFoundation", kind = "framework")]
    extern "C" {
        static AVMediaTypeAudio: *const AnyObject;
    }
    // SAFETY: AVFoundation ships on every supported macOS (12+).
    unsafe { AVMediaTypeAudio }
}

fn av_capture_device_class() -> Option<&'static AnyClass> {
    AnyClass::get(c"AVCaptureDevice")
}

/// Non-prompting status check. Safe to call repeatedly.
///
/// Returns `Restricted` (the most conservative non-granted state) if the
/// `AVCaptureDevice` class can't be looked up — the probe is hot-path
/// (tray icon, onboarding poller) and must never panic.
pub fn authorization_status() -> MicAuthStatus {
    let Some(cls) = av_capture_device_class() else {
        log::warn!("mac_perm: AVCaptureDevice class missing — falling back to Restricted");
        return MicAuthStatus::Restricted;
    };
    // SAFETY: class method returning `AVAuthorizationStatus` (NSInteger).
    let raw: i64 = unsafe {
        msg_send![cls, authorizationStatusForMediaType: av_media_type_audio()]
    };
    MicAuthStatus::from_raw(raw)
}

/// Trigger the TCC microphone prompt. Returns `true` on grant.
///
/// Blocks until the system responds. Only `NotDetermined` shows UI; the
/// other states return near-instantly from the cached decision.
pub fn request_access_blocking() -> bool {
    log::info!("mac_perm: requestAccessForMediaType (will prompt if NotDetermined)");

    // Holder lets the `Fn` block `take()` the sender for its single send.
    let (tx, rx) = mpsc::channel::<bool>();
    let tx_holder: Mutex<Option<mpsc::Sender<bool>>> = Mutex::new(Some(tx));

    let handler = RcBlock::new(move |granted: Bool| {
        if let Ok(mut slot) = tx_holder.lock() {
            if let Some(tx) = slot.take() {
                let _ = tx.send(granted.as_bool());
            }
        }
    });

    let Some(cls) = av_capture_device_class() else {
        log::warn!(
            "mac_perm: AVCaptureDevice class missing during request_access_blocking — \
             treating as denied"
        );
        return false;
    };

    // SAFETY: `+requestAccessForMediaType:completionHandler:` takes an
    // `AVMediaType` (NSString *) and a `void (^)(BOOL)` block.
    unsafe {
        let _: () = msg_send![
            cls,
            requestAccessForMediaType: av_media_type_audio(),
            completionHandler: &*handler,
        ];
    }

    match rx.recv_timeout(Duration::from_secs(60)) {
        Ok(b) => {
            log::info!("mac_perm: granted={b}");
            b
        }
        Err(e) => {
            log::warn!("mac_perm: completion handler did not fire within timeout: {e}");
            false
        }
    }
}
