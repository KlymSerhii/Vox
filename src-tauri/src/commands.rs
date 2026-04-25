use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime, Webview};

use crate::{llm, onboarding, pill, settings, stt, transcripts};

#[derive(Debug, Serialize, Deserialize)]
pub struct TestConnectionResult {
    pub ok: bool,
    pub message: String,
}

/// Windows that are allowed to read/write API keys via `get_settings` /
/// `set_settings`. Other windows (the pill, the tray popover) must use
/// `get_public_settings` instead, which strips secrets.
const SECRET_WINDOWS: &[&str] = &["settings", "onboarding"];

fn caller_label<R: Runtime>(webview: &Webview<R>) -> String {
    webview.label().to_string()
}

fn assert_secret_window<R: Runtime>(webview: &Webview<R>) -> Result<(), String> {
    let label = caller_label(webview);
    if SECRET_WINDOWS.iter().any(|w| *w == label) {
        Ok(())
    } else {
        Err(format!(
            "command not available from window '{label}' — secrets are scoped to settings/onboarding"
        ))
    }
}

/// Settings WITH API keys. Only callable from the settings + onboarding
/// windows. Always prefer `get_public_settings` from anywhere else.
#[tauri::command]
pub fn get_settings<R: Runtime>(
    app: AppHandle<R>,
    webview: Webview<R>,
) -> Result<settings::Settings, String> {
    assert_secret_window(&webview)?;
    Ok(settings::load(&app))
}

/// Secrets-stripped settings — safe to call from any window. The
/// `groq_api_key` and `anthropic_api_key` fields are reused as
/// non-empty/empty *flags*: `"set"` if a key exists, otherwise `""`.
/// This lets the tray popover show "Listening ready" vs "Add your Groq
/// key" without ever shipping the secret over IPC.
#[tauri::command]
pub fn get_public_settings<R: Runtime>(app: AppHandle<R>) -> settings::Settings {
    let mut s = settings::load(&app);
    s.groq_api_key = if s.groq_api_key.trim().is_empty() {
        String::new()
    } else {
        "set".to_string()
    };
    s.anthropic_api_key = if s.anthropic_api_key.trim().is_empty() {
        String::new()
    } else {
        "set".to_string()
    };
    s
}

#[tauri::command]
pub fn set_settings<R: Runtime>(
    app: AppHandle<R>,
    webview: Webview<R>,
    settings: settings::Settings,
) -> Result<(), String> {
    assert_secret_window(&webview)?;
    settings::save(&app, &settings).map_err(|e| e.to_string())
}

/// Allow non-secret windows (tray popover) to flip the dictation toggle and
/// nothing else. Pass-through for safety: ignores any keys/hotkeys/dictionary
/// the caller might smuggle in.
#[tauri::command]
pub fn set_dictation_enabled<R: Runtime>(
    app: AppHandle<R>,
    enabled: bool,
) -> Result<(), String> {
    let mut s = settings::load(&app);
    s.dictation_enabled = enabled;
    settings::save(&app, &s).map_err(|e| e.to_string())
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

#[tauri::command]
pub async fn test_anthropic_connection(api_key: String) -> TestConnectionResult {
    match llm::test_key(&api_key).await {
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

#[tauri::command]
pub fn get_recent_transcripts() -> Vec<transcripts::RecentTranscript> {
    transcripts::snapshot()
}

#[tauri::command]
pub fn open_settings<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("settings") {
        win.show().map_err(|e| e.to_string())?;
        win.set_focus().map_err(|e| e.to_string())?;
    }
    // Hide the tray popover if it's open.
    if let Some(win) = app.get_webview_window("tray-popover") {
        let _ = win.hide();
    }
    Ok(())
}

#[tauri::command]
pub fn close_tray_popover<R: Runtime>(app: AppHandle<R>) {
    if let Some(win) = app.get_webview_window("tray-popover") {
        let _ = win.hide();
    }
}

#[tauri::command]
pub fn quit_app<R: Runtime>(app: AppHandle<R>) {
    app.exit(0);
}

#[tauri::command]
pub fn open_accessibility_settings<R: Runtime>(_app: AppHandle<R>) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn open_microphone_settings<R: Runtime>(_app: AppHandle<R>) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone")
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct PermissionStatus {
    pub microphone: bool,
    pub accessibility: bool,
}

/// macOS permission check — cheap, repeatable.
/// - Microphone: we can't check TCC without triggering a prompt, so we probe
///   cpal's default input device. If we got samples through before, this will
///   succeed.
/// - Accessibility: uses `AXIsProcessTrusted`.
#[tauri::command]
pub fn check_permissions<R: Runtime>(_app: AppHandle<R>) -> PermissionStatus {
    let microphone = probe_microphone();
    let accessibility = probe_accessibility();
    PermissionStatus {
        microphone,
        accessibility,
    }
}

#[cfg(target_os = "macos")]
fn probe_accessibility() -> bool {
    extern "C" {
        fn AXIsProcessTrusted() -> bool;
    }
    unsafe { AXIsProcessTrusted() }
}

#[cfg(not(target_os = "macos"))]
fn probe_accessibility() -> bool {
    true
}

fn probe_microphone() -> bool {
    use cpal::traits::{DeviceTrait, HostTrait};
    match cpal::default_host().default_input_device() {
        Some(device) => device.default_input_config().is_ok(),
        None => false,
    }
}

/// Trigger the OS mic permission prompt by opening + immediately dropping
/// an input stream. Returns true if we were able to open one.
#[tauri::command]
pub fn request_microphone() -> bool {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let host = cpal::default_host();
    let Some(device) = host.default_input_device() else {
        return false;
    };
    let Ok(cfg) = device.default_input_config() else {
        return false;
    };
    let sample_format = cfg.sample_format();
    let config: cpal::StreamConfig = cfg.into();
    let err_fn = |err| log::warn!("mic probe stream error: {err}");
    let stream = match sample_format {
        cpal::SampleFormat::F32 => {
            device.build_input_stream(&config, move |_: &[f32], _| {}, err_fn, None)
        }
        cpal::SampleFormat::I16 => {
            device.build_input_stream(&config, move |_: &[i16], _| {}, err_fn, None)
        }
        cpal::SampleFormat::U16 => {
            device.build_input_stream(&config, move |_: &[u16], _| {}, err_fn, None)
        }
        _ => return false,
    };
    match stream {
        Ok(s) => {
            let _ = s.play();
            // Immediately drop.
            true
        }
        Err(e) => {
            log::warn!("mic probe failed: {e}");
            false
        }
    }
}

#[tauri::command]
pub fn show_pill<R: Runtime>(app: AppHandle<R>) {
    pill::show(&app);
}

#[tauri::command]
pub fn hide_pill<R: Runtime>(app: AppHandle<R>) {
    pill::hide(&app);
}

#[tauri::command]
pub fn replay_onboarding<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    onboarding::show(&app).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn finish_onboarding<R: Runtime>(app: AppHandle<R>) {
    settings::mark_onboarded(&app);
    if let Some(win) = app.get_webview_window(onboarding::LABEL) {
        let _ = win.hide();
    }
}

/// Anchor + show the tray popover next to the tray icon. The frontend
/// drives content; we just position it.
#[tauri::command]
pub fn show_tray_popover<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    crate::tray::show_popover(&app).map_err(|e| e.to_string())
}
