//! Settings — backed by `tauri-plugin-store` for non-secret fields,
//! and the macOS Keychain (via the `keyring` crate) for API keys.
//!
//! On startup, [`load`] migrates any plaintext keys out of the JSON store
//! into the Keychain and clears them from disk. If Keychain access fails,
//! we transparently fall back to the plaintext store and log a warning —
//! dictation should still work.
//!
//! Local-first migration: existing users who already have a `groq_api_key`
//! keep their cloud engine assignments; new installs get
//! `stt_engine = "whisper-local"`, `llm_engine = "gemma-local"`. Either
//! group sees the migration dialog at most once.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime};
use tauri_plugin_store::StoreExt;

use crate::models;

pub const STORE_FILE: &str = "vox-settings.json";

const KEY_GROQ: &str = "groq_api_key";
const KEY_ANTHROPIC: &str = "anthropic_api_key";
const KEY_HOTKEY: &str = "hotkey";
const KEY_CANCEL_HOTKEY: &str = "cancel_hotkey";
const KEY_PASTE_LAST_HOTKEY: &str = "paste_last_hotkey";
const KEY_LAUNCH_AT_LOGIN: &str = "launch_at_login";
const KEY_PLAY_SOUND: &str = "play_sound";
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
const KEY_MIGRATION_SEEN: &str = "migration_seen";
const KEY_GEMMA_TOS_ACCEPTED: &str = "gemma_tos_accepted";

// Legacy keys we still read for migration but no longer write.
const KEY_LEGACY_CLEANUP_ENABLED: &str = "cleanup_enabled";
const KEY_LEGACY_SEND_AUDIO_CLOUD: &str = "send_audio_to_cloud";

const KEYRING_SERVICE: &str = "app.vox.vox";
const KEYRING_ACCOUNT_GROQ: &str = "groq";
const KEYRING_ACCOUNT_ANTHROPIC: &str = "anthropic";

fn default_hotkey() -> String {
    "AltRight".to_string()
}
fn default_cancel_hotkey() -> String {
    "Escape".to_string()
}
fn default_paste_last_hotkey() -> String {
    "Alt+Shift+V".to_string()
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
fn default_stt_model_id() -> String {
    models::recommended_stt_model_id().to_string()
}
fn default_llm_model_id() -> String {
    models::recommended_llm_model_id().to_string()
}

/// User-visible settings. API keys live in `Settings` so the UI can edit
/// them, but they are not persisted to disk — only to the Keychain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub groq_api_key: String,
    #[serde(default)]
    pub anthropic_api_key: String,

    #[serde(default = "default_hotkey")]
    pub hotkey: String,
    #[serde(default = "default_cancel_hotkey")]
    pub cancel_hotkey: String,
    #[serde(default = "default_paste_last_hotkey")]
    pub paste_last_hotkey: String,

    #[serde(default)]
    pub launch_at_login: bool,
    #[serde(default)]
    pub play_sound: bool,
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
    #[serde(default)]
    pub migration_seen: bool,
    #[serde(default)]
    pub gemma_tos_accepted: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            groq_api_key: String::new(),
            anthropic_api_key: String::new(),
            hotkey: default_hotkey(),
            cancel_hotkey: default_cancel_hotkey(),
            paste_last_hotkey: default_paste_last_hotkey(),
            launch_at_login: false,
            play_sound: false,
            dictation_enabled: true,
            hands_free_enabled: false,
            language: default_language(),
            onboarded: false,
            dictionary: Vec::new(),
            stt_engine: default_stt_engine(),
            stt_model_id: default_stt_model_id(),
            llm_engine: default_llm_engine(),
            llm_model_id: default_llm_model_id(),
            migration_seen: false,
            gemma_tos_accepted: false,
        }
    }
}

// ---------- Keyring wrappers ------------------------------------------------

fn keyring_entry(account: &str) -> Option<keyring::Entry> {
    match keyring::Entry::new(KEYRING_SERVICE, account) {
        Ok(e) => Some(e),
        Err(e) => {
            log::warn!("keyring entry create failed ({account}): {e}");
            None
        }
    }
}

fn keyring_get(account: &str) -> Option<String> {
    let entry = keyring_entry(account)?;
    match entry.get_password() {
        Ok(s) => Some(s),
        Err(keyring::Error::NoEntry) => None,
        Err(e) => {
            log::warn!("keyring get failed ({account}): {e}");
            None
        }
    }
}

