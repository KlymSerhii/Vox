//! Background auto-update scheduler.
//!
//! Owns the "check once a day for a new Vox release" state machine
//! and the cross-window `updates:state` event that the tray popover
//! banner + Settings → About + tray-icon dot all subscribe to. The
//! actual HTTPS round-trip to fetch `latest.json` and the post-pick
//! download + install path still live in `commands.rs` — we just
//! drive them from a wall-clock-aware loop.
//!
//! ## Cadence (decided by PM)
//!
//! - Initial check: 60 s after launch.
//! - Recurring: every 24 h while the app stays open.
//! - Wake-from-sleep: re-check if the last successful background
//!   check was >24 h ago, with a 30 s debounce so a flurry of
//!   wake events doesn't burst-check.
//! - Cooldown: never check more than once per 6 h regardless of
//!   trigger. The manual button in Settings → About bypasses the
//!   cooldown.
//! - Timer source: wall-clock `SystemTime`, not `tokio::Instant`,
//!   so sleeping a Mac for two days while the app is open doesn't
//!   silently skip a check. `should_check_now()` is the pure
//!   helper that decides; the tokio loop just polls every 60 s and
//!   asks it.
//!
//! Failed background checks are silent — no toast, no banner, no
//! pill. They retry on the next scheduled tick. The only place a
//! failed check surfaces is the manual button in Settings → About.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::commands::{self, UpdateCheckResult};
use crate::settings;

/// 24 h, the desired recurring cadence.
pub const RECURRING_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// 6 h floor. Any trigger that comes in faster than this gets
/// rate-limited unless it's the manual button in Settings.
pub const COOLDOWN: Duration = Duration::from_secs(6 * 60 * 60);
/// Initial-check delay after launch. Long enough that the user has
/// settled into onboarding before a network round-trip kicks off,
/// short enough that the popover banner is "live" for the rest of
/// the session.
pub const INITIAL_DELAY: Duration = Duration::from_secs(60);
/// Tokio polling granularity. We tick once a minute and compare
/// against wall-clock — that's robust to system sleep across a tick.
pub const POLL_INTERVAL: Duration = Duration::from_secs(60);
/// Debounce after wake-from-sleep so a flood of wake notifications
/// doesn't translate into a burst of HTTPS requests.
pub const WAKE_DEBOUNCE: Duration = Duration::from_secs(30);

/// What the frontend renders. Mirrors the `UpdateCheckResult` shape
/// from `commands.rs` but adds the dismissal-aware fields the tray
/// banner + tray-icon dot need to decide whether to render. Emitted
/// on `updates:state` whenever any field changes; the latest value
/// is also returned synchronously by the `get_update_state` IPC
/// command so a window opening fresh doesn't have to wait for the
/// next emit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateState {
    /// `idle` | `checking` | `available` | `up-to-date` |
    /// `error` | `unconfigured`. Mirrors `UpdateCheckResult::status`
    /// plus a `checking` transient for the in-flight UI.
    pub status: String,
    pub current_version: String,
    /// Version pulled from `latest.json`. Populated only on the
    /// `available` branch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available_version: Option<String>,
    /// Release notes body, same content as `UpdateCheckResult::release_notes`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_notes: Option<String>,
    /// Unix epoch seconds of the last successful HTTPS check
    /// (background or manual). `None` until the first check lands.
    /// Stored on the persistent on-disk `Settings` doc so a relaunch
    /// after a recent check doesn't trigger an immediate redundant
    /// fetch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_check_at: Option<i64>,
    /// Last error message from a failed check. Cleared on the next
    /// successful check. Surfaced in Settings → About; the tray
    /// banner deliberately ignores it (failed background checks are
    /// silent per spec).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// Version the user "Later"-dismissed. While this equals
    /// `available_version`, the tray banner + tray-icon dot stay
    /// hidden; Settings → About still shows the pending state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dismissed_version: Option<String>,
}

impl UpdateState {
    fn idle(current: String) -> Self {
        Self {
            status: "idle".into(),
            current_version: current,
            available_version: None,
            release_notes: None,
            last_check_at: None,
            last_error: None,
            dismissed_version: None,
        }
    }

    /// Banner + tray-dot visibility. The Settings → About row is the
    /// dedicated surface for the pending state and is intentionally
    /// not gated on the dismissal flag — that's why the predicate
    /// lives here on the state struct rather than baked into the
    /// `status == "available"` check on the frontend.
    pub fn banner_visible(&self) -> bool {
        if self.status != "available" {
            return false;
        }
        match (
            self.available_version.as_deref(),
            self.dismissed_version.as_deref(),
        ) {
            (Some(av), Some(dv)) => av != dv,
            (Some(_), None) => true,
            _ => false,
        }
    }
}

