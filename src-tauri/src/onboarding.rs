use tauri::{AppHandle, LogicalPosition, Manager, Runtime, WebviewUrl};

pub const LABEL: &str = "onboarding";
const WIDTH: f64 = 560.0;
// Tall enough to fit the models step's combined Whisper + Gemma model
// pickers plus the polish-engine cards without forcing the user to
// scroll on the common path. DownloadStep's container also falls back
// to overflow-y scrolling on small displays.
const HEIGHT: f64 = 640.0;

pub fn show<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    if let Some(win) = app.get_webview_window(LABEL) {
        // Re-centre on the display the user is on now (they may have
        // moved since the wizard was first opened, e.g. "Replay
        // onboarding").
        center_on_active_display(&win);
        win.show()?;
        win.set_focus()?;
        return Ok(());
    }

    // Build hidden, place it on the active display, THEN show — so the
    // window never flashes centred on the primary before hopping over.
    // The builder's own `.center()` only ever targets the primary
    // monitor, which is the bug this replaces.
    let win = tauri::WebviewWindowBuilder::new(
        app,
        LABEL,
        WebviewUrl::App("onboarding.html".into()),
    )
    .title("Welcome to Vox")
    .inner_size(WIDTH, HEIGHT)
    .resizable(false)
    .maximizable(false)
    .decorations(true)
    .focused(true)
    .visible(false)
    .build()?;

    center_on_active_display(&win);
    win.show()?;
    win.set_focus()?;

    Ok(())
}

/// Centre the wizard on the display the user is currently on (the one
/// holding the cursor), reusing the pill's shared active-display probe so
/// both surfaces agree on "active display." The origin is clamped so the
/// window's top-left never leaves the display — a wizard taller or wider
/// than a small external monitor must keep its title bar reachable, and
/// it's non-resizable. Falls back to Tauri's primary-monitor `center()`
/// if the active frame can't be resolved or positioning fails.
fn center_on_active_display<R: Runtime>(win: &tauri::WebviewWindow<R>) {
    if let Some((fx, fy, fw, fh)) = crate::pill::active_screen_logical_frame(win) {
        let x = (fx + (fw - WIDTH) / 2.0).max(fx);
        let y = (fy + (fh - HEIGHT) / 2.0).max(fy);
        if win.set_position(LogicalPosition::new(x, y)).is_ok() {
            return;
        }
    }
    let _ = win.center();
}
