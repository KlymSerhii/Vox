//! Notch List window (macOS-only). A transparent, decoration-less,
//! always-on-top panel anchored at the top-center of the chosen display —
//! a synthetic "notch" pill for v1 (no physical-notch fusing).
//!
//! The window builder + macOS panel style + top-center positioning are
//! lifted from `pill.rs` (the proven pattern). KEY DIFFERENCE from the
//! pill: we do NOT call `set_ignore_cursor_events(true)` — the notch needs
//! hover + click (the checkbox) to work.
//!
//! Whole module is `#[cfg(target_os = "macos")]` (gated at the `mod notch;`
//! declaration in `lib.rs`), so this file is only ever compiled on macOS.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Runtime, WebviewUrl};

pub const LABEL: &str = "notch";

/// Poll cadence for the display follower + fullscreen probe. Moving the
/// cursor to another monitor (or entering fullscreen) is a coarse user
/// action, so a ~300 ms tick keeps the notch on the right screen without
/// the cost of a tight loop. Mirrors `pill::FOLLOW_POLL_MS`.
const FOLLOW_POLL_MS: u64 = 300;

/// Bumped every time the notch window is created or destroyed. The
/// display follower captures the value at spawn time and stands down the
/// moment a create/destroy advances it — exactly mirroring
/// `pill::PILL_GENERATION`. Because the notch window is persistent (one
/// `create` per enable), the follower is spawned once at creation and
/// torn down on destroy.
static NOTCH_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Collapsed resting width/height. The window is created at the collapsed
/// size; the frontend grows it on hover via `setSize` (mirroring how the
/// pill drives its own sizing). Sized generously so the single top-item
/// line and the empty-state hotkey row both fit without clipping.
pub const WIDTH: f64 = 280.0;
pub const HEIGHT: f64 = 30.0;
/// Gap from the very top edge of the display so the notch doesn't kiss the
/// menu bar.
const TOP_MARGIN: f64 = 2.0;

/// Create the notch window if it doesn't already exist. Mirrors
/// `pill::create`: transparent, decoration-less, always-on-top,
/// skip-taskbar, non-focusing. Unlike the pill it stays interactive
/// (no `set_ignore_cursor_events`).
pub fn create<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    if app.get_webview_window(LABEL).is_some() {
        return Ok(());
    }

    let window =
        tauri::WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("notch.html".into()))
            .title("Vox Notch")
            .inner_size(WIDTH, HEIGHT)
            .resizable(false)
            .maximizable(false)
            .minimizable(false)
            .decorations(false)
            .transparent(true)
            // OS shadow follows the rounded alpha of the content on macOS
            // (same rationale as the pill). The notch is macOS-only, so we
            // always keep the OS shadow on.
            .shadow(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .focused(false)
            .visible(true)
            .accept_first_mouse(true)
            .build()?;

    if let Err(e) = reposition(&window) {
        log::warn!("notch reposition failed: {e}");
    }

    if let Err(e) = apply_macos_panel_style(&window) {
        log::warn!("notch: failed to apply macOS panel style: {e}");
    }

    // The window is persistent (one create per enable), so we spawn the
    // follower exactly once here, keyed on a fresh generation. A later
    // destroy bumps the generation and the follower stands down.
    let generation = NOTCH_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    spawn_display_follower(app.clone(), generation);

    Ok(())
}

/// Destroy the notch window (used when the user turns the feature off).
pub fn destroy<R: Runtime>(app: &AppHandle<R>) {
    // Bump the generation FIRST so the running follower stands down on its
    // next tick instead of touching a window we're about to close.
    NOTCH_GENERATION.fetch_add(1, Ordering::SeqCst);
    if let Some(win) = app.get_webview_window(LABEL) {
        if let Err(e) = win.close() {
            log::warn!("notch: failed to close window: {e}");
        }
    }
}

