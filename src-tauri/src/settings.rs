//! Settings — backed by `tauri-plugin-store` for non-secret fields.
//!
//! Local-only build: `load` scrubs any historical cloud engine selections
//! and Keychain entries from upgrades of the pre-local build.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime};
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

pub const KEY_STREAMING_PILL_ENABLED: &str = "streaming_pill_enabled";

const KEY_ENABLE_THINKING: &str = "enable_thinking";

// Process-identification token passed to `llama-server` as `--api-key`,
// reused across sessions so the orphan reaper can spot stale children.
const KEY_INFERENCE_API_KEY: &str = "inference_api_key";

// Legacy keys read once at load for scrubbing — see the cloud-scrub
// block in `load`.
const KEY_LEGACY_CLEANUP_ENABLED: &str = "cleanup_enabled";
const KEY_LEGACY_SEND_AUDIO_CLOUD: &str = "send_audio_to_cloud";
const KEY_LEGACY_GROQ: &str = "groq_api_key";
const KEY_LEGACY_ANTHROPIC: &str = "anthropic_api_key";
const KEY_LEGACY_MIGRATION_SEEN: &str = "migration_seen";

const KEYRING_SERVICE: &str = "app.vox.vox";
const KEYRING_ACCOUNT_LEGACY_GROQ: &str = "groq";
const KEYRING_ACCOUNT_LEGACY_ANTHROPIC: &str = "anthropic";

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
fn default_streaming_pill_enabled() -> bool {
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

    /// Show transcription / polish text streaming inside the pill after
    /// the hotkey is released. When false, only the existing
    /// stage labels are shown (matching the pre-streaming build).
    #[serde(default = "default_streaming_pill_enabled")]
    pub streaming_pill_enabled: bool,

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
            streaming_pill_enabled: default_streaming_pill_enabled(),
            enable_thinking: false,
            inference_api_key: String::new(),
        }
    }
}

