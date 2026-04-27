//! Settings — backed by `tauri-plugin-store` for non-secret fields.
//!
//! 2026-04-26: Vox went fully on-device. Cloud engines (Groq STT,
//! Anthropic LLM cleanup) and their API keys were removed. Settings now
//! persists only local-first preferences. Any historical cloud values
//! are scrubbed at load time:
//!
//! - `stt_engine = "groq"`            → `"whisper-local"`
//! - `llm_engine = "anthropic"`       → `"none"` (raw transcript)
//! - `groq_api_key` / `anthropic_api_key` (plaintext) → deleted
//! - Keychain entries `groq` / `anthropic` → deleted
//!
//! API keys never leave the user's machine in any form afterward — the
//! HTTP code paths that used them are gone from the binary.

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

// New local-first keys.
const KEY_STT_ENGINE: &str = "stt_engine";
const KEY_STT_MODEL_ID: &str = "stt_model_id";
const KEY_LLM_ENGINE: &str = "llm_engine";
const KEY_LLM_MODEL_ID: &str = "llm_model_id";
const KEY_GEMMA_TOS_ACCEPTED: &str = "gemma_tos_accepted";
const KEY_POLISHING_ENABLED: &str = "polishing_enabled";

// Voice Modes keys.
const KEY_ACTIVE_MODE_ID: &str = "active_mode_id";
const KEY_MODE_CYCLE_HOTKEY: &str = "mode_cycle_hotkey";
const KEY_AUTO_SUGGEST_MODES: &str = "auto_suggest_modes";

// Legacy keys we still read once for migration. Any time we touch the
// store we delete these so they never come back.
const KEY_LEGACY_CLEANUP_ENABLED: &str = "cleanup_enabled";
const KEY_LEGACY_SEND_AUDIO_CLOUD: &str = "send_audio_to_cloud";
const KEY_LEGACY_GROQ: &str = "groq_api_key";
const KEY_LEGACY_ANTHROPIC: &str = "anthropic_api_key";
const KEY_LEGACY_MIGRATION_SEEN: &str = "migration_seen";

const KEYRING_SERVICE: &str = "app.vox.vox";
const KEYRING_ACCOUNT_LEGACY_GROQ: &str = "groq";
const KEYRING_ACCOUNT_LEGACY_ANTHROPIC: &str = "anthropic";

fn default_hotkey() -> String {
    "Cmd+Ctrl+Space".to_string()
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
    // Local cleanup engine — runs Gemma on-device via llama-cpp-2.
    "gemma-local".into()
}
fn default_polishing_enabled() -> bool {
    // Polishing is on by default: raw Whisper output is run through
    // local Gemma to fix punctuation, drop filler words, and apply the
    // active Voice Mode.
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

/// User-visible settings. Local-only — no API keys, no cloud secrets.
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

    // ---- Local-first ----
    #[serde(default = "default_stt_engine")]
    pub stt_engine: String,
    #[serde(default = "default_stt_model_id")]
    pub stt_model_id: String,
    #[serde(default = "default_llm_engine")]
    pub llm_engine: String,
    #[serde(default = "default_llm_model_id")]
    pub llm_model_id: String,
    /// User-facing on/off switch for transcript polishing. Independent of
    /// `llm_engine` so the user's preference survives across periods
    /// when no working local LLM is available — when the engine is
    /// fixed, flipping this back on flips `llm_engine` to `gemma-local`
    /// without needing to remember the model id.
    #[serde(default = "default_polishing_enabled")]
    pub polishing_enabled: bool,
    #[serde(default)]
    pub gemma_tos_accepted: bool,

    // ---- Voice Modes ----
    #[serde(default = "default_active_mode_id")]
    pub active_mode_id: String,
    #[serde(default = "default_mode_cycle_hotkey")]
    pub mode_cycle_hotkey: String,
    #[serde(default = "default_auto_suggest_modes")]
    pub auto_suggest_modes: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: default_hotkey(),
            cancel_hotkey: default_cancel_hotkey(),
            launch_at_login: false,
            dictation_enabled: true,
            // 2026-04-26: toggle is the new default — press the hotkey
            // once to start dictating, press again to stop. Holding the
            // key is still supported (it just acts as a no-op down/up
            // toggle pair), but discoverability of "press once" is much
            // better than "hold for the whole sentence".
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
        }
    }
}

