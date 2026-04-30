use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

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

    fn is_animated(self) -> bool {
        matches!(self, Self::Recording | Self::Processing)
    }

    /// Recording is the only state that ships in colour (solid coral) — the
    /// rest are template images that AppKit tints to match the menu bar.
    fn is_template(self) -> bool {
        !matches!(self, Self::Recording)
    }
}

static STATE: OnceCell<Arc<TrayStateInner>> = OnceCell::new();

struct TrayStateInner {
    state: AtomicU8,
    // Lock-free guard so we never spawn two animation loops; the loop
    // itself clears it on exit.
    anim_running: AtomicBool,
    tooltip: Mutex<String>,
}

impl Default for TrayStateInner {
    fn default() -> Self {
        Self {
            state: AtomicU8::new(TrayState::Idle.as_u8()),
            anim_running: AtomicBool::new(false),
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

fn image_for_state(state: TrayState, phase: f32) -> Image<'static> {
    // 32 px keeps the icon sharp on Retina menu bars (rendered at 2x);
    // macOS down-samples to 16 px on non-Retina automatically.
    const W: u32 = 32;
    let buf = match state {
        TrayState::Idle => tray_glyphs::idle(W as i32),
        TrayState::Recording => tray_glyphs::recording(W as i32, phase),
        TrayState::Processing => tray_glyphs::processing(W as i32, phase),
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
    apply_frame(app, state, 0.0);
    let _ = update_tooltip(app, &inner.tooltip.lock());

    // The animation loop is shared between Recording and Processing. It
    // self-terminates and clears `anim_running` once the state moves to
    // anything else.
    if state.is_animated() && !inner.anim_running.swap(true, Ordering::SeqCst) {
        spawn_anim_loop(app.clone());
    }
}

fn apply_frame<R: Runtime>(app: &AppHandle<R>, state: TrayState, phase: f32) {
    let Some(tray) = app.tray_by_id("vox-tray") else {
        return;
    };
    let img = image_for_state(state, phase);
    let _ = tray.set_icon(Some(img));
    let _ = tray.set_icon_as_template(state.is_template());
}

fn update_tooltip<R: Runtime>(app: &AppHandle<R>, tip: &str) -> tauri::Result<()> {
    let Some(tray) = app.tray_by_id("vox-tray") else {
        return Ok(());
    };
    tray.set_tooltip(Some(tip))?;
    Ok(())
}

/// Single timer that drives both the Recording breathing and the
/// Processing rotation. Picks per-state cycle length and tick rate.
///
/// - **Recording**: 1000 ms cycle (1 Hz, the "comfortable breath" rate
///   from the research) sampled at ~30 FPS so the alpha curve stays
///   smooth.
/// - **Processing**: 1500 ms cycle (~0.66 Hz, one revolution every
///   1.5 s) sampled at ~12 FPS — fast enough to read as motion but not
///   so fast it competes with the cursor blink.
fn spawn_anim_loop<R: Runtime>(app: AppHandle<R>) {
    let inner = Arc::clone(inner());
    let start = Instant::now();
    tauri::async_runtime::spawn(async move {
        loop {
            let cur = TrayState::from_u8(inner.state.load(Ordering::SeqCst));
            let (period_ms, frame_ms): (u64, u64) = match cur {
                TrayState::Recording => (1000, 33),
                TrayState::Processing => (1500, 83),
                _ => {
                    // Settle on a non-animating frame so the icon doesn't
                    // freeze mid-cycle.
                    apply_frame(&app, cur, 0.0);
                    inner.anim_running.store(false, Ordering::SeqCst);
                    return;
                }
            };
            let elapsed = start.elapsed().as_millis() as u64;
            let phase = (elapsed % period_ms) as f32 / period_ms as f32;
            apply_frame(&app, cur, phase);
            tokio::time::sleep(Duration::from_millis(frame_ms)).await;
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
