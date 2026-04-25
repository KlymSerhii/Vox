mod audio;
mod commands;
mod engines;
mod frontmost;
mod hotkey;
mod inject;
#[cfg(target_os = "macos")]
mod mac_chord;
#[cfg(target_os = "macos")]
mod mac_perm;
mod migration;
mod models;
mod modes;
mod onboarding;
mod pill;
mod session;
mod settings;
mod transcripts;
mod tray;
mod tray_glyphs;
mod tray_icon;

use tauri::Manager;

use crate::audio::AudioHandle;
use crate::session::SessionCtx;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = env_logger::Builder::from_env(
        env_logger::Env::default().filter_or("VOX_LOG", "info"),
    )
    .try_init();

    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::get_public_settings,
            commands::set_settings,
            commands::set_dictation_enabled,
            commands::test_groq_connection,
            commands::test_anthropic_connection,
            commands::get_recent_transcripts,
            commands::open_settings,
            commands::close_tray_popover,
            commands::quit_app,
            commands::open_accessibility_settings,
            commands::open_microphone_settings,
            commands::check_permissions,
            commands::request_microphone,
            commands::show_pill,
            commands::hide_pill,
            commands::replay_onboarding,
            commands::finish_onboarding,
            commands::show_tray_popover,
            commands::get_launch_at_login,
            commands::set_launch_at_login,
            commands::paste_last_transcript,
            commands::check_for_updates,
            commands::list_models,
            commands::download_model,
            commands::cancel_download,
            commands::delete_model,
            commands::load_model_warm,
            commands::recommend_tier,
            commands::accept_gemma_tos,
            commands::dismiss_migration,
            commands::list_modes,
            commands::get_mode,
            commands::save_mode,
            commands::delete_mode,
            commands::duplicate_mode,
            commands::set_active_mode,
            commands::export_mode,
            commands::import_mode_json,
            commands::reset_default_modes,
            commands::cycle_active_mode,
            commands::suggest_active_mode,
            commands::get_active_hotkey,
            commands::diagnose_hotkeys,
            commands::reveal_dev_binary_path,
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            {
                app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            }

            let app_handle = app.handle();

            // Audio capture thread.
            let audio = AudioHandle::spawn();
            let ctx = SessionCtx::new(audio);
            app.manage(ctx.clone());

            // Hotkey-binding state. We register this BEFORE
            // `hotkey::register` so the per-purpose snapshots are
            // ready for the IPC handlers from the very first call.
            app.manage(crate::hotkey::ActiveHotkeyState::default());

            // Engine registry — lazy-built, kept warm.
            app.manage(crate::engines::Registry::new());

            // Voice Modes store. First-launch seed is gated inside
            // `init_with_root` (only writes built-ins when the dir is
            // empty), so it's safe to call on every boot.
            let mode_store = std::sync::Arc::new(crate::modes::ModeStore::new());
            let modes_root = crate::modes::modes_dir(app_handle);
            if let Err(e) = mode_store.init_with_root(modes_root) {
                log::warn!("voice modes init failed: {e}");
            }
            app.manage(mode_store);

            // Load settings (also migrates plaintext keys into the Keychain).
            let s = settings::load(app_handle);

            // Register global hotkeys.
            match hotkey::register(
                app_handle,
                &s.hotkey,
                &s.cancel_hotkey,
                &s.paste_last_hotkey,
                &s.mode_cycle_hotkey,
                ctx.clone(),
            ) {
                Ok(resolved) => log::info!("push-to-talk bound to {resolved}"),
                Err(e) => log::error!(
                    "failed to register global shortcut '{}': {e}. Grant Accessibility and restart.",
                    s.hotkey
                ),
            }

            // Create the listening-pill window up-front (hidden).
            if let Err(e) = pill::create(app_handle) {
                log::error!("failed to create pill window: {e}");
            }

            // Tray icon.
            if let Err(e) = tray::build(app_handle) {
                log::error!("failed to build tray icon: {e}");
            }

            // Keep settings window hidden until user opens it.
            if let Some(win) = app.get_webview_window("settings") {
                let _ = win.hide();
            }

            // Local-first soft migration: existing cloud users see a
            // one-time dialog. New installs go to onboarding instead.
            if !s.onboarded {
                if let Err(e) = onboarding::show(app_handle) {
                    log::warn!("failed to show onboarding: {e}");
                }
            } else if !s.migration_seen && settings::is_legacy_cloud_user(app_handle) {
                if let Err(e) = migration::show(app_handle) {
                    log::warn!("failed to show migration dialog: {e}");
                }
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                match window.label() {
                    "migration" => {
                        // Closing the title-bar X has the same dismissive
                        // effect as the "Keep using cloud" button: we
                        // flip `migration_seen = true` so the dialog
                        // doesn't re-appear on every launch. Engine
                        // settings are left untouched.
                        crate::settings::mark_migration_seen(window.app_handle());
                        let _ = window.hide();
                        api.prevent_close();
                    }
                    "settings" | "tray-popover" | "pill" => {
                        let _ = window.hide();
                        api.prevent_close();
                    }
                    "onboarding" => {
                        // Let onboarding truly close — state is persisted.
                    }
                    _ => {}
                }
            }
            if let tauri::WindowEvent::Focused(false) = event {
                if window.label() == "tray-popover" {
                    let _ = window.hide();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