fn keyring_set(account: &str, value: &str) -> bool {
    let Some(entry) = keyring_entry(account) else {
        return false;
    };
    if value.is_empty() {
        match entry.delete_credential() {
            Ok(_) => true,
            Err(keyring::Error::NoEntry) => true,
            Err(e) => {
                log::warn!("keyring delete failed ({account}): {e}");
                false
            }
        }
    } else {
        match entry.set_password(value) {
            Ok(_) => true,
            Err(e) => {
                log::warn!("keyring set failed ({account}): {e}");
                false
            }
        }
    }
}

// ---------- Public single-key helpers --------------------------------------

pub fn get_groq_key<R: Runtime>(app: &AppHandle<R>) -> String {
    if let Some(s) = keyring_get(KEYRING_ACCOUNT_GROQ) {
        return s;
    }
    read_string(app, KEY_GROQ)
}

pub fn set_groq_key<R: Runtime>(app: &AppHandle<R>, value: &str) {
    if keyring_set(KEYRING_ACCOUNT_GROQ, value) {
        clear_string(app, KEY_GROQ);
    } else {
        write_string(app, KEY_GROQ, value);
    }
}

pub fn get_anthropic_key<R: Runtime>(app: &AppHandle<R>) -> String {
    if let Some(s) = keyring_get(KEYRING_ACCOUNT_ANTHROPIC) {
        return s;
    }
    read_string(app, KEY_ANTHROPIC)
}

pub fn set_anthropic_key<R: Runtime>(app: &AppHandle<R>, value: &str) {
    if keyring_set(KEYRING_ACCOUNT_ANTHROPIC, value) {
        clear_string(app, KEY_ANTHROPIC);
    } else {
        write_string(app, KEY_ANTHROPIC, value);
    }
}

// ---------- Primitive store helpers ----------------------------------------

fn read_string<R: Runtime>(app: &AppHandle<R>, key: &str) -> String {
    app.store(STORE_FILE)
        .ok()
        .and_then(|s| s.get(key))
        .and_then(|v| v.as_str().map(|x| x.to_string()))
        .unwrap_or_default()
}

fn write_string<R: Runtime>(app: &AppHandle<R>, key: &str, value: &str) {
    if let Ok(store) = app.store(STORE_FILE) {
        store.set(key, serde_json::Value::String(value.to_string()));
        let _ = store.save();
    }
}

fn clear_string<R: Runtime>(app: &AppHandle<R>, key: &str) {
    if let Ok(store) = app.store(STORE_FILE) {
        store.delete(key);
        let _ = store.save();
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

    let hotkey = store
        .get(KEY_HOTKEY)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(default_hotkey);
    let cancel_hotkey = store
        .get(KEY_CANCEL_HOTKEY)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(default_cancel_hotkey);
    let paste_last_hotkey = store
        .get(KEY_PASTE_LAST_HOTKEY)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(default_paste_last_hotkey);
    let language = store
        .get(KEY_LANGUAGE)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(default_language);

    let get_bool = |k: &str, d: bool| store.get(k).and_then(|v| v.as_bool()).unwrap_or(d);

    let launch_at_login = get_bool(KEY_LAUNCH_AT_LOGIN, false);
    let play_sound = get_bool(KEY_PLAY_SOUND, false);
    let dictation_enabled = get_bool(KEY_DICTATION_ENABLED, true);
    let hands_free_enabled = get_bool(KEY_HANDS_FREE, false);
    let onboarded = get_bool(KEY_ONBOARDED, false);
    let migration_seen = get_bool(KEY_MIGRATION_SEEN, false);
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

    // Engine selection. New keys override; if absent, fall back to a
    // migration of the legacy `cleanup_enabled` / `send_audio_to_cloud`
    // pair.
    let legacy_cleanup_enabled =
        store.get(KEY_LEGACY_CLEANUP_ENABLED).and_then(|v| v.as_bool());
    let legacy_send_audio_cloud =
        store.get(KEY_LEGACY_SEND_AUDIO_CLOUD).and_then(|v| v.as_bool());

    let stt_engine = store
        .get(KEY_STT_ENGINE)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| {
            // Existing user: had cloud STT. Brand-new: default local.
            if legacy_send_audio_cloud.is_some() && !get_groq_key(app).is_empty() {
                "groq".into()
            } else {
                default_stt_engine()
            }
        });
    let stt_model_id = store
        .get(KEY_STT_MODEL_ID)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(default_stt_model_id);

    let llm_engine = store
        .get(KEY_LLM_ENGINE)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| {
            if legacy_cleanup_enabled == Some(false) {
                "none".into()
            } else if !get_anthropic_key(app).is_empty() {
                "anthropic".into()
            } else if legacy_send_audio_cloud.is_some() {
                // Existing cloud user without an Anthropic key: cleanup off.
                "none".into()
            } else {
                default_llm_engine()
            }
        });
    let llm_model_id = store
        .get(KEY_LLM_MODEL_ID)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(default_llm_model_id);

    // Migrate plaintext keys → Keychain once.
    migrate_key_from_store(app, &store, KEY_GROQ, KEYRING_ACCOUNT_GROQ);
    migrate_key_from_store(app, &store, KEY_ANTHROPIC, KEYRING_ACCOUNT_ANTHROPIC);

    let groq_api_key = get_groq_key(app);
    let anthropic_api_key = get_anthropic_key(app);

    Settings {
        groq_api_key,
        anthropic_api_key,
        hotkey,
        cancel_hotkey,
        paste_last_hotkey,
        launch_at_login,
        play_sound,
        dictation_enabled,
        hands_free_enabled,
        language,
        onboarded,
        dictionary,
        stt_engine,
        stt_model_id,
        llm_engine,
        llm_model_id,
        migration_seen,
        gemma_tos_accepted,
    }
}

