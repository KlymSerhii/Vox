//! Windows native audio-control leaf module — the Windows half of the
//! OS-agnostic `audio_attenuation` facade (the macOS half lives in
//! `mac_audio.rs`). It exposes whole-device output volume get/set, a
//! best-effort "is sound coming out right now" probe, and a synthetic
//! media Play/Pause keypress. Used to duck / pause other apps' audio while
//! Vox is listening.
//!
//! ## Volume + activity: WASAPI via the `windows` crate
//!
//! Volume is read/written on the **default render endpoint** through
//! `IAudioEndpointVolume::{Get,Set}MasterVolumeLevelScalar` — the whole
//! device, not a per-session volume, to stay symmetric with the macOS
//! default-output-device behaviour. Activity is the default endpoint's
//! `IAudioMeterInformation::GetPeakValue` (chosen over session enumeration:
//! one Activate call, no `IAudioSessionEnumerator` walk, and it answers the
//! exact question — "is the speaker moving").
//!
//! Every WASAPI entry point initialises COM first (`ensure_com_initialized`,
//! idempotent and tolerant of an already-initialised thread) and then runs
//! the work in a `windows::core::Result`-returning closure so any `HRESULT`
//! failure collapses to `None`/`false` via `?`. The `windows` crate's COM
//! interfaces are refcounted and `Drop`-release their underlying `IUnknown`,
//! so there is no manual `Release` and we deliberately never call
//! `CoUninitialize` (the thread may be reused for more audio calls, and
//! over-uninitialising COM is a footgun).
//!
//! ## Media key: SendInput
//!
//! `send_play_pause` mirrors `win_paste::post_ctrl_v` exactly — a `KEYBDINPUT`
//! down/up pair for `VK_MEDIA_PLAY_PAUSE` posted with `SendInput`. Like the
//! paste poster, this needs no Accessibility-style grant, so
//! `supports_play_pause()` is unconditionally `true`.

use std::sync::atomic::{AtomicBool, Ordering};

use windows::core::{Result as WinResult, GUID};
use windows::Win32::Foundation::{RPC_E_CHANGED_MODE, S_FALSE, S_OK};
use windows::Win32::Media::Audio::Endpoints::{IAudioEndpointVolume, IAudioMeterInformation};
use windows::Win32::Media::Audio::{
    eConsole, eRender, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    VIRTUAL_KEY, VK_MEDIA_PLAY_PAUSE,
};

/// A peak value at or below this is treated as silence. `GetPeakValue`
/// returns a 0.0..=1.0 amplitude; real playback (even quiet) sits well
/// above this floor, while a truly idle endpoint reads exactly 0.0.
const SILENCE_EPSILON: f32 = 0.0001;

/// COM is initialised at most once per process from this module. WASAPI is
/// happy on an STA thread (`COINIT_APARTMENTTHREADED`); if some other part of
/// the app already put this thread into a different apartment, `CoInitializeEx`
/// returns `RPC_E_CHANGED_MODE` and we proceed on the existing apartment
/// rather than treating it as fatal — the endpoint-volume / meter interfaces
/// work from either apartment.
fn ensure_com_initialized() {
    static COM_READY: AtomicBool = AtomicBool::new(false);
    if COM_READY.load(Ordering::Acquire) {
        return;
    }

    // SAFETY: `CoInitializeEx` takes a null reserved pointer (`None`) and an
    // apartment flag. It is safe to call repeatedly; we interpret its HRESULT
    // rather than trusting a single call. We never pair this with
    // `CoUninitialize` (see module docs).
    let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };

    // S_OK: we initialised it. S_FALSE: already initialised on this thread
    // (still success). RPC_E_CHANGED_MODE: a different apartment is already
    // in force — fine for our use. Any other failure: leave the flag unset so
    // a later call can retry, but don't block this call's attempt either.
    if hr == S_OK || hr == S_FALSE || hr == RPC_E_CHANGED_MODE {
        COM_READY.store(true, Ordering::Release);
    }
}

