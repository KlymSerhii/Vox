use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime, Webview, Wry};
use tauri_plugin_autostart::ManagerExt as _;

use crate::hotkey::{ActiveHotkey, ActiveHotkeyState};
use crate::modes::{self, VoiceMode};
use crate::settings::HotkeyPurpose;
use crate::{frontmost, models, onboarding, pill, session, settings, transcripts};

// `TestConnectionResult` previously fronted the cloud key-test commands.
// Cloud engines have been removed (2026-04-26) — the type is no longer
// referenced but kept here as a small comment so future cloud experiments
// know where to plug back in.
#[derive(Debug, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct TestConnectionResult {
    pub ok: bool,
    pub message: String,
}

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
            "command not available from window '{label}' — restricted to settings/onboarding"
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
    // No cloud secrets to redact — local-only build.
    settings::load(&app)
}

#[tauri::command]
pub fn set_settings<R: Runtime>(
    app: AppHandle<R>,
    webview: Webview<R>,
    settings: settings::Settings,
) -> Result<(), String> {
    assert_secret_window(&webview)?;

    // Snapshot the previous values so we know whether any of the
    // hotkey-related fields actually changed. Re-binding global
    // shortcuts is cheap but not free (and on macOS it briefly
    // unregisters everything), so we only do it when necessary.
    let before = settings::load(&app);
    let hotkeys_changed = before.hotkey != settings.hotkey
        || before.cancel_hotkey != settings.cancel_hotkey
        || before.mode_cycle_hotkey != settings.mode_cycle_hotkey;

    settings::save(&app, &settings).map_err(|e| e.to_string())?;

    if hotkeys_changed {
        if let Some(ctx) = app.try_state::<session::SessionCtx>() {
            let ctx_clone = (*ctx).clone();
            match crate::hotkey::register(
                &app,
                &settings.hotkey,
                &settings.cancel_hotkey,
                &settings.mode_cycle_hotkey,
                ctx_clone,
            ) {
                Ok(resolved) => {
                    log::info!("hotkeys re-bound after settings save: {resolved}");
                }
                Err(e) => {
                    log::error!(
                        "hotkey re-bind failed after settings save: {e}. \
                         The previous bindings are gone — settings still saved."
                    );
                }
            }
        } else {
            log::warn!(
                "hotkeys changed but SessionCtx is not in app state; \
                 binding will pick up new values on next launch."
            );
        }
    }

    // Notify every other window so they can refresh anything cached
    // off the settings (mainly the displayed hotkey glyphs in the pill,
    // tray popover and onboarding).
    if let Err(e) = app.emit("settings:changed", &settings) {
        log::warn!("failed to emit settings:changed: {e}");
    }

    Ok(())
}

#[tauri::command]
pub fn set_dictation_enabled<R: Runtime>(
    app: AppHandle<R>,
    enabled: bool,
) -> Result<(), String> {
    let mut s = settings::load(&app);
    s.dictation_enabled = enabled;
    settings::save(&app, &s).map_err(|e| e.to_string())?;
    // Broadcast so the Settings window's General tab and the tray
    // popover's status line update without a manual refresh.
    if let Err(e) = app.emit("settings:changed", &s) {
        log::warn!("failed to emit settings:changed: {e}");
    }
    Ok(())
}

#[tauri::command]
pub fn set_hands_free_enabled<R: Runtime>(
    app: AppHandle<R>,
    enabled: bool,
) -> Result<(), String> {
    let mut s = settings::load(&app);
    s.hands_free_enabled = enabled;
    settings::save(&app, &s).map_err(|e| e.to_string())?;
    if let Err(e) = app.emit("settings:changed", &s) {
        log::warn!("failed to emit settings:changed: {e}");
    }
    Ok(())
}