fn migrate_key_from_store<R: Runtime>(
    app: &AppHandle<R>,
    store: &std::sync::Arc<tauri_plugin_store::Store<R>>,
    store_key: &str,
    account: &str,
) {
    let plaintext = store
        .get(store_key)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_default();

    if plaintext.is_empty() {
        return;
    }
    if keyring_set(account, &plaintext) {
        log::info!("migrated {store_key} from plaintext store → Keychain");
        clear_string(app, store_key);
    } else {
        log::warn!(
            "could not migrate {store_key} to Keychain — staying on plaintext store"
        );
    }
}

pub fn save<R: Runtime>(app: &AppHandle<R>, settings: &Settings) -> anyhow::Result<()> {
    let store = app
        .store(STORE_FILE)
        .map_err(|e| anyhow::anyhow!("open store: {e}"))?;

    // Secrets → Keychain.
    let groq = settings.groq_api_key.trim();
    if !groq.is_empty() || get_groq_key(app).is_empty() {
        set_groq_key(app, groq);
    }
    let anthropic = settings.anthropic_api_key.trim();
    if !anthropic.is_empty() || get_anthropic_key(app).is_empty() {
        set_anthropic_key(app, anthropic);
    }

    store.set(
        KEY_HOTKEY,
        serde_json::Value::String(settings.hotkey.clone()),
    );
    store.set(
        KEY_CANCEL_HOTKEY,
        serde_json::Value::String(settings.cancel_hotkey.clone()),
    );
    store.set(
        KEY_PASTE_LAST_HOTKEY,
        serde_json::Value::String(settings.paste_last_hotkey.clone()),
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
    store.set(KEY_PLAY_SOUND, serde_json::Value::Bool(settings.play_sound));
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
        KEY_MIGRATION_SEEN,
        serde_json::Value::Bool(settings.migration_seen),
    );
    store.set(
        KEY_GEMMA_TOS_ACCEPTED,
        serde_json::Value::Bool(settings.gemma_tos_accepted),
    );

    // Drop the deprecated keys on save — they're superseded.
    store.delete(KEY_LEGACY_CLEANUP_ENABLED);
    store.delete(KEY_LEGACY_SEND_AUDIO_CLOUD);

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

pub fn mark_migration_seen<R: Runtime>(app: &AppHandle<R>) {
    if let Ok(store) = app.store(STORE_FILE) {
        store.set(KEY_MIGRATION_SEEN, serde_json::Value::Bool(true));
        let _ = store.save();
    }
}

/// True if a previous Vox install on this machine had cloud config. Used
/// to decide whether to surface the soft migration dialog.
pub fn is_legacy_cloud_user<R: Runtime>(app: &AppHandle<R>) -> bool {
    if get_groq_key(app).trim().is_empty() {
        return false;
    }
    let Ok(store) = app.store(STORE_FILE) else {
        return false;
    };
    // If the user already has explicit local-first keys, we treat them as
    // already-migrated regardless of the legacy bool.
    if store.get(KEY_STT_ENGINE).is_some() {
        return false;
    }
    true
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
    }
}
