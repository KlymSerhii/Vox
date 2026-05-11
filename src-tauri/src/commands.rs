use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime, Webview, Wry};
use tauri_plugin_autostart::ManagerExt as _;

use crate::hotkey::{ActiveHotkey, ActiveHotkeyState};
use crate::modes::{self, VoiceMode};
use crate::session::SessionManager;
use crate::settings::HotkeyPurpose;
use crate::transcripts::SessionsSnapshot;
use crate::{
    frontmost, inference_proc, inference_server, models, onboarding, pill, settings, transcripts,
};

#[derive(Debug, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct TestConnectionResult {
    pub ok: bool,
    pub message: String,
}

const SECRET_WINDOWS: &[&str] = &["settings", "onboarding"];

fn assert_secret_window<R: Runtime>(webview: &Webview<R>) -> Result<(), String> {
    let label = webview.label();
    if SECRET_WINDOWS.contains(&label) {
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

/// Public projection of [`settings::Settings`] for the unrestricted
/// `get_public_settings` command. Mirrors the shape `tray-popover.tsx`
/// expects but redacts `inference_api_key` so a "public" command's
/// response never carries the loopback token. The full settings payload
/// (including the api key) is reachable only via `get_settings`, which
/// is gated to the privileged `settings`/`onboarding` windows.
///
/// The api key is loopback-only and not security-critical — but keeping
/// it out of the public command's response means a future window with
/// no business reading it can't accidentally pick it up.
#[tauri::command]
pub fn get_public_settings<R: Runtime>(app: AppHandle<R>) -> settings::Settings {
    let mut s = settings::load(&app);
    // Blank the api key for the public surface. Settings::save is never
    // called from this command, so the persisted value is preserved.
    s.inference_api_key = String::new();
    s
}

#[tauri::command]
pub fn set_settings(
    app: AppHandle<Wry>,
    webview: Webview<Wry>,
    settings: settings::Settings,
) -> Result<(), String> {
    assert_secret_window(&webview)?;

    // Snapshot before-values so we only re-bind hotkeys / restart engines
    // when the relevant fields actually changed. Engine lifecycle effects
    // run AFTER save so on-disk state matches what supervisors see.
    let before = settings::load(&app);
    let hotkeys_changed = before.hotkey != settings.hotkey
        || before.cancel_hotkey != settings.cancel_hotkey
        || before.mode_cycle_hotkey != settings.mode_cycle_hotkey;
    let appearance_changed = before.appearance != settings.appearance;
    let polish_off = before.polishing_enabled && !settings.polishing_enabled;
    let llm_left_gemma =
        before.llm_engine == "gemma-local" && settings.llm_engine != "gemma-local";
    let stt_changed = before.stt_engine != settings.stt_engine
        || before.stt_model_id != settings.stt_model_id;

    settings::save(&app, &settings).map_err(|e| e.to_string())?;

    if appearance_changed {
        let theme = if settings.appearance == "light" {
            Some(tauri::Theme::Light)
        } else {
            Some(tauri::Theme::Dark)
        };
        for label in ["settings", "onboarding"] {
            if let Some(win) = app.get_webview_window(label) {
                if let Err(e) = win.set_theme(theme) {
                    log::warn!("set_theme failed on '{label}': {e}");
                }
            }
        }
    }

    if hotkeys_changed {
        if let Some(mgr) = app.try_state::<Arc<SessionManager>>() {
            match crate::hotkey::register(
                &app,
                &settings.hotkey,
                &settings.cancel_hotkey,
                &settings.mode_cycle_hotkey,
                mgr.inner().clone(),
            ) {
                Ok(resolved) => log::info!("hotkeys re-bound after settings save: {resolved}"),
                Err(e) => log::error!(
                    "hotkey re-bind failed after settings save: {e}. \
                     The previous bindings are gone — settings still saved."
                ),
            }
        } else {
            log::warn!(
                "hotkeys changed but SessionManager is not in app state; \
                 binding will pick up new values on next launch."
            );
        }
    }

    if polish_off || llm_left_gemma {
        inference_proc::drop_managed(&app);
    }
    inference_proc::prewarm_in_background(&app);
    if stt_changed {
        crate::engines::prewarm_stt_in_background(&app);
    }

    if let Err(e) = app.emit("settings:changed", &settings) {
        log::warn!("failed to emit settings:changed: {e}");
    }

    Ok(())
}

#[tauri::command]
pub fn set_dictation_enabled(
    app: AppHandle<Wry>,
    enabled: bool,
) -> Result<(), String> {
    let mut s = settings::load(&app);
    s.dictation_enabled = enabled;
    settings::save(&app, &s).map_err(|e| e.to_string())?;
    if enabled {
        crate::engines::prewarm_stt_in_background(&app);
    }
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
    // Preserve the user's engine choice (apple-fm / gemma-local) across
    // the toggle: session.rs short-circuits to the no-op when
    // polishing_enabled is false, so we don't need to overwrite
    // llm_engine. Only fix up the edge case where engine was left as
    // "none" — pick gemma-local as the safe default when polishing is
    // being turned ON.
    if enabled && s.llm_engine == "none" {
        s.llm_engine = "gemma-local".into();
    }
    settings::save(&app, &s).map_err(|e| e.to_string())?;

    // Drive the inference subprocess in lockstep with the toggle so the
    // user sees the RAM change immediately, not lazily on next dictation.
    // `prewarm_in_background` is engine-aware — it no-ops on apple-fm.
    if enabled {
        inference_proc::prewarm_in_background(&app);
    } else {
        inference_proc::drop_managed(&app);
    }

    if let Err(e) = app.emit("settings:changed", &s) {
        log::warn!("failed to emit settings:changed: {e}");
    }
    Ok(())
}

#[tauri::command]
pub fn set_streaming_pill_enabled<R: Runtime>(
    app: AppHandle<R>,
    enabled: bool,
) -> Result<(), String> {
    let mut s = settings::load(&app);
    s.streaming_pill_enabled = enabled;
    settings::save(&app, &s).map_err(|e| e.to_string())?;
    if let Err(e) = app.emit("settings:changed", &s) {
        log::warn!("failed to emit settings:changed: {e}");
    }
    Ok(())
}

#[tauri::command]
pub fn set_enable_thinking<R: Runtime>(
    app: AppHandle<R>,
    enabled: bool,
) -> Result<(), String> {
    let mut s = settings::load(&app);
    if s.enable_thinking == enabled {
        return Ok(());
    }
    s.enable_thinking = enabled;
    settings::save(&app, &s).map_err(|e| e.to_string())?;
    if let Err(e) = app.emit("settings:changed", &s) {
        log::warn!("failed to emit settings:changed: {e}");
    }
    Ok(())
}

#[tauri::command]
pub fn set_appearance<R: Runtime>(
    app: AppHandle<R>,
    appearance: String,
) -> Result<(), String> {
    let theme = match appearance.as_str() {
        "light" => tauri::Theme::Light,
        "dark" => tauri::Theme::Dark,
        other => return Err(format!("invalid appearance: {other}")),
    };

    let mut s = settings::load(&app);
    s.appearance = appearance;
    settings::save(&app, &s).map_err(|e| e.to_string())?;

    for label in ["settings", "onboarding"] {
        if let Some(win) = app.get_webview_window(label) {
            if let Err(e) = win.set_theme(Some(theme)) {
                log::warn!("set_theme failed on '{label}': {e}");
            }
        }
    }

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
pub fn get_time_saved_stats<R: Runtime>(
    app: AppHandle<R>,
    period: String,
) -> Result<crate::stats::TimeSavedStats, String> {
    let p = crate::stats::Period::parse(&period)
        .ok_or_else(|| format!("invalid period: {period}"))?;
    Ok(crate::stats::aggregate(&app, p))
}

#[tauri::command]
pub fn open_settings<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("settings") {
        crate::tray::position_settings_on_active_display(&win);
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
    // RunEvent::Exit doesn't always fire reliably on macOS for menu-bar
    // (Accessory) apps, so we tear the inference subprocess down on the
    // synchronous quit path before app.exit() to guarantee the
    // `llama-server` child doesn't outlive us.
    crate::inference_proc::shutdown_managed(&app);
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

/// Open an `http(s)://` URL in the user's default browser. Tauri webviews
/// ignore `target="_blank"` on anchor tags (no default-browser fallback),
/// so the frontend has to round-trip through this command to launch
/// external links. Restricted to http/https to prevent the renderer from
/// driving `open` into arbitrary file:// or x-apple-* URI handlers.
#[tauri::command]
pub fn open_external_url<R: Runtime>(_app: AppHandle<R>, url: String) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("only http(s) URLs are allowed".into());
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&url)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

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
    let bundle_identifier = app.config().identifier.clone();
    // Bundled .app paths contain `target/<profile>/` too — exclude them
    // via the `.app/Contents/MacOS/` segment so they aren't flagged dev.
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
    PermissionStatus {
        microphone: probe_microphone(),
        accessibility: probe_accessibility(),
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

/// Non-prompting microphone status probe. MUST stay non-prompting: it's
/// called repeatedly (onboarding poller, tray startup) and any path that
/// touches cpal on macOS 14+ implicitly trips the TCC consent dialog.
fn probe_microphone() -> bool {
    #[cfg(target_os = "macos")]
    {
        crate::mac_perm::authorization_status().is_granted()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

/// Explicitly request microphone access. The ONLY entry point allowed to
/// show the macOS TCC dialog — wired to the onboarding mic step.
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

/// Frontend-driven size update used by the streaming pill's
/// ResizeObserver. Width and height are independently optional —
/// the frontend reports whichever dimension actually changed so
/// the OS window only moves when it has to. Rust clamps each side
/// to its allowed range before applying.
#[derive(serde::Deserialize)]
pub struct PillResizeArgs {
    pub width: Option<f64>,
    pub height: Option<f64>,
}

#[tauri::command]
pub fn pill_resize<R: Runtime>(app: AppHandle<R>, args: PillResizeArgs) {
    pill::set_size(&app, args.width, args.height);
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
        // destroy() bypasses close-event handlers; close() can silently
        // no-op in dev when the window handle is stale.
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
pub fn get_app_version<R: Runtime>(app: AppHandle<R>) -> String {
    app.package_info().version.to_string()
}

#[tauri::command]
pub async fn check_for_updates<R: Runtime>(app: AppHandle<R>) -> UpdateCheckResult {
    use tauri_plugin_updater::UpdaterExt as _;

    let cur = app.package_info().version.to_string();

    // Bundles ship with a placeholder endpoint + empty pubkey until
    // signing/notarization is wired. Skip the network call: the host
    // won't resolve (5-15 s DNS timeout leaks "Vox is running" to
    // resolver watchers) and there's no pubkey to pin against.
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

/// True when running with the placeholder host or empty pubkey — the
/// updater is unconfigured and we skip the network call.
fn updater_is_stub<R: Runtime>(app: &AppHandle<R>) -> bool {
    // Walk plugin config raw — tauri_plugin_updater::Config shape may shift.
    let Some(plugins) = app.config().plugins.0.get("updater") else {
        return true;
    };
    let pubkey = plugins.get("pubkey").and_then(|v| v.as_str()).unwrap_or("");
    if pubkey.trim().is_empty() {
        return true;
    }
    let Some(endpoints) = plugins.get("endpoints").and_then(|v| v.as_array()) else {
        return true;
    };
    if endpoints.is_empty() {
        return true;
    }
    endpoints.iter().any(|e| {
        e.as_str()
            .map(|s| s.contains("updates.vox.local") || s.contains("placeholder"))
            .unwrap_or(false)
    })
}

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

/// Pre-warm an engine. STT loads the model file here; LLM intentionally
/// does not spawn llama-server (left to `inference_proc::prewarm_in_background`)
/// so onboarding doesn't pin a ~5 GB GGUF for users who may never dictate.
#[tauri::command]
pub async fn load_model_warm(app: AppHandle<Wry>, id: String) -> Result<(), String> {
    let descriptor = models::descriptor_for(&id)
        .ok_or_else(|| format!("unknown model id: {id}"))?;
    let registry = app
        .try_state::<crate::engines::Registry>()
        .ok_or_else(|| "engine registry missing".to_string())?;
    match descriptor.kind {
        models::ModelKind::Stt => {
            if let Some(engine) = registry.stt(&app, descriptor.engine, descriptor.id) {
                if let Err(e) = engine.prewarm().await {
                    log::warn!("stt prewarm '{}' failed: {e}", descriptor.id);
                }
            }
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

#[tauri::command]
pub async fn download_inference_server(app: AppHandle<Wry>) -> Result<(), String> {
    inference_server::download_and_extract(&app)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn cancel_inference_server_download() -> bool {
    inference_server::cancel_download()
}

#[tauri::command]
pub fn inference_server_status(app: AppHandle<Wry>) -> inference_server::InferenceServerStatus {
    inference_server::status(&app)
}

#[tauri::command]
pub fn restart_inference_server(app: AppHandle<Wry>) -> Result<(), String> {
    // Supervisor respawns lazily on next polish call; dropping is enough.
    inference_proc::drop_managed(&app);
    Ok(())
}

#[tauri::command]
pub fn delete_inference_server(app: AppHandle<Wry>) -> Result<(), String> {
    // Kill child first so files aren't deleted while held open (Windows).
    inference_proc::drop_managed(&app);
    inference_server::delete(&app).map_err(|e| e.to_string())
}

/// Status of Apple Intelligence (Foundation Models) on this Mac.
/// `available` drives the onboarding "recommended" pill and the Settings
/// "Use" button; `code` lets the UI tailor the disabled-state message
/// (download in progress vs unsupported hardware vs OS too old).
#[derive(Debug, Serialize)]
pub struct AppleFmAvailabilityResult {
    pub available: bool,
    pub code: String,
    pub message: String,
    /// `false` on non-macOS / when the apple-fm feature was compiled
    /// out — frontend hides the entire Apple Intelligence UI section.
    pub supported_host: bool,
}

#[tauri::command]
pub fn apple_fm_availability() -> AppleFmAvailabilityResult {
    #[cfg(all(target_os = "macos", feature = "apple-fm"))]
    {
        use crate::engines::apple_fm;
        use crate::engines::apple_fm_bridge::AvailabilityCode;
        let code = crate::engines::apple_fm_bridge::availability();
        let code_str = match code {
            AvailabilityCode::Available => "available",
            AvailabilityCode::DeviceNotEligible => "device_not_eligible",
            AvailabilityCode::IntelligenceNotEnabled => "intelligence_not_enabled",
            AvailabilityCode::ModelNotReady => "model_not_ready",
            AvailabilityCode::OsTooOld => "os_too_old",
            AvailabilityCode::InternalError => "internal_error",
            AvailabilityCode::FrameworkMissing => "framework_missing",
        };
        AppleFmAvailabilityResult {
            available: matches!(code, AvailabilityCode::Available),
            code: code_str.into(),
            message: apple_fm::availability_message(code).into(),
            supported_host: true,
        }
    }
    #[cfg(not(all(target_os = "macos", feature = "apple-fm")))]
    {
        AppleFmAvailabilityResult {
            available: false,
            code: "framework_missing".into(),
            message: "Apple Intelligence is only available on macOS".into(),
            supported_host: false,
        }
    }
}

/// Status of the Parakeet TDT v3 STT engine on this host. The reason
/// gating is compile-time (cfg!) so the runtime call is just a flag check
/// — frontends call this once in onboarding / Settings → Models to decide
/// whether to surface the Parakeet card.
#[derive(Debug, Serialize)]
pub struct ParakeetAvailability {
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[tauri::command]
pub async fn parakeet_availability() -> ParakeetAvailability {
    let available = crate::engines::parakeet_bridge::is_available();
    if available {
        return ParakeetAvailability { available: true, reason: None };
    }
    let reason = if !cfg!(target_os = "macos") {
        "macOS only"
    } else if !cfg!(target_arch = "aarch64") {
        "requires Apple Silicon"
    } else if !cfg!(feature = "parakeet") {
        "Parakeet not compiled into this build"
    } else {
        "Parakeet not available on this host"
    };
    ParakeetAvailability {
        available: false,
        reason: Some(reason.into()),
    }
}

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

#[tauri::command]
pub fn diagnose_hotkeys<R: Runtime>(app: AppHandle<R>) -> DiagnoseHotkeys {
    let entries = match app.try_state::<ActiveHotkeyState>() {
        Some(state) => vec![
            HotkeyDiagnostic::from_active("push-to-talk", state.snapshot(HotkeyPurpose::PushToTalk)),
            HotkeyDiagnostic::from_active("cancel", state.snapshot(HotkeyPurpose::Cancel)),
            HotkeyDiagnostic::from_active("mode-cycle", state.snapshot(HotkeyPurpose::ModeCycle)),
        ],
        None => Vec::new(),
    };
    DiagnoseHotkeys { entries }
}

/// Cancel the specified in-flight session. Wired to the popover X
/// button. Background sessions can ONLY be cancelled this way; the
/// Escape hotkey targets the foreground recording (see
/// `SessionManager::cancel_foreground`).
#[tauri::command]
pub fn cancel_session(app: AppHandle<Wry>, session_id: u64) -> Result<(), String> {
    let mgr = match app.try_state::<Arc<SessionManager>>() {
        Some(m) => m.inner().clone(),
        None => return Err("session manager not initialised".into()),
    };
    mgr.cancel_session(session_id, &app);
    Ok(())
}

/// Snapshot of in-flight + recent sessions for the tray popover. The
/// popover subscribes to `sessions:changed` for live updates and uses
/// this for first-load.
#[tauri::command]
pub fn sessions_snapshot(app: AppHandle<Wry>) -> SessionsSnapshot {
    match app.try_state::<Arc<SessionManager>>() {
        Some(m) => m.inner().snapshot(),
        None => SessionsSnapshot {
            in_progress: Vec::new(),
            recent: transcripts::snapshot_completed(),
        },
    }
}
