//! Global hotkey registration: push-to-talk, cancel, mode-cycle.
//!
//! When `hands_free_enabled` is true the push-to-talk hotkey toggles instead
//! of acting as press-and-hold; the dispatch reads the current pref each time.
//!
//! Fallback policy:
//! - **Chord PTT configs never fall back to a key.** If the user picked a
//!   modifier-only chord and it can't install (typically: Accessibility not
//!   granted), we surface the error verbatim. Substituting an unrelated key
//!   would be confusing — the user might not even have it on their keyboard.
//! - **Key PTT configs walk `KEY_PTT_FALLBACKS`** if the chosen key fails to
//!   bind. `tauri-plugin-global-shortcut` silently fails for keys missing
//!   from its Carbon scancode map (notably bare `AltRight`), so the chain is
//!   our only signal to the user that something went wrong.
//! - **Cancel / mode-cycle** are discrete events; chords don't apply.

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

/// Consulted only when the user picked a key (not a chord) and it failed
/// to bind. See the module-level docs.
const KEY_PTT_FALLBACKS: &[&str] = &[
    "CommandOrControl+Shift+Space",
    "CommandOrControl+Shift+Semicolon",
    "F13",
    "F14",
    "F15",
];
const CANCEL_FALLBACKS: &[&str] = &["Escape"];
const MODE_CYCLE_FALLBACKS: &[&str] =
    &["Alt+M", "Alt+Shift+M", "CommandOrControl+Shift+M"];

/// Snapshot of what's actually bound for one hotkey purpose at runtime.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ActiveHotkey {
    /// User-picked value. `""` means explicitly disabled.
    pub configured: String,
    /// What's bound now. Empty when nothing bound.
    pub bound: String,
    /// `true` if the configured value failed and a fallback was used.
    pub fell_back: bool,
    /// `"shortcut"` (Carbon RegisterEventHotKey), `"chord"` (CGEventTap),
    /// or `""` when nothing is bound.
    pub mechanism: String,
    pub error: Option<String>,
    /// Per-attempt errors, oldest first. Drives the diagnostics panel.
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

/// Tauri state mirror of the active bindings, read by the IPC layer.
#[derive(Default)]
pub struct ActiveHotkeyState {
    pub push_to_talk: Mutex<ActiveHotkey>,
    pub cancel: Mutex<ActiveHotkey>,
    pub mode_cycle: Mutex<ActiveHotkey>,
    /// Owns the `CGEventTap`-backed chord listener. Re-register drops
    /// the prior handle BEFORE installing the new one so two taps
    /// never compete.
    #[cfg(target_os = "macos")]
    pub chord: Mutex<Option<ChordHandle>>,
}

impl ActiveHotkeyState {
    fn cell(&self, purpose: HotkeyPurpose) -> &Mutex<ActiveHotkey> {
        match purpose {
            HotkeyPurpose::PushToTalk => &self.push_to_talk,
            HotkeyPurpose::Cancel => &self.cancel,
            HotkeyPurpose::ModeCycle => &self.mode_cycle,
        }
    }

    pub fn snapshot(&self, purpose: HotkeyPurpose) -> ActiveHotkey {
        match self.cell(purpose).lock() {
            Ok(g) => g.clone(),
            Err(p) => p.into_inner().clone(),
        }
    }

