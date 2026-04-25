mod audio;
mod commands;
mod hotkey;
mod inject;
mod session;
mod settings;
mod stt;
mod tray;

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
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::set_settings,
            commands::test_groq_connection,
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

            // Register global hotkey from settings.
            let s = settings::load(app_handle);
            match hotkey::register(app_handle, &s.hotkey, ctx.clone()) {
                Ok(resolved) => {
                    log::info!("push-to-talk bound to {resolved}");
                }
                Err(e) => {
                    log::error!(
                        "failed to register global shortcut '{}': {e}. \
                         Grant Accessibility in System Settings and restart.",
                        s.hotkey
                    );
                }
            }

            // Tray icon.
            if let Err(e) = tray::build(app_handle) {
                log::error!("failed to build tray icon: {e}");
            }

            // Keep settings window hidden until user opens it.
            if let Some(win) = app.get_webview_window("settings") {
                let _ = win.hide();
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "settings" {
                    // Hide instead of quitting — user quits via tray menu.
                    let _ = window.hide();
                    api.prevent_close();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
