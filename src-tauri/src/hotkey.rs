//! Global hotkey registration.
//!
//! Primary: push-to-talk on the user-selected key (default `F13`).
//! Secondary: a cancel hotkey (default `Escape`) that aborts the in-flight
//! session without pasting.
//! Tertiary: a mode-cycle hotkey (default `Alt+M`).
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
//! Behavior:
//!
//! 1. **Push-to-talk: chord configurations never auto-fall-back to a
//!    key.** When the user has a modifier-only chord configured (e.g.
//!    the default `Alt+Control`) and the chord install fails — typically
//!    because Accessibility hasn't been granted — we return a clear
//!    error to the UI. We do NOT silently substitute F13 or any other
//!    key, because the user might not have F13 (recent Apple keyboards
//!    don't), and "looks-like-it's-working but the wrong key" is a
//!    worse failure than "nothing works and the UI tells you why".
//! 2. **Push-to-talk: explicit-key configurations may fall back through
//!    other key options.** If the user has manually picked F13 / F14 /
//!    F15 / Cmd+Shift+Space etc. and that key fails to bind, we walk
//!    `KEY_PTT_FALLBACKS` looking for the next bindable explicit-key
//!    option. This is opt-in by virtue of the user choosing a key in
//!    the picker.
//! 3. **Cancel / mode-cycle** keep the same behavior they had before —
//!    they're discrete events, not press+release, and a chord doesn't
//!    make sense for them.
//!
//! When push-to-talk binding fails we surface the error string through
//! `ActiveHotkey.error` so the UI can render "Chord requires
//! Accessibility permission. Open System Settings → Privacy & Security
//! → Accessibility and add Vox." on the picker row.

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

