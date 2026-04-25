//! Soft local-first migration dialog.
//!
//! Shown once on app startup if the user has an existing `groq_api_key`
//! and `migration_seen == false`. Two outcomes:
//!
//! 1. "Switch to local models" — flips engines to `whisper-local` /
//!    `gemma-local` and kicks off the model downloads inside the same
//!    window via the existing `download_model` command.
//! 2. "Keep using cloud" — locks in `groq` / `anthropic` (and `none`
//!    when no Anthropic key is present) and dismisses.
//!
//! Both branches set `migration_seen = true` so the dialog never
//! re-appears.

use tauri::{AppHandle, Manager, Runtime, WebviewUrl};

pub const LABEL: &str = "migration";
const WIDTH: f64 = 520.0;
const HEIGHT: f64 = 460.0;

pub fn show<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    if let Some(win) = app.get_webview_window(LABEL) {
        win.show()?;
        win.set_focus()?;
        return Ok(());
    }

    tauri::WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("migration.html".into()))
        .title("Vox — Now Offline by Default")
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