pub struct UpdaterState {
    pub current: Mutex<UpdateState>,
    /// Guard against concurrent `perform_check` calls. Two triggers
    /// arriving simultaneously (wake + tokio tick + manual click) used
    /// to each fire their own HTTPS round-trip and event emit; we now
    /// short-circuit the second caller. Wrapped in an `InFlightGuard`
    /// so it resets on early return / panic too.
    pub in_flight: AtomicBool,
    /// Wake-from-sleep debouncer flag. Set to true on the first wake
    /// notification in a burst; the spawned 30 s task clears it right
    /// before running the check. Subsequent wake events during the
    /// 30 s window are coalesced into the already-pending fire.
    pub wake_armed: AtomicBool,
}

impl UpdaterState {
    pub fn new(current_version: String) -> Self {
        Self {
            current: Mutex::new(UpdateState::idle(current_version)),
            in_flight: AtomicBool::new(false),
            wake_armed: AtomicBool::new(false),
        }
    }
}

/// RAII guard that releases the `in_flight` flag on drop so an early
/// return / panic in `perform_check` doesn't deadlock subsequent
/// callers.
struct InFlightGuard {
    state: Arc<UpdaterState>,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.state.in_flight.store(false, Ordering::Release);
    }
}

/// Emit the current updater state to every webview AND poke the tray
/// icon so the corner dot mirrors the banner visibility. Re-reads the
/// authoritative snapshot from the in-process state before emitting
/// so a concurrent mutation doesn't ship a stale payload.
fn emit_state<R: Runtime>(app: &AppHandle<R>) {
    let Some(state) = app.try_state::<Arc<UpdaterState>>() else {
        return;
    };
    let snap = state.current.lock().clone();
    let badge_visible = snap.banner_visible();
    if let Err(e) = app.emit("updates:state", &snap) {
        log::warn!("failed to emit updates:state: {e}");
    }
    crate::tray_icon::set_update_pending(app, badge_visible);
}

/// Pure scheduling decision — does the current moment warrant a
/// background check? `last_check_at` is the wall-clock timestamp of
/// the most recent SUCCESSFUL check (failed checks don't refresh it,
/// so a long offline run still re-tries on every poll). `now` is
/// `SystemTime::now()` in production; the `cooldown` parameter is
/// taken from `COOLDOWN` in production but threaded through here for
/// unit-testability.
///
/// Returns `true` when no previous check has been recorded, OR when
/// `now - last_check_at` has elapsed BOTH the 6 h cooldown AND the
/// 24 h recurring cadence. The manual button does NOT route through
/// this helper; it bypasses cooldown by design.
pub fn should_check_now(
    last_check_at: Option<SystemTime>,
    now: SystemTime,
    cooldown: Duration,
) -> bool {
    let Some(last) = last_check_at else {
        // First-ever check — only the post-launch initial-delay gate
        // (handled by the caller) and the cooldown apply; with no
        // prior check the cooldown trivially passes.
        return true;
    };
    let elapsed = now.duration_since(last).unwrap_or_default();
    if elapsed < cooldown {
        return false;
    }
    elapsed >= RECURRING_INTERVAL
}

/// Spawn the wall-clock-aware scheduler loop. Cheap and idempotent at
/// startup — we don't track whether a loop is already running because
/// `lib.rs::run` only calls this once during `setup()`.
///
/// The first check fires after `INITIAL_DELAY`, subsequent checks
/// every `POLL_INTERVAL` ticks, with `should_check_now` deciding
/// whether each tick translates into an actual HTTPS round-trip.
/// `auto_update_check_enabled = false` in Settings turns the check
/// step into a no-op without stopping the loop — flipping the toggle
/// back on resumes immediately on the next tick.
pub fn spawn_scheduler<R: Runtime>(app: &AppHandle<R>) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        // Hold the initial delay before the first check so the user
        // isn't competing with onboarding-time network use.
        tokio::time::sleep(INITIAL_DELAY).await;
        // First check eagerly (subject to cooldown — a relaunch
        // within 6 h of the last check sees the cooldown short-circuit
        // and waits for the next interval).
        run_scheduled_check(&handle).await;
        loop {
            tokio::time::sleep(POLL_INTERVAL).await;
            run_scheduled_check(&handle).await;
        }
    });
}

