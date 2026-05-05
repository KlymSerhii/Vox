//! Global hotkey registration: push-to-talk, cancel, mode-cycle.
//!
//! When `hands_free_enabled` is true the push-to-talk hotkey toggles instead
//! of acting as press-and-hold; the dispatch reads the current pref each time.
//!
//! Fallback policy:
//! - **Push-to-talk has no fallback.** The configured spec is the only
//!   thing tried — chord-first on macOS, then a single key bind. The key
//!   is currently hardcoded to `Cmd+Alt+Period` and has no UI picker, so
//!   substituting an unrelated key would silently swap behaviour with no
//!   way to communicate the swap. If the bind fails we surface the error.
//! - **Cancel / mode-cycle** walk a small fallback list and persist any
//!   substitution back to settings; their pickers are visible in the UI.

use std::sync::Arc;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_global_shortcut::{
    Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutEvent, ShortcutState,
};

#[cfg(target_os = "macos")]
use crate::mac_chord::{self, ChordHandle};
use crate::session::{self, BeginOutcome, SessionManager, MAX_CONCURRENT_SESSIONS};
use crate::settings::{self, HotkeyPurpose};

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
    /// Cached cancel binding. The cancel hotkey (default `Escape`) is
    /// only registered with the OS while a dictation session is active —
    /// otherwise pressing Escape in any other app would be swallowed by
    /// our global grab and never reach the focused window. We resolve
    /// the binding once at `register()` time (so we know which key
    /// actually works on this machine), immediately release the OS
    /// grab, and stash the spec here for `arm_cancel` / `disarm_cancel`
    /// to flip on/off at session boundaries.
    pub cancel_armed: Mutex<Option<CancelArm>>,
    /// Owns the `CGEventTap`-backed chord listener. Re-register drops
    /// the prior handle BEFORE installing the new one so two taps
    /// never compete.
    #[cfg(target_os = "macos")]
    pub chord: Mutex<Option<ChordHandle>>,
}

/// Resolved cancel binding kept in `ActiveHotkeyState` for arm/disarm
/// to consult.
pub struct CancelArm {
    pub shortcut: Shortcut,
    pub manager: Arc<SessionManager>,
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
        "F13" => Some(Shortcut::new(None, Code::F13)),
        "F14" => Some(Shortcut::new(None, Code::F14)),
        "F15" => Some(Shortcut::new(None, Code::F15)),
        "Escape" | "Esc" => Some(Shortcut::new(None, Code::Escape)),
        "Cmd+Alt+Period" => Some(Shortcut::new(
            Some(Modifiers::META | Modifiers::ALT),
            Code::Period,
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
    manager: Arc<SessionManager>,
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

    let ptt_active: ActiveHotkey =
        register_push_to_talk(app, preferred, gs, manager.clone());
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
            // Probe with a no-op handler. We unregister immediately
            // below — the real arming happens in `arm_cancel` at
            // session start. The bind walk only exists here to surface
            // fallbacks and validation errors during launch.
            gs.register(sc).map_err(|e| e.to_string())
        },
    );
    let cancel_bound_sc = shortcut_from_key(&cancel_active.bound);
    // Release the OS grab immediately. Without this the cancel hotkey
    // (default `Escape`) would intercept the key in every other app for
    // the entire lifetime of Vox, even with no dictation in flight.
    if let Some(sc) = cancel_bound_sc {
        let _ = gs.unregister(sc);
        if let Some(state) = app.try_state::<ActiveHotkeyState>() {
            if let Ok(mut g) = state.cancel_armed.lock() {
                *g = Some(CancelArm {
                    shortcut: sc,
                    manager: manager.clone(),
                });
            }
        }
    } else if let Some(state) = app.try_state::<ActiveHotkeyState>() {
        // Probe failed — make sure no stale spec from a previous
        // register() can re-arm later.
        if let Ok(mut g) = state.cancel_armed.lock() {
            *g = None;
        }
    }

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

    // If the user reconfigures hotkeys mid-dictation, the
    // `unregister_all()` at the top wiped the cancel grab we'd put down
    // in `arm_cancel`. Re-arm so Escape still cancels the in-flight
    // session.
    if manager.is_recording() {
        arm_cancel(app);
    }

    Ok(ptt_active.bound)
}

