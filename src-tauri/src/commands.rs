use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime, Webview, Wry};
use tauri_plugin_autostart::ManagerExt as _;

use crate::engines::{anthropic, groq};
use crate::{models, onboarding, pill, session, settings, transcripts};

#[derive(Debug, Serialize, Deserialize)]
pub struct TestConnectionResult {
    pub ok: bool,
    pub message: String,
}

const SECRET_WINDOWS: &[&str] = &["settings", "onboarding", "migration"];

fn caller_label<R: Runtime>(webview: &Webview<R>) -> String {
    webview.label().to_string()
}

fn assert_secret_window<R: Runtime>(webview: &Webview<R>) -> Result<(), String> {
    let label = caller_label(webview);
    if SECRET_WINDOWS.iter().any(|w| *w == label) {
        Ok(())
    } else {
        Err(format!(
            "command not available from window '{label}' — secrets are scoped to settings/onboarding/migration"
        ))
    }
}

#[tauri::command]
pub fn get_settings<R: Runtime>(
    app: AppHandle<R>,
    webview: Webview<R>,
) -> Result<settings::Settings, String> {
    assert_secret_window(&webview)?;
    Ok(settings::load(&app))
}

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
    match groq::test_key(&api_key).await {
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
    match anthropic::test_key(&api_key).await {
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

#[tauri::command]
pub fn show_tray_popover<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    crate::tray::show_popover(&app).map_err(|e| e.to_string())
}

// -------- Launch-at-login ----------------------------------------------------

#[tauri::command]
pub fn get_launch_at_login<R: Runtime>(app: AppHandle<R>) -> Result<bool, String> {
    let mgr = app.autolaunch();
    mgr.is_enabled().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_launch_at_login<R: Runtime>(
    app: AppHandle<R>,
    enabled: bool,
) -> Result<(), String> {
    let mgr = app.autolaunch();
    if enabled {
        mgr.enable().map_err(|e| e.to_string())?;
    } else {
        mgr.disable().map_err(|e| e.to_string())?;
    }
    let mut s = settings::load(&app);
    s.launch_at_login = enabled;
    let _ = settings::save(&app, &s);
    Ok(())
}

// -------- Paste-last-transcript ----------------------------------------------

#[tauri::command]
pub fn paste_last_transcript<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    if let Some(ctx) = app.try_state::<session::SessionCtx>() {
        if ctx.is_pipeline_active() {
            return Err("Vox is busy with another dictation.".into());
        }
    }

    let snap = transcripts::snapshot();
    let Some(latest) = snap.first() else {
        pill::show(&app);
        pill::emit_state(
            &app,
            pill::PillState::Error,
            Some("No recent transcript yet.".into()),
        );
        let app_arc = std::sync::Arc::new(app.clone());
        pill::hide_after(&app_arc, 2000);
        return Err("No recent transcript.".into());
    };

    let text = latest.text.clone();
    tauri::async_runtime::spawn(async move {
        let result = tauri::async_runtime::spawn_blocking(move || {
            crate::inject::paste_text(&text)
        })
        .await;

        match result {
            Ok(Ok(())) => {
                pill::show(&app);
                pill::emit_state(&app, pill::PillState::Done, None);
                let app_arc = std::sync::Arc::new(app.clone());
                pill::hide_after(&app_arc, 700);
            }
            Ok(Err(e)) => {
                log::error!("paste-last failed: {e}");
                pill::show(&app);
                pill::emit_state(
                    &app,
                    pill::PillState::Error,
                    Some("Copied — press Cmd+V to paste.".into()),
                );
                let app_arc = std::sync::Arc::new(app.clone());
                pill::hide_after(&app_arc, 3000);
            }
            Err(e) => {
                log::error!("paste-last task panicked: {e}");
            }
        }
    });
    Ok(())
}

// -------- Auto-update --------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct UpdateCheckResult {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[tauri::command]
pub async fn check_for_updates<R: Runtime>(app: AppHandle<R>) -> UpdateCheckResult {
    use tauri_plugin_updater::UpdaterExt as _;

    let cur = app.package_info().version.to_string();

    let updater = match app.updater() {
        Ok(u) => u,
        Err(e) => {
            return UpdateCheckResult {
                status: "unconfigured".into(),
                current_version: Some(cur),
                available_version: None,
                message: Some(format!("Auto-update not yet configured ({e}).")),
            };
        }
    };

    match updater.check().await {
        Ok(Some(update)) => UpdateCheckResult {
            status: "available".into(),
            current_version: Some(cur),
            available_version: Some(update.version.clone()),
            message: Some(format!("Vox {} is available.", update.version)),
        },
        Ok(None) => UpdateCheckResult {
            status: "up-to-date".into(),
            current_version: Some(cur),
            available_version: None,
            message: Some("Vox is up to date.".into()),
        },
        Err(e) => UpdateCheckResult {
            status: "error".into(),
            current_version: Some(cur),
            available_version: None,
            message: Some(format!("Update check failed: {e}")),
        },
    }
}

// -------- Models tab ---------------------------------------------------------

#[tauri::command]
pub fn list_models(app: AppHandle<Wry>) -> Vec<models::ModelStatus> {
    models::list(&app)
}

#[tauri::command]
pub async fn download_model(app: AppHandle<Wry>, id: String) -> Result<(), String> {
    models::download_model(app, id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn cancel_download(id: String) -> bool {
    models::cancel_download(&id)
}

#[tauri::command]
pub fn delete_model(app: AppHandle<Wry>, id: String) -> Result<(), String> {
    models::delete_model(&app, &id).map_err(|e| e.to_string())
}

/// Pre-warm an engine into RAM. Used by onboarding's "Set up local
/// models" step so the first dictation doesn't pay the cold-load tax.
#[tauri::command]
pub async fn load_model_warm(app: AppHandle<Wry>, id: String) -> Result<(), String> {
    let descriptor = models::descriptor_for(&id)
        .ok_or_else(|| format!("unknown model id: {id}"))?;
    let registry = app
        .try_state::<crate::engines::Registry>()
        .ok_or_else(|| "engine registry missing".to_string())?;
    match descriptor.kind {
        models::ModelKind::Stt => {
            let _ = registry.stt(&app, descriptor.engine, descriptor.id);
        }
        models::ModelKind::Llm => {
            let _ = registry.llm(&app, descriptor.engine, descriptor.id);
        }
    }
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct TierRecommendation {
    pub tier: String,
    pub stt_model_id: String,
    pub llm_model_id: String,
    pub physical_ram_gb: u64,
    pub low_ram_warning: bool,
}

#[tauri::command]
pub fn recommend_tier() -> TierRecommendation {
    let ram = models::physical_ram_gb();
    let tier = models::recommend_tier();
    TierRecommendation {
        tier: tier.to_string(),
        stt_model_id: models::recommended_stt_model_id().to_string(),
        llm_model_id: models::recommended_llm_model_id().to_string(),
        physical_ram_gb: ram,
        low_ram_warning: ram > 0 && ram < 12,
    }
}

#[tauri::command]
pub fn accept_gemma_tos<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    let mut s = settings::load(&app);
    s.gemma_tos_accepted = true;
    settings::save(&app, &s).map_err(|e| e.to_string())
}

/// Persist the migration dialog dismissal. Frontend should patch
/// `stt_engine` / `llm_engine` via `set_settings` *before* calling this
/// — we just flip the seen flag here.
#[tauri::command]
pub fn dismiss_migration<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    settings::mark_migration_seen(&app);
    if let Some(win) = app.get_webview_window(crate::migration::LABEL) {
        let _ = win.hide();
    }
    Ok(())
}
