//! Settings — backed by `tauri-plugin-store` for non-secret fields,
//! and the macOS Keychain (via the `keyring` crate) for API keys.
//!
//! On startup, [`load`] migrates any plaintext keys out of the JSON store
//! into the Keychain and clears them from disk. If Keychain access fails,
//! we transparently fall back to the plaintext store and log a warning —
//! dictation should still work.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime};
use tauri_plugin_store::StoreExt;

pub const STORE_FILE: &str = "vox-settings.json";

const KEY_GROQ: &str = "groq_api_key";
const KEY_ANTHROPIC: &str = "anthropic_api_key";
const KEY_HOTKEY: &str = "hotkey";
const KEY_CANCEL_HOTKEY: &str = "cancel_hotkey";
const KEY_CLEANUP_ENABLED: &str = "cleanup_enabled";
const KEY_LAUNCH_AT_LOGIN: &str = "launch_at_login";
const KEY_PLAY_SOUND: &str = "play_sound";
const KEY_SEND_AUDIO_CLOUD: &str = "send_audio_to_cloud";
const KEY_ONBOARDED: &str = "onboarded";
const KEY_DICTATION_ENABLED: &str = "dictation_enabled";
const KEY_DICTIONARY: &str = "dictionary";

const KEYRING_SERVICE: &str = "app.vox.vox";
const KEYRING_ACCOUNT_GROQ: &str = "groq";
const KEYRING_ACCOUNT_ANTHROPIC: &str = "anthropic";

fn default_hotkey() -> String {
    "AltRight".to_string()
}
fn default_cancel_hotkey() -> String {
    "Escape".to_string()
}
fn default_true() -> bool {
    true
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

    #[serde(default = "default_true")]
    pub cleanup_enabled: bool,
    #[serde(default)]
    pub launch_at_login: bool,
    #[serde(default)]
    pub play_sound: bool,
    #[serde(default = "default_true")]
    pub send_audio_to_cloud: bool,
    #[serde(default = "default_true")]
    pub dictation_enabled: bool,

    #[serde(default)]
    pub onboarded: bool,

    #[serde(default)]
    pub dictionary: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            groq_api_key: String::new(),
            anthropic_api_key: String::new(),
            hotkey: default_hotkey(),
            cancel_hotkey: default_cancel_hotkey(),
            cleanup_enabled: true,
            launch_at_login: false,
            play_sound: false,
            send_audio_to_cloud: true,
            dictation_enabled: true,
            onboarded: false,
            dictionary: Vec::new(),
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
        // Delete the entry on empty.
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
    // Fallback: plaintext store.
    read_string(app, KEY_GROQ)
}

pub fn set_groq_key<R: Runtime>(app: &AppHandle<R>, value: &str) {
    if keyring_set(KEYRING_ACCOUNT_GROQ, value) {
        // Clear any plaintext copy.
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

    let get_bool = |k: &str, d: bool| store.get(k).and_then(|v| v.as_bool()).unwrap_or(d);

    let cleanup_enabled = get_bool(KEY_CLEANUP_ENABLED, true);
    let launch_at_login = get_bool(KEY_LAUNCH_AT_LOGIN, false);
    let play_sound = get_bool(KEY_PLAY_SOUND, false);
    let send_audio_to_cloud = get_bool(KEY_SEND_AUDIO_CLOUD, true);
    let dictation_enabled = get_bool(KEY_DICTATION_ENABLED, true);
    let onboarded = get_bool(KEY_ONBOARDED, false);

    let dictionary = store
        .get(KEY_DICTIONARY)
        .and_then(|v| v.as_array().cloned())
        .map(|arr| {
            arr.into_iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

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
        cleanup_enabled,
        launch_at_login,
        play_sound,
        send_audio_to_cloud,
        dictation_enabled,
        onboarded,
        dictionary,
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

    // Secrets → Keychain. Refuse to wipe a stored key if the incoming value
    // is empty — empty almost always means a UI race or an out-of-date
    // snapshot, never a real "the user typed nothing on purpose."
    let groq = settings.groq_api_key.trim();
    if !groq.is_empty() || get_groq_key(app).is_empty() {
        set_groq_key(app, groq);
    }
    let anthropic = settings.anthropic_api_key.trim();
    if !anthropic.is_empty() || get_anthropic_key(app).is_empty() {
        set_anthropic_key(app, anthropic);
    }

    // Non-secret fields.
    store.set(
        KEY_HOTKEY,
        serde_json::Value::String(settings.hotkey.clone()),
    );
    store.set(
        KEY_CANCEL_HOTKEY,
        serde_json::Value::String(settings.cancel_hotkey.clone()),
    );
    store.set(
        KEY_CLEANUP_ENABLED,
        serde_json::Value::Bool(settings.cleanup_enabled),
    );
    store.set(
        KEY_LAUNCH_AT_LOGIN,
        serde_json::Value::Bool(settings.launch_at_login),
    );
    store.set(KEY_PLAY_SOUND, serde_json::Value::Bool(settings.play_sound));
    store.set(
        KEY_SEND_AUDIO_CLOUD,
        serde_json::Value::Bool(settings.send_audio_to_cloud),
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
    pub cleanup_enabled: bool,
    pub send_audio_to_cloud: bool,
    pub dictation_enabled: bool,
    pub dictionary: Vec<String>,
}

pub fn prefs<R: Runtime>(app: &AppHandle<R>) -> Prefs {
    let Ok(store) = app.store(STORE_FILE) else {
        return Prefs {
            cleanup_enabled: true,
            send_audio_to_cloud: true,
            dictation_enabled: true,
            dictionary: Vec::new(),
        };
    };

    let get_bool = |k: &str, d: bool| store.get(k).and_then(|v| v.as_bool()).unwrap_or(d);

    let dictionary = store
        .get(KEY_DICTIONARY)
        .and_then(|v| v.as_array().cloned())
        .map(|arr| {
            arr.into_iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Prefs {
        cleanup_enabled: get_bool(KEY_CLEANUP_ENABLED, true),
        send_audio_to_cloud: get_bool(KEY_SEND_AUDIO_CLOUD, true),
        dictation_enabled: get_bool(KEY_DICTATION_ENABLED, true),
        dictionary,
    }
}
