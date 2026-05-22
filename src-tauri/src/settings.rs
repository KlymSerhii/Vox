//! Settings — backed by `tauri-plugin-store` for non-secret fields.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Runtime};
use tauri_plugin_store::StoreExt;

use crate::models;

pub const STORE_FILE: &str = "vox-settings.json";

const KEY_HOTKEY: &str = "hotkey";
const KEY_CANCEL_HOTKEY: &str = "cancel_hotkey";
const KEY_LAUNCH_AT_LOGIN: &str = "launch_at_login";
const KEY_ONBOARDED: &str = "onboarded";
const KEY_DICTATION_ENABLED: &str = "dictation_enabled";
const KEY_DICTIONARY: &str = "dictionary";
const KEY_HANDS_FREE: &str = "hands_free_enabled";
const KEY_LANGUAGE: &str = "language";

const KEY_STT_ENGINE: &str = "stt_engine";
const KEY_STT_MODEL_ID: &str = "stt_model_id";
const KEY_LLM_ENGINE: &str = "llm_engine";
const KEY_LLM_MODEL_ID: &str = "llm_model_id";
const KEY_GEMMA_TOS_ACCEPTED: &str = "gemma_tos_accepted";
const KEY_POLISHING_ENABLED: &str = "polishing_enabled";

const KEY_ACTIVE_MODE_ID: &str = "active_mode_id";
const KEY_MODE_CYCLE_HOTKEY: &str = "mode_cycle_hotkey";
const KEY_AUTO_SUGGEST_MODES: &str = "auto_suggest_modes";

const KEY_APPEARANCE: &str = "appearance";

const KEY_SHOW_DOCK_ICON: &str = "show_dock_icon";

pub const KEY_STREAMING_PILL_ENABLED: &str = "streaming_pill_enabled";

const KEY_AUTO_PASTE_ENABLED: &str = "auto_paste_enabled";

const KEY_LIVE_TRANSCRIPT_ENABLED: &str = "live_transcript_enabled";

const KEY_ENABLE_THINKING: &str = "enable_thinking";

const KEY_TRY_PARAKEET_DISMISSED: &str = "try_parakeet_dismissed";

const KEY_AUDIO_INPUT_DEVICE: &str = "audio_input_device";

// Process-identification token passed to `llama-server` as `--api-key`,
// reused across sessions so the orphan reaper can spot stale children.
const KEY_INFERENCE_API_KEY: &str = "inference_api_key";

// Auto-update scheduler keys. `last_update_check_at` is wall-clock
// Unix seconds of the most recent SUCCESSFUL background or manual
// check; the scheduler in `updater.rs` uses it to enforce the 6 h
// cooldown and 24 h cadence across relaunches.
// `dismissed_update_version` is the version a user "Later"-dismissed
// in the tray banner; while it equals the current available version,
// the banner + tray-icon dot stay hidden. Settings → About still
// surfaces the pending state regardless.
const KEY_AUTO_UPDATE_CHECK_ENABLED: &str = "auto_update_check_enabled";
const KEY_LAST_UPDATE_CHECK_AT: &str = "last_update_check_at";
const KEY_DISMISSED_UPDATE_VERSION: &str = "dismissed_update_version";
const KEY_UPDATE_WINDOW_SHOWN_VERSION: &str = "update_window_shown_version";

