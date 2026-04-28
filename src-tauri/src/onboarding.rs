use tauri::{AppHandle, Manager, Runtime, WebviewUrl};

pub const LABEL: &str = "onboarding";
const WIDTH: f64 = 560.0;
const HEIGHT: f64 = 520.0;

pub fn show<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    if let Some(win) = app.get_webview_window(LABEL) {
        win.show()?;
        win.set_focus()?;
        return Ok(());
    }

    tauri::WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("onboarding.html".into()))
        .title("Welcome to Vox")
        .inner_size(WIDTH, HEIGHT)
        .resizable(false)
        .maximizable(false)
        .decorations(true)
        .center()
        .focused(true)
        .visible(true)
        .build()?;

    Ok(())
}
