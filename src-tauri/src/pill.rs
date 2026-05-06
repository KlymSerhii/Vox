use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Runtime, WebviewUrl};

/// Bumped on every `show`/`hide`. The 10ms-deferred `pill:show` emit
/// captures this at scheduling time and drops the emit if a hide (or
/// another show) advanced the generation while it was sleeping. Without
/// this guard a sub-10ms hide-after-show paints a pill the user already
/// dismissed.
static PILL_GENERATION: AtomicU64 = AtomicU64::new(0);

pub const LABEL: &str = "pill";
// The window is sized larger than the visible pill (116×26) so glow
// effects on Done/Warning states have room to fall off. The main drop
// shadow is drawn by the OS (`hasShadow=YES`) outside the window,
// following the rounded alpha of the pill content.
pub const WIDTH: f64 = 144.0;
pub const HEIGHT: f64 = 36.0;
const BOTTOM_MARGIN: f64 = 96.0;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PillState {
    Listening,
    Transcribing,
    Polishing,
    Done,
    Error,
    Canceled,
    /// Surface for the parallel-sessions saturated case (4th attempt
    /// while 3 are in flight). Paired with a `pill:toast` event
    /// carrying the message + auto-hide ms. Constructed solely via
    /// the toast path; the `Saturated` arm in the tray-state match
    /// covers the variant for exhaustiveness.
    #[allow(dead_code)]
    Saturated,
}

#[derive(Debug, Clone, Serialize)]
pub struct PillStatePayload {
    pub state: PillState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// True when the state is `Done` but a non-fatal step (typically
    /// LLM polish) failed and we copied the raw transcript instead.
    /// Without this flag a silent fail-open is indistinguishable from
    /// a successful polish — the user has no idea their cleanup was
    /// skipped.
    pub warning: bool,
    /// Count of background (non-foreground) in-flight sessions.
    /// Skipped when zero so single-session payloads stay byte-identical
    /// to the pre-parallel build.
    #[serde(skip_serializing_if = "is_zero", default)]
    pub background_count: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct PillToastPayload {
    pub message: String,
    pub ms: u32,
}

/// Cumulative streaming text payload — `text` is the FULL accumulated
/// transcript / polish so far, not a delta. The frontend's rate limiter
/// reveals chars/sec smoothly. `final_` flips true on the last chunk so
/// the UI flushes to the full text immediately, skipping the limiter.
#[derive(Debug, Clone, Serialize)]
pub struct StreamPayload {
    pub session_id: u64,
    pub text: String,
    #[serde(rename = "final")]
    pub final_: bool,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

#[derive(Debug, Clone, Serialize)]
struct AudioLevelPayload {
    level: f32,
}

pub fn create<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    if app.get_webview_window(LABEL).is_some() {
        return Ok(());
    }

    let window = tauri::WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("pill.html".into()))
        .title("Vox")
        .inner_size(WIDTH, HEIGHT)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .decorations(false)
        .transparent(true)
        // OS shadow follows the rounded alpha of the pill content — the
        // CSS `box-shadow` was producing a rectangular halo clipped to
        // the NSWindow bounds (same trap documented for
        // `.liquid-glass-window`).
        .shadow(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        .accept_first_mouse(false)
        .build()?;

    // Reposition before native tweaks so the panel-style swap doesn't reset us.
    if let Err(e) = reposition(&window) {
        log::warn!("pill reposition failed: {e}");
    }

    #[cfg(target_os = "macos")]
    if let Err(e) = apply_macos_panel_style(&window) {
        log::warn!("pill: failed to apply macOS panel style: {e}");
    }

    let _ = window.set_ignore_cursor_events(true);

    Ok(())
}

pub fn show<R: Runtime>(app: &AppHandle<R>) {
    if let Some(win) = app.get_webview_window(LABEL) {
        let _ = reposition(&win);
        let _ = win.show();
        // Delayed emit so a freshly-created WebView can subscribe before the event fires.
        // The generation counter is bumped synchronously here so any hide
        // that arrives within the 10ms window will advance the gen and
        // cause this scheduled emit to drop on wakeup.
        let scheduled_gen = PILL_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
        let app_clone = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            if PILL_GENERATION.load(Ordering::SeqCst) == scheduled_gen {
                let _ = app_clone.emit_to(LABEL, "pill:show", ());
            }
        });
    }
}