/// Resolve the default audio **render** (output) endpoint for the console
/// role. Returns `Err(HRESULT)` if no device is present or COM activation
/// fails, which the callers map to `None`/`false`.
fn default_render_endpoint() -> WinResult<IMMDevice> {
    // SAFETY: standard WASAPI device-enumeration sequence. `MMDeviceEnumerator`
    // is the documented CLSID; `&MMDeviceEnumerator` coerces to the expected
    // `*const GUID`. `None` is the (no aggregation) outer-unknown. Each call's
    // `?` propagates a failing HRESULT, and the returned interfaces are
    // refcounted / `Drop`-released by the `windows` crate.
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        enumerator.GetDefaultAudioEndpoint(eRender, eConsole)
    }
}

/// Current default-output-device master volume, 0.0..=1.0. None if unavailable.
pub fn output_volume() -> Option<f32> {
    ensure_com_initialized();

    fn inner() -> WinResult<f32> {
        let device = default_render_endpoint()?;
        // SAFETY: `Activate::<IAudioEndpointVolume>` requests the endpoint
        // volume interface off the device; `None` = no activation params.
        let volume: IAudioEndpointVolume =
            unsafe { device.Activate(CLSCTX_ALL, None)? };
        // SAFETY: returns the master scalar in 0.0..=1.0.
        unsafe { volume.GetMasterVolumeLevelScalar() }
    }

    inner().ok().map(|v| v.clamp(0.0, 1.0))
}

/// Set default-output-device master volume (clamp 0.0..=1.0). Returns true if applied.
pub fn set_output_volume(v: f32) -> bool {
    ensure_com_initialized();
    let level = v.clamp(0.0, 1.0);

    fn inner(level: f32) -> WinResult<()> {
        let device = default_render_endpoint()?;
        // SAFETY: same activation as the getter.
        let volume: IAudioEndpointVolume =
            unsafe { device.Activate(CLSCTX_ALL, None)? };
        // SAFETY: a null event-context GUID means "no originating event
        // context", the documented value when we aren't a volume callback.
        unsafe { volume.SetMasterVolumeLevelScalar(level, std::ptr::null::<GUID>()) }
    }

    inner(level).is_ok()
}

/// Is audio currently playing on the default output device?
///
/// Best-effort: reads the default render endpoint's peak meter and reports
/// whether it is above a tiny silence floor. On any failure (no device, COM
/// error) it returns `false` so the caller fails *toward not pausing* other
/// apps rather than pausing on a false positive.
pub fn output_is_active() -> bool {
    ensure_com_initialized();

    fn inner() -> WinResult<f32> {
        let device = default_render_endpoint()?;
        // SAFETY: `IAudioMeterInformation` is the peak-meter interface for the
        // endpoint; `None` = no activation params.
        let meter: IAudioMeterInformation =
            unsafe { device.Activate(CLSCTX_ALL, None)? };
        // SAFETY: returns the most recent peak sample value, 0.0..=1.0.
        unsafe { meter.GetPeakValue() }
    }

    inner().map(|peak| peak > SILENCE_EPSILON).unwrap_or(false)
}

/// Can we post a media Play/Pause key right now? On Windows: always true
/// (SendInput needs no grant — see `win_paste`).
pub fn supports_play_pause() -> bool {
    true
}

/// Build a single keyboard `INPUT` event. Mirrors `win_paste::key_event`.
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

/// Post a media Play/Pause toggle (`VK_MEDIA_PLAY_PAUSE`) via SendInput.
/// Returns true if sent.
///
/// Identical shape to `win_paste::post_ctrl_v`: a down/up `KEYBDINPUT` pair
/// routed by virtual key. The OS delivers it to whichever app currently owns
/// media-transport control (the SystemMediaTransportControls session), so it
/// is not tied to the foreground window.
pub fn send_play_pause() -> bool {
    let inputs = [
        key_event(VK_MEDIA_PLAY_PAUSE, KEYBD_EVENT_FLAGS(0)),
        key_event(VK_MEDIA_PLAY_PAUSE, KEYEVENTF_KEYUP),
    ];

    // SAFETY: `inputs` is a fully-initialised array of `INPUT`;
    // `size_of::<INPUT>()` is the documented `cbSize`. `SendInput` copies the
    // events synchronously and does not retain the pointer. Returns the count
    // actually injected — anything short of the full pair is a failure.
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    sent == inputs.len() as u32
}
