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

#[cfg(target_os = "macos")]
use crate::mac_chord::{self, ChordHandle};
use crate::session::{self, SessionCtx};
use crate::settings::{self, HotkeyPurpose};

/// Default registration order when the configured push-to-talk hotkey
/// can't be bound. Same order is used as the search space for the
/// "first registerable" pass. The default chord (`Alt+Control`) is the
/// first entry; the F13 chain is the standard fallback when the chord
/// can't be installed (e.g. Accessibility revoked).
const PTT_FALLBACKS: &[&str] = &[
    "Alt+Control",
    "F13",
    "F14",
    "F15",
    "CommandOrControl+Shift+Space",
];
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
    /// Which underlying mechanism is providing the binding:
    ///  - `"shortcut"` — `tauri-plugin-global-shortcut` (Carbon
    ///    `RegisterEventHotKey` on macOS)
    ///  - `"chord"` — modifier-only via our `CGEventTap` (whispr-flow
    ///    UX)
    ///  - `""` — nothing bound
    pub mechanism: String,
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
            mechanism: String::new(),
            error: None,
            error_chain: Vec::new(),
        }
    }
}

/// Mutable snapshot of the currently bound hotkeys. Stored in Tauri
/// state so IPC handlers can expose it without re-running registration.
#[derive(Default)]
pub struct ActiveHotkeyState {
    pub push_to_talk: Mutex<ActiveHotkey>,
    pub cancel: Mutex<ActiveHotkey>,
    pub paste_last: Mutex<ActiveHotkey>,
    pub mode_cycle: Mutex<ActiveHotkey>,
    /// Owns the lifetime of the `CGEventTap`-backed chord listener (if
    /// any). On re-`register()` we drop the prior handle BEFORE
    /// installing the new one to avoid two taps competing.
    #[cfg(target_os = "macos")]
    pub chord: Mutex<Option<ChordHandle>>,
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

    // Drop any existing chord listener BEFORE we try to install a new
    // one, so two taps never run in parallel after a re-register.
    #[cfg(target_os = "macos")]
    if let Some(state) = app.try_state::<ActiveHotkeyState>() {
        if let Ok(mut g) = state.chord.lock() {
            *g = None;
        }
    }

    // ---- Push-to-talk -----------------------------------------------------
    //
    // For the push-to-talk slot we walk a preference chain that tries
    // the chord listener (CGEventTap, whispr-flow UX) first, then falls
    // back to the standard `tauri-plugin-global-shortcut` path. Cancel
    // / paste-last / mode-cycle stay on the regular plugin — they're
    // discrete events, not press+release, and modifier-only chords
    // would conflict with normal typing there.
    #[cfg(target_os = "macos")]
    {
        // Surface the current Accessibility status. We can't easily
        // detect it being revoked while we're running, so logging on
        // every call is the cheapest signal we can give a user staring
        // at `tauri dev`'s terminal.
        // SAFETY: AXIsProcessTrusted is a non-prompting check.
        extern "C" {
            fn AXIsProcessTrusted() -> bool;
        }
        let trusted = unsafe { AXIsProcessTrusted() };
        log::info!("hotkey: AXIsProcessTrusted={trusted}");
    }

