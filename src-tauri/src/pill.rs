//! Listening-pill window: a small floating overlay at bottom-center
//! that shows dictation state (listening / transcribing / polishing / done).
//!
//! The WebView renders the visuals; Rust owns the window lifecycle and
//! the macOS-native tweaks (non-activating panel, status-bar level, joins
//! all spaces, click-through toggle).
//!
//! Multi-monitor: the pill positions itself bottom-center of the screen
//! containing the cursor at show-time; falls back to the primary monitor
//! when no active screen can be determined.

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Runtime, WebviewUrl};

pub const LABEL: &str = "pill";
pub const WIDTH: f64 = 280.0;
pub const HEIGHT: f64 = 56.0;
/// Vertical margin from the bottom of the screen (above Dock).
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
}

#[derive(Debug, Clone, Serialize)]
struct AudioLevelPayload {
    level: f32,
}

/// Create the pill window (invisible, on top of everything, non-focusing).
/// Call once at app start.
pub fn create<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    if app.get_webview_window(LABEL).is_some() {
        return Ok(());
    }

    let builder = tauri::WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("pill.html".into()))
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
        .accept_first_mouse(false);

    let window = builder.build()?;

    // Position bottom-center on the active display. macOS: do this before
    // applying the native panel tweaks.
    if let Err(e) = reposition(&window) {
        log::warn!("pill reposition failed: {e}");
    }

    #[cfg(target_os = "macos")]
    {
        if let Err(e) = apply_macos_panel_style(&window) {
            log::warn!("pill: failed to apply macOS panel style: {e}");
        }
    }

    // Initial state: click-through is ON (we're not showing errors yet).
    let _ = window.set_ignore_cursor_events(true);

    Ok(())
}

pub fn show<R: Runtime>(app: &AppHandle<R>) {
    if let Some(win) = app.get_webview_window(LABEL) {
        let _ = reposition(&win);
        let _ = win.show();
        // Emit after show so the WebView has a chance to subscribe first time.
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
    // The pipeline has finished — return the tray to idle, unless the user
    // has a missing-permission situation (`tray_icon` will pick that up on
    // the next `init` call; we don't override it here).
    crate::tray_icon::set_state(app, crate::tray_icon::TrayState::Idle);
}

pub fn emit_state<R: Runtime>(app: &AppHandle<R>, state: PillState, message: Option<String>) {
    // Toggle click-through based on state: errors let users dismiss.
    let click_through = !matches!(state, PillState::Error);
    if let Some(win) = app.get_webview_window(LABEL) {
        let _ = win.set_ignore_cursor_events(click_through);
    }
    let payload = PillStatePayload { state, message };
    let _ = app.emit_to(LABEL, "pill:state", payload);

    // Mirror the pipeline state into the tray icon. Done is a transient
    // "checkmark" the pill auto-hides; the tray flips back to Idle when
    // pill::hide is eventually called.
    let tray_state = match state {
        PillState::Listening => crate::tray_icon::TrayState::Recording,
        PillState::Transcribing | PillState::Polishing => {
            crate::tray_icon::TrayState::Processing
        }
        PillState::Error => crate::tray_icon::TrayState::Error,
        PillState::Done | PillState::Canceled => crate::tray_icon::TrayState::Idle,
    };
    crate::tray_icon::set_state(app, tray_state);
}

pub fn emit_audio_level<R: Runtime>(app: &AppHandle<R>, level: f32) {
    let _ = app.emit_to(LABEL, "pill:audio_level", AudioLevelPayload { level });
}

/// Schedule a hide after `ms` (used for Done / Error auto-dismiss).
pub fn hide_after<R: Runtime>(app: &Arc<AppHandle<R>>, ms: u64) {
    let app_clone = Arc::clone(app);
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(ms)).await;
        hide(&app_clone);
    });
}

fn reposition<R: Runtime>(win: &tauri::WebviewWindow<R>) -> tauri::Result<()> {
    // Strategy: prefer the screen containing the mouse cursor (which is
    // almost always the screen the user just clicked or the screen with
    // the focused app). Fall back to the primary monitor.
    if let Some((mx, my, mw, mh)) = active_screen_logical_frame(win) {
        let x = mx + (mw - WIDTH) / 2.0;
        let y = my + mh - HEIGHT - BOTTOM_MARGIN;

        win.set_size(LogicalSize::new(WIDTH, HEIGHT))?;
        win.set_position(LogicalPosition::new(x, y))?;
        return Ok(());
    }

    if let Some(monitor) = win.primary_monitor()? {
        let scale = monitor.scale_factor();
        let size = monitor.size();
        let pos = monitor.position();
        let mw = size.width as f64 / scale;
        let mh = size.height as f64 / scale;
        let mx = pos.x as f64 / scale;
        let my = pos.y as f64 / scale;

        let x = mx + (mw - WIDTH) / 2.0;
        let y = my + mh - HEIGHT - BOTTOM_MARGIN;

        win.set_size(LogicalSize::new(WIDTH, HEIGHT))?;
        win.set_position(LogicalPosition::new(x, y))?;
    }
    Ok(())
}

