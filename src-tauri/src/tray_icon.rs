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

static STATE: OnceCell<Arc<TrayStateInner>> = OnceCell::new();

struct TrayStateInner {
    state: AtomicU8,
    // Lock-free guard so we never spawn two pulse loops; the loop itself clears it on exit.
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

fn image_for_state(state: TrayState, pulse_phase: bool) -> Image<'static> {
    // 32 px keeps the icon sharp on Retina menu bars (rendered at 2x);
    // macOS down-samples to 16 px on non-Retina automatically.
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

pub fn init<R: Runtime>(app: &AppHandle<R>) {
    let initial = if missing_permissions() {
        TrayState::Permission
    } else {
        TrayState::Idle
    };
    set_state(app, initial);
}

pub fn set_state<R: Runtime>(app: &AppHandle<R>, state: TrayState) {
    let inner = inner();
    let prev = TrayState::from_u8(inner.state.swap(state.as_u8(), Ordering::SeqCst));
    if prev == state {
        return;
    }

    *inner.tooltip.lock() = default_tooltip(state).to_string();
    apply_frame(app, state, false);
    let _ = update_tooltip(app, &inner.tooltip.lock());

    // Non-recording states are picked up by the running pulse loop on its next
    // tick, which self-terminates and clears `pulse_running`.
    if state == TrayState::Recording && !inner.pulse_running.swap(true, Ordering::SeqCst) {
        spawn_pulse_loop(app.clone());
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
                // Settle on a non-blink frame so a Recording→Processing
                // transition doesn't leave the icon mid-blink.
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

#[cfg(target_os = "macos")]
fn missing_permissions() -> bool {
    extern "C" {
        fn AXIsProcessTrusted() -> bool;
    }
    if !unsafe { AXIsProcessTrusted() } {
        return true;
    }
    // Must use the non-prompting AVFoundation probe here: cpal's
    // default_input_config() opens the device far enough to fire the TCC
    // microphone prompt on macOS 14+ before the user reaches onboarding.
    !crate::mac_perm::authorization_status().is_granted()
}

#[cfg(not(target_os = "macos"))]
fn missing_permissions() -> bool {
    false
}
