//! Notch List window (macOS-only). A transparent, decoration-less,
//! always-on-top panel anchored at the top-center of the chosen display —
//! a synthetic "notch" pill for v1 (no physical-notch fusing).
//!
//! The window builder + macOS panel style + top-center positioning are
//! lifted from `pill.rs` (the proven pattern). KEY DIFFERENCE from the
//! pill: we do NOT call `set_ignore_cursor_events(true)` — the notch needs
//! hover + click (the checkbox) to work.
//!
//! Whole module is `#[cfg(target_os = "macos")]` (gated at the `mod notch;`
//! declaration in `lib.rs`), so this file is only ever compiled on macOS.

use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Runtime, WebviewUrl};

pub const LABEL: &str = "notch";

/// Collapsed resting width/height. The window is created at the collapsed
/// size; the frontend grows it on hover via `setSize` (mirroring how the
/// pill drives its own sizing). Sized generously so the single top-item
/// line and the empty-state hotkey row both fit without clipping.
pub const WIDTH: f64 = 280.0;
pub const HEIGHT: f64 = 30.0;
/// Gap from the very top edge of the display so the notch doesn't kiss the
/// menu bar.
const TOP_MARGIN: f64 = 2.0;

/// Create the notch window if it doesn't already exist. Mirrors
/// `pill::create`: transparent, decoration-less, always-on-top,
/// skip-taskbar, non-focusing. Unlike the pill it stays interactive
/// (no `set_ignore_cursor_events`).
pub fn create<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    if app.get_webview_window(LABEL).is_some() {
        return Ok(());
    }

    let window =
        tauri::WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("notch.html".into()))
            .title("Vox Notch")
            .inner_size(WIDTH, HEIGHT)
            .resizable(false)
            .maximizable(false)
            .minimizable(false)
            .decorations(false)
            .transparent(true)
            // OS shadow follows the rounded alpha of the content on macOS
            // (same rationale as the pill). The notch is macOS-only, so we
            // always keep the OS shadow on.
            .shadow(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .focused(false)
            .visible(true)
            .accept_first_mouse(true)
            .build()?;

    if let Err(e) = reposition(&window) {
        log::warn!("notch reposition failed: {e}");
    }

    if let Err(e) = apply_macos_panel_style(&window) {
        log::warn!("notch: failed to apply macOS panel style: {e}");
    }

    Ok(())
}

/// Destroy the notch window (used when the user turns the feature off).
pub fn destroy<R: Runtime>(app: &AppHandle<R>) {
    if let Some(win) = app.get_webview_window(LABEL) {
        if let Err(e) = win.close() {
            log::warn!("notch: failed to close window: {e}");
        }
    }
}

/// Create or destroy the notch window to match `enabled`. Called from
/// `commands::set_settings` when `notch_list_enabled` flips.
pub fn create_or_destroy<R: Runtime>(app: &AppHandle<R>, enabled: bool) {
    if enabled {
        if let Err(e) = create(app) {
            log::error!("notch: failed to create window: {e}");
        }
    } else {
        destroy(app);
    }
}

/// Frontend-driven size update (hover expand / collapse). Width/height are
/// each optional; the window stays top-center anchored so growth expands
/// downward and stays horizontally centered. Mirrors `pill::set_size`'s
/// reposition-after-resize, but TOP-anchored instead of bottom-anchored.
pub fn set_size<R: Runtime>(app: &AppHandle<R>, width: Option<f64>, height: Option<f64>) {
    if width.is_none() && height.is_none() {
        return;
    }
    let Some(win) = app.get_webview_window(LABEL) else {
        return;
    };
    let frame = target_frame(&win);

    let scale = win.scale_factor().unwrap_or(1.0);
    let cur = win.inner_size().ok();
    let cur_w = cur.as_ref().map(|s| s.width as f64 / scale).unwrap_or(WIDTH);
    let cur_h = cur.map(|s| s.height as f64 / scale).unwrap_or(HEIGHT);

    let target_w = width.unwrap_or(cur_w).max(WIDTH);
    let target_h = height.unwrap_or(cur_h).max(HEIGHT);

    apply_size(&win, frame, target_w, target_h);
}

fn reposition<R: Runtime>(win: &tauri::WebviewWindow<R>) -> tauri::Result<()> {
    if let Some((mx, my, mw, _mh)) = target_frame(win) {
        let x = mx + (mw - WIDTH) / 2.0;
        let y = my + TOP_MARGIN;
        win.set_size(LogicalSize::new(WIDTH, HEIGHT))?;
        win.set_position(LogicalPosition::new(x, y))?;
    }
    Ok(())
}

fn apply_size<R: Runtime>(
    win: &tauri::WebviewWindow<R>,
    frame: Option<(f64, f64, f64, f64)>,
    target_w: f64,
    target_h: f64,
) {
    if let Some((mx, my, mw, _mh)) = frame {
        let x = mx + (mw - target_w) / 2.0;
        // Top-anchored: the top edge stays at TOP_MARGIN so hover growth
        // expands the panel downward from the notch.
        let y = my + TOP_MARGIN;
        let _ = win.set_size(LogicalSize::new(target_w, target_h));
        let _ = win.set_position(LogicalPosition::new(x, y));
    } else {
        let _ = win.set_size(LogicalSize::new(target_w, target_h));
    }
}

/// Logical frame `(x, y, w, h)` of the display the notch should attach to,
/// honoring `Settings::notch_list_display`:
/// - `"active"` → cursor's display (reuses pill's
///   `active_screen_logical_frame`),
/// - `"builtin"` → the built-in display (assumed primary on macOS),
/// - `"primary"` → the primary display.
/// Falls back to the Tauri primary monitor on any miss.
fn target_frame<R: Runtime>(
    win: &tauri::WebviewWindow<R>,
) -> Option<(f64, f64, f64, f64)> {
    let display = crate::settings::load(win.app_handle()).notch_list_display;

    match display.as_str() {
        "active" => crate::pill::active_screen_logical_frame(win).or_else(|| primary_frame(win)),
        // builtin/primary both resolve to the primary monitor here — macOS
        // reports the built-in panel as the primary in the common case,
        // and we don't ship a separate built-in probe in v1.
        _ => primary_frame(win),
    }
}

fn primary_frame<R: Runtime>(
    win: &tauri::WebviewWindow<R>,
) -> Option<(f64, f64, f64, f64)> {
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
}

/// Identical panel-style treatment as `pill::apply_macos_panel_style`:
/// status-bar level, all-spaces / fullscreen-auxiliary collection
/// behaviour, non-activating panel style. Hand-mirrored from pill.rs so
/// the objc2 FFI idioms match exactly.
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
        ns_window.setLevel(NSStatusWindowLevel);
        ns_window.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::FullScreenAuxiliary,
        );
        ns_window.setMovableByWindowBackground(false);
        ns_window.setHidesOnDeactivate(false);
        ns_window.setExcludedFromWindowsMenu(true);
        ns_window
            .setStyleMask(NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel);
        let raw_obj: &AnyObject = &*(ns_window as *const NSWindow as *const AnyObject);
        let _: () = msg_send![raw_obj, setCanHide: false];
    }
    Ok(())
}
