//! Notch List window (macOS-only). A transparent, decoration-less,
//! always-on-top **black** panel that TOP-anchors to the display's top edge
//! so its "ear" row flanks the physical camera cutout, with the task drawer
//! hanging below the bump (or just under the top edge on a notchless
//! display). When idle it shrinks to a slim "nub" no wider than the camera
//! bump; it grows downward into clear screen space as it shows tasks. The
//! black surface lets it read as an extension of the physical notch.
//!
//! The window builder + macOS panel style are lifted from `pill.rs` (the
//! proven pattern). KEY DIFFERENCE from the pill: we do NOT call
//! `set_ignore_cursor_events(true)` — the notch needs hover + click (the
//! checkbox) to work. Placement is TOP-anchored at the display top edge, not
//! the pill's bottom-anchored math.
//!
//! Hover is driven by a passive Rust cursor poll (`spawn_hover_poll`) that
//! emits a `notch:hover` bool on enter/exit of the window's on-screen rect.
//! AppKit only delivers `mouseEntered`/`mouseExited` to a non-activating
//! panel once it has been clicked, so CSS `:hover` alone makes the reveal
//! "do nothing unless the notch is focused" — the poll fixes that without an
//! event tap (no Accessibility / TCC) and without stealing keyboard focus.
//!
//! Whole module is `#[cfg(target_os = "macos")]` (gated at the `mod notch;`
//! declaration in `lib.rs`), so this file is only ever compiled on macOS.

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Runtime, WebviewUrl};

use crate::commands::NotchGeometry;

pub const LABEL: &str = "notch";

/// Poll cadence for the display follower + fullscreen probe. Moving the
/// cursor to another monitor (or entering fullscreen) is a coarse user
/// action, so a ~300 ms tick keeps the notch on the right screen without
/// the cost of a tight loop. Mirrors `pill::FOLLOW_POLL_MS`.
const FOLLOW_POLL_MS: u64 = 300;

/// Poll cadence for the hover detector. Hovering the notch must feel
/// instant, so this is far tighter than the display follower — but each
/// tick is cheap (one `NSEvent::mouseLocation()` read + arithmetic) and we
/// only emit on an enter/exit EDGE, so the steady state is near-free. ~70 ms
/// is responsive without the cost of a tight loop. The hot loop also
/// early-outs before the rect math when the cursor is nowhere near the top
/// band (see `hover_tick`).
const HOVER_POLL_MS: u64 = 70;

/// Vertical slack (logical pt) below the notch window's bottom edge inside
/// which the cursor still counts as "near the top band" for the cheap
/// early-out in `hover_tick`. Keeps the early-out from flickering right at
/// the drawer's lower edge.
const HOVER_NEAR_BAND_SLACK: f64 = 24.0;

/// Last emitted hover state (true = cursor inside the notch rect). The hover
/// poll only emits `notch:hover` on a transition, so this stores the edge.
/// Reset to `false` on create/destroy so a fresh window starts collapsed.
static NOTCH_HOVER_INSIDE: AtomicBool = AtomicBool::new(false);

/// Bumped every time the notch window is created or destroyed. The
/// display follower captures the value at spawn time and stands down the
/// moment a create/destroy advances it — exactly mirroring
/// `pill::PILL_GENERATION`. Because the notch window is persistent (one
/// `create` per enable), the follower is spawned once at creation and
/// torn down on destroy.
static NOTCH_GENERATION: AtomicU64 = AtomicU64::new(0);

/// PID of the application that was frontmost when the in-notch quick-add input
/// last took keyboard focus. Captured by `focus_input` BEFORE the notch becomes
/// key (the notch is a non-activating panel, so the OS still reports the user's
/// app as frontmost at that instant) and consumed by `release_input_focus` to
/// hand keyboard control back on Escape. `0` = none captured.
static PRIOR_FRONTMOST_PID: AtomicI32 = AtomicI32::new(0);