/// Push-to-talk fallbacks that are ONLY consulted when the user has
/// explicitly chosen a key-based shortcut (not a chord) AND that
/// specific key fails to bind. Chord configurations never fall back to
/// a key — see the file-level docs.
///
/// `F13`/`F14`/`F15` stay in the list because users with a full-size
/// keyboard sometimes pick them deliberately, and if their chosen
/// fn-row key fails to bind it's reasonable to try the next one. They
/// are NEVER reached when the user has `Alt+Control` (or any chord)
/// configured.
const KEY_PTT_FALLBACKS: &[&str] = &[
    "CommandOrControl+Shift+Space",
    "CommandOrControl+Shift+Semicolon",
    "F13",
    "F14",
    "F15",
];
const CANCEL_FALLBACKS: &[&str] = &["Escape"];
const MODE_CYCLE_FALLBACKS: &[&str] = &[
    "Alt+M",
    "Alt+Shift+M",
    "CommandOrControl+Shift+M",
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
    // / mode-cycle stay on the regular plugin — they're discrete events,
    // not press+release, and modifier-only chords would conflict with
    // normal typing there.
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
        state.store(HotkeyPurpose::ModeCycle, mode_active.clone());
    } else {
        let new_state = ActiveHotkeyState::default();
        new_state.store(HotkeyPurpose::PushToTalk, ptt_active.clone());
        new_state.store(HotkeyPurpose::Cancel, cancel_active.clone());
        new_state.store(HotkeyPurpose::ModeCycle, mode_active.clone());
        app.manage(new_state);
    }

    // Single info-level summary so the user can see what's actually
    // bound at startup without grep'ing through warnings. Includes the
    // mechanism (chord vs shortcut) for the push-to-talk binding.
    log::info!(
        "hotkey: push-to-talk={} mechanism={} (configured {}), \
         cancel={} (configured {}), mode-cycle={} (configured {})",
        display_bound(&ptt_active),
        if ptt_active.mechanism.is_empty() {
            "<none>"
        } else {
            ptt_active.mechanism.as_str()
        },
        nonempty(&ptt_active.configured),
        display_bound(&cancel_active),
        nonempty(&cancel_active.configured),
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

/// Register the push-to-talk hotkey.
///
/// Two distinct code paths, decided by whether the user's configured
/// value parses as a modifier-only chord:
///
/// * **Chord configuration** (`Alt+Control`, `Alt+Shift`, ...): we try
///   to install the `CGEventTap` listener via `mac_chord::install`. If
///   that fails (most often because Accessibility hasn't been granted),
///   we surface the error verbatim through `ActiveHotkey.error` and do
///   NOT auto-fall-back to a key. The user has signalled — by picking
///   a chord — that they want chord behaviour; substituting a key
///   silently would be confusing and they may not even have the
///   substitute key on their keyboard.
/// * **Key configuration** (`F13`, `Cmd+Shift+Space`, ...): standard
///   `tauri-plugin-global-shortcut` path with the `KEY_PTT_FALLBACKS`
///   chain.
///
/// On non-macOS targets the chord path is unavailable; we always go
/// straight to the shortcut plugin.
fn register_push_to_talk<R: Runtime>(
    app: &AppHandle<R>,
    preferred: &str,
    gs: &tauri_plugin_global_shortcut::GlobalShortcut<R>,
    ctx: SessionCtx,
) -> ActiveHotkey {
    if preferred.is_empty() {
        return ActiveHotkey::empty("");
    }

    // ---- Chord path (macOS only) -------------------------------------------
    #[cfg(target_os = "macos")]
    if let Some(spec) = mac_chord::try_parse_chord(preferred) {
        match install_chord(app, spec, ctx) {
            Ok(()) => {
                log::error!(
                    "push-to-talk: chord '{preferred}' installed via \
                     CGEventTap. Hold the modifiers to dictate; release \
                     to transcribe and paste."
                );
                return ActiveHotkey {
                    configured: preferred.to_string(),
                    bound: preferred.to_string(),
                    fell_back: false,
                    mechanism: "chord".to_string(),
                    error: None,
                    error_chain: Vec::new(),
                };
            }
            Err(e) => {
                let friendly = friendly_chord_error(&e);
                log::error!(
                    "push-to-talk: chord '{preferred}' could not be \
                     installed: {e}. NOT falling back to a key shortcut \
                     — the user explicitly chose a chord. Surface error: \
                     {friendly}"
                );
                return ActiveHotkey {
                    configured: preferred.to_string(),
                    bound: String::new(),
                    fell_back: false,
                    mechanism: String::new(),
                    error: Some(friendly.clone()),
                    error_chain: vec![format!("'{preferred}' (chord): {e}"), friendly],
                };
            }
        }
    }

    // ---- Key-shortcut path ------------------------------------------------
    let mut candidates: Vec<String> = vec![preferred.to_string()];
    for f in KEY_PTT_FALLBACKS {
        if !candidates.iter().any(|c| c == f) {
            candidates.push((*f).to_string());
        }
    }

    let mut error_chain: Vec<String> = Vec::new();
    let mut primary_error: Option<String> = None;

    for (idx, key) in candidates.iter().enumerate() {
        let is_primary = idx == 0;
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
                        "push-to-talk: configured key '{preferred}' could not \
                         be bound; using fallback key '{key}'. Errors: {}",
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
        "push-to-talk: NO key shortcut could be bound. Tried: {}",
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

/// Map a `mac_chord::ChordError` onto a single-sentence string the
/// settings UI / pill / tray can show the user. The message must be
/// actionable — assume the reader has never heard of "Accessibility
/// permission" before.
#[cfg(target_os = "macos")]
fn friendly_chord_error(e: &mac_chord::ChordError) -> String {
    match e {
        mac_chord::ChordError::PermissionDenied
        | mac_chord::ChordError::TapCreateFailed => {
            "Vox can't listen for the chord because Accessibility \
             permission isn't granted. Open System Settings → Privacy & \
             Security → Accessibility and add Vox."
                .into()
        }
        mac_chord::ChordError::RunLoopFailed => {
            "Vox could not start the chord listener thread. Restart Vox; \
             if this persists please report a bug."
                .into()
        }
        mac_chord::ChordError::EmptySpec => {
            "Configured push-to-talk chord has no modifiers. Pick a \
             chord like Option+Control in Settings → Hotkeys."
                .into()
        }
    }
}

#[cfg(target_os = "macos")]
fn install_chord<R: Runtime>(
    app: &AppHandle<R>,
    spec: mac_chord::ChordSpec,
    ctx: SessionCtx,
) -> Result<(), mac_chord::ChordError> {
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
    )?;

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