/// Create or destroy the notch window to match `enabled`. Called from
/// `commands::set_settings` when `notch_list_enabled` flips.
pub fn create_or_destroy<R: Runtime>(app: &AppHandle<R>, enabled: bool) {
    if enabled {
        if let Err(e) = create(app) {
            log::error!("notch: failed to create window: {e}");
        }
    } else {
        destroy(app);
    }
}

/// Frontend-driven size update (hover expand / collapse). Width/height are
/// each optional; the window stays top-center anchored so growth expands
/// downward and stays horizontally centered. Mirrors `pill::set_size`'s
/// reposition-after-resize, but TOP-anchored instead of bottom-anchored.
pub fn set_size<R: Runtime>(app: &AppHandle<R>, width: Option<f64>, height: Option<f64>) {
    if width.is_none() && height.is_none() {
        return;
    }
    let Some(win) = app.get_webview_window(LABEL) else {
        return;
    };
    let frame = target_frame(&win);

    let scale = win.scale_factor().unwrap_or(1.0);
    let cur = win.inner_size().ok();
    let cur_w = cur.as_ref().map(|s| s.width as f64 / scale).unwrap_or(WIDTH);
    let cur_h = cur.map(|s| s.height as f64 / scale).unwrap_or(HEIGHT);

    let target_w = width.unwrap_or(cur_w).max(WIDTH);
    let target_h = height.unwrap_or(cur_h).max(HEIGHT);

    apply_size(&win, frame, target_w, target_h);
}

/// Re-run the top-center placement for the current window. Public so
/// `commands::set_settings` can call it (macOS-gated) when
/// `notch_list_display` changes, so the notch hops to the newly-chosen
/// display without waiting for the next follower tick. No-op when the
/// window doesn't exist.
pub fn refresh_placement<R: Runtime>(app: &AppHandle<R>) {
    let Some(win) = app.get_webview_window(LABEL) else {
        return;
    };
    if let Err(e) = reposition(&win) {
        log::warn!("notch refresh_placement failed: {e}");
    }
}

/// Keep the notch on whatever display `notch_list_display` resolves to.
/// `"active"` follows the cursor's display (re-centering when the user
/// moves to another monitor mid-session); `"builtin"`/`"primary"` resolve
/// to a constant target frame, so the `frame != last` guard makes the
/// poll a cheap no-op there. TOP-anchored via `reposition`, never the
/// pill's bottom-anchored math.
///
/// Unlike the pill follower, this runs even on single-display Macs —
/// the Stage 6 fullscreen probe (added in `follower_tick`) needs the
/// poll regardless of monitor count.
///
/// Keyed on `NOTCH_GENERATION` (captured as `generation`): a create or
/// destroy bumps the counter and this task stands down on its next tick.
fn spawn_display_follower<R: Runtime>(app: AppHandle<R>, generation: u64) {
    tauri::async_runtime::spawn(async move {
        // Seed through the SAME placement path so the first tick compares
        // like-for-like and is a no-op (the window is already where
        // `create`/`reposition` put it).
        let mut last_frame = app
            .get_webview_window(LABEL)
            .and_then(|w| target_frame(&w));

        let mut ticker = tokio::time::interval(Duration::from_millis(FOLLOW_POLL_MS));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            // A create()/destroy() since spawn bumped the generation, so a
            // newer follower (or none) owns the notch — stand down.
            if NOTCH_GENERATION.load(Ordering::SeqCst) != generation {
                return;
            }
            if app.get_webview_window(LABEL).is_none() {
                return;
            }
            follower_tick(&app, &mut last_frame);
        }
    });
}