/// Wire the macOS wake-from-sleep notification through the same
/// `run_scheduled_check` pipeline as the periodic loop. The
/// notification fires on the notification's posting thread, so we
/// hop into tokio before doing any real work.
///
/// Coalesces a burst of wake notifications (which can fire repeatedly
/// during AC-attach / lid-open sequences) into a single fire 30 s
/// after the first event in the burst. The `wake_armed` flag is set
/// only when no debounce task is already pending; subsequent wakes
/// inside the 30 s window are dropped.
pub fn subscribe_wake<R: Runtime>(app: &AppHandle<R>) {
    let handle = app.clone();
    crate::mac_wake::subscribe(move || {
        let handle = handle.clone();
        let Some(state) = handle.try_state::<Arc<UpdaterState>>() else {
            return;
        };
        let state = state.inner().clone();
        if state
            .wake_armed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            // A debounce task is already pending — coalesce.
            return;
        }
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(WAKE_DEBOUNCE).await;
            // Clear the flag BEFORE running the check so a wake
            // notification arriving during the (possibly long) HTTPS
            // round-trip can rearm for the next fire.
            state.wake_armed.store(false, Ordering::Release);
            run_scheduled_check(&handle).await;
        });
    });
}

/// One scheduler tick. Reads the latest Settings ONCE per call so the
/// `auto_update_check_enabled` toggle takes effect within a poll
/// cycle (no need to restart the loop). Skips on disabled, on
/// cooldown, or when the cadence hasn't elapsed yet.
async fn run_scheduled_check<R: Runtime>(app: &AppHandle<R>) {
    let s = settings::load(app);
    if !s.auto_update_check_enabled {
        return;
    }
    let last_check_at = s
        .last_update_check_at
        .filter(|n| *n > 0)
        .map(|secs| SystemTime::UNIX_EPOCH + Duration::from_secs(secs as u64));
    let now = SystemTime::now();
    if !should_check_now(last_check_at, now, COOLDOWN) {
        return;
    }
    perform_check(app, /*manual=*/ false).await;
}

/// Perform one HTTPS round-trip against the updater plugin, update
/// the in-process state, emit `updates:state`, and (on success)
/// persist `last_update_check_at`. Manual checks bypass the cooldown
/// in `run_scheduled_check`; they call directly through here.
///
/// Two concurrent callers (e.g. wake + tokio tick + manual click) are
/// gated by the `in_flight` flag on `UpdaterState`. Background
/// re-entries return early and silently; a manual re-entry returns
/// the current snapshot rather than firing a redundant request.
///
/// Returns the resulting `UpdateState` so the manual command can
/// echo a snapshot back to its caller, but the IPC command itself
/// (in `commands.rs`) typically just relies on the emitted event.
pub async fn perform_check<R: Runtime>(app: &AppHandle<R>, manual: bool) -> UpdateState {
    // Grab the shared `UpdaterState` once up front; the in-flight guard
    // and the existing `current` mutation both need it.
    let state: Arc<UpdaterState> = match app.try_state::<Arc<UpdaterState>>() {
        Some(s) => s.inner().clone(),
        None => {
            return UpdateState::idle(app.package_info().version.to_string());
        }
    };

    if state
        .in_flight
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        // Another caller is already mid-check. Avoid a redundant HTTPS
        // round-trip / event emit. Manual callers still get a
        // synchronous snapshot of the current (in-flight) state so the
        // UI doesn't dead-end on the click.
        let snap = state.current.lock().clone();
        if manual {
            log::info!(
                "updater: manual check coalesced with in-flight check -> status={}",
                snap.status
            );
        }
        return snap;
    }
    // Holds the flag until function exit (including early return /
    // panic). Constructed AFTER the compare_exchange so we don't try
    // to release a flag we didn't acquire.
    let _guard = InFlightGuard {
        state: state.clone(),
    };

    // Transient `checking` state for the in-flight UI. We hold the
    // dismissed_version + current_version across the transition so
    // the tray-icon dot doesn't flash off mid-check.
    {
        let mut cur = state.current.lock();
        cur.status = "checking".into();
        cur.last_error = None;
    }
    emit_state(app);

    let result: UpdateCheckResult = commands::check_for_updates(app.clone()).await;

    // Merge the result into the in-process state. `available_version`
    // and `release_notes` follow the response; on error / up-to-date
    // they're cleared so the UI doesn't latch onto a stale "v1.5.0 is
    // ready" message after a successful follow-up check moves us to
    // up-to-date.
    let snapshot: UpdateState = {
        let mut cur = state.current.lock();
        cur.status = result.status.clone();
        if let Some(v) = result.current_version.clone() {
            cur.current_version = v;
        }
        match result.status.as_str() {
            "available" => {
                cur.available_version = result.available_version.clone();
                cur.release_notes = result.release_notes.clone();
                cur.last_error = None;
            }
            "up-to-date" => {
                cur.available_version = None;
                cur.release_notes = None;
                cur.last_error = None;
                // Clear the dismissal so the next future release
                // automatically re-arms the banner — without this, a
                // user who dismissed v1.5 would dismiss v1.6 too
                // because the field still matched.
                cur.dismissed_version = None;
            }
            "error" => {
                // Don't clear available_version on a transient
                // error — if a previous successful check found
                // v1.5.0, an offline tick shouldn't make the banner
                // vanish.
                cur.last_error = result.message.clone();
            }
            _ => {
                cur.last_error = result.message.clone();
            }
        }
        // Refresh last_check_at on every non-error response so a
        // string of "up-to-date" cycles doesn't make the cooldown
        // think we never checked.
        if result.status != "error" {
            cur.last_check_at = Some(current_unix_seconds());
        }
        cur.clone()
    };

    // Persist last_check_at to disk so a relaunch respects the
    // cooldown. Manual checks update the timestamp too — we don't
    // want a manual click followed by an immediate background tick
    // to fire a redundant HTTPS request.
    if result.status != "error" {
        if let Err(e) = settings::set_last_update_check_at(app, current_unix_seconds()) {
            log::warn!("updater: persist last_check_at failed: {e}");
        }
        // On up-to-date, also clear any stored dismissal so the
        // future release can re-arm. Disk + state are written in
        // tandem; the state mutation above already cleared the
        // in-memory copy.
        if result.status == "up-to-date" {
            if let Err(e) = settings::set_dismissed_update_version(app, None) {
                log::warn!("updater: clear dismissed_version failed: {e}");
            }
        }
    }

    emit_state(app);

    if manual {
        log::info!(
            "updater: manual check -> status={} available={:?}",
            result.status,
            result.available_version
        );
    } else {
        log::info!(
            "updater: scheduled check -> status={} available={:?}",
            result.status,
            result.available_version
        );
    }

    snapshot
}