/// Acquire the OS grab on the cancel hotkey. Called from
/// `session::begin_recording` so Escape only redirects to Vox while a
/// dictation session is in flight.
///
/// Deferred onto the tokio runtime to escape Carbon's
/// `RegisterEventHotKey` re-entrancy: the PTT handler runs inside the
/// Carbon event dispatcher on the main thread, and registering a fresh
/// hotkey from inside that dispatcher froze the app. Posting the work
/// back to a worker lets the current hotkey event finish unwinding
/// before the new shortcut is bound.
pub fn arm_cancel<R: Runtime>(app: &AppHandle<R>) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let Some(state) = app.try_state::<ActiveHotkeyState>() else {
            return;
        };
        let armed = match state.cancel_armed.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        let Some(arm) = armed.as_ref() else {
            return;
        };
        let sc = arm.shortcut;
        let manager = arm.manager.clone();
        drop(armed);

        let gs = app.global_shortcut();
        if gs.is_registered(sc) {
            return;
        }

        // The cancel hotkey is bound only on the Wry runtime — downcast
        // here so the inner closure can hand the typed AppHandle to
        // SessionManager::cancel_foreground.
        let app_handle = app.clone();
        if let Err(e) = gs.on_shortcut(sc, move |_app, _sc, event: ShortcutEvent| {
            if let ShortcutState::Pressed = event.state() {
                if let Some(wry_app) = try_into_wry(&app_handle) {
                    manager.cancel_foreground(&wry_app);
                }
            }
        }) {
            log::warn!("arm_cancel: failed to register cancel shortcut: {e}");
        }
    });
}

/// Release the OS grab on the cancel hotkey so Escape works normally
/// in every other app. Called when the dictation pipeline ends (any
/// terminal state: Done, Canceled, Error, audio-start failure).
///
/// Deferred for the same Carbon re-entrancy reason as `arm_cancel` —
/// `gs.unregister` ultimately calls `UnregisterEventHotKey`, which we
/// must not invoke from inside another hotkey's event dispatch.
pub fn disarm_cancel<R: Runtime>(app: &AppHandle<R>) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let Some(state) = app.try_state::<ActiveHotkeyState>() else {
            return;
        };
        let armed = match state.cancel_armed.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        let Some(arm) = armed.as_ref() else {
            return;
        };
        let sc = arm.shortcut;
        drop(armed);

        let gs = app.global_shortcut();
        if !gs.is_registered(sc) {
            return;
        }
        let _ = gs.unregister(sc);
    });
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