/// Best-effort delete of a Keychain entry left over from the cloud-era build.
fn keyring_delete(account: &str) {
    match keyring::Entry::new(KEYRING_SERVICE, account) {
        Ok(entry) => match entry.delete_credential() {
            Ok(_) => log::info!("scrubbed legacy keychain entry: {account}"),
            Err(keyring::Error::NoEntry) => {}
            Err(e) => log::warn!("keychain delete failed ({account}): {e}"),
        },
        Err(e) => log::warn!("keychain entry open failed ({account}): {e}"),
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

    // Hotkey is currently hardcoded; force the stored value to match so
    // the picker reflects reality.
    let hotkey = {
        let forced = default_hotkey();
        if get_str(KEY_HOTKEY).as_deref() != Some(forced.as_str()) {
            store.set(KEY_HOTKEY, serde_json::Value::String(forced.clone()));
            let _ = store.save();
        }
        forced
    };
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

    let mut store_dirty = false;

    // Rewrite historical cloud engine selections to local equivalents.
    let stt_engine = {
        let stored = get_str(KEY_STT_ENGINE).unwrap_or_else(default_stt_engine);
        if matches!(stored.as_str(), "groq" | "cloud") {
            log::error!("settings: migrating stt_engine '{stored}' -> 'whisper-local'.");
            store.set(KEY_STT_ENGINE, serde_json::Value::String("whisper-local".into()));
            store_dirty = true;
            "whisper-local".into()
        } else {
            stored
        }
    };
    let stt_model_id = get_str(KEY_STT_MODEL_ID).unwrap_or_else(default_stt_model_id);

    let llm_engine = {
        let stored = get_str(KEY_LLM_ENGINE).unwrap_or_else(default_llm_engine);
        match stored.as_str() {
            "anthropic" | "claude" | "cloud" => {
                log::error!("settings: migrating llm_engine '{stored}' -> 'none'.");
                store.set(KEY_LLM_ENGINE, serde_json::Value::String("none".into()));
                store_dirty = true;
                "none".to_string()
            }
            // Whitelist of engines the Registry knows how to construct.
            // Anything else (a setting from a future build, a hand-edit)
            // becomes the default so the pipeline doesn't dead-end.
            "gemma-local" | "apple-fm" | "none" => stored,
            other => {
                let replacement = default_llm_engine();
                log::error!(
                    "settings: unknown llm_engine '{other}' -> '{replacement}'."
                );
                store.set(KEY_LLM_ENGINE, serde_json::Value::String(replacement.clone()));
                store_dirty = true;
                replacement
            }
        }
    };
    // Rewrite retired or known-crashing model ids to the current default.
    let llm_model_id = {
        let stored = get_str(KEY_LLM_MODEL_ID).unwrap_or_else(default_llm_model_id);
        if models::is_known_broken_llm_id(&stored) {
            let replacement = default_llm_model_id();
            log::error!(
                "settings: migrating retired/broken llm_model_id '{stored}' -> '{replacement}'."
            );
            store.set(KEY_LLM_MODEL_ID, serde_json::Value::String(replacement.clone()));
            store_dirty = true;
            replacement
        } else {
            stored
        }
    };

    // First-run inference for upgraders: if cloud cleanup was on (or any
    // LLM engine was set), preserve "polishing on" so it returns when
    // Gemma is available.
    let polishing_enabled = match store.get(KEY_POLISHING_ENABLED) {
        Some(v) => v.as_bool().unwrap_or_else(default_polishing_enabled),
        None => {
            let inferred = get_bool(KEY_LEGACY_CLEANUP_ENABLED, false)
                || matches!(
                    get_str(KEY_LLM_ENGINE).as_deref(),
                    Some("anthropic") | Some("gemma-local")
                );
            store.set(KEY_POLISHING_ENABLED, serde_json::Value::Bool(inferred));
            store_dirty = true;
            inferred
        }
    };

    let active_mode_id = get_str(KEY_ACTIVE_MODE_ID).unwrap_or_else(default_active_mode_id);
    let mode_cycle_hotkey =
        get_str(KEY_MODE_CYCLE_HOTKEY).unwrap_or_else(default_mode_cycle_hotkey);
    let auto_suggest_modes = get_bool(KEY_AUTO_SUGGEST_MODES, default_auto_suggest_modes());

    let appearance = match get_str(KEY_APPEARANCE).as_deref() {
        Some("light") => "light".to_string(),
        Some("dark") => "dark".to_string(),
        _ => default_appearance(),
    };

    let streaming_pill_enabled =
        get_bool(KEY_STREAMING_PILL_ENABLED, default_streaming_pill_enabled());

    let enable_thinking = get_bool(KEY_ENABLE_THINKING, false);

    let inference_api_key = get_str(KEY_INFERENCE_API_KEY).unwrap_or_default();

    // Scrub legacy cloud-era keys + Keychain entries on first local-only launch.
    for key in [
        KEY_LEGACY_GROQ,
        KEY_LEGACY_ANTHROPIC,
        KEY_LEGACY_CLEANUP_ENABLED,
        KEY_LEGACY_SEND_AUDIO_CLOUD,
        KEY_LEGACY_MIGRATION_SEEN,
    ] {
        if store.get(key).is_some() {
            store.delete(key);
            store_dirty = true;
            log::info!("scrubbed legacy settings key: {key}");
        }
    }
    keyring_delete(KEYRING_ACCOUNT_LEGACY_GROQ);
    keyring_delete(KEYRING_ACCOUNT_LEGACY_ANTHROPIC);

    if store_dirty {
        let _ = store.save();
    }

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
        streaming_pill_enabled,
        enable_thinking,
        inference_api_key,
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

    b(KEY_STREAMING_PILL_ENABLED, settings.streaming_pill_enabled);
    b(KEY_ENABLE_THINKING, settings.enable_thinking);

    s(KEY_INFERENCE_API_KEY, &settings.inference_api_key);

    // Drop legacy keys defensively in case a downgraded build re-introduced them.
    for key in [
        KEY_LEGACY_GROQ,
        KEY_LEGACY_ANTHROPIC,
        KEY_LEGACY_CLEANUP_ENABLED,
        KEY_LEGACY_SEND_AUDIO_CLOUD,
        KEY_LEGACY_MIGRATION_SEEN,
    ] {
        store.delete(key);
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

    /// Mirror of `Settings::enable_thinking`. Per-Voice-Mode override
    /// (`VoiceMode::enable_thinking: Option<bool>`) takes precedence at
    /// session-begin time; this value is the global fallback.
    pub enable_thinking: bool,
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
        enable_thinking: s.enable_thinking,
    }
}

pub fn set_active_mode_id<R: Runtime>(app: &AppHandle<R>, id: &str) -> anyhow::Result<()> {
    let mut s = load(app);
    s.active_mode_id = id.to_string();
    save(app, &s)
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

        // And verify the default kicks in when the field is absent —
        // older installs upgrading to this build must opt in by default.
        let bare = "{}";
        let restored: Settings = serde_json::from_str(bare).expect("deserialize bare");
        assert!(restored.streaming_pill_enabled);
    }
}
