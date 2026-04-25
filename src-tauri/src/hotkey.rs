//! Global hotkey registration.
//!
//! We prefer push-to-talk on Right-Option (`AltRight`). If that specific code
//! cannot be parsed by `tauri-plugin-global-shortcut` on the current host, we
//! fall back to a chord like `CommandOrControl+Shift+Space`.

use tauri::{AppHandle, Runtime};
use tauri_plugin_global_shortcut::{
    Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutEvent, ShortcutState,
};

use crate::session::{self, SessionCtx};

/// Try to resolve the user-preferred hotkey string into a Shortcut. On failure,
/// fall back to Cmd+Shift+Space.
pub fn resolve_shortcut(preferred: &str) -> (Shortcut, String) {
    if let Some(s) = shortcut_from_key(preferred) {
        return (s, preferred.to_string());
    }
    log::warn!(
        "hotkey '{preferred}' not registerable; falling back to Cmd+Shift+Space"
    );
    (
        Shortcut::new(Some(Modifiers::META | Modifiers::SHIFT), Code::Space),
        "CommandOrControl+Shift+Space".to_string(),
    )
}

fn shortcut_from_key(id: &str) -> Option<Shortcut> {
    match id {
        "AltRight" => Some(Shortcut::new(None, Code::AltRight)),
        "AltLeft" => Some(Shortcut::new(None, Code::AltLeft)),
        "F13" => Some(Shortcut::new(None, Code::F13)),
        "F14" => Some(Shortcut::new(None, Code::F14)),
        "F15" => Some(Shortcut::new(None, Code::F15)),
        "CommandOrControl+Shift+Space" | "Cmd+Shift+Space" => Some(Shortcut::new(
            Some(Modifiers::META | Modifiers::SHIFT),
            Code::Space,
        )),
        _ => None,
    }
}

pub fn register<R: Runtime>(
    app: &AppHandle<R>,
    preferred: &str,
    ctx: SessionCtx,
) -> Result<String, String> {
    let (shortcut, resolved) = resolve_shortcut(preferred);
    let gs = app.global_shortcut();
    // Clear any existing registrations (safe if none).
    let _ = gs.unregister_all();

    let app_handle = app.clone();
    let ctx_clone = ctx.clone();
    gs.on_shortcut(shortcut, move |_app, _sc, event: ShortcutEvent| {
        match event.state() {
            ShortcutState::Pressed => session::begin_recording(&ctx_clone),
            ShortcutState::Released => {
                session::end_recording(app_handle.clone(), ctx_clone.clone())
            }
        }
    })
    .map_err(|e| e.to_string())?;

    log::info!("global hotkey registered: {}", resolved);
    Ok(resolved)
}
