use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Runtime, WebviewUrl};

pub const LABEL: &str = "pill";
// Slimmer pill — about half the previous footprint. The webview leaves
// a small slack around the visible element so the soft drop shadow can
// fade into transparency instead of clipping flush at the window edge.
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
        .shadow(false)
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
        let app_clone = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            let _ = app_clone.emit_to(LABEL, "pill:show", ());
        });
    }
}

pub fn hide<R: Runtime>(app: &AppHandle<R>) {
    if let Some(win) = app.get_webview_window(LABEL) {
        let _ = app.emit_to(LABEL, "pill:hide", ());
        let _ = win.hide();
    }
    crate::tray_icon::set_state(app, crate::tray_icon::TrayState::Idle);
}

pub fn emit_state<R: Runtime>(app: &AppHandle<R>, state: PillState, message: Option<String>) {
    emit_state_inner(app, state, message, false);
}

/// Same as [`emit_state`] but flags the payload with `warning=true`. Use
/// for `Done` states where the clipboard write succeeded but a soft
/// step (e.g. LLM polish) was skipped, so the frontend can swap the
/// green-success treatment for an amber warning treatment.
pub fn emit_state_warning<R: Runtime>(
    app: &AppHandle<R>,
    state: PillState,
    message: Option<String>,
) {
    emit_state_inner(app, state, message, true);
}

fn emit_state_inner<R: Runtime>(
    app: &AppHandle<R>,
    state: PillState,
    message: Option<String>,
    warning: bool,
) {
    // Errors are interactive (dismissable); everything else stays click-through.
    let click_through = !matches!(state, PillState::Error);
    if let Some(win) = app.get_webview_window(LABEL) {
        let _ = win.set_ignore_cursor_events(click_through);
    }
    let payload = PillStatePayload { state, message, warning };
    let _ = app.emit_to(LABEL, "pill:state", payload);

    let tray_state = match state {
        PillState::Listening => crate::tray_icon::TrayState::Recording,
        PillState::Transcribing | PillState::Polishing => crate::tray_icon::TrayState::Processing,
        PillState::Error => crate::tray_icon::TrayState::Error,
        PillState::Done | PillState::Canceled => crate::tray_icon::TrayState::Idle,
    };
    crate::tray_icon::set_state(app, tray_state);
}

pub fn emit_audio_level<R: Runtime>(app: &AppHandle<R>, level: f32) {
    let _ = app.emit_to(LABEL, "pill:audio_level", AudioLevelPayload { level });
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

pub fn hide_after<R: Runtime>(app: &Arc<AppHandle<R>>, ms: u64) {
    let app_clone = Arc::clone(app);
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(ms)).await;
        hide(&app_clone);
    });
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