fn default_hotkey() -> String {
    // ⌘⌥. — Cmd + Option + Period. Bound via Carbon
    // `RegisterEventHotKey` through `tauri-plugin-global-shortcut`, so
    // it doesn't need Accessibility. Period is the unshifted glyph
    // of its key, so no Shift collapsing is needed at display time.
    "Cmd+Alt+Period".to_string()
}
fn default_cancel_hotkey() -> String {
    "Escape".to_string()
}
fn default_language() -> String {
    "auto".to_string()
}
fn default_true() -> bool {
    true
}
fn default_stt_engine() -> String {
    "whisper-local".into()
}
fn default_llm_engine() -> String {
    "gemma-local".into()
}
fn default_polishing_enabled() -> bool {
    true
}
fn default_stt_model_id() -> String {
    models::recommended_stt_model_id().to_string()
}
fn default_llm_model_id() -> String {
    models::recommended_llm_model_id().to_string()
}
fn default_active_mode_id() -> String {
    "general".into()
}
fn default_mode_cycle_hotkey() -> String {
    "Alt+M".into()
}
fn default_auto_suggest_modes() -> bool {
    true
}
fn default_appearance() -> String {
    "dark".into()
}
fn default_show_dock_icon() -> bool {
    // Off by default. Vox is an ambient menu-bar utility — the default
    // experience stays Dock-free (`ActivationPolicy::Accessory`, backed
    // by `LSUIElement` in Info.plist). Flipping this on in
    // Settings → General switches the policy to `Regular` so the app
    // gains a Dock icon + ⌘-Tab entry; clicking the icon opens Settings
    // (handled by the `RunEvent::Reopen` arm in `lib.rs`). The menu-bar
    // icon is always present regardless — this toggle is purely additive.
    false
}
fn default_streaming_pill_enabled() -> bool {
    true
}
fn default_auto_paste_enabled() -> bool {
    // Opt-in. Pasting requires Accessibility, which Vox's default
    // pipeline deliberately doesn't need. Users who want the
    // hands-off "speak straight into the focused field" flow flip
    // this on in Settings → General.
    false
}
fn default_live_transcript_enabled() -> bool {
    // Opt-in. Reveals the speculative transcript inside the
    // pill while the user is still speaking (instead of the default
    // waveform-only listening view). Off by default because it
    // forces the speculative decoder to run even on battery and
    // changes the listening silhouette — users who explicitly want
    // a live caption flip this on.
    false
}
fn default_auto_update_check_enabled() -> bool {
    // On by default. The PM-decided behaviour is "check once a day,
    // never auto-install". Flipping this off disables the periodic
    // scheduler in `updater.rs`; the manual button in
    // Settings → About still works.
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default = "default_hotkey")]
    pub hotkey: String,
    #[serde(default = "default_cancel_hotkey")]
    pub cancel_hotkey: String,

    #[serde(default)]
    pub launch_at_login: bool,
    #[serde(default = "default_true")]
    pub dictation_enabled: bool,

    #[serde(default)]
    pub hands_free_enabled: bool,
    #[serde(default = "default_language")]
    pub language: String,

    #[serde(default)]
    pub onboarded: bool,

    #[serde(default)]
    pub dictionary: Vec<String>,

    #[serde(default = "default_stt_engine")]
    pub stt_engine: String,
    #[serde(default = "default_stt_model_id")]
    pub stt_model_id: String,
    #[serde(default = "default_llm_engine")]
    pub llm_engine: String,
    #[serde(default = "default_llm_model_id")]
    pub llm_model_id: String,
    /// Independent of `llm_engine` so the user's preference survives
    /// engine outages — flipping this back on restores `gemma-local`.
    #[serde(default = "default_polishing_enabled")]
    pub polishing_enabled: bool,
    #[serde(default)]
    pub gemma_tos_accepted: bool,

    #[serde(default = "default_active_mode_id")]
    pub active_mode_id: String,
    #[serde(default = "default_mode_cycle_hotkey")]
    pub mode_cycle_hotkey: String,
    #[serde(default = "default_auto_suggest_modes")]
    pub auto_suggest_modes: bool,

    #[serde(default = "default_appearance")]
    pub appearance: String,

    /// Show a Dock icon (and ⌘-Tab entry) for Vox. Off by default —
    /// Vox runs as a menu-bar-only accessory app. When true, the
    /// activation policy is switched to `Regular` at launch and live
    /// on toggle; clicking the Dock icon opens the Settings window. The
    /// menu-bar icon is unaffected either way.
    #[serde(default = "default_show_dock_icon")]
    pub show_dock_icon: bool,

    /// Show transcription / polish text streaming inside the pill after
    /// the hotkey is released. When false, only the existing
    /// stage labels are shown (matching the pre-streaming build).
    #[serde(default = "default_streaming_pill_enabled")]
    pub streaming_pill_enabled: bool,

    /// Auto-paste the polished transcript into the focused text field
    /// by synthesising ⌘V at the end of the pipeline. Requires
    /// Accessibility permission; when the toggle is on but
    /// Accessibility isn't granted, `session.rs` falls open to the
    /// clipboard-only path and surfaces a warning in the pill. Off by
    /// default — Vox's default pipeline deliberately doesn't require
    /// Accessibility.
    #[serde(default = "default_auto_paste_enabled")]
    pub auto_paste_enabled: bool,

    /// Show a live transcript inside the pill while the user is still
    /// speaking, in place of the default waveform. Driven by the same
    /// speculative transcription decoder that already feeds the
    /// post-stop reveal — flipping this on simply reveals what was
    /// previously buffered silently. Forces the speculative pass to run
    /// on battery (it would otherwise bail to save power). Off by
    /// default; the listening silhouette stays compact for users who
    /// don't opt in.
    #[serde(default = "default_live_transcript_enabled")]
    pub live_transcript_enabled: bool,

    /// Gemma 4 only. When true, the engine sends
    /// `chat_template_kwargs: { enable_thinking: true }` per request so
    /// the model emits a chain-of-thought pre-amble before the polished
    /// output. Off by default — Vox is a low-latency dictation tool, not
    /// a reasoning task. Per-Voice-Mode can override (see
    /// `modes::VoiceMode::enable_thinking`).
    #[serde(default)]
    pub enable_thinking: bool,

    /// Token passed to `llama-server` as `--api-key`; generated lazily
    /// on first spawn and reused so the orphan reaper can identify
    /// stale children. Empty = not yet generated.
    #[serde(default)]
    pub inference_api_key: String,

    /// User dismissed the "Try Parakeet" upgrade banner in
    /// Settings → Models. Sticky — once true, the banner stays hidden
    /// regardless of language / availability state.
    #[serde(default)]
    pub try_parakeet_dismissed: bool,

    /// Name of the audio input device Vox listens to. `None` (or
    /// missing) means follow the macOS system default — re-resolved
    /// every recording, so swapping the system input takes effect on
    /// the next dictation. When `Some(name)` and that device isn't
    /// connected at recording time, the session surfaces
    /// `AudioError::ConfiguredDeviceUnavailable` and the pill shows
    /// a clear error — we deliberately do NOT silently fall back to
    /// the system default (that quiet fallback was the bug this
    /// field was added to fix: a disconnected iPhone would route
    /// Whisper onto the MacBook mic with no user-visible signal).
    #[serde(default)]
    pub audio_input_device: Option<String>,

    /// Periodic auto-update scheduler. ON by default; turning it off
    /// disables the daily background check while keeping the manual
    /// "Check now" button in Settings → About working. Mirrored to
    /// disk via `set_settings` like every other top-level field.
    #[serde(default = "default_auto_update_check_enabled")]
    pub auto_update_check_enabled: bool,

    /// Unix epoch seconds of the most recent SUCCESSFUL update check
    /// (background or manual). Used by `updater::should_check_now`
    /// to gate the 24 h cadence and 6 h cooldown across relaunches.
    /// `None` until the first check lands.
    #[serde(default)]
    pub last_update_check_at: Option<i64>,

    /// Version the user "Later"-dismissed in the tray banner. While
    /// this equals the current available version, the banner +
    /// tray-icon dot stay hidden; Settings → About still surfaces
    /// the pending state. Cleared automatically once the available
    /// version moves past this string (a new release auto-rearms).
    #[serde(default)]
    pub dismissed_update_version: Option<String>,

    /// Version for which the prominent auto-show update window has
    /// already been surfaced. Set once the window builds successfully
    /// so it fires at most once per available version, ever — even
    /// across relaunches. The passive tray banner is gated separately
    /// on `dismissed_update_version`, so this never affects it.
    #[serde(default)]
    pub update_window_shown_version: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: default_hotkey(),
            cancel_hotkey: default_cancel_hotkey(),
            launch_at_login: false,
            dictation_enabled: true,
            hands_free_enabled: true,
            language: default_language(),
            onboarded: false,
            dictionary: Vec::new(),
            stt_engine: default_stt_engine(),
            stt_model_id: default_stt_model_id(),
            llm_engine: default_llm_engine(),
            llm_model_id: default_llm_model_id(),
            polishing_enabled: default_polishing_enabled(),
            gemma_tos_accepted: false,
            active_mode_id: default_active_mode_id(),
            mode_cycle_hotkey: default_mode_cycle_hotkey(),
            auto_suggest_modes: default_auto_suggest_modes(),
            appearance: default_appearance(),
            show_dock_icon: default_show_dock_icon(),
            streaming_pill_enabled: default_streaming_pill_enabled(),
            auto_paste_enabled: default_auto_paste_enabled(),
            live_transcript_enabled: default_live_transcript_enabled(),
            enable_thinking: false,
            inference_api_key: String::new(),
            try_parakeet_dismissed: false,
            audio_input_device: None,
            auto_update_check_enabled: default_auto_update_check_enabled(),
            last_update_check_at: None,
            dismissed_update_version: None,
            update_window_shown_version: None,
        }
    }
}

