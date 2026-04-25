mod audio;
mod commands;
mod hotkey;
mod inject;
mod llm;
mod onboarding;
mod pill;
mod session;
mod settings;
mod stt;
mod transcripts;
mod tray;
mod tray_glyphs;
mod tray_icon;

use tauri::Manager;

use crate::audio::AudioHandle;
use crate::session::SessionCtx;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Best-effort logger init. Set VOX_LOG=debug to get more output.
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
        ])
        .setup(|app| {
            // macOS: accessory mode — no Dock icon.
            #[cfg(target_os = "macos")]
            {
                app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            }

            let app_handle = app.handle();

            // Audio capture thread.
            let audio = AudioHandle::spawn();
            let ctx = SessionCtx::new(audio);
            app.manage(ctx.clone());

            // Load settings (also migrates plaintext keys into the Keychain).
            let s = settings::load(app_handle);

            // Register global hotkeys.
            match hotkey::register(
                app_handle,
                &s.hotkey,
                &s.cancel_hotkey,
                &s.paste_last_hotkey,
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

            // Launch onboarding on first run.
            if !s.onboarded {
                if let Err(e) = onboarding::show(app_handle) {
                    log::warn!("failed to show onboarding: {e}");
                }
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                match window.label() {
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
            // Hide the tray popover on blur (standard popover UX).
            if let tauri::WindowEvent::Focused(false) = event {
                if window.label() == "tray-popover" {
                    let _ = window.hide();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