pub fn hide<R: Runtime>(app: &AppHandle<R>) {
    // Bump generation so any in-flight delayed `pill:show` emit from a
    // recent show() drops itself on wakeup instead of painting a pill the
    // user just dismissed.
    PILL_GENERATION.fetch_add(1, Ordering::SeqCst);
    if let Some(win) = app.get_webview_window(LABEL) {
        let _ = app.emit_to(LABEL, "pill:hide", ());
        let _ = win.hide();
    }
    crate::tray_icon::set_state(app, crate::tray_icon::TrayState::Idle);
}

pub fn emit_state<R: Runtime>(app: &AppHandle<R>, state: PillState, message: Option<String>) {
    emit_state_inner(app, state, message, false, 0);
}

/// Same as [`emit_state`] but flags the payload with `warning=true`. Use
/// for `Done` states where the clipboard write succeeded but a soft
/// step (e.g. LLM polish) was skipped, so the frontend can swap the
/// green-success treatment for an amber warning treatment.
#[allow(dead_code)]
pub fn emit_state_warning<R: Runtime>(
    app: &AppHandle<R>,
    state: PillState,
    message: Option<String>,
) {
    emit_state_inner(app, state, message, true, 0);
}

/// Variant that carries a non-zero background-session count for the
/// pill's queue badge. Used by SessionManager::set_stage when there
/// are background pipelines running.
pub fn emit_state_with_bg<R: Runtime>(
    app: &AppHandle<R>,
    state: PillState,
    message: Option<String>,
    background_count: u32,
) {
    emit_state_inner(app, state, message, false, background_count);
}

pub fn emit_state_warning_with_bg<R: Runtime>(
    app: &AppHandle<R>,
    state: PillState,
    message: Option<String>,
    background_count: u32,
) {
    emit_state_inner(app, state, message, true, background_count);
}

fn emit_state_inner<R: Runtime>(
    app: &AppHandle<R>,
    state: PillState,
    message: Option<String>,
    warning: bool,
    background_count: u32,
) {
    // Errors are interactive (dismissable); everything else stays click-through.
    let click_through = !matches!(state, PillState::Error);
    if let Some(win) = app.get_webview_window(LABEL) {
        let _ = win.set_ignore_cursor_events(click_through);
    }
    let payload = PillStatePayload {
        state,
        message,
        warning,
        background_count,
    };
    let _ = app.emit_to(LABEL, "pill:state", payload);

    let tray_state = match state {
        PillState::Listening => crate::tray_icon::TrayState::Recording,
        PillState::Transcribing | PillState::Polishing => crate::tray_icon::TrayState::Processing,
        PillState::Error | PillState::Saturated => crate::tray_icon::TrayState::Error,
        PillState::Done | PillState::Canceled => crate::tray_icon::TrayState::Idle,
    };
    crate::tray_icon::set_state(app, tray_state);
}

/// Emit a transient toast on the pill window. The frontend reverts the
/// visible toast text after `ms`; the pill window itself stays visible —
/// the in-flight pipeline owns the lifetime of the window. Callers that
/// need an unconditional auto-hide must schedule it themselves.
pub fn emit_toast<R: Runtime>(app: &AppHandle<R>, message: String, ms: u32) {
    let payload = PillToastPayload {
        message: message.clone(),
        ms,
    };
    let _ = app.emit_to(LABEL, "pill:toast", payload);
}

pub fn emit_audio_level<R: Runtime>(app: &AppHandle<R>, level: f32) {
    let _ = app.emit_to(LABEL, "pill:audio_level", AudioLevelPayload { level });
}

/// Emit a transcript stream chunk for `session_id`. `text` is the
/// cumulative transcript so far (Whisper segments concatenated by
/// the engine). `final_` flips true on the last chunk.
pub fn emit_transcript_stream<R: Runtime>(app: &AppHandle<R>, sid: u64, text: String, final_: bool) {
    let payload = StreamPayload { session_id: sid, text, final_ };
    let _ = app.emit_to(LABEL, "pill:transcript_stream", payload);
}