fn current_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Persist the "Later" dismissal so the tray banner + dot stay
/// hidden until the next release. Returns the resulting state so
/// the caller can echo it back to the renderer.
pub fn dismiss_current_version<R: Runtime>(app: &AppHandle<R>) -> UpdateState {
    let snapshot = if let Some(state) = app.try_state::<Arc<UpdaterState>>() {
        let mut cur = state.current.lock();
        cur.dismissed_version = cur.available_version.clone();
        // Mirror to disk so the dismissal survives a relaunch.
        if let Some(ver) = cur.dismissed_version.clone() {
            if let Err(e) = settings::set_dismissed_update_version(app, Some(&ver)) {
                log::warn!("updater: persist dismissed_version failed: {e}");
            }
        }
        cur.clone()
    } else {
        return UpdateState::idle(app.package_info().version.to_string());
    };
    emit_state(app);
    snapshot
}

/// Seed the in-process state from disk on launch. Called from
/// `lib.rs::run::setup` so a relaunch picks up the last-known
/// `dismissed_version` and `last_check_at` without waiting for the
/// first scheduler tick.
pub fn hydrate_from_disk<R: Runtime>(app: &AppHandle<R>) {
    let Some(state) = app.try_state::<Arc<UpdaterState>>() else {
        return;
    };
    let s = settings::load(app);
    let mut cur = state.current.lock();
    cur.current_version = app.package_info().version.to_string();
    cur.last_check_at = s.last_update_check_at.filter(|v| *v > 0);
    cur.dismissed_version = s.dismissed_update_version.clone();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_check_always_allowed() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        assert!(should_check_now(None, now, COOLDOWN));
    }

    #[test]
    fn cooldown_blocks_recent_check() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let last = now - Duration::from_secs(60 * 30); // 30 min ago
        assert!(!should_check_now(Some(last), now, COOLDOWN));
    }

    #[test]
    fn cadence_gates_under_24h() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        // 7 h ago: past cooldown (6 h) but under cadence (24 h).
        let last = now - Duration::from_secs(7 * 3600);
        assert!(!should_check_now(Some(last), now, COOLDOWN));
    }

    #[test]
    fn cadence_elapses_at_24h() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        // 25 h ago: both gates clear.
        let last = now - Duration::from_secs(25 * 3600);
        assert!(should_check_now(Some(last), now, COOLDOWN));
    }

    #[test]
    fn banner_hides_for_dismissed_version() {
        let mut st = UpdateState::idle("1.4.0".into());
        st.status = "available".into();
        st.available_version = Some("1.5.0".into());
        assert!(st.banner_visible());
        st.dismissed_version = Some("1.5.0".into());
        assert!(!st.banner_visible());
        // New release re-arms.
        st.available_version = Some("1.6.0".into());
        assert!(st.banner_visible());
    }

    #[test]
    fn banner_hides_without_available_version() {
        let mut st = UpdateState::idle("1.4.0".into());
        st.status = "up-to-date".into();
        assert!(!st.banner_visible());
    }
}
