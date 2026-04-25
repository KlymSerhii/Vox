//! Global hotkey registration.
//!
//! Primary: push-to-talk on the user-selected key (default `F13`).
//! Secondary: a cancel hotkey (default `Escape`) that aborts the in-flight
//! session without pasting.
//! Tertiary: a paste-last-transcript hotkey (default `Alt+Shift+V`) that
//! re-pastes the most recent transcript at the cursor.
//! Quaternary: a mode-cycle hotkey (default `Alt+M`).
//!
//! When `hands_free_enabled` is true, the push-to-talk hotkey acts as a
//! toggle (press to start, press again to stop) instead of as
//! press-and-hold. We dispatch from the same shortcut by reading the
//! current pref at press time.
//!
//! ## Fallback chains
//!
//! `tauri-plugin-global-shortcut` returns an `Unknown scancode for X`
//! error on macOS for several keys whose Carbon-era scancode entries are
//! missing from its keymap (notably bare `AltRight`). When that happens
//! the user gets *no* feedback — no pill, no log surfaced in the UI — so
//! we silently fail to bind the dictation hotkey and the entire feature
//! looks broken.
//!
//! To make this self-healing each hotkey now ships with a fallback chain
//! and the FIRST entry that successfully binds becomes the active key.
//! We persist the resolved value back to settings so the picker UI shows
//! the truth and the user isn't confused by the difference between the
//! configured key and the bound key. We also expose the configured
//! vs. bound pair through `hotkey::active_state` (read by the
//! `get_active_hotkey` and `diagnose_hotkeys` IPCs) so the frontend can
//! render an explanatory note like "Configured ⌥ wasn't available on
//! your Mac — Vox is using F13."

use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_global_shortcut::{
    Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutEvent, ShortcutState,
};

use crate::session::{self, SessionCtx};
use crate::settings::{self, HotkeyPurpose};

/// Default registration order when the configured push-to-talk hotkey
/// can't be bound. Same order is used as the search space for the
/// "first registerable" pass.
const PTT_FALLBACKS: &[&str] = &["F13", "F14", "F15", "CommandOrControl+Shift+Space"];
const CANCEL_FALLBACKS: &[&str] = &["Escape"];
const PASTE_LAST_FALLBACKS: &[&str] = &[
    "Alt+Shift+V",
    "CommandOrControl+Shift+V",
    "F14",
    "F15",
];
const MODE_CYCLE_FALLBACKS: &[&str] = &[
    "Alt+M",
    "Alt+Shift+M",
    "CommandOrControl+Shift+M",
    "F15",
];

/// Snapshot of what's actually bound for one hotkey purpose at runtime.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ActiveHotkey {
    /// Configured value the user picked (or migrated to). `""` means
    /// the user explicitly disabled the hotkey ("Off" in the picker).
    pub configured: String,
    /// What's actually bound. Empty when nothing could be bound (either
    /// user-disabled or every fallback failed).
    pub bound: String,
    /// True when the configured value couldn't be bound and we fell
    /// through to a fallback. UIs use this to render an explanation.
    pub fell_back: bool,
    /// Last error from the configured key registration attempt, if any.
    /// `None` when registration succeeded outright.
    pub error: Option<String>,
    /// Per-attempt errors, oldest first. Useful for the diagnostics
    /// panel — lets the user see exactly why each attempt failed.
    pub error_chain: Vec<String>,
}

impl ActiveHotkey {
    fn empty(configured: &str) -> Self {
        Self {
            configured: configured.to_string(),
            bound: String::new(),
            fell_back: false,
            error: None,
            error_chain: Vec::new(),
        }
    }
}

/// Mutable snapshot of the currently bound hotkeys. Stored in Tauri
/// state so IPC handlers can expose it without re-running registration.
#[derive(Debug, Default)]
pub struct ActiveHotkeyState {
    pub push_to_talk: Mutex<ActiveHotkey>,
    pub cancel: Mutex<ActiveHotkey>,
    pub paste_last: Mutex<ActiveHotkey>,
    pub mode_cycle: Mutex<ActiveHotkey>,
}

