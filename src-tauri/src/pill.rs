//! Listening-pill window: a small floating overlay at bottom-center
//! that shows dictation state (listening / transcribing / polishing / done).
//!
//! The WebView renders the visuals; Rust owns the window lifecycle and
//! the macOS-native tweaks (non-activating panel, status-bar level, joins
//! all spaces, click-through toggle).

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

    // Position bottom-center on primary display. macOS: do this before
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
}

pub fn emit_state<R: Runtime>(app: &AppHandle<R>, state: PillState, message: Option<String>) {
    // Toggle click-through based on state: errors let users dismiss.
    let click_through = !matches!(state, PillState::Error);
    if let Some(win) = app.get_webview_window(LABEL) {
        let _ = win.set_ignore_cursor_events(click_through);
    }
    let payload = PillStatePayload { state, message };
    let _ = app.emit_to(LABEL, "pill:state", payload);
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
    if let Some(monitor) = win.primary_monitor()? {
        let scale = monitor.scale_factor();
        let size = monitor.size();
        let pos = monitor.position();
        // Convert physical → logical for positioning.
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

// ---------- macOS: make the pill a non-activating, status-bar panel --------

#[cfg(target_os = "macos")]
fn apply_macos_panel_style<R: Runtime>(win: &tauri::WebviewWindow<R>) -> anyhow::Result<()> {
    use cocoa::appkit::{NSWindow, NSWindowCollectionBehavior};
    use cocoa::base::id;
    use objc::{msg_send, sel, sel_impl};

    let ns_window = win.ns_window().map_err(|e| anyhow::anyhow!("{e}"))? as id;
    unsafe {
        // Status-bar level so the pill sits above every app, including
        // fullscreen video.
        const NS_STATUS_WINDOW_LEVEL: i64 = 25;
        ns_window.setLevel_(NS_STATUS_WINDOW_LEVEL);

        // Join all spaces + be visible in fullscreen (auxiliary).
        let behavior: NSWindowCollectionBehavior =
            NSWindowCollectionBehavior::NSWindowCollectionBehaviorCanJoinAllSpaces
                | NSWindowCollectionBehavior::NSWindowCollectionBehaviorStationary
                | NSWindowCollectionBehavior::NSWindowCollectionBehaviorFullScreenAuxiliary;
        ns_window.setCollectionBehavior_(behavior);

        // Don't accept mouse-moved events; don't become key or main.
        let _: () = msg_send![ns_window, setMovableByWindowBackground: false];
        let _: () = msg_send![ns_window, setHidesOnDeactivate: false];
        let _: () = msg_send![ns_window, setExcludedFromWindowsMenu: true];

        // Style mask: borderless + non-activating (0x80). We can't set the
        // full NSPanel mask without swapping the NSWindow class, but turning
        // on the non-activating bit works and keeps focus with the user's app.
        const NS_WINDOW_STYLE_MASK_BORDERLESS: u64 = 0;
        const NS_WINDOW_STYLE_MASK_NONACTIVATING_PANEL: u64 = 0x80;
        let mask: u64 =
            NS_WINDOW_STYLE_MASK_BORDERLESS | NS_WINDOW_STYLE_MASK_NONACTIVATING_PANEL;
        let _: () = msg_send![ns_window, setStyleMask: mask];
    }
    Ok(())
}