/// One follower iteration. The display-change detection runs here on the
/// async worker thread (`target_frame` resolves via Tauri's off-thread
/// `available_monitors`/`cursor_position` fallback), but the actual
/// reposition + fullscreen probe are dispatched onto the MAIN thread:
/// both depend on `NSScreen` reads (`safeAreaInsets`, `frame`,
/// `visibleFrame`) that require a `MainThreadMarker`, which is only
/// obtainable on the main thread. Without this hop the notch-fusion and
/// fullscreen auto-hide would silently no-op (every `MainThreadMarker::new`
/// returning `None` from the worker thread), degrading to v1 placement.
/// Split out so the hot loop stays readable and the two concerns are
/// independently fail-open.
fn follower_tick<R: Runtime>(
    app: &AppHandle<R>,
    last_frame: &mut Option<(f64, f64, f64, f64)>,
) {
    // Detect display changes off-thread. `target_frame` works without a
    // MainThreadMarker (Tauri fallback), so this stays a cheap no-op on a
    // single display and only flips on an actual cross-monitor move.
    let frame = app.get_webview_window(LABEL).and_then(|w| target_frame(&w));
    let moved = frame != *last_frame;
    *last_frame = frame;

    // Marshal the NSScreen-dependent work onto the main thread so the
    // notch-fusion read and the fullscreen probe actually resolve a
    // screen. Fire-and-forget; the window/show/hide calls inside are
    // themselves main-thread-safe.
    let app_main = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Some(win) = app_main.get_webview_window(LABEL) else {
            return;
        };
        // Re-center on a display change (preserving the current window
        // size so a hover-expanded panel keeps its width across the hop).
        if moved {
            if let Some(f) = target_frame(&win) {
                reposition_to_frame(&win, f);
            }
        }
        // Stage 6: hide the notch when the target display is in fullscreen
        // (menu bar hidden). Fail-open — a None/failed read never hides.
        apply_fullscreen_visibility(&win);
    });
}

/// Re-center the notch onto `frame` (logical x, y, w, h) while preserving
/// its current size — TOP-anchored, mirroring `apply_size`. Used by the
/// display follower so a hover-expanded panel keeps its dimensions when
/// it moves to another monitor.
fn reposition_to_frame<R: Runtime>(
    win: &tauri::WebviewWindow<R>,
    frame: (f64, f64, f64, f64),
) {
    let scale = win.scale_factor().unwrap_or(1.0);
    let cur = win.inner_size().ok();
    let cur_w = cur.as_ref().map(|s| s.width as f64 / scale).unwrap_or(WIDTH);
    let cur_h = cur.map(|s| s.height as f64 / scale).unwrap_or(HEIGHT);
    apply_size(win, Some(frame), cur_w, cur_h);
}

fn reposition<R: Runtime>(win: &tauri::WebviewWindow<R>) -> tauri::Result<()> {
    if let Some((mx, my, mw, _mh)) = target_frame(win) {
        let (x, y, w, h) = fuse_geometry(win, mx, my, mw, WIDTH, HEIGHT);
        win.set_size(LogicalSize::new(w, h))?;
        win.set_position(LogicalPosition::new(x, y))?;
    }
    Ok(())
}

fn apply_size<R: Runtime>(
    win: &tauri::WebviewWindow<R>,
    frame: Option<(f64, f64, f64, f64)>,
    target_w: f64,
    target_h: f64,
) {
    if let Some((mx, my, mw, _mh)) = frame {
        let (x, y, w, h) = fuse_geometry(win, mx, my, mw, target_w, target_h);
        let _ = win.set_size(LogicalSize::new(w, h));
        let _ = win.set_position(LogicalPosition::new(x, y));
    } else {
        let _ = win.set_size(LogicalSize::new(target_w, target_h));
    }
}

