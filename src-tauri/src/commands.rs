use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime, Webview, Wry};
use tauri_plugin_autostart::ManagerExt as _;

use crate::engines::{anthropic, groq};
use crate::modes::{self, VoiceMode};
use crate::{frontmost, models, onboarding, pill, session, settings, transcripts};

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

/// Non-prompting microphone status probe. Used by the onboarding poller
/// (every 2 s) and by the tray icon at startup, so it MUST be safe to
/// call repeatedly without re-prompting and must never escalate from a
/// "check" into a "request".
///
/// On macOS we use AVFoundation's
/// `AVCaptureDevice.authorizationStatusForMediaType:`, which returns the
/// cached TCC state without showing the consent dialog. Crucially we do
/// NOT touch cpal here: opening (or sometimes even configuring) a CPAL
/// input device on macOS 14+ implicitly trips the TCC microphone gate
/// and shows the prompt — exactly what callers of this function must
/// avoid.
fn probe_microphone() -> bool {
    #[cfg(target_os = "macos")]
    {
        crate::mac_perm::authorization_status().is_granted()
    }
    #[cfg(not(target_os = "macos"))]
    {
        // No TCC on Linux/Windows; treat as always granted at the
        // permissions layer. Real device errors surface during capture.
        true
    }
}

/// Explicitly request microphone access. This is the ONLY entry point
/// allowed to show the macOS TCC dialog; it is wired up to the
/// "Allow microphone" button on the onboarding mic step.
///
/// Returns `true` if access ends up granted (either freshly granted by
/// the user, or already granted from a prior decision).
#[tauri::command]
pub fn request_microphone() -> bool {
    #[cfg(target_os = "macos")]
    {
        log::info!(
            "commands::request_microphone invoked — about to call \
             AVCaptureDevice requestAccessForMediaType:"
        );
        crate::mac_perm::request_access_blocking()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
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
pub fn finish_onboarding<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    // Logged at error level so it's visible regardless of VOX_LOG/RUST_LOG.
    log::error!("onboarding: finish_onboarding IPC entered");
    settings::mark_onboarded(&app);
    if let Some(win) = app.get_webview_window(onboarding::LABEL) {
        // Use `destroy()` to bypass any close-event handlers and tear the
        // window down unconditionally. `close()` can race with rebuilds
        // and silently no-op in dev when the window handle is stale.
        if let Err(e) = win.destroy() {
            log::error!("onboarding: destroy failed: {e}");
            return Err(e.to_string());
        }
        log::error!("onboarding: window destroyed");
    } else {
        log::error!("onboarding: finish called but window not found");
    }
    Ok(())
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

    // Stub-config short-circuit. The released bundles ship with the
    // placeholder endpoint `https://updates.vox.local/...` and an empty
    // pubkey until signing is wired up (see
    // `docs/signing-and-notarization.md`). In that state we MUST NOT
    // make a real network request: the placeholder host won't resolve
    // (5-15 s DNS timeout, leaks "Vox is running" to anyone watching
    // the resolver), and an empty pubkey + a real GET would have no
    // signature pin to fail-closed against if the endpoint config is
    // ever flipped to a hostile host.
    if updater_is_stub(&app) {
        return UpdateCheckResult {
            status: "unconfigured".into(),
            current_version: Some(cur),
            available_version: None,
            message: Some("Auto-update is not yet configured for this build.".into()),
        };
    }

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

/// True if the running config is the documented stub (placeholder host
/// or empty pubkey). When this is the case `check_for_updates` returns
/// `"unconfigured"` *without* hitting the network.
fn updater_is_stub<R: Runtime>(app: &AppHandle<R>) -> bool {
    let cfg = app.config();
    // Walk the plugin config raw — we don't depend on a typed view since
    // `tauri_plugin_updater::Config` shape may shift.
    let plugins = match cfg.plugins.0.get("updater") {
        Some(v) => v,
        None => return true, // no config at all → stub
    };
    let pubkey = plugins
        .get("pubkey")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if pubkey.trim().is_empty() {
        return true;
    }
    let endpoints = plugins.get("endpoints").and_then(|v| v.as_array());
    let Some(endpoints) = endpoints else {
        return true;
    };
    if endpoints.is_empty() {
        return true;
    }
    // Any endpoint pointing at the placeholder host counts as stub.
    endpoints.iter().any(|e| {
        e.as_str()
            .map(|s| s.contains("updates.vox.local") || s.contains("placeholder"))
            .unwrap_or(false)
    })
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

// -------- Voice Modes --------------------------------------------------------

#[tauri::command]
pub fn list_modes<R: Runtime>(app: AppHandle<R>) -> Vec<VoiceMode> {
    modes::list_modes(&app)
}

#[tauri::command]
pub fn get_mode<R: Runtime>(app: AppHandle<R>, id: String) -> Option<VoiceMode> {
    modes::get_mode(&app, &id)
}

#[tauri::command]
pub fn save_mode<R: Runtime>(
    app: AppHandle<R>,
    webview: Webview<R>,
    mode: VoiceMode,
) -> Result<(), String> {
    assert_secret_window(&webview)?;
    modes::save_mode(&app, &mode).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_mode<R: Runtime>(
    app: AppHandle<R>,
    webview: Webview<R>,
    id: String,
) -> Result<(), String> {
    assert_secret_window(&webview)?;
    modes::delete_mode(&app, &id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn duplicate_mode<R: Runtime>(
    app: AppHandle<R>,
    webview: Webview<R>,
    id: String,
) -> Result<VoiceMode, String> {
    assert_secret_window(&webview)?;
    modes::duplicate_mode(&app, &id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_active_mode<R: Runtime>(app: AppHandle<R>, id: String) -> Result<VoiceMode, String> {
    let mode = modes::get_mode(&app, &id).ok_or_else(|| "unknown mode id".to_string())?;
    settings::set_active_mode_id(&app, &mode.id).map_err(|e| e.to_string())?;
    Ok(mode)
}

#[tauri::command]
pub fn export_mode<R: Runtime>(app: AppHandle<R>, id: String) -> Result<String, String> {
    modes::export_mode(&app, &id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn import_mode_json<R: Runtime>(
    app: AppHandle<R>,
    webview: Webview<R>,
    json: String,
) -> Result<VoiceMode, String> {
    assert_secret_window(&webview)?;
    modes::import_mode_json(&app, &json).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn reset_default_modes<R: Runtime>(
    app: AppHandle<R>,
    webview: Webview<R>,
) -> Result<usize, String> {
    assert_secret_window(&webview)?;
    modes::reset_defaults(&app).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn cycle_active_mode<R: Runtime>(app: AppHandle<R>) -> Result<VoiceMode, String> {
    let s = settings::load(&app);
    let next = modes::cycle_active(&app, &s.active_mode_id)
        .ok_or_else(|| "no modes available".to_string())?;
    settings::set_active_mode_id(&app, &next.id).map_err(|e| e.to_string())?;
    Ok(next)
}

#[tauri::command]
pub fn suggest_active_mode<R: Runtime>(app: AppHandle<R>) -> Option<String> {
    let (bundle_id, title) = frontmost::detect()?;
    modes::suggest_mode_for(&app, Some(&bundle_id), title.as_deref())
}