pub fn load<R: Runtime>(app: &AppHandle<R>) -> Settings {
    let store = match app.store(STORE_FILE) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("failed to open settings store: {e}");
            return Settings::default();
        }
    };

    let get_str =
        |k: &str| store.get(k).and_then(|v| v.as_str().map(|s| s.to_string()));
    let get_bool = |k: &str, d: bool| store.get(k).and_then(|v| v.as_bool()).unwrap_or(d);

    let hotkey = get_str(KEY_HOTKEY).unwrap_or_else(default_hotkey);

    let cancel_hotkey = get_str(KEY_CANCEL_HOTKEY).unwrap_or_else(default_cancel_hotkey);
    let language = get_str(KEY_LANGUAGE).unwrap_or_else(default_language);

    let launch_at_login = get_bool(KEY_LAUNCH_AT_LOGIN, false);
    let dictation_enabled = get_bool(KEY_DICTATION_ENABLED, true);
    let hands_free_enabled = get_bool(KEY_HANDS_FREE, true);
    let onboarded = get_bool(KEY_ONBOARDED, false);
    let gemma_tos_accepted = get_bool(KEY_GEMMA_TOS_ACCEPTED, false);

    let dictionary = store
        .get(KEY_DICTIONARY)
        .and_then(|v| v.as_array().cloned())
        .map(|arr| {
            arr.into_iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let stt_engine = get_str(KEY_STT_ENGINE).unwrap_or_else(default_stt_engine);
    let stt_model_id = get_str(KEY_STT_MODEL_ID).unwrap_or_else(default_stt_model_id);
    let llm_engine = get_str(KEY_LLM_ENGINE).unwrap_or_else(default_llm_engine);
    let llm_model_id = get_str(KEY_LLM_MODEL_ID).unwrap_or_else(default_llm_model_id);

    let polishing_enabled = get_bool(KEY_POLISHING_ENABLED, default_polishing_enabled());

    let active_mode_id = get_str(KEY_ACTIVE_MODE_ID).unwrap_or_else(default_active_mode_id);
    let mode_cycle_hotkey =
        get_str(KEY_MODE_CYCLE_HOTKEY).unwrap_or_else(default_mode_cycle_hotkey);
    let auto_suggest_modes = get_bool(KEY_AUTO_SUGGEST_MODES, default_auto_suggest_modes());

    let appearance = match get_str(KEY_APPEARANCE).as_deref() {
        Some("light") => "light".to_string(),
        Some("dark") => "dark".to_string(),
        _ => default_appearance(),
    };

    let show_dock_icon = get_bool(KEY_SHOW_DOCK_ICON, default_show_dock_icon());

    let streaming_pill_enabled =
        get_bool(KEY_STREAMING_PILL_ENABLED, default_streaming_pill_enabled());

    let auto_paste_enabled =
        get_bool(KEY_AUTO_PASTE_ENABLED, default_auto_paste_enabled());

    let live_transcript_enabled = get_bool(
        KEY_LIVE_TRANSCRIPT_ENABLED,
        default_live_transcript_enabled(),
    );

    let enable_thinking = get_bool(KEY_ENABLE_THINKING, false);

    let inference_api_key = get_str(KEY_INFERENCE_API_KEY).unwrap_or_default();

    let try_parakeet_dismissed = get_bool(KEY_TRY_PARAKEET_DISMISSED, false);

    let audio_input_device = store
        .get(KEY_AUDIO_INPUT_DEVICE)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .filter(|s| !s.is_empty());

    let auto_update_check_enabled = get_bool(
        KEY_AUTO_UPDATE_CHECK_ENABLED,
        default_auto_update_check_enabled(),
    );

    let last_update_check_at = store
        .get(KEY_LAST_UPDATE_CHECK_AT)
        .and_then(|v| v.as_i64())
        .filter(|n| *n > 0);

    let dismissed_update_version = store
        .get(KEY_DISMISSED_UPDATE_VERSION)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .filter(|s| !s.is_empty());

    let update_window_shown_version = store
        .get(KEY_UPDATE_WINDOW_SHOWN_VERSION)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .filter(|s| !s.is_empty());

    let _ = store.save();

    Settings {
        hotkey,
        cancel_hotkey,
        launch_at_login,
        dictation_enabled,
        hands_free_enabled,
        language,
        onboarded,
        dictionary,
        stt_engine,
        stt_model_id,
        llm_engine,
        llm_model_id,
        polishing_enabled,
        gemma_tos_accepted,
        active_mode_id,
        mode_cycle_hotkey,
        auto_suggest_modes,
        appearance,
        show_dock_icon,
        streaming_pill_enabled,
        auto_paste_enabled,
        live_transcript_enabled,
        enable_thinking,
        inference_api_key,
        try_parakeet_dismissed,
        audio_input_device,
        auto_update_check_enabled,
        last_update_check_at,
        dismissed_update_version,
        update_window_shown_version,
    }
}

pub fn save<R: Runtime>(app: &AppHandle<R>, settings: &Settings) -> anyhow::Result<()> {
    let store = app
        .store(STORE_FILE)
        .map_err(|e| anyhow::anyhow!("open store: {e}"))?;

    let s = |k: &str, v: &str| store.set(k, serde_json::Value::String(v.to_string()));
    let b = |k: &str, v: bool| store.set(k, serde_json::Value::Bool(v));

    s(KEY_HOTKEY, &settings.hotkey);
    s(KEY_CANCEL_HOTKEY, &settings.cancel_hotkey);
    s(KEY_LANGUAGE, &settings.language);
    b(KEY_HANDS_FREE, settings.hands_free_enabled);
    b(KEY_LAUNCH_AT_LOGIN, settings.launch_at_login);
    b(KEY_DICTATION_ENABLED, settings.dictation_enabled);
    b(KEY_ONBOARDED, settings.onboarded);
    store.set(
        KEY_DICTIONARY,
        serde_json::Value::Array(
            settings
                .dictionary
                .iter()
                .map(|s| serde_json::Value::String(s.clone()))
                .collect(),
        ),
    );

    s(KEY_STT_ENGINE, &settings.stt_engine);
    s(KEY_STT_MODEL_ID, &settings.stt_model_id);
    s(KEY_LLM_ENGINE, &settings.llm_engine);
    s(KEY_LLM_MODEL_ID, &settings.llm_model_id);
    b(KEY_POLISHING_ENABLED, settings.polishing_enabled);
    b(KEY_GEMMA_TOS_ACCEPTED, settings.gemma_tos_accepted);

    s(KEY_ACTIVE_MODE_ID, &settings.active_mode_id);
    s(KEY_MODE_CYCLE_HOTKEY, &settings.mode_cycle_hotkey);
    b(KEY_AUTO_SUGGEST_MODES, settings.auto_suggest_modes);

    let appearance = if settings.appearance == "light" { "light" } else { "dark" };
    s(KEY_APPEARANCE, appearance);
    b(KEY_SHOW_DOCK_ICON, settings.show_dock_icon);

    b(KEY_STREAMING_PILL_ENABLED, settings.streaming_pill_enabled);
    b(KEY_AUTO_PASTE_ENABLED, settings.auto_paste_enabled);
    b(
        KEY_LIVE_TRANSCRIPT_ENABLED,
        settings.live_transcript_enabled,
    );
    b(KEY_ENABLE_THINKING, settings.enable_thinking);

    s(KEY_INFERENCE_API_KEY, &settings.inference_api_key);

    b(KEY_TRY_PARAKEET_DISMISSED, settings.try_parakeet_dismissed);

    match &settings.audio_input_device {
        Some(name) if !name.is_empty() => {
            s(KEY_AUDIO_INPUT_DEVICE, name);
        }
        _ => {
            store.set(KEY_AUDIO_INPUT_DEVICE, serde_json::Value::Null);
        }
    }

    b(KEY_AUTO_UPDATE_CHECK_ENABLED, settings.auto_update_check_enabled);
    match settings.last_update_check_at {
        Some(n) if n > 0 => {
            store.set(KEY_LAST_UPDATE_CHECK_AT, serde_json::Value::from(n));
        }
        _ => {
            store.set(KEY_LAST_UPDATE_CHECK_AT, serde_json::Value::Null);
        }
    }
    match &settings.dismissed_update_version {
        Some(v) if !v.is_empty() => {
            s(KEY_DISMISSED_UPDATE_VERSION, v);
        }
        _ => {
            store.set(KEY_DISMISSED_UPDATE_VERSION, serde_json::Value::Null);
        }
    }

    store
        .save()
        .map_err(|e| anyhow::anyhow!("save store: {e}"))?;
    Ok(())
}

/// Persist just the `last_update_check_at` field. Used by
/// `updater::perform_check` to update the wall-clock timestamp
/// without rewriting the rest of the Settings document on every
/// background tick. Mirrors `set_hotkey_field`.
pub fn set_last_update_check_at<R: Runtime>(
    app: &AppHandle<R>,
    secs: i64,
) -> anyhow::Result<()> {
    let store = app
        .store(STORE_FILE)
        .map_err(|e| anyhow::anyhow!("open store: {e}"))?;
    if secs > 0 {
        store.set(KEY_LAST_UPDATE_CHECK_AT, serde_json::Value::from(secs));
    } else {
        store.set(KEY_LAST_UPDATE_CHECK_AT, serde_json::Value::Null);
    }
    store
        .save()
        .map_err(|e| anyhow::anyhow!("save store: {e}"))?;
    Ok(())
}

/// Persist just the `dismissed_update_version` field. `None` /
/// empty string clears the dismissal so the next "Later" press
/// can be re-applied without a settings round-trip.
pub fn set_dismissed_update_version<R: Runtime>(
    app: &AppHandle<R>,
    value: Option<&str>,
) -> anyhow::Result<()> {
    let store = app
        .store(STORE_FILE)
        .map_err(|e| anyhow::anyhow!("open store: {e}"))?;
    match value {
        Some(v) if !v.is_empty() => {
            store.set(
                KEY_DISMISSED_UPDATE_VERSION,
                serde_json::Value::String(v.to_string()),
            );
        }
        _ => {
            store.set(KEY_DISMISSED_UPDATE_VERSION, serde_json::Value::Null);
        }
    }
    store
        .save()
        .map_err(|e| anyhow::anyhow!("save store: {e}"))?;
    Ok(())
}

/// Persist just the `update_window_shown_version` field. Written once
/// the prominent auto-show update window builds successfully so the
/// window fires at most once per available version. `None` / empty
/// clears the marker.
pub fn set_update_window_shown_version<R: Runtime>(
    app: &AppHandle<R>,
    value: Option<&str>,
) -> anyhow::Result<()> {
    let store = app
        .store(STORE_FILE)
        .map_err(|e| anyhow::anyhow!("open store: {e}"))?;
    match value {
        Some(v) if !v.is_empty() => {
            store.set(
                KEY_UPDATE_WINDOW_SHOWN_VERSION,
                serde_json::Value::String(v.to_string()),
            );
        }
        _ => {
            store.set(KEY_UPDATE_WINDOW_SHOWN_VERSION, serde_json::Value::Null);
        }
    }
    store
        .save()
        .map_err(|e| anyhow::anyhow!("save store: {e}"))?;
    Ok(())
}

pub fn mark_onboarded<R: Runtime>(app: &AppHandle<R>) {
    if let Ok(store) = app.store(STORE_FILE) {
        store.set(KEY_ONBOARDED, serde_json::Value::Bool(true));
        let _ = store.save();
    }
}

/// Lightweight view of the user's preferences for the hotkey thread.
#[derive(Debug, Clone)]
pub struct Prefs {
    pub dictation_enabled: bool,
    pub hands_free_enabled: bool,
    pub language: String,
    pub dictionary: Vec<String>,

    pub stt_engine: String,
    pub stt_model_id: String,
    pub llm_engine: String,
    pub llm_model_id: String,
    pub polishing_enabled: bool,

    pub active_mode_id: String,
    pub auto_suggest_modes: bool,

    /// Mirror of `Settings::streaming_pill_enabled`; the pipeline reads
    /// from a fresh `settings::load` to guarantee the value is current
    /// at the moment streaming begins, but `Prefs` carries it too so
    /// callers that already hold a `Prefs` don't need a second load.
    #[allow(dead_code)]
    pub streaming_pill_enabled: bool,

    /// Mirror of `Settings::auto_paste_enabled`. Captured at session
    /// start so a mid-pipeline toggle doesn't change the delivery
    /// behaviour for an in-flight transcript.
    pub auto_paste_enabled: bool,

    /// Mirror of `Settings::live_transcript_enabled`. Captured at
    /// session start so a mid-utterance toggle can't swap the pill
    /// silhouette under the user. Drives the speculative decoder's
    /// battery override and opens the post-release transcript stream
    /// even when `streaming_pill_enabled` is off.
    pub live_transcript_enabled: bool,

    /// Mirror of `Settings::enable_thinking`. Per-Voice-Mode override
    /// (`VoiceMode::enable_thinking: Option<bool>`) takes precedence at
    /// session-begin time; this value is the global fallback.
    pub enable_thinking: bool,

    /// Mirror of `Settings::audio_input_device`. `None` = follow macOS
    /// system default; `Some(name)` is matched against
    /// `cpal::Host::input_devices()` at `audio::begin_capture` time.
    pub audio_input_device: Option<String>,
}

pub fn prefs<R: Runtime>(app: &AppHandle<R>) -> Prefs {
    let s = load(app);
    Prefs {
        dictation_enabled: s.dictation_enabled,
        hands_free_enabled: s.hands_free_enabled,
        language: s.language,
        dictionary: s.dictionary,
        stt_engine: s.stt_engine,
        stt_model_id: s.stt_model_id,
        llm_engine: s.llm_engine,
        llm_model_id: s.llm_model_id,
        polishing_enabled: s.polishing_enabled,
        active_mode_id: s.active_mode_id,
        auto_suggest_modes: s.auto_suggest_modes,
        streaming_pill_enabled: s.streaming_pill_enabled,
        auto_paste_enabled: s.auto_paste_enabled,
        live_transcript_enabled: s.live_transcript_enabled,
        enable_thinking: s.enable_thinking,
        audio_input_device: s.audio_input_device,
    }
}

pub fn set_active_mode_id<R: Runtime>(app: &AppHandle<R>, id: &str) -> anyhow::Result<()> {
    let mut s = load(app);
    s.active_mode_id = id.to_string();
    save(app, &s)?;
    // Mirror `commands::set_settings`: any change to the live `Settings`
    // doc must fan out so the tray popover, Settings window, and other
    // listeners stay in sync. Without this, the keyboard mode-cycle
    // and the `set_active_mode` / `cycle_active_mode` IPC commands all
    // updated disk silently and any open UI showed stale state until
    // the next full settings save.
    if let Err(e) = app.emit("settings:changed", &s) {
        log::warn!("failed to emit settings:changed after active mode update: {e}");
    }
    Ok(())
}

/// Persist a single hotkey field without rewriting the rest of the
/// settings document — used when `hotkey::register` falls back so the
/// picker reflects the bound shortcut.
pub fn set_hotkey_field<R: Runtime>(
    app: &AppHandle<R>,
    purpose: HotkeyPurpose,
    value: &str,
) -> anyhow::Result<()> {
    let store = app
        .store(STORE_FILE)
        .map_err(|e| anyhow::anyhow!("open store: {e}"))?;
    let key = match purpose {
        HotkeyPurpose::PushToTalk => KEY_HOTKEY,
        HotkeyPurpose::Cancel => KEY_CANCEL_HOTKEY,
        HotkeyPurpose::ModeCycle => KEY_MODE_CYCLE_HOTKEY,
    };
    store.set(key, serde_json::Value::String(value.to_string()));
    store
        .save()
        .map_err(|e| anyhow::anyhow!("save store: {e}"))?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyPurpose {
    PushToTalk,
    Cancel,
    ModeCycle,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaming_pill_enabled_defaults_to_true() {
        let s = Settings::default();
        assert!(s.streaming_pill_enabled);
    }

    #[test]
    fn streaming_pill_enabled_roundtrips_through_serde() {
        let mut s = Settings::default();
        s.streaming_pill_enabled = false;
        let json = serde_json::to_string(&s).expect("serialize");
        let restored: Settings = serde_json::from_str(&json).expect("deserialize");
        assert!(!restored.streaming_pill_enabled);

        // And verify the default kicks in when the field is absent.
        let bare = "{}";
        let restored: Settings = serde_json::from_str(bare).expect("deserialize bare");
        assert!(restored.streaming_pill_enabled);
    }

    #[test]
    fn live_transcript_enabled_defaults_to_false() {
        let s = Settings::default();
        assert!(!s.live_transcript_enabled);
    }

    #[test]
    fn live_transcript_enabled_roundtrips_through_serde() {
        let mut s = Settings::default();
        s.live_transcript_enabled = true;
        let json = serde_json::to_string(&s).expect("serialize");
        let restored: Settings = serde_json::from_str(&json).expect("deserialize");
        assert!(restored.live_transcript_enabled);

        let bare = "{}";
        let restored: Settings = serde_json::from_str(bare).expect("deserialize bare");
        assert!(!restored.live_transcript_enabled);
    }

    #[test]
    fn hotkey_roundtrips_through_serde() {
        // A user-picked PTT spec must survive serialize / deserialize
        // without being silently replaced by the default.
        let mut s = Settings::default();
        s.hotkey = "Cmd+Shift+J".to_string();
        let json = serde_json::to_string(&s).expect("serialize");
        let restored: Settings = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored.hotkey, "Cmd+Shift+J");

        // And verify the default kicks in when the field is absent.
        let bare = "{}";
        let restored: Settings = serde_json::from_str(bare).expect("deserialize bare");
        assert_eq!(restored.hotkey, default_hotkey());
    }
}