/// Compute the final `(x, y, w, h)` for the notch given the resolved
/// screen frame origin/width (`mx`, `my`, `mw`) and the desired content
/// size (`req_w`, `req_h`).
///
/// FALLBACK LADDER (the v1.1 safety contract — every rung must be ≥ as
/// good as v1):
/// - `notch_geometry()` → `None`         : v1 placement (centered, top
///   edge at `TOP_MARGIN`). Identical to pre-v1.1.
/// - `top_inset == 0` (no physical notch) : v1 placement.
/// - `top_inset > 0`, ears `None`         : fuse vertically — kiss the
///   very top (`y = my`, no `TOP_MARGIN`), grow the collapsed strip to at
///   least the inset height so it visually sits in the menu-bar band, and
///   keep it horizontally centered. Expanded panels (taller than the
///   inset) start at `my + top_inset` so they drop BELOW the physical
///   notch instead of behind it.
/// - `top_inset > 0`, ears `Some`         : same as ears-`None` for v1.1.
///   We READ + log the ear rects but deliberately do NOT depend on them
///   for layout this release (conservative).
fn fuse_geometry<R: Runtime>(
    win: &tauri::WebviewWindow<R>,
    mx: f64,
    my: f64,
    mw: f64,
    req_w: f64,
    req_h: f64,
) -> (f64, f64, f64, f64) {
    let metrics = notch_geometry(win);
    match metrics {
        Some(m) if m.top_inset > 0.0 => {
            if let Some(ears) = m.left_ear {
                log::debug!("notch: left ear (logical, top-left) = {:?}", ears);
            }
            if let Some(ears) = m.right_ear {
                log::debug!("notch: right ear (logical, top-left) = {:?}", ears);
            }
            // Collapsed strip kisses the top and is at least as tall as
            // the inset so it reads as occupying the menu-bar band; an
            // already-expanded (taller) panel keeps its requested height.
            let is_expanded = req_h > m.top_inset + 1.0;
            let h = req_h.max(m.top_inset).max(HEIGHT);
            let x = mx + (mw - req_w) / 2.0;
            let y = if is_expanded {
                // Drop the expanded panel below the physical notch.
                my + m.top_inset
            } else {
                // Kiss the very top — no TOP_MARGIN gap on a notched
                // display (the inset already accounts for the chin).
                my
            };
            (x, y, req_w, h)
        }
        // None, or a zero inset (no physical notch) → v1 placement.
        _ => {
            let x = mx + (mw - req_w) / 2.0;
            let y = my + TOP_MARGIN;
            (x, y, req_w, req_h)
        }
    }
}

/// Logical frame `(x, y, w, h)` of the display the notch should attach to,
/// honoring `Settings::notch_list_display`:
/// - `"active"` → cursor's display (reuses pill's
///   `active_screen_logical_frame`),
/// - `"builtin"` → the built-in display (assumed primary on macOS),
/// - `"primary"` → the primary display.
/// Falls back to the Tauri primary monitor on any miss.
fn target_frame<R: Runtime>(
    win: &tauri::WebviewWindow<R>,
) -> Option<(f64, f64, f64, f64)> {
    let display = crate::settings::load(win.app_handle()).notch_list_display;

    match display.as_str() {
        "active" => crate::pill::active_screen_logical_frame(win).or_else(|| primary_frame(win)),
        // builtin/primary both resolve to the primary monitor here — macOS
        // reports the built-in panel as the primary in the common case,
        // and we don't ship a separate built-in probe in v1.
        _ => primary_frame(win),
    }
}

fn primary_frame<R: Runtime>(
    win: &tauri::WebviewWindow<R>,
) -> Option<(f64, f64, f64, f64)> {
    win.primary_monitor().ok().flatten().map(|m| {
        let scale = m.scale_factor();
        let s = m.size();
        let p = m.position();
        (
            p.x as f64 / scale,
            p.y as f64 / scale,
            s.width as f64 / scale,
            s.height as f64 / scale,
        )
    })
}

/// Physical-notch geometry for the target display, in **logical**,
/// **top-left-origin** coordinates (the same space `target_frame`
/// returns). All fields are `Option`/zero-defaulted so a failed FFI read
/// degrades to v1 placement via the `fuse_geometry` fallback ladder.
pub(crate) struct NotchMetrics {
    /// `safeAreaInsets.top` — the height of the menu-bar band reserved by
    /// the physical notch (0 on a notchless display / pre-macOS-12).
    top_inset: f64,
    /// `auxiliaryTopLeftArea` — the usable "ear" to the LEFT of the notch
    /// (logical, top-left). Read + logged for v1.1; NOT used for layout.
    left_ear: Option<(f64, f64, f64, f64)>,
    /// `auxiliaryTopRightArea` — the usable "ear" to the RIGHT of the
    /// notch. Read + logged for v1.1; NOT used for layout.
    right_ear: Option<(f64, f64, f64, f64)>,
}