// ---------- Legacy cloud key scrubbing -------------------------------------

/// Best-effort delete of a Keychain entry. Used only to scrub legacy
/// cloud API keys when an old install upgrades to the local-only build;
/// any error is logged and ignored — the keys are useless to a binary
/// that doesn't ship the cloud HTTP client.
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

// ---------- Load / save full settings --------------------------------------

pub fn load<R: Runtime>(app: &AppHandle<R>) -> Settings {
    let store = match app.store(STORE_FILE) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("failed to open settings store: {e}");
            return Settings::default();
        }
    };

    let hotkey = {
        let stored = store
            .get(KEY_HOTKEY)
            .and_then(|v| v.as_str().map(|s| s.to_string()));
        let forced = default_hotkey();
        if stored.as_deref() != Some(forced.as_str()) {
            store.set(
                KEY_HOTKEY,
                serde_json::Value::String(forced.clone()),
            );
            let _ = store.save();
        }
        forced
    };
    let cancel_hotkey = store
        .get(KEY_CANCEL_HOTKEY)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(default_cancel_hotkey);
    let language = store
        .get(KEY_LANGUAGE)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(default_language);

    let get_bool = |k: &str, d: bool| store.get(k).and_then(|v| v.as_bool()).unwrap_or(d);

    let launch_at_login = get_bool(KEY_LAUNCH_AT_LOGIN, false);
    let dictation_enabled = get_bool(KEY_DICTATION_ENABLED, true);
    // Defaults to true: see `Settings::default` and the file-level docs.
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

    // -------- Local-only engine selection ---------------------------------
    //
    // Any historical cloud value coming back from disk is rewritten to
    // its local equivalent before we even hand it to the rest of the
    // app. The store is updated in-place so the migration only logs once.
    let mut store_dirty = false;

    let stt_engine = {
        let stored = store
            .get(KEY_STT_ENGINE)
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(default_stt_engine);
        if stored == "groq" || stored == "cloud" {
            log::error!(
                "settings: migrating stt_engine '{stored}' -> 'whisper-local' \
                 (Vox is now fully local)."
            );
            store.set(
                KEY_STT_ENGINE,
                serde_json::Value::String("whisper-local".into()),
            );
            store_dirty = true;
            "whisper-local".into()
        } else {
            stored
        }
    };
    let stt_model_id = store
        .get(KEY_STT_MODEL_ID)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(default_stt_model_id);

    let llm_engine = {
        let stored = store
            .get(KEY_LLM_ENGINE)
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(default_llm_engine);
        // Cloud cleanup was removed — any historical `anthropic` /
        // `claude` / `cloud` value gets rewritten to `none` so the
        // transcript pipeline finishes successfully without ever
        // attempting a network call. `gemma-local` is preserved as-is.
        match stored.as_str() {
            "anthropic" | "claude" | "cloud" => {
                log::error!(
                    "settings: migrating llm_engine '{stored}' -> 'none' \
                     (cloud polishing was removed; Vox is fully local now)."
                );
                store.set(
                    KEY_LLM_ENGINE,
                    serde_json::Value::String("none".into()),
                );
                store_dirty = true;
                "none".to_string()
            }
            _ => stored,
        }
    };
    let llm_model_id = store
        .get(KEY_LLM_MODEL_ID)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(default_llm_model_id);

    // The polishing on/off toggle. If the user has never seen the new
    // toggle and previously had cloud cleanup configured, treat that as
    // "polishing was on" so when Gemma comes back online we can restore
    // their intent. Otherwise default to off.
    let polishing_enabled = match store.get(KEY_POLISHING_ENABLED) {
        Some(v) => v.as_bool().unwrap_or_else(default_polishing_enabled),
        None => {
            // First-run inference for upgraders.
            let was_using_cloud_cleanup = store
                .get(KEY_LEGACY_CLEANUP_ENABLED)
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
                || matches!(
                    store
                        .get(KEY_LLM_ENGINE)
                        .and_then(|v| v.as_str().map(|s| s.to_string()))
                        .as_deref(),
                    Some("anthropic") | Some("gemma-local")
                );
            let inferred = was_using_cloud_cleanup;
            store.set(KEY_POLISHING_ENABLED, serde_json::Value::Bool(inferred));
            store_dirty = true;
            inferred
        }
    };

    let active_mode_id = store
        .get(KEY_ACTIVE_MODE_ID)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(default_active_mode_id);
    let mode_cycle_hotkey = store
        .get(KEY_MODE_CYCLE_HOTKEY)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(default_mode_cycle_hotkey);
    let auto_suggest_modes = get_bool(KEY_AUTO_SUGGEST_MODES, default_auto_suggest_modes());

    // -------- Scrub legacy cloud secrets ----------------------------------
    //
    // Historic plaintext API keys and the old `migration_seen` /
    // cleanup-enabled bools are deleted on the first local-only launch.
    // These are best-effort: a failure here only means a stale, unusable
    // key sits in the Keychain until the next save — there's no live
    // code path that reads it.
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
    }
}