/// Emit a polish stream chunk for `session_id`. `text` is the
/// cumulative polished output so far (deltas accumulated by the
/// engine). `final_` flips true on the last chunk.
pub fn emit_polish_stream<R: Runtime>(app: &AppHandle<R>, sid: u64, text: String, final_: bool) {
    let payload = StreamPayload { session_id: sid, text, final_ };
    let _ = app.emit_to(LABEL, "pill:polish_stream", payload);
}

/// Wide width when streaming text is showing. Capped to (screen_w - 64)
/// so the pill never bleeds off-screen on small displays.
const WIDE_WIDTH_MAX: f64 = 760.0;
const WIDE_WIDTH_MIN: f64 = 144.0;
/// Floor for the streaming-mode pill once content-driven width feedback
/// kicks in. Below this we revert to the pure compact silhouette via
/// `set_wide(false)`. Raised from 240 → 360 to match the frontend's
/// new W_MIN floor (`Pill.tsx`); the buffer covers WINDOW_PADDING_W
/// (16 px) so a 360-px content min-width round-trips through the IPC
/// without the Rust clamp dropping it below the pill's content floor.
pub const WIDE_WIDTH_MIN_DYNAMIC: f64 = 360.0;
/// Caps for content-driven height growth during streaming. `HEIGHT` is
/// the compact single-line frame; `HEIGHT_MAX` keeps long polishes from
/// taking over the screen — the inner div scrolls beyond that.
pub const HEIGHT_MAX: f64 = 240.0;

/// Resize the pill window. `wide=true` enters streaming mode at the
/// dynamic floor (360 px); the frontend's ResizeObserver then drives
/// width up via `set_size` as the streamed text grows. `wide=false`
/// snaps back to the compact 144 px. Repositions afterwards so the
/// pill stays horizontally centred AND bottom-anchored so vertical
/// growth grows upward from the dock area instead of pushing the
/// bottom edge down.
pub fn set_wide<R: Runtime>(app: &AppHandle<R>, wide: bool) {
    let Some(win) = app.get_webview_window(LABEL) else { return };

    let frame = active_screen_logical_frame(&win).or_else(|| {
        win.primary_monitor().ok().flatten().map(|m| {
            let scale = m.scale_factor();
            let s = m.size();
            let p = m.position();
            (
                p.x as f64 / scale,
                p.y as f64 / scale,
                s.width as f64 / scale,
                s.height as f64 / scale,
            )
        })
    });

    let target_w = if wide {
        // Start small — the frontend will push the actual content size
        // back via `set_size` as soon as the first chunk renders. This
        // way short polishes never widen past their text and only long
        // ones grow to the cap.
        let screen_cap = frame.map(|(_, _, w, _)| (w - 64.0).max(WIDE_WIDTH_MIN));
        match screen_cap {
            Some(cap) => WIDE_WIDTH_MIN_DYNAMIC.min(cap),
            None => WIDE_WIDTH_MIN_DYNAMIC,
        }
    } else {
        WIDTH
    };

    // Snapping non-wide always resets height to the compact frame so
    // the Done/Error/Canceled labels render at the original silhouette.
    let target_h = HEIGHT;

    apply_size(&win, frame, target_w, target_h);
}

/// Set the pill's height while keeping the current width unchanged.
/// Convenience wrapper around `set_size` for callers that only care
/// about height; new code should prefer `set_size` directly. Kept on
/// the public surface so external callers (or future code) can opt
/// for the height-only path without repeating the `None` plumbing.
#[allow(dead_code)]
pub fn set_height<R: Runtime>(app: &AppHandle<R>, height: f64) {
    set_size(app, None, Some(height));
}