/// Resolve the target `NSScreen` for the current `notch_list_display`
/// setting, mirroring the screen hit-test in
/// `pill::macos_active_screen_frame` (cursor hit-test for `"active"`,
/// firstObject otherwise), and invoke `f` with the borrowed screen plus
/// the primary screen's height (needed for the AppKit→top-left Y flip).
///
/// Passing a closure rather than returning an owned `Retained<NSScreen>`
/// keeps us on the exact borrowed-`&NSScreen` idiom proven in `pill.rs`
/// (`screens.iter().map(|s| s.frame())`) — no `retain()` guesswork. The
/// whole module is macOS-gated, so this only ever compiles there.
/// Returns `None` off the main thread (no screen resolved).
fn with_target_nsscreen<T>(
    display: &str,
    f: impl FnOnce(&objc2_app_kit::NSScreen, f64) -> T,
) -> Option<T> {
    use objc2_app_kit::{NSEvent, NSScreen};
    use objc2_foundation::MainThreadMarker;

    let mtm = MainThreadMarker::new()?;
    let screens = NSScreen::screens(mtm);
    let primary = screens.firstObject()?;
    let primary_height = primary.frame().size.height;

    match display {
        "active" => {
            let cursor = NSEvent::mouseLocation();
            // `iter()` yields borrowed `&NSScreen` — same idiom pill.rs
            // uses. Operate on the borrow in place so nothing needs to
            // outlive the array.
            let hit = screens.iter().find(|s| {
                let fr = s.frame();
                cursor.x >= fr.origin.x
                    && cursor.x < fr.origin.x + fr.size.width
                    && cursor.y >= fr.origin.y
                    && cursor.y < fr.origin.y + fr.size.height
            });
            match hit {
                Some(s) => Some(f(&s, primary_height)),
                None => Some(f(&primary, primary_height)),
            }
        }
        // builtin/primary both resolve to the primary screen — macOS
        // reports the built-in (notched) panel as primary in the common
        // case, matching `target_frame`'s resolution.
        _ => Some(f(&primary, primary_height)),
    }
}

/// Read the physical-notch geometry for the target display via objc2's
/// TYPED `NSScreen` accessors (`safeAreaInsets` / `auxiliaryTopLeftArea` /
/// `auxiliaryTopRightArea`) — the same typed surface `apply_fullscreen_
/// visibility` and `pill.rs` use for `frame()`/`visibleFrame()`, so the
/// struct-return ABI is generated by objc2 rather than hand-rolled
/// `msg_send!`. These selectors have existed on `NSScreen` since macOS 12;
/// Vox's deployment target is macOS 14, so no `respondsToSelector:` guard
/// is needed (they always resolve). A zero `top_inset` (notchless display)
/// returns `None` so the caller falls back to v1 placement. The ear rects
/// are AppKit bottom-left NSRects, converted to logical top-left like
/// `pill.rs`. (The whole module is already macOS-gated at `mod notch;`.)
fn notch_geometry<R: Runtime>(win: &tauri::WebviewWindow<R>) -> Option<NotchMetrics> {
    use objc2_foundation::NSRect;

    let display = crate::settings::load(win.app_handle()).notch_list_display;

    with_target_nsscreen(&display, |screen, primary_height| {
        let top_inset = screen.safeAreaInsets().top;
        if top_inset <= 0.0 {
            // No physical notch on this display → let the caller use v1.
            return None;
        }

        // Each ear is a bottom-left-origin AppKit NSRect; convert to
        // logical top-left like pill.rs. A zero-area rect means "no ear".
        let to_top_left = |rect: NSRect| -> Option<(f64, f64, f64, f64)> {
            if rect.size.width <= 0.0 || rect.size.height <= 0.0 {
                return None;
            }
            let logical_y = primary_height - (rect.origin.y + rect.size.height);
            Some((rect.origin.x, logical_y, rect.size.width, rect.size.height))
        };

        let left_ear = to_top_left(screen.auxiliaryTopLeftArea());
        let right_ear = to_top_left(screen.auxiliaryTopRightArea());

        Some(NotchMetrics {
            top_inset,
            left_ear,
            right_ear,
        })
    })
    .flatten()
}