    let ptt_active: ActiveHotkey = register_push_to_talk(app, preferred, gs, ctx.clone());
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
                    mechanism: "shortcut".to_string(),
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
                    mechanism: String::new(),
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
                    mechanism: "shortcut".to_string(),
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
                    mechanism: String::new(),
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
                    mechanism: "shortcut".to_string(),
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
                    mechanism: String::new(),
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
    // bound at startup without grep'ing through warnings. Includes the
    // mechanism (chord vs shortcut) for the push-to-talk binding.
    log::info!(
        "hotkey: push-to-talk={} mechanism={} (configured {}), \
         cancel={} (configured {}), paste-last={} (configured {}), \
         mode-cycle={} (configured {})",
        display_bound(&ptt_active),
        if ptt_active.mechanism.is_empty() {
            "<none>"
        } else {
            ptt_active.mechanism.as_str()
        },
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

/// Register the push-to-talk hotkey, walking the chord-or-shortcut
/// preference chain and falling back through the `PTT_FALLBACKS` list.
///
/// On macOS we try modifier-only chords first via `mac_chord::install`
/// (a `CGEventTap` listener) before reaching for
/// `tauri-plugin-global-shortcut`. On other platforms we go directly to
/// the standard plugin path.
fn register_push_to_talk<R: Runtime>(
    app: &AppHandle<R>,
    preferred: &str,
    gs: &tauri_plugin_global_shortcut::GlobalShortcut<R>,
    ctx: SessionCtx,
) -> ActiveHotkey {
    // Build candidate list: configured first, then fallbacks.
    let mut candidates: Vec<String> = Vec::with_capacity(1 + PTT_FALLBACKS.len());
    if !preferred.is_empty() {
        candidates.push(preferred.to_string());
    }
    for f in PTT_FALLBACKS {
        if !candidates.iter().any(|c| c == f) {
            candidates.push((*f).to_string());
        }
    }

    let mut error_chain: Vec<String> = Vec::new();
    let mut primary_error: Option<String> = None;

    for (idx, key) in candidates.iter().enumerate() {
        let is_primary = idx == 0 && !preferred.is_empty();
        // Try chord first (macOS only) — falls through to shortcut
        // when the spec doesn't parse as a pure chord.
        #[cfg(target_os = "macos")]
        if let Some(spec) = mac_chord::try_parse_chord(key) {
            match install_chord(app, spec, ctx.clone()) {
                Ok(()) => {
                    let fell_back = idx > 0;
                    if fell_back {
                        log::error!(
                            "push-to-talk: configured '{preferred}' could not \
                             be bound; using fallback chord '{key}'. Errors: {}",
                            error_chain.join("; ")
                        );
                        if let Err(e) = settings::set_hotkey_field(
                            app,
                            HotkeyPurpose::PushToTalk,
                            key,
                        ) {
                            log::warn!(
                                "could not persist resolved push-to-talk hotkey: {e}"
                            );
                        }
                    }
                    return ActiveHotkey {
                        configured: preferred.to_string(),
                        bound: key.clone(),
                        fell_back,
                        mechanism: "chord".to_string(),
                        error: primary_error,
                        error_chain,
                    };
                }
                Err(e) => {
                    let msg = format!("'{key}' (chord): {e}");
                    if is_primary {
                        primary_error = Some(msg.clone());
                    }
                    error_chain.push(msg);
                    continue;
                }
            }
        }

        // Standard `tauri-plugin-global-shortcut` path.
        let Some(sc) = shortcut_from_key(key) else {
            let msg = format!("'{key}': unrecognised shortcut id");
            if is_primary {
                primary_error = Some(msg.clone());
            }
            error_chain.push(msg);
            continue;
        };
        let app_handle = app.clone();
        let ctx_clone = ctx.clone();
        let r = gs.on_shortcut(sc, move |_app, _sc, event: ShortcutEvent| {
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
        });
        match r {
            Ok(()) => {
                let fell_back = idx > 0;
                if fell_back {
                    log::error!(
                        "push-to-talk: configured '{preferred}' could not be \
                         bound; using fallback '{key}'. Errors: {}",
                        error_chain.join("; ")
                    );
                    if let Err(e) = settings::set_hotkey_field(
                        app,
                        HotkeyPurpose::PushToTalk,
                        key,
                    ) {
                        log::warn!(
                            "could not persist resolved push-to-talk hotkey: {e}"
                        );
                    }
                }
                return ActiveHotkey {
                    configured: preferred.to_string(),
                    bound: key.clone(),
                    fell_back,
                    mechanism: "shortcut".to_string(),
                    error: primary_error,
                    error_chain,
                };
            }
            Err(e) => {
                let msg = format!("'{key}': {e}");
                if is_primary {
                    primary_error = Some(msg.clone());
                }
                error_chain.push(msg);
            }
        }
    }

    log::error!(
        "push-to-talk: NO hotkey could be bound. Tried: {}",
        error_chain.join("; ")
    );
    ActiveHotkey {
        configured: preferred.to_string(),
        bound: String::new(),
        fell_back: false,
        mechanism: String::new(),
        error: primary_error.or_else(|| error_chain.first().cloned()),
        error_chain,
    }
}

#[cfg(target_os = "macos")]
fn install_chord<R: Runtime>(
    app: &AppHandle<R>,
    spec: mac_chord::ChordSpec,
    ctx: SessionCtx,
) -> Result<(), String> {
    let app_press = app.clone();
    let ctx_press = ctx.clone();
    let app_release = app.clone();
    let ctx_release = ctx.clone();
    let handle = mac_chord::install(
        spec,
        move || {
            let prefs = settings::prefs(&app_press);
            if prefs.hands_free_enabled {
                // Hands-free: chord-press toggles record state.
                if ctx_press.is_recording() {
                    session::end_recording(app_press.clone(), ctx_press.clone());
                } else {
                    session::begin_recording(app_press.clone(), &ctx_press);
                }
            } else {
                session::begin_recording(app_press.clone(), &ctx_press);
            }
        },
        move || {
            let prefs = settings::prefs(&app_release);
            if !prefs.hands_free_enabled {
                session::end_recording(app_release.clone(), ctx_release.clone());
            }
        },
    )
    .map_err(|e| e.to_string())?;

    // Stash the handle on the active state so it stays alive for the
    // app's lifetime. We `manage` the state lazily if the host hasn't
    // installed it yet (matches `register()`'s symmetric path).
    if let Some(state) = app.try_state::<ActiveHotkeyState>() {
        if let Ok(mut g) = state.chord.lock() {
            *g = Some(handle);
        }
    } else {
        let new_state = ActiveHotkeyState::default();
        if let Ok(mut g) = new_state.chord.lock() {
            *g = Some(handle);
        }
        app.manage(new_state);
    }
    Ok(())
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