    fn store(&self, purpose: HotkeyPurpose, active: ActiveHotkey) {
        match self.cell(purpose).lock() {
            Ok(mut g) => *g = active,
            Err(p) => *p.into_inner() = active,
        }
    }
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

/// Result of walking `[configured] + fallbacks` until one binds.
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

/// Common path for the cancel / mode-cycle bindings: walks `try_chain`,
/// logs + persists fallbacks, returns the populated `ActiveHotkey`.
fn bind_discrete<R, F>(
    app: &AppHandle<R>,
    preferred: &str,
    fallbacks: &[&str],
    purpose: HotkeyPurpose,
    label: &str,
    register: F,
) -> ActiveHotkey
where
    R: Runtime,
    F: FnMut(&str, Shortcut) -> Result<(), String>,
{
    if preferred.is_empty() {
        return ActiveHotkey::empty("");
    }
    match try_chain(preferred, fallbacks, register) {
        Ok(out) => {
            if out.fell_back {
                log::error!(
                    "{label}: configured '{preferred}' could not be bound; \
                     using fallback '{}'. Errors: {}",
                    out.bound,
                    out.error_chain.join("; ")
                );
                if let Err(e) = settings::set_hotkey_field(app, purpose, &out.bound) {
                    log::warn!("could not persist resolved {label} hotkey: {e}");
                }
            }
            ActiveHotkey {
                configured: preferred.to_string(),
                bound: out.bound,
                fell_back: out.fell_back,
                mechanism: "shortcut".to_string(),
                error: out.primary_error,
                error_chain: out.error_chain,
            }
        }
        Err(chain) => {
            log::error!("{label}: NO hotkey could be bound. Tried: {}", chain.join("; "));
            ActiveHotkey {
                configured: preferred.to_string(),
                bound: String::new(),
                fell_back: false,
                mechanism: String::new(),
                error: chain.first().cloned(),
                error_chain: chain,
            }
        }
    }
}

pub fn register<R: Runtime>(
    app: &AppHandle<R>,
    preferred: &str,
    cancel_preferred: &str,
    mode_cycle_preferred: &str,
    ctx: SessionCtx,
) -> Result<String, String> {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();

    // Drop any existing chord listener BEFORE installing a new one so
    // two taps never run in parallel after a re-register.
    #[cfg(target_os = "macos")]
    if let Some(state) = app.try_state::<ActiveHotkeyState>() {
        if let Ok(mut g) = state.chord.lock() {
            *g = None;
        }
    }

    #[cfg(target_os = "macos")]
    {
        // SAFETY: AXIsProcessTrusted is a non-prompting check.
        extern "C" {
            fn AXIsProcessTrusted() -> bool;
        }
        log::info!("hotkey: AXIsProcessTrusted={}", unsafe { AXIsProcessTrusted() });
    }

    let ptt_active: ActiveHotkey = register_push_to_talk(app, preferred, gs, ctx.clone());
    let ptt_bound_sc = shortcut_from_key(&ptt_active.bound);

    let cancel_active = bind_discrete(
        app,
        cancel_preferred,
        CANCEL_FALLBACKS,
        HotkeyPurpose::Cancel,
        "cancel",
        |key, sc| {
            if Some(sc) == ptt_bound_sc {
                return Err(format!("'{key}' collides with push-to-talk; not registering"));
            }
            let app_handle = app.clone();
            let ctx_clone = ctx.clone();
            gs.on_shortcut(sc, move |_app, _sc, event: ShortcutEvent| {
                if let ShortcutState::Pressed = event.state() {
                    session::cancel(app_handle.clone(), ctx_clone.clone());
                }
            })
            .map_err(|e| e.to_string())
        },
    );
    let cancel_bound_sc = shortcut_from_key(&cancel_active.bound);

    let mode_active = bind_discrete(
        app,
        mode_cycle_preferred,
        MODE_CYCLE_FALLBACKS,
        HotkeyPurpose::ModeCycle,
        "mode-cycle",
        |key, sc| {
            if Some(sc) == ptt_bound_sc {
                return Err(format!("'{key}' collides with push-to-talk; not registering"));
            }
            if Some(sc) == cancel_bound_sc {
                return Err(format!("'{key}' collides with cancel; not registering"));
            }
            let app_handle = app.clone();
            gs.on_shortcut(sc, move |_app, _sc, event: ShortcutEvent| {
                if let ShortcutState::Pressed = event.state() {
                    spawn_cycle_active_mode(app_handle.clone());
                }
            })
            .map_err(|e| e.to_string())
        },
    );

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

/// PTT bind: chord path first (macOS only) — never falls back to a key.
/// Otherwise walks `KEY_PTT_FALLBACKS` via the standard shortcut plugin.
fn register_push_to_talk<R: Runtime>(
    app: &AppHandle<R>,
    preferred: &str,
    gs: &tauri_plugin_global_shortcut::GlobalShortcut<R>,
    ctx: SessionCtx,
) -> ActiveHotkey {
    if preferred.is_empty() {
        return ActiveHotkey::empty("");
    }

    #[cfg(target_os = "macos")]
    if let Some(spec) = mac_chord::try_parse_chord(preferred) {
        match install_chord(app, spec, ctx) {
            Ok(()) => {
                log::error!("push-to-talk: chord '{preferred}' installed via CGEventTap");
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
                    "push-to-talk: chord '{preferred}' install failed: {e}. \
                     NOT falling back — user chose a chord. {friendly}"
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
            let prefs = settings::prefs(&app_handle);
            match (prefs.hands_free_enabled, event.state()) {
                (true, ShortcutState::Pressed) => {
                    if ctx_clone.is_recording() {
                        session::end_recording(app_handle.clone(), ctx_clone.clone());
                    } else {
                        session::begin_recording(app_handle.clone(), &ctx_clone);
                    }
                }
                (false, ShortcutState::Pressed) => {
                    session::begin_recording(app_handle.clone(), &ctx_clone)
                }
                (false, ShortcutState::Released) => {
                    session::end_recording(app_handle.clone(), ctx_clone.clone())
                }
                _ => {}
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

/// Single actionable sentence shown in Settings / pill / tray. Assume the
/// reader has never heard of "Accessibility permission" before.
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
            if settings::prefs(&app_press).hands_free_enabled {
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
            if !settings::prefs(&app_release).hands_free_enabled {
                session::end_recording(app_release.clone(), ctx_release.clone());
            }
        },
    )?;

    // Keep the handle alive on the Tauri-managed state. Lazily-manage if
    // the host hasn't installed `ActiveHotkeyState` yet.
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
        let Some(next) = crate::modes::cycle_active(&app, &prefs.active_mode_id) else {
            return;
        };
        if let Err(e) = settings::set_active_mode_id(&app, &next.id) {
            log::warn!("failed to persist mode cycle: {e}");
            return;
        }
        // 800 ms HUD so the user sees the new mode.
        crate::pill::show(&app);
        crate::pill::emit_active_mode(&app, &next);
        let app_clone = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(800)).await;
            if let Some(ctx) = app_clone.try_state::<SessionCtx>() {
                if ctx.is_recording() || ctx.is_pipeline_active() {
                    return;
                }
            }
            crate::pill::hide(&app_clone);
        });
    });
}