/// Fullscreen auto-hide probe (Stage 6). Compares the target display's
/// typed `frame()` vs `visibleFrame()`: when the visible frame fills the
/// full frame height AND shares its bottom origin, the menu bar is hidden
/// (likely an app in fullscreen), so we `hide()` the notch; otherwise we
/// `show()` it. Both reads are on objc2's typed surface (no raw
/// `msg_send!`). FAIL-OPEN: a `None`/failed read never hides — the v1
/// behaviour is always-visible. Logs a debug line on every visibility
/// EDGE so an on-device tester can validate the heuristic. Must be called
/// on the MAIN thread (it resolves an `NSScreen` via `MainThreadMarker`) —
/// the follower dispatches it through `run_on_main_thread`.
fn apply_fullscreen_visibility<R: Runtime>(win: &tauri::WebviewWindow<R>) {
    use std::sync::atomic::AtomicBool;

    // Remember the last applied visibility so we only log + act on edges.
    static LAST_HIDDEN: AtomicBool = AtomicBool::new(false);

    let display = crate::settings::load(win.app_handle()).notch_list_display;
    // Compare typed frame() vs visibleFrame() on the borrowed screen.
    // None (couldn't resolve a screen) → never hide (fail-open to v1).
    let Some(fullscreen) = with_target_nsscreen(&display, |screen, _ph| {
        let frame = screen.frame();
        let visible = screen.visibleFrame();
        // Menu bar hidden ⇒ the visible frame reclaims its height and
        // bottom origin. ~1px tolerance for fractional scaling.
        let full_height = (visible.size.height - frame.size.height).abs() <= 1.0;
        let same_bottom = (visible.origin.y - frame.origin.y).abs() <= 1.0;
        full_height && same_bottom
    }) else {
        return;
    };

    let was_hidden = LAST_HIDDEN.load(Ordering::SeqCst);
    if fullscreen && !was_hidden {
        log::debug!("notch: target display entered fullscreen (menu bar hidden) — hiding notch");
        let _ = win.hide();
        LAST_HIDDEN.store(true, Ordering::SeqCst);
    } else if !fullscreen && was_hidden {
        log::debug!("notch: target display left fullscreen (menu bar visible) — showing notch");
        let _ = win.show();
        LAST_HIDDEN.store(false, Ordering::SeqCst);
    }
}

/// Identical panel-style treatment as `pill::apply_macos_panel_style`:
/// status-bar level, all-spaces / fullscreen-auxiliary collection
/// behaviour, non-activating panel style. Hand-mirrored from pill.rs so
/// the objc2 FFI idioms match exactly.
fn apply_macos_panel_style<R: Runtime>(win: &tauri::WebviewWindow<R>) -> anyhow::Result<()> {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::{
        NSStatusWindowLevel, NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask,
    };

    let raw = win
        .ns_window()
        .map_err(|e| anyhow::anyhow!("ns_window: {e}"))?;
    if raw.is_null() {
        return Err(anyhow::anyhow!("ns_window returned NULL"));
    }
    let ns_window: &NSWindow = unsafe { &*(raw as *const NSWindow) };

    unsafe {
        ns_window.setLevel(NSStatusWindowLevel);
        ns_window.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::FullScreenAuxiliary,
        );
        ns_window.setMovableByWindowBackground(false);
        ns_window.setHidesOnDeactivate(false);
        ns_window.setExcludedFromWindowsMenu(true);
        ns_window
            .setStyleMask(NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel);
        let raw_obj: &AnyObject = &*(ns_window as *const NSWindow as *const AnyObject);
        let _: () = msg_send![raw_obj, setCanHide: false];
    }
    Ok(())
}