/// Initial window size at creation time. The frontend immediately measures
/// its content and drives the real size via `set_size` — the idle "nub" is
/// ~camera-bump wide and slim — so these are just pre-mount placeholders,
/// kept close to the resting nub to avoid a first-paint jump.
pub const WIDTH: f64 = 200.0;
pub const HEIGHT: f64 = 16.0;
/// Floors for frontend-driven sizing. The idle nub is deliberately small
/// ("no wider than the camera bump, very little height"), so these sit well
/// below any natural content size and only guard a zero/negative request.
const MIN_WIDTH: f64 = 48.0;
const MIN_HEIGHT: f64 = 8.0;
/// Gap below the physical notch band (or below the top edge on a notchless
/// display) so the panel reads as a black pill HANGING beneath the camera
/// bump rather than fused into it — and, crucially, never behind it.
const TOP_MARGIN: f64 = 2.0;

/// Cached physical-notch geometry for the current target display, refreshed
/// on the macOS main thread by `refresh_geometry_cache` (from `create` and
/// each follower tick, where the `NSScreen` reads resolve). Two consumers:
/// `commands::notch_geometry` hands this snapshot to the webview on mount,
/// and `fuse_geometry` reads it for placement so the hot resize path never
/// needs a main-thread `NSScreen` call. The `notch:geometry` event pushes
/// changes (e.g. a cross-monitor hop) to the webview.
static GEOMETRY: Mutex<NotchGeometry> = Mutex::new(NotchGeometry {
    has_notch: false,
    width: 0.0,
    height: 0.0,
});

/// Snapshot the cached notch geometry. Cheap (a mutex read) and thread-safe,
/// so the command handler can call it directly off the main thread.
pub fn geometry_snapshot() -> NotchGeometry {
    GEOMETRY.lock().map(|g| *g).unwrap_or_default()
}

/// Re-read the target display's physical-notch geometry (MAIN THREAD only —
/// it touches `NSScreen`), store it in `GEOMETRY`, and on a change emit
/// `notch:geometry` so the webview can resize its nub. Returns whether the
/// geometry changed (the follower uses this to re-place the window when the
/// notch metrics shift without a cursor move). Fail-open: a missing window
/// or failed read leaves the cache untouched and returns `false`.
fn refresh_geometry_cache<R: Runtime>(app: &AppHandle<R>) -> bool {
    let Some(win) = app.get_webview_window(LABEL) else {
        return false;
    };
    let next = match notch_geometry(&win) {
        Some(m) => NotchGeometry {
            has_notch: true,
            width: m.notch_width,
            height: m.top_inset,
        },
        // Notchless display (or an unreadable screen) → no notch.
        None => NotchGeometry::default(),
    };
    let changed = match GEOMETRY.lock() {
        Ok(mut slot) => {
            let changed = *slot != next;
            *slot = next;
            changed
        }
        Err(_) => false,
    };
    if changed {
        if let Err(e) = app.emit("notch:geometry", next) {
            log::warn!("notch: failed to emit notch:geometry: {e}");
        }
    }
    changed
}

