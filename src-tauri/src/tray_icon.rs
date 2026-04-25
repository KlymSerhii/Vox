//! Tray icon state machine.
//!
//! The tray icon reflects the current dictation pipeline state at all times:
//!
//! - `Idle` — mic outline (light grey when tinted)
//! - `Recording` — filled mic + red dot, alternating with dot-off frame at
//!   2 Hz so the user gets unmistakable visual feedback that the recording
//!   is live.
//! - `Processing` — spinner glyph (transcribing or polishing).
//! - `Error` — mic + exclamation badge.
//! - `Permission` — padlock glyph, used when accessibility/microphone TCC
//!   has not been granted.
//!
//! All glyphs are alpha-only template images so macOS tints them to match
//! the menu-bar appearance (white in dark mode, black in light mode).
//!
//! Pulse animation is implemented as a simple `tokio::spawn`-ed loop guarded
//! by an `AtomicBool` so we never have multiple pulse loops running and
//! never leak a loop after the state moves away from `Recording`.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;

use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use tauri::{image::Image, AppHandle, Runtime};

use crate::tray_glyphs;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayState {
    Idle,
    Recording,
    Processing,
    Error,
    Permission,
}

impl TrayState {
    fn as_u8(self) -> u8 {
        match self {
            Self::Idle => 0,
            Self::Recording => 1,
            Self::Processing => 2,
            Self::Error => 3,
            Self::Permission => 4,
        }
    }

    fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Recording,
            2 => Self::Processing,
            3 => Self::Error,
            4 => Self::Permission,
            _ => Self::Idle,
        }
    }
}

/// Process-wide tray state. We use atomics so the pulse loop can read it
/// without locking, and we keep it on the heap behind `OnceCell` so the
/// public setter doesn't need an `&AppHandle` to look up the tray.
static STATE: OnceCell<Arc<TrayStateInner>> = OnceCell::new();

struct TrayStateInner {
    state: AtomicU8,
    /// Set when a pulse loop is currently active. The loop holds it; whoever
    /// transitions out of `Recording` clears it so the loop self-terminates.
    pulse_running: AtomicBool,
    tooltip: Mutex<String>,
}

impl Default for TrayStateInner {
    fn default() -> Self {
        Self {
            state: AtomicU8::new(TrayState::Idle.as_u8()),
            pulse_running: AtomicBool::new(false),
            tooltip: Mutex::new(default_tooltip(TrayState::Idle).into()),
        }
    }
}

fn inner() -> &'static Arc<TrayStateInner> {
    STATE.get_or_init(|| Arc::new(TrayStateInner::default()))
}

fn default_tooltip(state: TrayState) -> &'static str {
    match state {
        TrayState::Idle => "Vox — hold hotkey to dictate",
        TrayState::Recording => "Vox — listening…",
        TrayState::Processing => "Vox — transcribing…",
        TrayState::Error => "Vox — error (click for details)",
        TrayState::Permission => "Vox — needs Accessibility / Mic permission",
    }
}

// ---- Image construction ----------------------------------------------------

/// Build a Tauri `Image` for a given state at @1x. Tauri will ask for the
/// @2x variant separately via the macOS template image path; for our
/// procedurally-generated icons the tint is a flat scale, so the @1x is
/// sufficient — the menu bar will scale it. (We wrote @2x PNGs to disk in
/// build.rs so the assets exist, but the runtime path uses a single buffer
/// for simplicity. Visual quality at 16px is identical.)
fn image_for_state(state: TrayState, pulse_phase: bool) -> Image<'static> {
    // Generate at 32 px so on a Retina menu bar (which renders at 2x) the
    // icon is sharp; macOS will down-sample for non-Retina automatically.
    const W: u32 = 32;
    let buf = match state {
        TrayState::Idle => tray_glyphs::idle(W as i32),
        TrayState::Recording => tray_glyphs::recording(W as i32, !pulse_phase),
        TrayState::Processing => tray_glyphs::processing(W as i32),
        TrayState::Error => tray_glyphs::error(W as i32),
        TrayState::Permission => tray_glyphs::permission(W as i32),
    };
    Image::new_owned(buf, W, W)
}

