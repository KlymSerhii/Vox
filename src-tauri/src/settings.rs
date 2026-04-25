use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime};
use tauri_plugin_store::StoreExt;

const STORE_FILE: &str = "vox-settings.json";
const KEY_GROQ: &str = "groq_api_key";
const KEY_ANTHROPIC: &str = "anthropic_api_key";
const KEY_HOTKEY: &str = "hotkey";

// TODO (Phase 2): migrate secret keys to macOS Keychain via the `keyring` crate.
// For now they live in a plain JSON store on disk.

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Settings {
    #[serde(default)]
    pub groq_api_key: String,
    #[serde(default)]
    pub anthropic_api_key: String,
    #[serde(default = "default_hotkey")]
    pub hotkey: String,
}

fn default_hotkey() -> String {
    "AltRight".to_string()
}

pub fn load<R: Runtime>(app: &AppHandle<R>) -> Settings {
    let store = match app.store(STORE_FILE) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("failed to open settings store: {e}");
            return Settings::default();
        }
    };

    let groq = store
        .get(KEY_GROQ)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_default();
    let anthropic = store
        .get(KEY_ANTHROPIC)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_default();
    let hotkey = store
        .get(KEY_HOTKEY)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(default_hotkey);

    Settings {
        groq_api_key: groq,
        anthropic_api_key: anthropic,
        hotkey,
    }
}

pub fn save<R: Runtime>(app: &AppHandle<R>, settings: &Settings) -> anyhow::Result<()> {
    let store = app
        .store(STORE_FILE)
        .map_err(|e| anyhow::anyhow!("open store: {e}"))?;
    store.set(
        KEY_GROQ,
        serde_json::Value::String(settings.groq_api_key.clone()),
    );
    store.set(
        KEY_ANTHROPIC,
        serde_json::Value::String(settings.anthropic_api_key.clone()),
    );
    store.set(
        KEY_HOTKEY,
        serde_json::Value::String(settings.hotkey.clone()),
    );
    store
        .save()
        .map_err(|e| anyhow::anyhow!("save store: {e}"))?;
    Ok(())
}
