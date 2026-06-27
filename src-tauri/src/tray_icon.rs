use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use tauri::{image::Image, AppHandle, Runtime};

use crate::tray_glyphs;

/// Colour baked into the animated Processing frames. Always white so
/// the icon visually matches every other template-tinted icon in a
/// dark menu bar (the common case). `defaults read -g
/// AppleInterfaceStyle` only reports the system's *Appearance*
/// preference, which doesn't actually predict what the menu bar looks
/// like — a Light-mode user with a dark wallpaper sees a dark menu
/// bar with white-tinted template icons. Picking white unconditionally
/// is the closest match to that majority appearance and was the
/// explicit user preference after the first run.
const PROCESSING_INK: [u8; 3] = [255, 255, 255];

/// Accent colour for the update-pending dot. Matches the teal
/// `--color-accent` token in `src/styles.css` (#3fb8b0) so the
/// in-menu-bar cue is visually consistent with the in-window banner.
/// Baked colour, not template, because a template dot would tint to
/// match the mic outline and read as part of the glyph rather than
/// an attention cue.
const UPDATE_DOT_INK: [u8; 3] = [0x3f, 0xb8, 0xb0];

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

    /// Recording (coral) and Processing (white/grey baked from
    /// `menu_bar_ink`) ship as non-template RGBA so the rotating ring
    /// is immune to the tray-icon 0.21 `setTemplate(NO)` flicker (see
    /// `tray_glyphs::put_colour`). Idle / Error / Permission stay as
    /// alpha-only templates: they never animate, so the one-time flicker
    /// at a state transition is invisible in practice.
    fn is_template(self) -> bool {
        !matches!(self, Self::Recording | Self::Processing)
    }
}

static STATE: OnceCell<Arc<TrayStateInner>> = OnceCell::new();

struct TrayStateInner {
    state: AtomicU8,
    // Lock-free guard so we never spawn two animation loops; the loop
    // itself clears it on exit.
    anim_running: AtomicBool,
    // Update-pending flag. Drives the Idle glyph variant — when true,
    // the icon renders with a small accent-coloured dot in the
    // upper-right corner. Deliberately suppressed during the animated
    // Recording / Processing states so the dot doesn't compete with
    // the active-dictation glyph.
    update_pending: AtomicBool,
    tooltip: Mutex<String>,
}

impl Default for TrayStateInner {
    fn default() -> Self {
        Self {
            state: AtomicU8::new(TrayState::Idle.as_u8()),
            anim_running: AtomicBool::new(false),
            update_pending: AtomicBool::new(false),
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
        TrayState::Permission => "Vox — needs Microphone access (click to grant)",
    }
}

fn image_for_state(state: TrayState, phase: f32, update_pending: bool) -> Image<'static> {
    // 32 px keeps the icon sharp on Retina menu bars (rendered at 2x);
    // macOS down-samples to 16 px on non-Retina automatically.
    const W: u32 = 32;
    let buf = match state {
        // Update dot only renders on Idle so it doesn't compete with
        // the active-dictation glyphs (Recording / Processing) or the
        // attention-demanding ones (Error / Permission). Idle is the
        // only "you're not doing anything else" surface, which is
        // also where a "by the way, there's an update" cue belongs.
        TrayState::Idle if update_pending => {
            tray_glyphs::idle_with_update(W as i32, UPDATE_DOT_INK)
        }
        TrayState::Idle => tray_glyphs::idle(W as i32),
        TrayState::Recording => tray_glyphs::recording(W as i32, phase),
        TrayState::Processing => tray_glyphs::processing(W as i32, phase, PROCESSING_INK),
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
    let update_pending = inner().update_pending.load(Ordering::SeqCst);
    let img = image_for_state(state, phase, update_pending);
    let _ = tray.set_icon(Some(img));
    // Re-assert the template flag on every install for alpha-only
    // states. tray-icon 0.21's `set_icon` hardcodes `setTemplate(NO)`
    // (https://github.com/tauri-apps/tray-icon/pull/204), which would
    // render an alpha-only RGB=0 glyph as a black silhouette on
    // transparent — exactly what was happening to Idle/Error/Permission
    // when the animation loop's settle path called `apply_frame` after
    // a state transition. Animated states (Recording, Processing) bake
    // their colour into RGBA up front so the hardcoded `false` is what
    // they want; they skip the flip to keep the per-frame loop free of
    // the two-call redraw flicker.
    //
    // The update-pending Idle variant carries baked RGBA pixels for
    // the dot but stays template overall — AppKit happily tints the
    // alpha-only mic body around them. We keep the template flip for
    // the Idle case because skipping it would turn the body to a
    // black silhouette on every state transition while preserving the
    // dot colour.
    if state.is_template() {
        let _ = tray.set_icon_as_template(true);
    }
}

/// Toggle the "update available" badge on the Idle glyph. Idempotent:
/// re-asserts the flag and only triggers a redraw when the value
/// actually changes. Cross-window — called from the updater state
/// emitter so the tray icon and the in-window banner stay in sync.
pub fn set_update_pending<R: Runtime>(app: &AppHandle<R>, pending: bool) {
    let inner = inner();
    let prev = inner.update_pending.swap(pending, Ordering::SeqCst);
    if prev == pending {
        return;
    }
    // Only redraw immediately when the current state is Idle —
    // Recording / Processing have their own animation loop and would
    // pick up the new flag on the next frame anyway, while the
    // permission / error states deliberately don't show the dot. The
    // dot shows up the next time Vox returns to Idle.
    let cur = TrayState::from_u8(inner.state.load(Ordering::SeqCst));
    if matches!(cur, TrayState::Idle) {
        apply_frame(app, cur, 0.0);
    }
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
///   1.5 s) sampled at ~30 FPS. The previous 12 FPS sampling stepped
///   the gap ~20° per frame, which combined with the formerly-hard
///   gap edges read as a flickering ring rather than smooth rotation.
fn spawn_anim_loop<R: Runtime>(app: AppHandle<R>) {
    let inner = Arc::clone(inner());
    let start = Instant::now();
    tauri::async_runtime::spawn(async move {
        loop {
            let cur = TrayState::from_u8(inner.state.load(Ordering::SeqCst));
            let (period_ms, frame_ms): (u64, u64) = match cur {
                TrayState::Recording => (1000, 33),
                TrayState::Processing => (1500, 33),
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
    // Only the mic is required for the default `Cmd+Alt+Period` (⌘⌥.)
    // pipeline — Carbon `RegisterEventHotKey` doesn't need a TCC grant.
    // Accessibility is optional (chord PTT path), so a missing AX grant
    // must not surface the lock-badge tray state on launch: every user
    // who skips chord setup would otherwise see a "needs permission"
    // indicator forever.
    //
    // Must use the non-prompting AVFoundation probe here: cpal's
    // default_input_config() opens the device far enough to fire the TCC
    // microphone prompt on macOS 14+ before the user reaches onboarding.
    !crate::mac_perm::authorization_status().is_granted()
}

#[cfg(not(target_os = "macos"))]
fn missing_permissions() -> bool {
    false
}
