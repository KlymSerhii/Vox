use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime};

use crate::{settings, stt};

#[derive(Debug, Serialize, Deserialize)]
pub struct TestConnectionResult {
    pub ok: bool,
    pub message: String,
}

#[tauri::command]
pub fn get_settings<R: Runtime>(app: AppHandle<R>) -> settings::Settings {
    settings::load(&app)
}

#[tauri::command]
pub fn set_settings<R: Runtime>(
    app: AppHandle<R>,
    settings: settings::Settings,
) -> Result<(), String> {
    settings::save(&app, &settings).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn test_groq_connection(api_key: String) -> TestConnectionResult {
    match stt::test_key(&api_key).await {
        Ok(_) => TestConnectionResult {
            ok: true,
            message: "OK — key accepted.".into(),
        },
        Err(e) => TestConnectionResult {
            ok: false,
            message: format!("Failed: {e}"),
        },
    }
}