/// Create (or re-show) the notch window. Mirrors `pill::create`:
/// transparent, decoration-less, always-on-top, skip-taskbar, non-focusing.
/// Unlike the pill it stays interactive (no `set_ignore_cursor_events`).
///
/// The notch window is **persistent**: turning the feature off HIDES it (the
/// `CloseRequested` handler in `lib.rs` keeps it alive), so a later re-enable
/// finds the window still in the registry. In that case we re-show + re-arm
/// it instead of early-returning — the old early-return left the hidden window
/// hidden forever, so toggling the notch off then on in Settings showed
/// nothing.
pub fn create<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window(LABEL) {
        if let Err(e) = window.show() {
            log::warn!("notch: failed to re-show window: {e}");
        }
        arm(app, &window);
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
            // NO OS drop shadow. Against the wallpaper its soft halo reads as a
            // greyish rim around the otherwise pure-#000 ears, making them look
            // less black than the physical (true-black) camera bump they're
            // meant to extend. The panel is solid #000 with rounded corners and
            // needs no extra depth, so we draw it flat — the ears stay pitch
            // black with crisp edges.
            .shadow(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .focused(false)
            .visible(true)
            .accept_first_mouse(true)
            .build()?;

    // The macOS panel style (status-bar level, all-spaces collection
    // behaviour, non-activating) persists on the live window, so it's applied
    // once here on first build — `arm` (which also runs on re-show) skips it.
    if let Err(e) = apply_macos_panel_style(&window) {
        log::warn!("notch: failed to apply macOS panel style: {e}");
    }

    arm(app, &window);

    Ok(())
}

/// (Re)establish the notch's geometry cache, top-anchored placement, hover
/// edge, and the two background pollers under a fresh generation. Run once on
/// first build AND on every re-show: a `destroy` bumps the generation to stand
/// the old followers down, so a re-enabled notch needs them spawned anew.
fn arm<R: Runtime>(app: &AppHandle<R>, window: &tauri::WebviewWindow<R>) {
    // Populate the geometry cache up front so the very first `reposition`
    // (and the webview's first `notch_geometry` read) already know the
    // camera-bump width and band height. On the common launch path this runs
    // on the main thread and resolves the real notch; on the off-main
    // settings-toggle path the `NSScreen` read no-ops and the follower
    // refreshes + re-places within one tick.
    refresh_geometry_cache(app);

    if let Err(e) = reposition(window) {
        log::warn!("notch reposition failed: {e}");
    }

    // The window is persistent, so we re-key the pollers on a fresh
    // generation each time: a later destroy bumps the counter and the old
    // follower + hover poll stand down. A fresh/re-shown window starts
    // collapsed (not hovered) — seed the edge state so the first real enter
    // emits a `true` transition.
    let generation = NOTCH_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    NOTCH_HOVER_INSIDE.store(false, Ordering::SeqCst);
    spawn_display_follower(app.clone(), generation);
    spawn_hover_poll(app.clone(), generation);
}

/// Hide the notch window when the user turns the feature off. The window is
/// **persistent** (kept alive in the registry, mirroring the pill/settings
/// windows) so a later re-enable re-shows it instantly via `create`; we hide
/// rather than close. Named `destroy` for its lifecycle role — the user-facing
/// effect is the notch disappearing.
pub fn destroy<R: Runtime>(app: &AppHandle<R>) {
    // Bump the generation FIRST so the running follower + hover poll stand
    // down on their next tick instead of touching a window we're about to
    // hide.
    NOTCH_GENERATION.fetch_add(1, Ordering::SeqCst);
    // Clear the hover edge so a re-shown window starts collapsed.
    NOTCH_HOVER_INSIDE.store(false, Ordering::SeqCst);
    if let Some(win) = app.get_webview_window(LABEL) {
        if let Err(e) = win.hide() {
            log::warn!("notch: failed to hide window: {e}");
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

    let target_w = width.unwrap_or(cur_w).max(MIN_WIDTH);
    let target_h = height.unwrap_or(cur_h).max(MIN_HEIGHT);

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

/// Make the notch the key window so its in-notch quick-add input receives
/// keystrokes, first stashing the currently-frontmost app's PID so Escape can
/// restore it. The notch's `NonactivatingPanel` style mask means becoming key
/// does NOT activate Vox — the user's app stays the active application; only
/// the key window moves to the notch. tao leaves the window `focusable`, so
/// `canBecomeKeyWindow` is true and `makeKeyAndOrderFront:` succeeds.
///
/// We deliberately do NOT use Tauri/tao `set_focus()`: tao's implementation
/// also fires `[NSApp activateIgnoringOtherApps:YES]`, which would
/// force-foreground the whole Vox process (stealing the menu bar / app
/// activation) — exactly what the non-activating panel design avoids. Sending
/// `makeKeyAndOrderFront:` straight to the NSWindow keeps activation untouched.
pub fn focus_input<R: Runtime>(app: &AppHandle<R>) {
    // Capture the frontmost app BEFORE we take key focus. Because the notch is
    // a non-activating panel, taking key focus won't change the active app, so
    // this snapshot is the app keystrokes should return to on dismiss.
    let pid = frontmost_app_pid().unwrap_or(0);
    PRIOR_FRONTMOST_PID.store(pid, Ordering::SeqCst);
    // `makeKeyAndOrderFront:` isn't thread-safe — hop to the main thread.
    let app_main = app.clone();
    let _ = app.run_on_main_thread(move || make_notch_key(&app_main));
}

/// Send `makeKeyAndOrderFront:` directly to the notch's NSWindow (MAIN THREAD)
/// so it becomes the key window WITHOUT activating Vox. Mirrors the raw NSWindow
/// access in `apply_macos_panel_style`. No window / null handle → no-op.
fn make_notch_key<R: Runtime>(app: &AppHandle<R>) {
    use objc2_app_kit::NSWindow;
    let Some(win) = app.get_webview_window(LABEL) else {
        return;
    };
    let raw = match win.ns_window() {
        Ok(p) if !p.is_null() => p,
        _ => return,
    };
    let ns_window: &NSWindow = unsafe { &*(raw as *const NSWindow) };
    // `makeKeyAndOrderFront:` is a safe objc2 method — only the pointer deref
    // above needs `unsafe`. Becoming key here does NOT activate Vox: the panel's
    // NonactivatingPanel style mask keeps the user's app the active app.
    ns_window.makeKeyAndOrderFront(None);
}

/// Hand keyboard focus back to the app that was frontmost when the quick-add
/// input opened (Escape path). Best-effort: a zero/stale PID or a failed lookup
/// just leaves focus where it is. The AppKit activation runs on the main
/// thread. Clears the stored PID either way so a later open re-captures fresh.
pub fn release_input_focus<R: Runtime>(app: &AppHandle<R>) {
    let pid = PRIOR_FRONTMOST_PID.swap(0, Ordering::SeqCst);
    if pid <= 0 {
        return;
    }
    let _ = app.run_on_main_thread(move || reactivate_app_by_pid(pid));
}

/// PID of the frontmost application via `NSWorkspace`. `None` on failure.
/// Mirrors `frontmost::frontmost_id_and_pid`'s NSWorkspace read (safe off the
/// main thread). macOS-only (the whole module is macOS-gated).
fn frontmost_app_pid() -> Option<i32> {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::NSWorkspace;
    unsafe {
        let workspace = NSWorkspace::sharedWorkspace();
        let app = workspace.frontmostApplication()?;
        let app_obj: &AnyObject = &app;
        let pid: i32 = msg_send![app_obj, processIdentifier];
        (pid > 0).then_some(pid)
    }
}

/// Re-activate the app with `pid` so its focused window becomes key again,
/// returning keyboard control to it. MUST run on the main thread (AppKit
/// activation). Best-effort: a missing process just no-ops.
fn reactivate_app_by_pid(pid: i32) {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    unsafe {
        let cls = class!(NSRunningApplication);
        let running: *mut AnyObject = msg_send![cls, runningApplicationWithProcessIdentifier: pid];
        if running.is_null() {
            return;
        }
        // `activateWithOptions:` with no flags re-keys the app's focused window
        // (it was already the active app, since the notch panel is
        // non-activating — only the key window moved). Deprecated in macOS 14
        // but still functional, and the simplest "give the keyboard back" call.
        let _: bool = msg_send![running, activateWithOptions: 0u64];
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
        // Refresh the cached camera-bump geometry first (MAIN-THREAD
        // `NSScreen` read) and learn whether it changed. A change with no
        // cursor move still needs a re-place: `fuse_geometry` anchors the
        // panel off the cached notch band height, so a notch that just
        // became known (off-main create) or differs on the new display must
        // re-run placement.
        let geo_changed = refresh_geometry_cache(&app_main);
        // Re-center on a display change OR a geometry change (preserving the
        // current window size so a hover-expanded panel keeps its width
        // across the hop).
        if moved || geo_changed {
            if let Some(f) = target_frame(&win) {
                reposition_to_frame(&win, f);
            }
        }
        // Stage 6: hide the notch when the target display is in fullscreen
        // (menu bar hidden). Fail-open — a None/failed read never hides.
        apply_fullscreen_visibility(&win);
    });
}

/// Passive hover detector. Polls the global cursor position on a tight
/// cadence and emits `notch:hover` (a bare bool) whenever the cursor ENTERS
/// or LEAVES the notch window's on-screen rect. This is the IPC the frontend
/// uses to expand/collapse the panel, and it fixes "hover does nothing unless
/// the notch is focused": a non-activating panel only gets
/// `mouseEntered`/`mouseExited` after it's been clicked, so CSS `:hover`
/// alone is dead until first focus.
///
/// FOCUS-SAFE / NO TCC: this reads `NSEvent::mouseLocation()` — a passive
/// global cursor read, NOT an event tap — so it needs no Accessibility grant
/// and never steals keyboard focus; the foreground app stays key. Same
/// passive read `with_target_nsscreen` already uses for the `"active"`
/// display hit-test.
///
/// Keyed on `NOTCH_GENERATION` (captured as `generation`) exactly like the
/// display follower: a create/destroy bumps the counter and this task stands
/// down on its next tick.
fn spawn_hover_poll<R: Runtime>(app: AppHandle<R>, generation: u64) {
    tauri::async_runtime::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_millis(HOVER_POLL_MS));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            // A create()/destroy() since spawn bumped the generation — stand
            // down so we never touch a window we no longer own.
            if NOTCH_GENERATION.load(Ordering::SeqCst) != generation {
                return;
            }
            if app.get_webview_window(LABEL).is_none() {
                return;
            }
            // The cursor + screen reads are main-thread-only (NSEvent /
            // NSScreen via MainThreadMarker), so hop on. The closure is
            // cheap: one cursor read, one rect compute, an edge compare, and
            // an emit only on a transition.
            let app_main = app.clone();
            let _ = app.run_on_main_thread(move || hover_tick(&app_main));
        }
    });
}

