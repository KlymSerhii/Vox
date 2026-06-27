//! Dedicated "a new version is available" pop-up window.
//!
//! A *background* update check makes a release available; this window
//! is then surfaced from the dictation pipeline's success branch
//! (post-delivery, the same safe point as the donation window) to
//! announce it with the release notes and an Install button. It fires
//! at most once per available version, ever — the persisted
//! [`settings::update_window_shown_version`] marker is written after a
//! successful build, so it survives relaunches and a force-quit.
//!
//! This window is deliberately destroy-on-close: it is NOT in the
//! hide-don't-destroy list in `lib.rs`'s `CloseRequested` handler, so
//! closing it lets `get_webview_window` return `None` again. The
//! once-per-version guarantee comes from the persisted marker rather
//! than from keeping the window alive.
//!
//! Coexists with the passive tray-popover `UpdateBanner` + tray-icon
//! dot, which remain the gentle reminder for as long as the update is
//! available and undismissed (gated separately on
//! `dismissed_update_version`). This window is the one-shot prominent
//! announcement; the banner is the persistent one.

use std::sync::Arc;

use tauri::{AppHandle, LogicalPosition, Manager, Runtime, WebviewUrl};

use crate::settings;
use crate::updater::UpdaterState;

pub const LABEL: &str = "update";
const WIDTH: f64 = 440.0;
const HEIGHT: f64 = 560.0;

/// Surface the update window for the currently available version, if
/// one is available, undismissed, and hasn't already had its window
/// shown. No-op otherwise, or when an update window is already open
/// (don't stack a second).
///
/// Generic over `R: Runtime` because the caller is the dictation
/// pipeline (`session::run_pipeline`'s success branch), which also runs
/// OFF the main thread — so the actual window build is dispatched onto
/// the main thread (macOS requires AppKit window creation there). The
/// shown-version marker is persisted only after a successful build,
/// inside the same main-thread closure, so a build failure leaves it
/// free to re-fire.
pub fn maybe_show<R: Runtime>(app: &AppHandle<R>) {
    if app.get_webview_window(LABEL).is_some() {
        return;
    }

    let Some(state) = app.try_state::<Arc<UpdaterState>>() else {
        return;
    };
    let snap = state.current.lock().clone();
    if !snap.banner_visible() {
        return;
    }
    // `banner_visible()` guarantees this is Some, but guard anyway so
    // the marker we persist below can never be an empty/None version.
    let Some(av) = snap.available_version.clone() else {
        return;
    };

    if settings::load(app).update_window_shown_version.as_deref() == Some(av.as_str()) {
        return;
    }

    let app_for_build = app.clone();
    if let Err(dispatch_err) = app.run_on_main_thread(move || {
        // Re-check on the main thread: a concurrent check could have
        // raced us to build the window between the guard above and now.
        if app_for_build.get_webview_window(LABEL).is_some() {
            return;
        }
        match build(&app_for_build) {
            Ok(()) => {
                if let Err(e) =
                    settings::set_update_window_shown_version(&app_for_build, Some(&av))
                {
                    log::warn!("update window: persist shown_version failed: {e}");
                }
            }
            Err(e) => log::warn!("update window: build failed: {e}"),
        }
    }) {
        log::warn!("update window: run_on_main_thread failed: {dispatch_err}");
    }
}

/// Build, place on the active display, then show — mirroring the
/// onboarding/donation windows so the pop-up appears on the screen the
/// user is on rather than flashing on the primary first. The window
/// fetches its data via `getUpdateState()` on mount, so the URL needs
/// no query param.
fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let win = tauri::WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("update.html".into()))
        .title("Update Available")
        .inner_size(WIDTH, HEIGHT)
        .resizable(false)
        .maximizable(false)
        .decorations(true)
        .focused(true)
        .visible(false)
        .build()?;

    center_on_active_display(&win);
    win.show()?;
    win.set_focus()?;

    Ok(())
}

/// Centre on the display the user is currently on (the one holding the
/// cursor), reusing the pill's shared active-display probe so every
/// surface agrees on "active display." Clamps the origin so the title
/// bar stays reachable, and falls back to Tauri's primary-monitor
/// `center()` if the active frame can't be resolved or positioning
/// fails.
fn center_on_active_display<R: Runtime>(win: &tauri::WebviewWindow<R>) {
    if let Some((fx, fy, fw, fh)) = crate::pill::active_screen_logical_frame(win) {
        let x = (fx + (fw - WIDTH) / 2.0).max(fx);
        let y = (fy + (fh - HEIGHT) / 2.0).max(fy);
        if win.set_position(LogicalPosition::new(x, y)).is_ok() {
            return;
        }
    }
    let _ = win.center();
}