impl ActiveHotkeyState {
    pub fn snapshot(&self, purpose: HotkeyPurpose) -> ActiveHotkey {
        let guard = match purpose {
            HotkeyPurpose::PushToTalk => self.push_to_talk.lock(),
            HotkeyPurpose::Cancel => self.cancel.lock(),
            HotkeyPurpose::PasteLast => self.paste_last.lock(),
            HotkeyPurpose::ModeCycle => self.mode_cycle.lock(),
        };
        match guard {
            Ok(g) => g.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    fn store(&self, purpose: HotkeyPurpose, active: ActiveHotkey) {
        let cell = match purpose {
            HotkeyPurpose::PushToTalk => &self.push_to_talk,
            HotkeyPurpose::Cancel => &self.cancel,
            HotkeyPurpose::PasteLast => &self.paste_last,
            HotkeyPurpose::ModeCycle => &self.mode_cycle,
        };
        match cell.lock() {
            Ok(mut g) => *g = active,
            Err(poisoned) => *poisoned.into_inner() = active,
        }
    }
}

/// Translate a stable string id into a `Shortcut`. Returns `None` for
/// anything we don't have a mapping for, so callers can decide whether
/// to log + fall back or treat the value as user-disabled (`""`).
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
        "CommandOrControl+Shift+Semicolon" | "Cmd+Shift+;" => Some(Shortcut::new(
            Some(Modifiers::META | Modifiers::SHIFT),
            Code::Semicolon,
        )),
        "Alt+Shift+V" => Some(Shortcut::new(
            Some(Modifiers::ALT | Modifiers::SHIFT),
            Code::KeyV,
        )),
        "Cmd+Shift+V" | "CommandOrControl+Shift+V" => Some(Shortcut::new(
            Some(Modifiers::META | Modifiers::SHIFT),
            Code::KeyV,
        )),
        "Alt+M" => Some(Shortcut::new(Some(Modifiers::ALT), Code::KeyM)),
        "Alt+Shift+M" => Some(Shortcut::new(
            Some(Modifiers::ALT | Modifiers::SHIFT),
            Code::KeyM,
        )),
        "CommandOrControl+Shift+M" | "Cmd+Shift+M" => Some(Shortcut::new(
            Some(Modifiers::META | Modifiers::SHIFT),
            Code::KeyM,
        )),
        _ => None,
    }
}

/// Walk `[configured] + fallbacks` and return the first key id whose
/// `register` callback succeeds. Already-attempted ids are skipped.
/// `register` should perform `gs.on_shortcut(...)` and return the result.
///
/// The returned `Outcome` carries enough information to populate
/// `ActiveHotkey` AND tell the caller whether to persist a migration to
/// the resolved value.
struct Outcome {
    bound: String,
    fell_back: bool,
    primary_error: Option<String>,
    error_chain: Vec<String>,
}

fn try_chain<F>(
    configured: &str,
    fallbacks: &[&str],
    mut register: F,
) -> Result<Outcome, Vec<String>>
where
    F: FnMut(&str, Shortcut) -> Result<(), String>,
{
    let mut tried: Vec<&str> = Vec::with_capacity(1 + fallbacks.len());
    let mut error_chain: Vec<String> = Vec::new();
    let mut primary_error: Option<String> = None;

    let candidates: Vec<&str> = if configured.is_empty() {
        fallbacks.to_vec()
    } else {
        let mut v = vec![configured];
        for f in fallbacks {
            if *f != configured {
                v.push(*f);
            }
        }
        v
    };

    for (idx, key) in candidates.iter().enumerate() {
        if tried.contains(key) {
            continue;
        }
        tried.push(*key);
        let Some(sc) = shortcut_from_key(key) else {
            let msg = format!("'{key}': unrecognised shortcut id");
            if idx == 0 && !configured.is_empty() {
                primary_error = Some(msg.clone());
            }
            error_chain.push(msg);
            continue;
        };
        match register(key, sc) {
            Ok(()) => {
                return Ok(Outcome {
                    bound: (*key).to_string(),
                    fell_back: idx > 0,
                    primary_error,
                    error_chain,
                });
            }
            Err(e) => {
                let msg = format!("'{key}': {e}");
                if idx == 0 && !configured.is_empty() {
                    primary_error = Some(msg.clone());
                }
                error_chain.push(msg);
            }
        }
    }

    Err(error_chain)
}

