//! Dedicated milestone-donation pop-up window.
//!
//! When a dictation completes and the user has just crossed a time-saved
//! tier (1h / 10h / 50h) without being opted out or snoozed, this window
//! auto-appears once to ask for support. The gating decision lives in
//! [`crate::donation::prompt`]; this module only renders it and records
//! the tier as surfaced via [`crate::donation::mark_shown`] so it fires
//! exactly once, ever — even if the user closes the window rather than
//! interacting with it, and even across a force-quit.
//!
//! Coexists with the passive tray-popover `DonateBanner`: both read the
//! same store keys, so marking a tier shown here also clears the banner's
//! prompt for that tier.

use tauri::{AppHandle, LogicalPosition, Manager, Runtime, WebviewUrl, Wry};

pub const LABEL: &str = "donation";
const WIDTH: f64 = 380.0;
const HEIGHT: f64 = 440.0;

/// Surface the donation window for the highest un-acted milestone the
/// user has crossed, if any. No-op when nothing should show, or when a
/// donation window is already open (don't stack a second).
///
/// Called from the pipeline's success branch, which runs off the main
/// thread — so the actual window build is dispatched onto the main thread
/// (macOS requires AppKit window creation there). The tier is marked
/// shown only after a successful build, inside the same main-thread
/// closure, so a build failure leaves it free to re-fire next time.
pub fn maybe_show(app: &AppHandle<Wry>) {
    // This window is deliberately destroy-on-close (it is NOT in the
    // hide-don't-destroy list in lib.rs's CloseRequested handler): once
    // closed, get_webview_window returns None, which is exactly what
    // re-arms this guard so the *next* tier can fire. Don't move it into
    // the hide list — that would silently stop later milestones showing.
    if app.get_webview_window(LABEL).is_some() {
        return;
    }
    let Some(prompt) = crate::donation::prompt(app) else {
        return;
    };

    let app_for_build = app.clone();
    if let Err(dispatch_err) = app.run_on_main_thread(move || {
        // Re-check on the main thread: a concurrent pipeline could have
        // raced us to build the window between the guard above and now.
        if app_for_build.get_webview_window(LABEL).is_some() {
            return;
        }
        match build(&app_for_build, prompt.saved_secs) {
            Ok(()) => {
                if let Err(e) = crate::donation::mark_shown(&app_for_build, prompt.milestone_secs) {
                    log::warn!("donation window: mark_shown failed: {e}");
                }
            }
            Err(e) => log::warn!("donation window: build failed: {e}"),
        }
    }) {
        log::warn!("donation window: run_on_main_thread failed: {dispatch_err}");
    }
}

/// Build, place on the active display, then show — mirroring the
/// onboarding window so the pop-up appears on the screen the user is on
/// rather than flashing on the primary first. `saved_secs` is carried in
/// the URL so the copy renders without a re-fetch (the gating prompt
/// returns `None` once the tier is marked shown).
fn build<R: Runtime>(app: &AppHandle<R>, saved_secs: f64) -> tauri::Result<()> {
    let win = tauri::WebviewWindowBuilder::new(
        app,
        LABEL,
        WebviewUrl::App(format!("donation.html?s={saved_secs}").into()),
    )
    .title("Support Vox")
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
/// surface agrees on "active display." Clamps the origin so the title bar
/// stays reachable, and falls back to Tauri's primary-monitor `center()`
/// if the active frame can't be resolved or positioning fails.
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