pub fn save<R: Runtime>(app: &AppHandle<R>, settings: &Settings) -> anyhow::Result<()> {
    let store = app
        .store(STORE_FILE)
        .map_err(|e| anyhow::anyhow!("open store: {e}"))?;

    // Local-only build — no cloud secrets to persist anywhere.

    store.set(
        KEY_HOTKEY,
        serde_json::Value::String(settings.hotkey.clone()),
    );
    store.set(
        KEY_CANCEL_HOTKEY,
        serde_json::Value::String(settings.cancel_hotkey.clone()),
    );
    store.set(
        KEY_LANGUAGE,
        serde_json::Value::String(settings.language.clone()),
    );
    store.set(
        KEY_HANDS_FREE,
        serde_json::Value::Bool(settings.hands_free_enabled),
    );
    store.set(
        KEY_LAUNCH_AT_LOGIN,
        serde_json::Value::Bool(settings.launch_at_login),
    );
    store.set(
        KEY_DICTATION_ENABLED,
        serde_json::Value::Bool(settings.dictation_enabled),
    );
    store.set(KEY_ONBOARDED, serde_json::Value::Bool(settings.onboarded));
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

    store.set(
        KEY_STT_ENGINE,
        serde_json::Value::String(settings.stt_engine.clone()),
    );
    store.set(
        KEY_STT_MODEL_ID,
        serde_json::Value::String(settings.stt_model_id.clone()),
    );
    store.set(
        KEY_LLM_ENGINE,
        serde_json::Value::String(settings.llm_engine.clone()),
    );
    store.set(
        KEY_LLM_MODEL_ID,
        serde_json::Value::String(settings.llm_model_id.clone()),
    );
    store.set(
        KEY_POLISHING_ENABLED,
        serde_json::Value::Bool(settings.polishing_enabled),
    );
    store.set(
        KEY_GEMMA_TOS_ACCEPTED,
        serde_json::Value::Bool(settings.gemma_tos_accepted),
    );

    store.set(
        KEY_ACTIVE_MODE_ID,
        serde_json::Value::String(settings.active_mode_id.clone()),
    );
    store.set(
        KEY_MODE_CYCLE_HOTKEY,
        serde_json::Value::String(settings.mode_cycle_hotkey.clone()),
    );
    store.set(
        KEY_AUTO_SUGGEST_MODES,
        serde_json::Value::Bool(settings.auto_suggest_modes),
    );

    // Drop deprecated keys defensively on every save — these may have
    // been re-introduced by a downgraded build, and any cloud-related
    // value here is unreachable code in this binary.
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

/// Lightweight, secret-free view of the user's preferences. Cheap enough
/// to call on the hotkey thread because it never touches the Keychain.
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
    }
}

pub fn set_active_mode_id<R: Runtime>(app: &AppHandle<R>, id: &str) -> anyhow::Result<()> {
    let mut s = load(app);
    s.active_mode_id = id.to_string();
    save(app, &s)
}

/// Persist a single hotkey field by name, without rewriting the rest of
/// the settings document. Used by `hotkey::register` when a configured
/// shortcut couldn't be bound and we resolved a fallback — we mirror the
/// truth back to disk so the picker shows what is actually active.
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

/// Identifies which hotkey field a `set_hotkey_field` call refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyPurpose {
    PushToTalk,
    Cancel,
    ModeCycle,
}
