//! Menu-bar tray icon + popover.
//!
//! Left-click: show the popover anchored near the tray icon.
//! Right-click: show a native fallback menu (Open Settings / Quit).

use parking_lot::Mutex;
use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, LogicalPosition, LogicalSize, Manager, PhysicalPosition, Runtime, WebviewUrl,
};

use once_cell::sync::OnceCell;

pub const POPOVER_LABEL: &str = "tray-popover";
const POPOVER_W: f64 = 320.0;
const POPOVER_H: f64 = 400.0;

/// Most recently observed tray icon rect (physical coords). We capture this
/// from the TrayIconEvent so we can anchor the popover under it.
static LAST_TRAY_RECT: OnceCell<Mutex<Option<PhysicalPosition<f64>>>> = OnceCell::new();

fn rect_store() -> &'static Mutex<Option<PhysicalPosition<f64>>> {
    LAST_TRAY_RECT.get_or_init(|| Mutex::new(None))
}

pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let open_item = MenuItem::with_id(app, "open-settings", "Open Settings", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit Vox", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open_item, &quit_item])?;

    let icon = Some(placeholder_icon());

    let mut builder = TrayIconBuilder::with_id("vox-tray")
        .tooltip("Vox — hold hotkey to dictate")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "open-settings" => show_settings(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            match event {
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    position,
                    ..
                } => {
                    // Remember where the icon was so we can anchor the popover.
                    *rect_store().lock() = Some(position);
                    let app = tray.app_handle();
                    if let Err(e) = show_popover(app) {
                        log::warn!("failed to show tray popover: {e}; opening settings instead");
                        show_settings(app);
                    }
                }
                TrayIconEvent::Enter { position, .. }
                | TrayIconEvent::Move { position, .. } => {
                    *rect_store().lock() = Some(position);
                }
                _ => {}
            }
        });

    if let Some(img) = icon {
        builder = builder.icon(img).icon_as_template(true);
    }

    builder.build(app)?;
    Ok(())
}

fn show_settings<R: Runtime>(app: &AppHandle<R>) {
    if let Some(win) = app.get_webview_window("settings") {
        let _ = win.show();
        let _ = win.set_focus();
    }
    // Close the popover if open.
    if let Some(win) = app.get_webview_window(POPOVER_LABEL) {
        let _ = win.hide();
    }
}

pub fn show_popover<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let win = match app.get_webview_window(POPOVER_LABEL) {
        Some(w) => w,
        None => tauri::WebviewWindowBuilder::new(
            app,
            POPOVER_LABEL,
            WebviewUrl::App("tray-popover.html".into()),
        )
        .title("Vox")
        .inner_size(POPOVER_W, POPOVER_H)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(true)
        .visible(false)
        .build()?,
    };

    // Toggle: if already visible, hide.
    if win.is_visible().unwrap_or(false) {
        let _ = win.hide();
        return Ok(());
    }

    // Position: anchor below the tray icon if we have its position; otherwise
    // top-right of the primary display, 8px below the menu bar.
    if let Some(monitor) = win.primary_monitor()? {
        let scale = monitor.scale_factor();
        let mw = monitor.size().width as f64 / scale;
        let mx = monitor.position().x as f64 / scale;
        let my = monitor.position().y as f64 / scale;

        let (cx_logical, top_y) = match *rect_store().lock() {
            Some(p) => {
                let cx = p.x / scale;
                let cy = p.y / scale;
                (cx, cy + 24.0)
            }
            None => (mx + mw - 32.0, my + 8.0 + 22.0),
        };

        let mut x = cx_logical - POPOVER_W / 2.0;
        // Keep on-screen.
        if x + POPOVER_W > mx + mw - 8.0 {
            x = mx + mw - POPOVER_W - 8.0;
        }
        if x < mx + 8.0 {
            x = mx + 8.0;
        }
        let y = top_y.max(my + 8.0 + 22.0);

        let _ = win.set_size(LogicalSize::new(POPOVER_W, POPOVER_H));
        let _ = win.set_position(LogicalPosition::new(x, y));
    }

    win.show()?;
    win.set_focus()?;
    Ok(())
}

/// Return a simple programmatic tray icon. In Phase 1 we skip bundled PNGs
/// and draw a minimal alpha-only "mic" template so the app always boots.
fn placeholder_icon() -> Image<'static> {
    const W: u32 = 16;
    const H: u32 = 16;
    let mut rgba = vec![0u8; (W * H * 4) as usize];
    for y in 2..13 {
        for x in 6..10 {
            let i = ((y * W + x) * 4) as usize;
            rgba[i] = 0;
            rgba[i + 1] = 0;
            rgba[i + 2] = 0;
            rgba[i + 3] = 255;
        }
    }
    for y in 12..15 {
        for x in 7..9 {
            let i = ((y * W + x) * 4) as usize;
            rgba[i + 3] = 255;
        }
    }
    Image::new_owned(rgba, W, H)
}
