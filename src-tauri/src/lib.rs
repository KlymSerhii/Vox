mod audio;
mod audio_attenuation;
mod commands;
mod donation;
mod donation_window;
mod engines;
mod frontmost;
mod hotkey;
mod inference_proc;
mod inference_server;
mod inject;
#[cfg(target_os = "macos")]
mod mac_audio;
#[cfg(target_os = "macos")]
mod mac_chord;
#[cfg(target_os = "macos")]
mod mac_paste;
#[cfg(target_os = "macos")]
mod mac_perm;
mod mac_power;
mod mac_wake;
#[cfg(target_os = "windows")]
mod win_audio;
#[cfg(target_os = "windows")]
mod win_paste;
mod models;
mod modes;
#[cfg(target_os = "macos")]
mod notch;
mod notch_list;
mod onboarding;
mod pill;
mod session;
mod settings;
mod stats;
mod transcripts;
mod tray;
mod tray_glyphs;
mod tray_icon;
mod update_window;
mod updater;
mod wake;

use std::sync::Arc;

use tauri::Manager;

use crate::audio::AudioHandle;
use crate::session::SessionManager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = env_logger::Builder::from_env(
        env_logger::Env::default().filter_or("VOX_LOG", "info"),
    )
    .try_init();

    let app = tauri::Builder::default()
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
            commands::set_hands_free_enabled,
            commands::set_polishing_enabled,
            commands::set_streaming_pill_enabled,
            commands::set_auto_paste_enabled,
            commands::set_live_transcript_enabled,
            commands::set_enable_thinking,
            commands::set_appearance,
            commands::set_audio_attenuation,
            commands::copy_text,
            commands::get_recent_transcripts,
            commands::get_time_saved_stats,
            commands::get_donation_prompt,
            commands::donation_opt_out,
            commands::donation_snooze,
            commands::open_settings,
            commands::close_tray_popover,
            commands::quit_app,
            commands::open_accessibility_settings,
            commands::open_microphone_settings,
            commands::list_audio_input_devices,
            commands::set_audio_input_device,
            commands::open_external_url,
            commands::uninstall_app,
            commands::check_permissions,
            commands::request_microphone,
            commands::show_pill,
            commands::hide_pill,
            commands::pill_resize,
            commands::replay_onboarding,
            commands::finish_onboarding,
            commands::show_tray_popover,
            commands::get_launch_at_login,
            commands::set_launch_at_login,
            commands::check_for_updates,
            commands::check_for_updates_now,
            commands::download_and_install_update,
            commands::get_app_version,
            commands::get_update_state,
            commands::dismiss_update,
            commands::set_auto_update_check_enabled,
            commands::list_models,
            commands::download_model,
            commands::cancel_download,
            commands::delete_model,
            commands::load_model_warm,
            commands::recommend_tier,
            commands::accept_gemma_tos,
            commands::download_inference_server,
            commands::cancel_inference_server_download,
            commands::inference_server_status,
            commands::restart_inference_server,
            commands::delete_inference_server,
            commands::apple_fm_availability,
            commands::parakeet_availability,
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
            commands::notch_list_get,
            commands::notch_resize,
            commands::notch_set_hit_shape,
            commands::notch_geometry,
            commands::notch_list_mark_done,
            commands::notch_list_add,
            commands::notch_focus_input,
            commands::notch_release_input_focus,
            commands::notch_list_edit,
            commands::notch_list_remove,
            commands::notch_list_reorder,
            commands::notch_list_set_priority,
            commands::notch_list_restore,
            commands::notch_list_clear_completed,
            commands::notch_list_undo,
            commands::get_active_hotkey,
            commands::diagnose_hotkeys,
            commands::reveal_dev_binary_path,
            commands::cancel_session,
            commands::sessions_snapshot,
            commands::re_polish_transcript,
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            {
                // Activation policy == Dock-icon visibility. Default is
                // menu-bar-only accessory (matching `LSUIElement` in
                // Info.plist); `LSUIElement` forces accessory at launch
                // regardless, so we must explicitly switch to `Regular` for
                // users who opted into the Dock icon. Its click opens
                // Settings via the `RunEvent::Reopen` arm below. Decided
                // here, before `app.handle()` borrows `app`, so the
                // `&mut App` setter is free to run.
                let policy = if settings::load(app.handle()).show_dock_icon {
                    tauri::ActivationPolicy::Regular
                } else {
                    tauri::ActivationPolicy::Accessory
                };
                app.set_activation_policy(policy);
            }

            let app_handle = app.handle();

            let audio = match AudioHandle::spawn() {
                Ok(a) => a,
                Err(e) => {
                    log::error!(
                        "failed to spawn audio thread: {e}. \
                         Recording will fail until the next launch."
                    );
                    // We still need *some* AudioHandle for SessionManager to
                    // hold; the caller will surface a clean audio error on
                    // the first record attempt rather than panicking here.
                    // Try once more — if even the second spawn fails we
                    // log and bail out of setup rather than crashing.
                    match AudioHandle::spawn() {
                        Ok(a) => a,
                        Err(e2) => {
                            log::error!("audio thread re-spawn failed: {e2}");
                            return Err(Box::new(e2) as Box<dyn std::error::Error>);
                        }
                    }
                }
            };
            let manager = Arc::new(SessionManager::new(audio));
            app.manage(manager.clone());

            // Register hotkey state BEFORE hotkey::register so per-purpose
            // snapshots are ready for IPC handlers from the first call.
            app.manage(crate::hotkey::ActiveHotkeyState::default());
            app.manage(crate::engines::Registry::new());

            crate::inference_proc::install_state(app_handle);
            // macOS has no PR_SET_PDEATHSIG; reap orphan llama-servers from
            // a previously force-killed parent before they confuse spawn.
            crate::inference_proc::reap_orphans_at_startup(app_handle);
            crate::inference_server::cleanup_stale_versions(app_handle);

            let mode_store = std::sync::Arc::new(crate::modes::ModeStore::new());
            let modes_root = crate::modes::modes_dir(app_handle);
            if let Err(e) = mode_store.init_with_root(modes_root) {
                log::warn!("voice modes init failed: {e}");
            }
            app.manage(mode_store);

            // Notch List store (single JSON file). Cross-platform; the
            // notch *window* is macOS-only and created below.
            let notch_store = std::sync::Arc::new(crate::notch_list::NotchStore::new());
            let notch_path = crate::notch_list::notch_list_path(app_handle);
            if let Err(e) = notch_store.init_with_path(notch_path) {
                log::warn!("notch list init failed: {e}");
            }
            app.manage(notch_store);

            let s = settings::load(app_handle);

            match hotkey::register(
                app_handle,
                &s.hotkey,
                &s.cancel_hotkey,
                &s.mode_cycle_hotkey,
                &s.notch_list_voice_hotkey,
                manager.clone(),
            ) {
                Ok(resolved) => log::info!("push-to-talk bound to {resolved}"),
                Err(e) => log::error!(
                    "failed to register global shortcut '{}': {e}. Grant Accessibility and restart.",
                    s.hotkey
                ),
            }

            if let Err(e) = pill::create(app_handle) {
                log::error!("failed to create pill window: {e}");
            }
            // Notch List window — macOS-only, and only when enabled
            // (opt-out, so on by default). Errors are non-fatal: the rest
            // of the app runs fine without the notch.
            #[cfg(target_os = "macos")]
            if s.notch_list_enabled {
                if let Err(e) = crate::notch::create(app_handle) {
                    log::error!("failed to create notch window: {e}");
                }
            }
            if let Err(e) = tray::build(app_handle) {
                log::error!("failed to build tray icon: {e}");
            }
            if let Some(win) = app.get_webview_window("settings") {
                let _ = win.hide();
            }

            // Pre-warm both engines so the first dictation doesn't pay
            // Whisper's ~1-2 s GGML load and Gemma's ~5-10 s spawn. Both
            // helpers no-op when their feature is off/uninstalled, and are
            // safe to call again later from settings/tray toggles.
            crate::inference_proc::prewarm_in_background(app_handle);
            crate::engines::prewarm_stt_in_background(app_handle);
            // "Hey Vox" voice trigger. The thread idles (no mic open, no
            // CPU) until the user opts in via Settings → General; refresh
            // calls from the settings commands keep it in sync after that.
            crate::wake::install(app_handle, manager.clone());
            // Trigger the macOS NSPasteboard / Gatekeeper first-use
            // signature check during onboarding rather than on the
            // user's first dictation, where it otherwise stalls the
            // post-polish clipboard write for 30-60 s with a
            // "Verifying com.vox…" system dialog. Windows has no such
            // first-use check, so the round-trip is a needless clipboard
            // touch there — macOS-only.
            #[cfg(target_os = "macos")]
            crate::inject::prewarm_in_background();

            // Auto-update scheduler. State is installed BEFORE the
            // scheduler spawn so the first emitted `updates:state` event
            // (60 s in) finds the in-process state container. Hydration
            // from disk seeds the persisted `dismissed_version` and
            // `last_check_at` so the cooldown survives relaunches.
            let updater_state =
                Arc::new(crate::updater::UpdaterState::new(
                    app_handle.package_info().version.to_string(),
                ));
            app.manage(updater_state);
            crate::updater::hydrate_from_disk(app_handle);
            crate::updater::spawn_scheduler(app_handle);
            // NSWorkspace wake notification → re-evaluate the cadence
            // immediately after the Mac wakes from sleep. The
            // 30 s debounce + 6 h cooldown in `updater.rs` ensure a
            // burst of wake events doesn't translate into burst HTTPS.
            crate::updater::subscribe_wake(app_handle);

            if !s.onboarded {
                if let Err(e) = onboarding::show(app_handle) {
                    log::warn!("failed to show onboarding: {e}");
                }
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if matches!(window.label(), "settings" | "tray-popover" | "pill" | "notch") {
                    let _ = window.hide();
                    api.prevent_close();
                }
            }
            if let tauri::WindowEvent::Focused(false) = event {
                if window.label() == "tray-popover" {
                    let _ = window.hide();
                }
            }
        })
        .build(tauri::generate_context!());

    let app = match app {
        Ok(a) => a,
        Err(e) => {
            log::error!("Tauri application failed to build: {e:?}");
            std::process::exit(1);
        }
    };

    app.run(|app_handle, event| {
        // Tear down on Exit (not ExitRequested) so a canceled Cmd+Q
        // doesn't kill the inference subprocess prematurely.
        if let tauri::RunEvent::Exit = event {
            crate::inference_proc::shutdown_managed(app_handle);
        }
        // Dock-icon click (NSApplicationDelegate applicationShouldHandleReopen)
        // → bring Vox's primary window forward. Only fires for users who
        // enabled the Dock icon, and never on the initial launch, so it can't
        // pop Settings open on a login-item start.
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Reopen { .. } = event {
            focus_primary_window(app_handle);
        }
    });
}

/// Surface Vox's primary window in response to a Dock-icon click. Prefers
/// the onboarding window while first-run is still active (so the click
/// doesn't reveal the hidden Settings window behind the wizard); otherwise
/// shows and focuses Settings, mirroring `commands::open_settings`.
#[cfg(target_os = "macos")]
fn focus_primary_window<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    if !settings::load(app).onboarded {
        if let Some(win) = app.get_webview_window(onboarding::LABEL) {
            let _ = win.show();
            let _ = win.set_focus();
            return;
        }
    }
    if let Some(win) = app.get_webview_window("settings") {
        crate::tray::position_settings_on_active_display(&win);
        let _ = win.show();
        let _ = win.set_focus();
    }
}