/// PTT bind: chord path first (macOS only), otherwise a single key bind
/// via the standard shortcut plugin. No fallback walk — the picker is
/// hardcoded and we'd rather surface the explicit error than smuggle in
/// an unrelated key.
fn register_push_to_talk<R: Runtime>(
    app: &AppHandle<R>,
    preferred: &str,
    gs: &tauri_plugin_global_shortcut::GlobalShortcut<R>,
    manager: Arc<SessionManager>,
) -> ActiveHotkey {
    if preferred.is_empty() {
        return ActiveHotkey::empty("");
    }

    #[cfg(target_os = "macos")]
    if let Some(spec) = mac_chord::try_parse_chord(preferred) {
        match install_chord(app, spec, manager.clone()) {
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

    let Some(sc) = shortcut_from_key(preferred) else {
        let msg = format!("'{preferred}': unrecognised shortcut id");
        log::error!("push-to-talk: {msg}");
        return ActiveHotkey {
            configured: preferred.to_string(),
            bound: String::new(),
            fell_back: false,
            mechanism: String::new(),
            error: Some(msg.clone()),
            error_chain: vec![msg],
        };
    };

    let app_handle = app.clone();
    let manager_press = manager.clone();
    let manager_release = manager.clone();
    match gs.on_shortcut(sc, move |_app, _sc, event: ShortcutEvent| {
        let prefs = settings::prefs(&app_handle);
        match (prefs.hands_free_enabled, event.state()) {
            (true, ShortcutState::Pressed) => {
                if let Some(wry_app) = try_into_wry(&app_handle) {
                    if manager_press.foreground_id().is_some() {
                        manager_press.end_recording(wry_app);
                    } else {
                        handle_begin(&wry_app, &manager_press);
                    }
                }
            }
            (false, ShortcutState::Pressed) => {
                if let Some(wry_app) = try_into_wry(&app_handle) {
                    handle_begin(&wry_app, &manager_press);
                }
            }
            (false, ShortcutState::Released) => {
                if let Some(wry_app) = try_into_wry(&app_handle) {
                    manager_release.end_recording(wry_app);
                }
            }
            _ => {}
        }
    }) {
        Ok(()) => ActiveHotkey {
            configured: preferred.to_string(),
            bound: preferred.to_string(),
            fell_back: false,
            mechanism: "shortcut".to_string(),
            error: None,
            error_chain: Vec::new(),
        },
        Err(e) => {
            let msg = format!("'{preferred}': {e}");
            log::error!("push-to-talk: bind failed — {msg}");
            ActiveHotkey {
                configured: preferred.to_string(),
                bound: String::new(),
                fell_back: false,
                mechanism: String::new(),
                error: Some(msg.clone()),
                error_chain: vec![msg],
            }
        }
    }
}

/// Shared handler for the PTT-press path that needs to begin a new
/// session. Translates BeginOutcome variants into the appropriate
/// pill / log effects. Lives next to the bind sites so each
/// platform's hotkey closure can call into one place.
fn handle_begin(app: &tauri::AppHandle<tauri::Wry>, manager: &Arc<SessionManager>) {
    match manager.begin(app) {
        BeginOutcome::Started(_) => {
            // begin already showed the pill, emitted Listening, and
            // armed the cancel grab.
        }
        BeginOutcome::Saturated { in_flight } => {
            log::info!(
                "ptt: refusing new session — {in_flight}/{MAX_CONCURRENT_SESSIONS} already in flight"
            );
            session::emit_saturated_toast(app, in_flight, MAX_CONCURRENT_SESSIONS);
        }
        BeginOutcome::DictationDisabled => {
            // No-op — the master kill-switch is in effect; logged inside begin().
        }
        BeginOutcome::AlreadyRecording => {
            // Race-loser path: another caller raced us in. Silently ignore.
        }
        BeginOutcome::AudioError(_) => {
            // begin already emitted the friendly pill error and
            // scheduled an auto-hide.
        }
        BeginOutcome::PipelineBusy => {
            // begin already logged a WARN and emitted the busy toast;
            // the user retries once the pill clears.
        }
    }
}

/// Downcast a generic AppHandle to the Wry runtime so the hotkey
/// closures can hand a typed handle to SessionManager (the manager
/// is registered as a Wry-typed app state).
///
/// In this binary the downcast is **always an identity** — every
/// hotkey site is reached with `R = Wry` because `tauri::Builder`
/// in `lib.rs::run` is itself Wry-only. We keep the `R: Runtime`
/// generic signature on the surrounding `register` / `arm_cancel`
/// / `register_push_to_talk` chain because:
///   1. `tauri-plugin-global-shortcut`'s `on_shortcut` callback is
///      handed an `&AppHandle<R>` parameterised on the same runtime
///      the plugin was constructed with — there is no Wry-only
///      shortcut variant to call instead.
///   2. The `mac_chord` chord-listener path also flows through here
///      from a `tauri::Manager`-bound generic `R`.
/// Tightening to `R = Wry` would require either a custom shortcut
/// plugin or duplicating every register-* helper for Wry-typed
/// AppHandles, neither of which earns its complexity. A
/// `debug_assert!` at startup would also be wrong — `Any::is` on a
/// non-Wry handle would silently return None just like the failing
/// downcast we already handle.
fn try_into_wry<R: Runtime>(app: &AppHandle<R>) -> Option<AppHandle<tauri::Wry>> {
    use std::any::Any;
    let any: &dyn Any = app as &dyn Any;
    any.downcast_ref::<AppHandle<tauri::Wry>>().cloned()
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
    manager: Arc<SessionManager>,
) -> Result<(), mac_chord::ChordError> {
    let app_press = app.clone();
    let manager_press = manager.clone();
    let app_release = app.clone();
    let manager_release = manager.clone();
    let handle = mac_chord::install(
        spec,
        move || {
            if let Some(wry_app) = try_into_wry(&app_press) {
                if settings::prefs(&app_press).hands_free_enabled {
                    if manager_press.foreground_id().is_some() {
                        manager_press.end_recording(wry_app);
                    } else {
                        handle_begin(&wry_app, &manager_press);
                    }
                } else {
                    handle_begin(&wry_app, &manager_press);
                }
            }
        },
        move || {
            if !settings::prefs(&app_release).hands_free_enabled {
                if let Some(wry_app) = try_into_wry(&app_release) {
                    manager_release.end_recording(wry_app);
                }
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
            if let Some(mgr) = app_clone.try_state::<Arc<SessionManager>>() {
                if mgr.is_recording() || mgr.has_active_pipeline() {
                    return;
                }
            }
            crate::pill::hide(&app_clone);
        });
    });
}