/// Find the logical frame of the screen the user is currently working on.
///
/// On macOS, queries `NSEvent::mouseLocation()` and walks `NSScreen::screens()`
/// to find the screen whose AppKit frame contains it. Translates the AppKit
/// (bottom-up) coordinate space to the top-left logical coordinate space
/// Tauri uses for window positioning.
///
/// Returns `(x, y, width, height)` in logical (point) coordinates with `(0,0)`
/// at the top-left of the primary display, matching `monitor.position()` /
/// `set_position` on Tauri.
fn active_screen_logical_frame<R: Runtime>(
    win: &tauri::WebviewWindow<R>,
) -> Option<(f64, f64, f64, f64)> {
    #[cfg(target_os = "macos")]
    {
        if let Some(frame) = macos_active_screen_frame() {
            return Some(frame);
        }
    }

    // Cross-platform fallback: look at all monitors Tauri knows about and
    // find the one whose physical rect contains the cursor.
    let monitors = win.available_monitors().ok()?;
    if monitors.is_empty() {
        return None;
    }
    let cursor = win.cursor_position().ok()?;
    for m in &monitors {
        let scale = m.scale_factor();
        let pos = m.position();
        let size = m.size();
        let mx = pos.x as f64;
        let my = pos.y as f64;
        let mw = size.width as f64;
        let mh = size.height as f64;
        if cursor.x >= mx && cursor.x < mx + mw && cursor.y >= my && cursor.y < my + mh {
            return Some((mx / scale, my / scale, mw / scale, mh / scale));
        }
    }
    // Cursor wasn't on any known monitor — caller will fall back to primary.
    None
}

// ---------- macOS: active screen + non-activating panel --------------------

#[cfg(target_os = "macos")]
fn macos_active_screen_frame() -> Option<(f64, f64, f64, f64)> {
    use objc2_app_kit::{NSEvent, NSScreen};
    use objc2_foundation::MainThreadMarker;

    // NSScreen / NSEvent methods that return values touching the AppKit
    // global state must run on the main thread. The pill's reposition() is
    // currently called from the Tauri main thread (through `pill::show` on
    // the hotkey callback's app handle, which dispatches to the main run
    // loop), so we *should* be on the main thread — but we guard anyway.
    let mtm = MainThreadMarker::new()?;

    // mouseLocation is an AppKit class method; coordinate origin is the
    // bottom-left of the primary display (AppKit Y goes up). All NSScreen
    // frames live in the same coordinate space.
    let cursor = NSEvent::mouseLocation();

    // We need the primary screen to translate AppKit coords (origin
    // bottom-left of primary) to Tauri logical coords (origin top-left of
    // primary). The "primary" screen is the first element of `screens()`.
    let screens = NSScreen::screens(mtm);
    let primary = screens.firstObject()?;
    let primary_frame = primary.frame();
    let primary_height = primary_frame.size.height;

    // Locate the screen containing the cursor.
    let mut chosen_frame = None;
    for screen in screens.iter() {
        let f = screen.frame();
        let in_x = cursor.x >= f.origin.x && cursor.x < f.origin.x + f.size.width;
        let in_y = cursor.y >= f.origin.y && cursor.y < f.origin.y + f.size.height;
        if in_x && in_y {
            chosen_frame = Some(f);
            break;
        }
    }
    let frame = chosen_frame.or(Some(primary_frame))?;

    // AppKit (bottom-up) → Tauri logical (top-down) conversion.
    // For the primary screen `origin.y == 0`, so logical_y == 0.
    // For a screen above the primary (origin.y > 0), logical_y is negative.
    // For a screen below (origin.y < 0), logical_y is positive.
    let logical_x = frame.origin.x;
    let logical_y = primary_height - (frame.origin.y + frame.size.height);

    Some((logical_x, logical_y, frame.size.width, frame.size.height))
}

#[cfg(not(target_os = "macos"))]
#[allow(dead_code)]
fn macos_active_screen_frame() -> Option<(f64, f64, f64, f64)> {
    None
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
    // Tauri hands us a `*mut c_void` to the underlying NSWindow. Re-borrow it
    // as an objc2 NSWindow reference. The Tauri runtime owns the actual
    // retain count; we just need a typed handle to call methods on.
    let ns_window: &NSWindow = unsafe { &*(raw as *const NSWindow) };

    unsafe {
        // Status-bar level so the pill sits above every app, including
        // fullscreen video.
        ns_window.setLevel(NSStatusWindowLevel);

        // Join all spaces + be visible in fullscreen (auxiliary) + stay
        // anchored to the active display.
        let behavior = NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::FullScreenAuxiliary;
        ns_window.setCollectionBehavior(behavior);

        // No drag-to-move on the empty area; don't hide on app deactivate;
        // don't appear in the Window menu.
        ns_window.setMovableByWindowBackground(false);
        ns_window.setHidesOnDeactivate(false);
        ns_window.setExcludedFromWindowsMenu(true);

        // Style mask: borderless + non-activating panel. We can't fully
        // swap the NSWindow class to NSPanel without rebuilding the window,
        // but turning on the NonactivatingPanel bit is enough to keep focus
        // with the user's current app on click.
        let mask = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
        ns_window.setStyleMask(mask);

        // Belt-and-braces: tell AppKit not to make the window key or main
        // even if a stray click slips through. These two selectors don't
        // exist as setters on NSWindow's typed surface, so we keep them as
        // dynamic msg_send! calls — but routed through objc2's runtime.
        let raw_obj: &AnyObject = &*(ns_window as *const NSWindow as *const AnyObject);
        let _: () = msg_send![raw_obj, setCanHide: false];
    }
    Ok(())
}