/// Frontend-driven width and/or height update used by the streaming
/// pill's ResizeObserver. Each side is independently optional —
/// pass `None` to preserve the current value. Width is clamped to the
/// streaming-mode dynamic range plus the screen-width safety cap;
/// height to the [HEIGHT, HEIGHT_MAX] band. Bottom-anchored so the
/// dock edge stays put as the pill grows upward.
pub fn set_size<R: Runtime>(
    app: &AppHandle<R>,
    width: Option<f64>,
    height: Option<f64>,
) {
    if width.is_none() && height.is_none() {
        return;
    }
    let Some(win) = app.get_webview_window(LABEL) else { return };
    let frame = active_screen_logical_frame(&win).or_else(|| {
        win.primary_monitor().ok().flatten().map(|m| {
            let scale = m.scale_factor();
            let s = m.size();
            let p = m.position();
            (
                p.x as f64 / scale,
                p.y as f64 / scale,
                s.width as f64 / scale,
                s.height as f64 / scale,
            )
        })
    });

    let scale = win.scale_factor().unwrap_or(1.0);
    let cur_size = win.inner_size().ok();
    let cur_w = cur_size
        .as_ref()
        .map(|s| s.width as f64 / scale)
        .unwrap_or(WIDTH);
    let cur_h = cur_size
        .map(|s| s.height as f64 / scale)
        .unwrap_or(HEIGHT);

    // Below-floor escape hatch: the streaming pill's frontend asks
    // for compact dimensions (~132 x 46 px) when transitioning out of
    // streaming so the Done/Error/Canceled labels don't render at the
    // streamed width while we wait for the next `set_wide(false)` IPC
    // to arrive. Detect that here and route through the compact path —
    // otherwise we'd just clamp those values back up to the dynamic
    // floor and visibly leave the pill wide for ~150 ms.
    if let Some(w) = width {
        if w < WIDE_WIDTH_MIN_DYNAMIC {
            apply_size(&win, frame, WIDTH, HEIGHT);
            return;
        }
    }

    let target_w = match width {
        Some(w) => {
            let screen_cap = frame.map(|(_, _, fw, _)| fw - 64.0);
            let upper = match screen_cap {
                Some(cap) => WIDE_WIDTH_MAX.min(cap),
                None => WIDE_WIDTH_MAX,
            };
            // Defensive: on a tiny logical display (or split-screen)
            // `screen_w - 64` can drop below WIDE_WIDTH_MIN_DYNAMIC,
            // which would invert the clamp bounds and panic. Bumping
            // `upper` up to the floor first keeps `clamp` well-formed
            // and yields the floor itself in that pathological case.
            let upper = upper.max(WIDE_WIDTH_MIN_DYNAMIC);
            w.clamp(WIDE_WIDTH_MIN_DYNAMIC, upper)
        }
        None => cur_w,
    };

    let target_h = match height {
        Some(h) => h.clamp(HEIGHT, HEIGHT_MAX),
        None => cur_h,
    };

    apply_size(&win, frame, target_w, target_h);
}