// ---- Public API ------------------------------------------------------------

/// Apply the initial tray state at startup. Called once after `tray::build`.
pub fn init<R: Runtime>(app: &AppHandle<R>) {
    let initial = if missing_permissions() {
        TrayState::Permission
    } else {
        TrayState::Idle
    };
    set_state(app, initial);
}

/// Update the visible tray icon for the given state. Idempotent — calling
/// `set_state(app, Recording)` while already recording does NOT restart
/// the pulse loop.
pub fn set_state<R: Runtime>(app: &AppHandle<R>, state: TrayState) {
    let inner = inner();
    let prev = TrayState::from_u8(inner.state.swap(state.as_u8(), Ordering::SeqCst));
    if prev == state {
        return;
    }

    // Update tooltip + static frame immediately so the icon doesn't lag.
    *inner.tooltip.lock() = default_tooltip(state).to_string();
    apply_frame(app, state, false);
    let _ = update_tooltip(app, &inner.tooltip.lock());

    if state == TrayState::Recording {
        // Start the pulse loop if one isn't already running. The flag is
        // *only* cleared by the loop itself when it exits, so simultaneous
        // re-entries can't double-spawn.
        if !inner.pulse_running.swap(true, Ordering::SeqCst) {
            spawn_pulse_loop(app.clone());
        }
    } else {
        // Any non-recording state: the running pulse loop will see
        // `state != Recording` on its next tick and self-terminate. We
        // don't toggle `pulse_running` directly — the loop owns it.
    }
}

fn apply_frame<R: Runtime>(app: &AppHandle<R>, state: TrayState, pulse_phase: bool) {
    let Some(tray) = app.tray_by_id("vox-tray") else {
        return;
    };
    let img = image_for_state(state, pulse_phase);
    let _ = tray.set_icon(Some(img));
    let _ = tray.set_icon_as_template(true);
}

fn update_tooltip<R: Runtime>(app: &AppHandle<R>, tip: &str) -> tauri::Result<()> {
    let Some(tray) = app.tray_by_id("vox-tray") else {
        return Ok(());
    };
    tray.set_tooltip(Some(tip))?;
    Ok(())
}

fn spawn_pulse_loop<R: Runtime>(app: AppHandle<R>) {
    let inner = Arc::clone(inner());
    tauri::async_runtime::spawn(async move {
        let mut phase = false;
        loop {
            let cur = TrayState::from_u8(inner.state.load(Ordering::SeqCst));
            if cur != TrayState::Recording {
                // Settle on the static (with-dot) frame before exiting so we
                // don't leave the icon mid-blink if the user lingers in the
                // Recording → Processing transition.
                apply_frame(&app, cur, false);
                inner.pulse_running.store(false, Ordering::SeqCst);
                return;
            }
            apply_frame(&app, TrayState::Recording, phase);
            phase = !phase;
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });
}

// ---- Permission probe ------------------------------------------------------

#[cfg(target_os = "macos")]
fn missing_permissions() -> bool {
    extern "C" {
        fn AXIsProcessTrusted() -> bool;
    }
    let accessibility = unsafe { AXIsProcessTrusted() };
    if !accessibility {
        return true;
    }
    // Microphone — non-prompting status check via AVFoundation. Critically,
    // we MUST NOT call `cpal::default_input_config()` here at startup: on
    // macOS 14+ that opens the audio device far enough to trigger the TCC
    // microphone prompt before the user has reached the onboarding mic
    // step. `mac_perm::authorization_status()` reads the cached TCC state
    // without escalating it.
    !crate::mac_perm::authorization_status().is_granted()
}

#[cfg(not(target_os = "macos"))]
fn missing_permissions() -> bool {
    false
}