/// One hover iteration (MAIN THREAD — touches `NSEvent`/`NSScreen`). Computes
/// the notch window's current on-screen rect, hit-tests the global cursor
/// against it, and emits `notch:hover` only on an enter/exit EDGE.
///
/// The rect is derived from the SAME `target_frame()` + `fuse_geometry()`
/// math the placement path uses (reading the live window size so the hit rect
/// tracks the hover-expanded drawer, not just the idle nub) rather than
/// reading the window's reported frame — this keeps the hit rect in lockstep
/// with where we actually placed the window, including the new top-anchor.
///
/// COORDINATES: `NSEvent::mouseLocation()` is in AppKit's bottom-left origin
/// (relative to the primary screen); `fuse_geometry` returns Tauri's
/// top-left logical frame. We flip the cursor's Y into top-left space the
/// same way `pill::macos_active_screen_frame` does
/// (`top_left_y = primary_height - appkit_y`) before comparing.
fn hover_tick<R: Runtime>(app: &AppHandle<R>) {
    use objc2_app_kit::{NSEvent, NSScreen};
    use objc2_foundation::MainThreadMarker;

    let Some(win) = app.get_webview_window(LABEL) else {
        return;
    };

    // Skip while hidden (e.g. fullscreen auto-hide): never hit-test/emit on a
    // window the user can't see. If we were "inside" when it hid, drop the flag
    // (and emit a final `false`) so a later re-show + hover re-triggers cleanly.
    if let Ok(false) = win.is_visible() {
        if NOTCH_HOVER_INSIDE.swap(false, Ordering::SeqCst) {
            let _ = app.emit_to(LABEL, "notch:hover", false);
        }
        return;
    }

    // Resolve the cursor (AppKit bottom-left) + the primary screen height for
    // the Y flip. Bail (no state change) if we can't read a screen.
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(primary) = NSScreen::screens(mtm).firstObject() else {
        return;
    };
    let primary_height = primary.frame().size.height;
    let cursor = NSEvent::mouseLocation();
    // Flip the cursor into Tauri's top-left logical space.
    let cx = cursor.x;
    let cy = primary_height - cursor.y;

    // The notch's current on-screen rect, derived from the same placement
    // math (top-anchored) using the live window size so the rect covers the
    // ears AND the expanded drawer.
    let Some((mx, my, mw, _mh)) = target_frame(&win) else {
        return;
    };
    let scale = win.scale_factor().unwrap_or(1.0);
    let cur = win.inner_size().ok();
    let cur_w = cur.as_ref().map(|s| s.width as f64 / scale).unwrap_or(WIDTH);
    let cur_h = cur.map(|s| s.height as f64 / scale).unwrap_or(HEIGHT);
    let (x, y, w, h) = fuse_geometry(mx, my, mw, cur_w, cur_h);

    // Cheap early-out: if the cursor sits well below the window's bottom edge
    // (and we're not currently inside), it can't be a hover — skip the full
    // compare. The slack keeps this from flickering right at the drawer edge.
    let was_inside = NOTCH_HOVER_INSIDE.load(Ordering::SeqCst);
    if !was_inside && cy > y + h + HOVER_NEAR_BAND_SLACK {
        return;
    }

    let inside = cx >= x && cx < x + w && cy >= y && cy < y + h;
    if inside == was_inside {
        return; // No edge — stay quiet.
    }
    NOTCH_HOVER_INSIDE.store(inside, Ordering::SeqCst);
    if let Err(e) = app.emit_to(LABEL, "notch:hover", inside) {
        log::warn!("notch: failed to emit notch:hover: {e}");
    }
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
        let (x, y, w, h) = fuse_geometry(mx, my, mw, WIDTH, HEIGHT);
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
        let (x, y, w, h) = fuse_geometry(mx, my, mw, target_w, target_h);
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
/// PLACEMENT: on a notched display the window TOP-anchors to the very top
/// edge of the display (`y = my`) so it spans the menu-bar/notch band. The
/// frontend lays out the "Vox"/"Notch" ear blocks beside the camera at the
/// top and hangs the task drawer below the bump — so the window must START at
/// the top edge and be tall enough (height is frontend-driven via
/// `notchResize`/`set_size`) for the drawer to clear the bump. The anchor
/// moved up from the prior "hang just below the band" math because the ears
/// have to FLANK the physical cutout, which is only possible if the window
/// reaches the top edge. TRADE-OFF: in the ear regions the window now overlaps
/// the immediate menu-bar band beside the camera — legitimate, since the
/// window is `NSStatusWindowLevel` + always-on-top and draws there by design.
/// On a notchless display there's no bump to flank, so we keep the slight
/// `TOP_MARGIN` offset below the top edge (`y = my + TOP_MARGIN`).
///
/// Either way the panel is centered on the cutout (screen center) and its size
/// is exactly what the frontend requested — the idle "nub" is sized to the
/// camera-bump width by the webview, and taller list panels grow downward
/// into clear screen space.
///
/// Reads the cached `GEOMETRY` (kept fresh on the main thread by
/// `refresh_geometry_cache`) rather than calling `NSScreen` live, so this
/// hot path (every hover resize) is thread-agnostic and cheap.
fn fuse_geometry(mx: f64, my: f64, mw: f64, req_w: f64, req_h: f64) -> (f64, f64, f64, f64) {
    let geo = geometry_snapshot();
    let y = if geo.has_notch && geo.height > 0.0 {
        // Top-anchor to the display's top edge so the ear row sits in the
        // notch band beside the camera and the drawer hangs below the bump.
        my
    } else {
        // Notchless display: no bump to flank, so sit just under the top edge.
        my + TOP_MARGIN
    };
    let x = mx + (mw - req_w) / 2.0;
    (x, y, req_w, req_h.max(MIN_HEIGHT))
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

/// Physical-notch geometry for the target display, in **logical** points.
/// A failed FFI read degrades to a notchless `None` (→ top-edge placement)
/// via the `notch_geometry` early return.
pub(crate) struct NotchMetrics {
    /// `safeAreaInsets.top` — the height of the menu-bar band reserved by
    /// the physical notch (0 on a notchless display / pre-macOS-12). Used
    /// as the vertical anchor: the panel hangs just below this band.
    top_inset: f64,
    /// Width of the physical notch itself, derived as the screen width minus
    /// the two flanking "ear" areas (`auxiliaryTopLeft/RightArea`). The
    /// webview sizes its idle nub to this so it never spills past the camera
    /// bump. `0.0` when the ears can't be read (→ webview default nub width).
    notch_width: f64,
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
/// returns `None` so the caller treats the display as notchless. The notch
/// width is derived from the two ear NSRects' widths (origin/flip are
/// irrelevant — only the widths matter). (The whole module is already
/// macOS-gated at `mod notch;`.)
fn notch_geometry<R: Runtime>(win: &tauri::WebviewWindow<R>) -> Option<NotchMetrics> {
    use objc2_foundation::NSRect;

    let display = crate::settings::load(win.app_handle()).notch_list_display;

    with_target_nsscreen(&display, |screen, _primary_height| {
        let top_inset = screen.safeAreaInsets().top;
        if top_inset <= 0.0 {
            // No physical notch on this display → caller treats as notchless.
            return None;
        }

        // Notch width = full screen width − the two flanking ear areas. Only
        // each ear's WIDTH matters; a zero-area ear means "unreadable", so we
        // report width 0 and let the webview fall back to a default nub.
        let ear_width = |rect: NSRect| -> Option<f64> {
            (rect.size.width > 0.0 && rect.size.height > 0.0).then_some(rect.size.width)
        };
        let notch_width = match (
            ear_width(screen.auxiliaryTopLeftArea()),
            ear_width(screen.auxiliaryTopRightArea()),
        ) {
            (Some(left), Some(right)) => (screen.frame().size.width - left - right).max(0.0),
            _ => 0.0,
        };

        Some(NotchMetrics {
            top_inset,
            notch_width,
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
        // Cross-focus hover is driven by the Rust `notch:hover` poll
        // (`hover_tick`), NOT by AppKit moved events: a non-activating panel
        // won't deliver mouseMoved / mouseEntered / mouseExited to the WebView
        // unless it's the key window, so DOM `:hover` alone wouldn't fire while
        // another app is focused. This call keeps CSS `:hover` / onMouseEnter
        // working as a retained SECONDARY path for when the notch IS key (e.g.
        // just after a click); harmless otherwise, so we keep it.
        let _: () = msg_send![raw_obj, setAcceptsMouseMovedEvents: true];
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both scenarios live in one test (run sequentially) so the two writes
    /// to the global `GEOMETRY` cache can't race — `cargo test` runs separate
    /// test functions on parallel threads, and this is the only test that
    /// touches the cache.
    #[test]
    fn fuse_geometry_top_anchors_on_notched_display() {
        // Notched display: the window TOP-anchors to the display's top edge
        // (y == my) so the ear row flanks the camera cutout, and stays
        // centered on the notch (= screen center) at exactly the requested
        // size. The drawer hangs below the bump via the frontend-driven
        // height.
        *GEOMETRY.lock().unwrap() = NotchGeometry {
            has_notch: true,
            width: 200.0,
            height: 37.0,
        };
        let (x, y, w, h) = fuse_geometry(100.0, 50.0, 1512.0, 200.0, 14.0);
        assert_eq!(y, 50.0, "top-anchored to the display top edge (my)");
        assert_eq!(x, 100.0 + (1512.0 - 200.0) / 2.0, "centered on the notch");
        assert_eq!(w, 200.0, "width is honored verbatim");
        assert_eq!(h, 14.0, "height is honored verbatim");

        // Notchless display: just under the top edge (no bump to flank), and
        // the height floor guards a too-small request.
        *GEOMETRY.lock().unwrap() = NotchGeometry::default();
        let (_x, y2, _w, h2) = fuse_geometry(0.0, 0.0, 1440.0, 168.0, 4.0);
        assert_eq!(y2, TOP_MARGIN, "notchless: just below the top edge");
        assert_eq!(h2, MIN_HEIGHT, "height floored at MIN_HEIGHT");
    }
}
