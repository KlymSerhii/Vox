use parking_lot::Mutex;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, LogicalPosition, LogicalSize, Manager, PhysicalPosition, Runtime, WebviewUrl,
};

use once_cell::sync::OnceCell;

use crate::tray_icon;

pub const POPOVER_LABEL: &str = "tray-popover";
const POPOVER_W: f64 = 320.0;
// 540 px gives the `flex-1` Recent list ~200 px of room (enough for all
// five transcripts at once, which is the cap from `transcripts::snapshot`),
// and pushes the Voice-mode trigger far enough up the popover that its
// dropdown — absolutely positioned and clipped by the popover's
// `overflow-hidden` root — has room to expand downward without truncation.
const POPOVER_H: f64 = 540.0;

static LAST_TRAY_RECT: OnceCell<Mutex<Option<PhysicalPosition<f64>>>> = OnceCell::new();

fn rect_store() -> &'static Mutex<Option<PhysicalPosition<f64>>> {
    LAST_TRAY_RECT.get_or_init(|| Mutex::new(None))
}

pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let open_item = MenuItem::with_id(app, "open-settings", "Open Settings", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit Vox", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open_item, &quit_item])?;

    let initial_icon = tauri::image::Image::new_owned(crate::tray_glyphs::idle(32), 32, 32);

    TrayIconBuilder::with_id("vox-tray")
        .tooltip("Vox — hold hotkey to dictate")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .icon(initial_icon)
        .icon_as_template(true)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "open-settings" => show_settings(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| match event {
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                position,
                ..
            } => {
                *rect_store().lock() = Some(position);
                let app = tray.app_handle();
                if let Err(e) = show_popover(app) {
                    log::warn!("failed to show tray popover: {e}; opening settings instead");
                    show_settings(app);
                }
            }
            TrayIconEvent::Enter { position, .. } | TrayIconEvent::Move { position, .. } => {
                *rect_store().lock() = Some(position);
            }
            _ => {}
        })
        .build(app)?;

    tray_icon::init(app);
    Ok(())
}

fn show_settings<R: Runtime>(app: &AppHandle<R>) {
    if let Some(win) = app.get_webview_window("settings") {
        position_settings_on_active_display(&win);
        let _ = win.show();
        let _ = win.set_focus();
    }
    if let Some(win) = app.get_webview_window(POPOVER_LABEL) {
        let _ = win.hide();
    }
}

pub(crate) fn position_settings_on_active_display<R: Runtime>(win: &tauri::WebviewWindow<R>) {
    let Some(icon_pos) = *rect_store().lock() else {
        return;
    };
    let Some(monitor) = monitor_for_position(win, icon_pos) else {
        return;
    };

    let scale = monitor.scale_factor();
    let mx = monitor.position().x as f64 / scale;
    let my = monitor.position().y as f64 / scale;
    let mw = monitor.size().width as f64 / scale;
    let mh = monitor.size().height as f64 / scale;

    // Honour user-resized settings; fall back to the configured initial size.
    let (w, h) = win
        .outer_size()
        .ok()
        .map(|s| (s.width as f64 / scale, s.height as f64 / scale))
        .filter(|(w, h)| *w > 0.0 && *h > 0.0)
        .unwrap_or((660.0, 620.0));

    let x = mx + ((mw - w) / 2.0).max(0.0);
    let y = my + ((mh - h) / 2.0).max(0.0);
    let _ = win.set_position(LogicalPosition::new(x, y));
}

fn monitor_for_position<R: Runtime>(
    win: &tauri::WebviewWindow<R>,
    pos: PhysicalPosition<f64>,
) -> Option<tauri::Monitor> {
    win.available_monitors().ok()?.into_iter().find(|m| {
        let mp = m.position();
        let ms = m.size();
        let (mx, my) = (mp.x as f64, mp.y as f64);
        pos.x >= mx
            && pos.x < mx + ms.width as f64
            && pos.y >= my
            && pos.y < my + ms.height as f64
    })
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

    if win.is_visible().unwrap_or(false) {
        let _ = win.hide();
        return Ok(());
    }

    // Pick the monitor under the icon so multi-display setups land the popover
    // on the correct screen with the correct DPI scale.
    let icon_pos = *rect_store().lock();
    let monitor = icon_pos
        .and_then(|p| monitor_for_position(&win, p))
        .or_else(|| win.primary_monitor().ok().flatten());

    if let Some(monitor) = monitor {
        let scale = monitor.scale_factor();
        let mw = monitor.size().width as f64 / scale;
        let mx = monitor.position().x as f64 / scale;
        let my = monitor.position().y as f64 / scale;

        let (cx_logical, top_y) = match icon_pos {
            Some(p) => {
                let cx = p.x / scale;
                let cy = p.y / scale;
                (cx, cy + 24.0)
            }
            None => (mx + mw - 32.0, my + 8.0 + 22.0),
        };

        let x = (cx_logical - POPOVER_W / 2.0)
            .min(mx + mw - POPOVER_W - 8.0)
            .max(mx + 8.0);
        let y = top_y.max(my + 8.0 + 22.0);

        let _ = win.set_size(LogicalSize::new(POPOVER_W, POPOVER_H));
        let _ = win.set_position(LogicalPosition::new(x, y));
    }

    win.show()?;
    win.set_focus()?;
    Ok(())
}

