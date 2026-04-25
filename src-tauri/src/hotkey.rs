//! Global hotkey registration.
//!
//! Primary: push-to-talk on the user-selected key (default `AltRight`).
//! Secondary: a cancel hotkey (default `Escape`) that aborts the in-flight
//! session without pasting.
//! Tertiary: a paste-last-transcript hotkey (default `Alt+Shift+V`) that
//! re-pastes the most recent transcript at the cursor.
//!
//! When `hands_free_enabled` is true, the push-to-talk hotkey acts as a
//! toggle (press to start, press again to stop) instead of as
//! press-and-hold. We dispatch from the same shortcut by reading the
//! current pref at press time.

use tauri::{AppHandle, Runtime};
use tauri_plugin_global_shortcut::{
    Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutEvent, ShortcutState,
};

use crate::session::{self, SessionCtx};
use crate::settings;

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
        "Alt+Shift+V" => Some(Shortcut::new(
            Some(Modifiers::ALT | Modifiers::SHIFT),
            Code::KeyV,
        )),
        "Cmd+Shift+V" | "CommandOrControl+Shift+V" => Some(Shortcut::new(
            Some(Modifiers::META | Modifiers::SHIFT),
            Code::KeyV,
        )),
        _ => None,
    }
}

pub fn register<R: Runtime>(
    app: &AppHandle<R>,
    preferred: &str,
    cancel_preferred: &str,
    paste_last_preferred: &str,
    ctx: SessionCtx,
) -> Result<String, String> {
    let (shortcut, resolved) = resolve_shortcut(preferred);
    let gs = app.global_shortcut();
    // Clear any existing registrations (safe if none).
    let _ = gs.unregister_all();

    // Push-to-talk *or* hands-free toggle — we read the user's pref at press
    // time so flipping the toggle in Settings takes effect immediately
    // without a hotkey re-bind.
    {
        let app_handle = app.clone();
        let ctx_clone = ctx.clone();
        gs.on_shortcut(shortcut, move |_app, _sc, event: ShortcutEvent| {
            // Cheap read — never touches the Keychain.
            let prefs = settings::prefs(&app_handle);
            if prefs.hands_free_enabled {
                // Toggle behavior: a press starts a session; a press during
                // an active session stops it. We don't react to releases.
                if let ShortcutState::Pressed = event.state() {
                    if ctx_clone.is_recording() {
                        // Stop and run pipeline.
                        session::end_recording(app_handle.clone(), ctx_clone.clone());
                    } else {
                        session::begin_recording(app_handle.clone(), &ctx_clone);
                    }
                }
            } else {
                // Push-to-talk: press starts, release ends.
                match event.state() {
                    ShortcutState::Pressed => {
                        session::begin_recording(app_handle.clone(), &ctx_clone)
                    }
                    ShortcutState::Released => {
                        session::end_recording(app_handle.clone(), ctx_clone.clone())
                    }
                }
            }
        })
        .map_err(|e| e.to_string())?;
    }

    // Cancel (PRD §4.2: must abort the *entire* operation — recording,
    // transcribing, or polishing — not just the recording phase). The
    // gating is delegated to `session::cancel`, which races the in-flight
    // pipeline via `wait_for_cancel`. Gating here would re-introduce the
    // exact bug the staff review C2 was meant to fix.
    if let Some(cancel_sc) = shortcut_from_key(cancel_preferred) {
        // Don't double-register if the user set cancel == PTT.
        if cancel_sc != shortcut {
            let app_handle = app.clone();
            let ctx_clone = ctx.clone();
            if let Err(e) = gs.on_shortcut(cancel_sc, move |_app, _sc, event: ShortcutEvent| {
                if let ShortcutState::Pressed = event.state() {
                    session::cancel(app_handle.clone(), ctx_clone.clone());
                }
            }) {
                log::warn!("could not register cancel hotkey '{cancel_preferred}': {e}");
            }
        }
    } else {
        log::warn!("cancel hotkey '{cancel_preferred}' not recognized");
    }

    // Paste-last-transcript (PRD §4.4). Independent from the PTT/cancel
    // pair. Skip if the user set it to the same combo as PTT or cancel.
    if !paste_last_preferred.is_empty() {
        if let Some(paste_sc) = shortcut_from_key(paste_last_preferred) {
            if paste_sc != shortcut
                && shortcut_from_key(cancel_preferred)
                    .map(|s| s != paste_sc)
                    .unwrap_or(true)
            {
                let app_handle = app.clone();
                if let Err(e) = gs.on_shortcut(
                    paste_sc,
                    move |_app, _sc, event: ShortcutEvent| {
                        if let ShortcutState::Pressed = event.state() {
                            // The command-style helper in commands.rs handles
                            // the empty-buffer + already-pasting cases.
                            let _ =
                                crate::commands::paste_last_transcript(app_handle.clone());
                        }
                    },
                ) {
                    log::warn!(
                        "could not register paste-last hotkey '{paste_last_preferred}': {e}"
                    );
                }
            }
        } else {
            log::warn!("paste-last hotkey '{paste_last_preferred}' not recognized");
        }
    }

    log::info!("global hotkey registered: {}", resolved);
    Ok(resolved)
}