pub fn register<R: Runtime>(
    app: &AppHandle<R>,
    preferred: &str,
    cancel_preferred: &str,
    paste_last_preferred: &str,
    mode_cycle_preferred: &str,
    ctx: SessionCtx,
) -> Result<String, String> {
    let gs = app.global_shortcut();
    // Clear any existing registrations (safe if none).
    let _ = gs.unregister_all();

    // ---- Push-to-talk -----------------------------------------------------
    //
    // We intentionally pull the SessionCtx-owning closure into its own
    // scope so the borrow checker is happy moving `ctx` into successive
    // `register` attempts within `try_chain`.
    let ptt_outcome = {
        let ctx_outer = ctx.clone();
        try_chain(preferred, PTT_FALLBACKS, |_id, sc| {
            let app_handle = app.clone();
            let ctx_clone = ctx_outer.clone();
            gs.on_shortcut(sc, move |_app, _sc, event: ShortcutEvent| {
                // Cheap read — never touches the Keychain.
                let prefs = settings::prefs(&app_handle);
                if prefs.hands_free_enabled {
                    if let ShortcutState::Pressed = event.state() {
                        if ctx_clone.is_recording() {
                            session::end_recording(app_handle.clone(), ctx_clone.clone());
                        } else {
                            session::begin_recording(app_handle.clone(), &ctx_clone);
                        }
                    }
                } else {
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
            .map_err(|e| e.to_string())
        })
    };

    let ptt_active = match ptt_outcome {
        Ok(out) => {
            if out.fell_back {
                log::error!(
                    "push-to-talk: configured '{preferred}' could not be \
                     bound; using fallback '{}'. Errors: {}",
                    out.bound,
                    out.error_chain.join("; ")
                );
                // Persist the migration so the picker shows the truth.
                if let Err(e) = settings::set_hotkey_field(
                    app,
                    HotkeyPurpose::PushToTalk,
                    &out.bound,
                ) {
                    log::warn!("could not persist resolved push-to-talk hotkey: {e}");
                }
            }
            ActiveHotkey {
                configured: preferred.to_string(),
                bound: out.bound,
                fell_back: out.fell_back,
                error: out.primary_error,
                error_chain: out.error_chain,
            }
        }
        Err(chain) => {
            log::error!(
                "push-to-talk: NO hotkey could be bound. Tried: {}",
                chain.join("; ")
            );
            ActiveHotkey {
                configured: preferred.to_string(),
                bound: String::new(),
                fell_back: false,
                error: chain.first().cloned(),
                error_chain: chain,
            }
        }
    };

    let ptt_bound_sc = shortcut_from_key(&ptt_active.bound);

    // ---- Cancel -----------------------------------------------------------
    let cancel_active = if cancel_preferred.is_empty() {
        ActiveHotkey::empty("")
    } else {
        let outcome = try_chain(cancel_preferred, CANCEL_FALLBACKS, |key, sc| {
            // Don't double-register if it collides with the PTT shortcut
            // we just bound — register would fail with a confusing
            // "already registered" message.
            if Some(sc) == ptt_bound_sc {
                return Err(format!(
                    "'{key}' collides with push-to-talk; not registering"
                ));
            }
            let app_handle = app.clone();
            let ctx_clone = ctx.clone();
            gs.on_shortcut(sc, move |_app, _sc, event: ShortcutEvent| {
                if let ShortcutState::Pressed = event.state() {
                    session::cancel(app_handle.clone(), ctx_clone.clone());
                }
            })
            .map_err(|e| e.to_string())
        });
        match outcome {
            Ok(out) => {
                if out.fell_back {
                    log::error!(
                        "cancel: configured '{cancel_preferred}' could not \
                         be bound; using fallback '{}'. Errors: {}",
                        out.bound,
                        out.error_chain.join("; ")
                    );
                    if let Err(e) = settings::set_hotkey_field(
                        app,
                        HotkeyPurpose::Cancel,
                        &out.bound,
                    ) {
                        log::warn!("could not persist resolved cancel hotkey: {e}");
                    }
                }
                ActiveHotkey {
                    configured: cancel_preferred.to_string(),
                    bound: out.bound,
                    fell_back: out.fell_back,
                    error: out.primary_error,
                    error_chain: out.error_chain,
                }
            }
            Err(chain) => {
                log::error!(
                    "cancel: NO hotkey could be bound. Tried: {}",
                    chain.join("; ")
                );
                ActiveHotkey {
                    configured: cancel_preferred.to_string(),
                    bound: String::new(),
                    fell_back: false,
                    error: chain.first().cloned(),
                    error_chain: chain,
                }
            }
        }
    };

    let cancel_bound_sc = shortcut_from_key(&cancel_active.bound);

    // ---- Paste last -------------------------------------------------------
    let paste_active = if paste_last_preferred.is_empty() {
        ActiveHotkey::empty("")
    } else {
        let outcome = try_chain(paste_last_preferred, PASTE_LAST_FALLBACKS, |key, sc| {
            if Some(sc) == ptt_bound_sc {
                return Err(format!(
                    "'{key}' collides with push-to-talk; not registering"
                ));
            }
            if Some(sc) == cancel_bound_sc {
                return Err(format!(
                    "'{key}' collides with cancel; not registering"
                ));
            }
            let app_handle = app.clone();
            gs.on_shortcut(sc, move |_app, _sc, event: ShortcutEvent| {
                if let ShortcutState::Pressed = event.state() {
                    log::error!(
                        "hotkey: paste-last fired; dispatching to \
                         commands::paste_last_transcript"
                    );
                    if let Err(e) =
                        crate::commands::paste_last_transcript(app_handle.clone())
                    {
                        log::error!("paste_last_transcript dispatch failed: {e}");
                    }
                }
            })
            .map_err(|e| e.to_string())
        });
        match outcome {
            Ok(out) => {
                if out.fell_back {
                    log::error!(
                        "paste-last: configured '{paste_last_preferred}' \
                         could not be bound; using fallback '{}'. \
                         Errors: {}",
                        out.bound,
                        out.error_chain.join("; ")
                    );
                    if let Err(e) = settings::set_hotkey_field(
                        app,
                        HotkeyPurpose::PasteLast,
                        &out.bound,
                    ) {
                        log::warn!("could not persist resolved paste-last hotkey: {e}");
                    }
                }
                ActiveHotkey {
                    configured: paste_last_preferred.to_string(),
                    bound: out.bound,
                    fell_back: out.fell_back,
                    error: out.primary_error,
                    error_chain: out.error_chain,
                }
            }
            Err(chain) => {
                log::error!(
                    "paste-last: NO hotkey could be bound. Tried: {}",
                    chain.join("; ")
                );
                ActiveHotkey {
                    configured: paste_last_preferred.to_string(),
                    bound: String::new(),
                    fell_back: false,
                    error: chain.first().cloned(),
                    error_chain: chain,
                }
            }
        }
    };

    let paste_bound_sc = shortcut_from_key(&paste_active.bound);

    // ---- Mode cycle -------------------------------------------------------
    let mode_active = if mode_cycle_preferred.is_empty() {
        ActiveHotkey::empty("")
    } else {
        let outcome = try_chain(mode_cycle_preferred, MODE_CYCLE_FALLBACKS, |key, sc| {
            if Some(sc) == ptt_bound_sc {
                return Err(format!(
                    "'{key}' collides with push-to-talk; not registering"
                ));
            }
            if Some(sc) == cancel_bound_sc {
                return Err(format!(
                    "'{key}' collides with cancel; not registering"
                ));
            }
            if Some(sc) == paste_bound_sc {
                return Err(format!(
                    "'{key}' collides with paste-last; not registering"
                ));
            }
            let app_handle = app.clone();
            gs.on_shortcut(sc, move |_app, _sc, event: ShortcutEvent| {
                if let ShortcutState::Pressed = event.state() {
                    spawn_cycle_active_mode(app_handle.clone());
                }
            })
            .map_err(|e| e.to_string())
        });
        match outcome {
            Ok(out) => {
                if out.fell_back {
                    log::error!(
                        "mode-cycle: configured '{mode_cycle_preferred}' \
                         could not be bound; using fallback '{}'. \
                         Errors: {}",
                        out.bound,
                        out.error_chain.join("; ")
                    );
                    if let Err(e) = settings::set_hotkey_field(
                        app,
                        HotkeyPurpose::ModeCycle,
                        &out.bound,
                    ) {
                        log::warn!("could not persist resolved mode-cycle hotkey: {e}");
                    }
                }
                ActiveHotkey {
                    configured: mode_cycle_preferred.to_string(),
                    bound: out.bound,
                    fell_back: out.fell_back,
                    error: out.primary_error,
                    error_chain: out.error_chain,
                }
            }
            Err(chain) => {
                log::error!(
                    "mode-cycle: NO hotkey could be bound. Tried: {}",
                    chain.join("; ")
                );
                ActiveHotkey {
                    configured: mode_cycle_preferred.to_string(),
                    bound: String::new(),
                    fell_back: false,
                    error: chain.first().cloned(),
                    error_chain: chain,
                }
            }
        }
    };

    // ---- Persist active state into Tauri state for the IPC layer ---------
    if let Some(state) = app.try_state::<ActiveHotkeyState>() {
        state.store(HotkeyPurpose::PushToTalk, ptt_active.clone());
        state.store(HotkeyPurpose::Cancel, cancel_active.clone());
        state.store(HotkeyPurpose::PasteLast, paste_active.clone());
        state.store(HotkeyPurpose::ModeCycle, mode_active.clone());
    } else {
        let new_state = ActiveHotkeyState::default();
        new_state.store(HotkeyPurpose::PushToTalk, ptt_active.clone());
        new_state.store(HotkeyPurpose::Cancel, cancel_active.clone());
        new_state.store(HotkeyPurpose::PasteLast, paste_active.clone());
        new_state.store(HotkeyPurpose::ModeCycle, mode_active.clone());
        app.manage(new_state);
    }

    // Single info-level summary so the user can see what's actually
    // bound at startup without grep'ing through warnings.
    log::info!(
        "hotkey: push-to-talk={} (configured {}), cancel={} (configured {}), \
         paste-last={} (configured {}), mode-cycle={} (configured {})",
        display_bound(&ptt_active),
        nonempty(&ptt_active.configured),
        display_bound(&cancel_active),
        nonempty(&cancel_active.configured),
        display_bound(&paste_active),
        nonempty(&paste_active.configured),
        display_bound(&mode_active),
        nonempty(&mode_active.configured),
    );

    if ptt_active.bound.is_empty() {
        return Err(format!(
            "could not bind any push-to-talk hotkey: {}",
            ptt_active
                .error_chain
                .last()
                .cloned()
                .unwrap_or_else(|| "no fallbacks left".into())
        ));
    }

    Ok(ptt_active.bound)
}

fn display_bound(a: &ActiveHotkey) -> String {
    if a.bound.is_empty() {
        "<unbound>".to_string()
    } else {
        a.bound.clone()
    }
}

fn nonempty(s: &str) -> &str {
    if s.is_empty() {
        "<off>"
    } else {
        s
    }
}

fn spawn_cycle_active_mode<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn_blocking(move || {
        let prefs = settings::prefs(&app);
        let next = match crate::modes::cycle_active(&app, &prefs.active_mode_id) {
            Some(m) => m,
            None => return,
        };
        if let Err(e) = settings::set_active_mode_id(&app, &next.id) {
            log::warn!("failed to persist mode cycle: {e}");
            return;
        }
        // 800 ms HUD on the pill so the user sees the new mode.
        crate::pill::show(&app);
        crate::pill::emit_active_mode(&app, &next);
        let app_clone = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(800)).await;
            // Only auto-hide if no recording started in the meantime.
            if let Some(ctx) = app_clone.try_state::<SessionCtx>() {
                if ctx.is_recording() || ctx.is_pipeline_active() {
                    return;
                }
            }
            crate::pill::hide(&app_clone);
        });
    });
}