fn apply_size<R: Runtime>(
    win: &tauri::WebviewWindow<R>,
    frame: Option<(f64, f64, f64, f64)>,
    target_w: f64,
    target_h: f64,
) {
    if let Some((mx, my, mw, mh)) = frame {
        let x = mx + (mw - target_w) / 2.0;
        // Bottom-anchored: the pill's bottom edge stays at the same Y so
        // height growth pushes the top edge up rather than the bottom
        // edge down (which would clip into the dock area).
        let y = my + mh - target_h - BOTTOM_MARGIN;
        let _ = win.set_size(LogicalSize::new(target_w, target_h));
        let _ = win.set_position(LogicalPosition::new(x, y));
    } else {
        let _ = win.set_size(LogicalSize::new(target_w, target_h));
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ActiveModePayload {
    pub id: String,
    pub name: String,
    pub icon: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModeSuggestionPayload {
    pub id: String,
    pub name: String,
    pub icon: String,
}

pub fn emit_active_mode<R: Runtime>(app: &AppHandle<R>, mode: &crate::modes::VoiceMode) {
    let payload = ActiveModePayload {
        id: mode.id.clone(),
        name: mode.name.clone(),
        icon: mode.icon.clone(),
    };
    let _ = app.emit_to(LABEL, "pill:active_mode", payload);
}

pub fn emit_mode_suggestion<R: Runtime>(
    app: &AppHandle<R>,
    suggestion: Option<crate::modes::VoiceMode>,
) {
    let payload = suggestion.map(|m| ModeSuggestionPayload {
        id: m.id,
        name: m.name,
        icon: m.icon,
    });
    let _ = app.emit_to(LABEL, "pill:mode_suggestion", payload);
}

fn reposition<R: Runtime>(win: &tauri::WebviewWindow<R>) -> tauri::Result<()> {
    let frame = active_screen_logical_frame(win).or_else(|| {
        win.primary_monitor().ok().flatten().map(|m| {
            let scale = m.scale_factor();
            let s = m.size();
            let p = m.position();
            (
                p.x as f64 / scale,
                p.y as f64 / scale,
                s.width as f64 / scale,
                s.height as f64 / scale,
            )
        })
    });
    if let Some((mx, my, mw, mh)) = frame {
        let x = mx + (mw - WIDTH) / 2.0;
        let y = my + mh - HEIGHT - BOTTOM_MARGIN;
        win.set_size(LogicalSize::new(WIDTH, HEIGHT))?;
        win.set_position(LogicalPosition::new(x, y))?;
    }
    Ok(())
}

fn active_screen_logical_frame<R: Runtime>(
    win: &tauri::WebviewWindow<R>,
) -> Option<(f64, f64, f64, f64)> {
    #[cfg(target_os = "macos")]
    if let Some(frame) = macos_active_screen_frame() {
        return Some(frame);
    }

    let monitors = win.available_monitors().ok()?;
    let cursor = win.cursor_position().ok()?;
    monitors.iter().find_map(|m| {
        let scale = m.scale_factor();
        let pos = m.position();
        let size = m.size();
        let (mx, my) = (pos.x as f64, pos.y as f64);
        let (mw, mh) = (size.width as f64, size.height as f64);
        (cursor.x >= mx && cursor.x < mx + mw && cursor.y >= my && cursor.y < my + mh)
            .then(|| (mx / scale, my / scale, mw / scale, mh / scale))
    })
}

#[cfg(target_os = "macos")]
fn macos_active_screen_frame() -> Option<(f64, f64, f64, f64)> {
    use objc2_app_kit::{NSEvent, NSScreen};
    use objc2_foundation::MainThreadMarker;

    // NSScreen / NSEvent are main-thread-only. reposition() is reached from
    // the Tauri main thread, but we guard here anyway.
    let mtm = MainThreadMarker::new()?;
    let cursor = NSEvent::mouseLocation();
    let screens = NSScreen::screens(mtm);
    let primary = screens.firstObject()?;
    let primary_frame = primary.frame();
    let primary_height = primary_frame.size.height;

    let frame = screens
        .iter()
        .map(|s| s.frame())
        .find(|f| {
            cursor.x >= f.origin.x
                && cursor.x < f.origin.x + f.size.width
                && cursor.y >= f.origin.y
                && cursor.y < f.origin.y + f.size.height
        })
        .unwrap_or(primary_frame);

    // AppKit origin is bottom-left of the primary; Tauri origin is top-left.
    let logical_y = primary_height - (frame.origin.y + frame.size.height);
    Some((frame.origin.x, logical_y, frame.size.width, frame.size.height))
}

#[cfg(target_os = "macos")]
fn apply_macos_panel_style<R: Runtime>(win: &tauri::WebviewWindow<R>) -> anyhow::Result<()> {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::{
        NSStatusWindowLevel, NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask,
    };

    let raw = win
        .ns_window()
        .map_err(|e| anyhow::anyhow!("ns_window: {e}"))?;
    if raw.is_null() {
        return Err(anyhow::anyhow!("ns_window returned NULL"));
    }
    let ns_window: &NSWindow = unsafe { &*(raw as *const NSWindow) };

    unsafe {
        // Status-bar level + FullScreenAuxiliary + stationary keep the pill
        // floating above every app (including fullscreen video) on every space.
        ns_window.setLevel(NSStatusWindowLevel);
        ns_window.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::FullScreenAuxiliary,
        );
        ns_window.setMovableByWindowBackground(false);
        ns_window.setHidesOnDeactivate(false);
        ns_window.setExcludedFromWindowsMenu(true);
        // NonactivatingPanel keeps focus with the user's current app on click —
        // we can't swap the NSWindow class to NSPanel without rebuilding,
        // but the style bit is enough.
        ns_window.setStyleMask(NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel);
        // setCanHide isn't on objc2's typed NSWindow surface; route via msg_send.
        let raw_obj: &AnyObject = &*(ns_window as *const NSWindow as *const AnyObject);
        let _: () = msg_send![raw_obj, setCanHide: false];
    }
    Ok(())
}