#[tauri::command]
pub async fn copy_text(text: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || crate::inject::copy_to_clipboard(&text))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_polishing_enabled<R: Runtime>(
    app: AppHandle<R>,
    enabled: bool,
) -> Result<(), String> {
    let mut s = settings::load(&app);
    s.polishing_enabled = enabled;
    s.llm_engine = if enabled { "gemma-local".into() } else { "none".into() };
    settings::save(&app, &s).map_err(|e| e.to_string())?;
    if let Err(e) = app.emit("settings:changed", &s) {
        log::warn!("failed to emit settings:changed: {e}");
    }
    Ok(())
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

/// Reveal the absolute path of the currently-running binary so the user
/// can paste it into System Settings → Privacy & Security → Accessibility
/// (the "+" button on that pane accepts a file path via Cmd+Shift+G).
///
/// Returned struct is consumed by the Settings → About row in dev mode:
/// the UI copies `binary_path` to the clipboard and shows
/// `bundle_identifier` so the user can verify whether they're looking
/// at the dev binary (`vox-<hash>`) or the proper bundle (`com.vox.app`).
#[derive(Debug, Serialize)]
pub struct DevBinaryInfo {
    pub binary_path: String,
    pub bundle_identifier: String,
    pub is_dev_build: bool,
    pub accessibility_granted: bool,
}

#[tauri::command]
pub fn reveal_dev_binary_path<R: Runtime>(app: AppHandle<R>) -> DevBinaryInfo {
    let binary_path = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|e| format!("<unknown: {e}>"));
    let bundle_identifier = app
        .config()
        .identifier
        .clone();
    // A "dev build" is any binary whose path contains `target/debug` /
    // `target/release` AND whose parent isn't a `.app` bundle's MacOS
    // dir. The .app bundle path looks like
    // `<...>/target/debug/bundle/macos/Vox.app/Contents/MacOS/vox`,
    // which we want to treat as "real bundle, not dev".
    let is_dev_build = (binary_path.contains("/target/debug/")
        || binary_path.contains("/target/release/"))
        && !binary_path.contains(".app/Contents/MacOS/");
    DevBinaryInfo {
        binary_path,
        bundle_identifier,
        is_dev_build,
        accessibility_granted: probe_accessibility(),
    }
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
    // pubkey until signing + notarization are wired up. In that state
    // we MUST NOT make a real network request: the placeholder host
    // won't resolve
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

// -------- Hotkey diagnostics ------------------------------------------------

#[derive(Debug, Serialize)]
pub struct HotkeyDiagnostic {
    pub purpose: String,
    pub configured: String,
    pub bound: String,
    pub fell_back: bool,
    /// `"shortcut"`, `"chord"`, or `""` when nothing is bound.
    pub mechanism: String,
    pub error: Option<String>,
    pub error_chain: Vec<String>,
}

impl HotkeyDiagnostic {
    fn from_active(purpose: &str, a: ActiveHotkey) -> Self {
        Self {
            purpose: purpose.to_string(),
            configured: a.configured,
            bound: a.bound,
            fell_back: a.fell_back,
            mechanism: a.mechanism,
            error: a.error,
            error_chain: a.error_chain,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct DiagnoseHotkeys {
    pub entries: Vec<HotkeyDiagnostic>,
}

/// Snapshot of the push-to-talk binding for the UI to render. The
/// frontend uses this in onboarding and Settings to show the user the
/// truth about what's actually bound versus what was configured.
#[tauri::command]
pub fn get_active_hotkey<R: Runtime>(app: AppHandle<R>) -> ActiveHotkey {
    match app.try_state::<ActiveHotkeyState>() {
        Some(state) => state.snapshot(HotkeyPurpose::PushToTalk),
        None => ActiveHotkey {
            configured: settings::load(&app).hotkey,
            bound: String::new(),
            fell_back: false,
            mechanism: String::new(),
            error: Some("hotkey state not initialised".into()),
            error_chain: vec!["hotkey state not initialised".into()],
        },
    }
}

/// Snapshot of every hotkey binding. Used by the Settings → About
/// "Diagnostics" button to render a full picture in a `<pre>` block.
#[tauri::command]
pub fn diagnose_hotkeys<R: Runtime>(app: AppHandle<R>) -> DiagnoseHotkeys {
    let entries = match app.try_state::<ActiveHotkeyState>() {
        Some(state) => vec![
            HotkeyDiagnostic::from_active(
                "push-to-talk",
                state.snapshot(HotkeyPurpose::PushToTalk),
            ),
            HotkeyDiagnostic::from_active(
                "cancel",
                state.snapshot(HotkeyPurpose::Cancel),
            ),
            HotkeyDiagnostic::from_active(
                "mode-cycle",
                state.snapshot(HotkeyPurpose::ModeCycle),
            ),
        ],
        None => Vec::new(),
    };
    DiagnoseHotkeys { entries }
}
