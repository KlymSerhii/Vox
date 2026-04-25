//! Global hotkey registration.
//!
//! Primary: push-to-talk on the user-selected key (default `AltRight`).
//! Secondary: a cancel hotkey (default `Escape`) that aborts the in-flight
//! session without pasting.

use tauri::{AppHandle, Runtime};
use tauri_plugin_global_shortcut::{
    Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutEvent, ShortcutState,
};

use crate::session::{self, SessionCtx};

pub fn resolve_shortcut(preferred: &str) -> (Shortcut, String) {
    if let Some(s) = shortcut_from_key(preferred) {
        return (s, preferred.to_string());
    }
    log::warn!("hotkey '{preferred}' not registerable; falling back to Cmd+Shift+Space");
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
        "Escape" | "Esc" => Some(Shortcut::new(None, Code::Escape)),
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
    cancel_preferred: &str,
    ctx: SessionCtx,
) -> Result<String, String> {
    let (shortcut, resolved) = resolve_shortcut(preferred);
    let gs = app.global_shortcut();
    // Clear any existing registrations (safe if none).
    let _ = gs.unregister_all();

    // Push-to-talk.
    {
        let app_handle = app.clone();
        let ctx_clone = ctx.clone();
        gs.on_shortcut(shortcut, move |_app, _sc, event: ShortcutEvent| {
            match event.state() {
                ShortcutState::Pressed => {
                    session::begin_recording(app_handle.clone(), &ctx_clone)
                }
                ShortcutState::Released => {
                    session::end_recording(app_handle.clone(), ctx_clone.clone())
                }
            }
        })
        .map_err(|e| e.to_string())?;
    }

    // Cancel (only has an effect while recording).
    if let Some(cancel_sc) = shortcut_from_key(cancel_preferred) {
        // Don't double-register if the user set cancel == PTT.
        if cancel_sc != shortcut {
            let app_handle = app.clone();
            let ctx_clone = ctx.clone();
            if let Err(e) = gs.on_shortcut(cancel_sc, move |_app, _sc, event: ShortcutEvent| {
                if let ShortcutState::Pressed = event.state() {
                    if ctx_clone.is_recording() {
                        session::cancel(app_handle.clone(), ctx_clone.clone());
                    }
                }
            }) {
                log::warn!("could not register cancel hotkey '{cancel_preferred}': {e}");
            }
        }
    } else {
        log::warn!("cancel hotkey '{cancel_preferred}' not recognized");
    }

    log::info!("global hotkey registered: {}", resolved);
    Ok(resolved)
}
