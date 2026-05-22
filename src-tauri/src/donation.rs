//! Donation nudge bookkeeping.
//!
//! Vox is free and stores nothing in the cloud, so we cannot tell
//! whether a user has donated. The nudge is therefore honour-system: it
//! surfaces only after the user has accumulated real, demonstrable value
//! (time saved), escalates across a few milestone tiers, and takes the
//! user at their word when they click "I already donated".
//!
//! State lives in the same `tauri-plugin-store` file as Settings under
//! its own keys — it's sticky-dismissal bookkeeping, the same genre as
//! `dismissed_update_version` — but is intentionally NOT part of the
//! `Settings` struct. The milestone decision is encapsulated in
//! [`prompt`] (and the pure [`decide`]) so the frontend never
//! re-implements the gating and can't drift from it.

use serde::Serialize;
use tauri::{AppHandle, Runtime};
use tauri_plugin_store::StoreExt;

use crate::settings::STORE_FILE;
use crate::stats;

/// Cumulative-time-saved thresholds (seconds) at which a nudge fires,
/// ascending: 1 hour, 10 hours, 50 hours. Each tier shows at most once;
/// once the largest is acted on the nudge goes silent forever and only
/// the always-present passive "Support Vox" row remains.
const MILESTONES_SECS: [u64; 3] = [3_600, 36_000, 180_000];

/// Minimum gap after a "Maybe later" before any further nudge can show,
/// even if the user crosses the next milestone sooner. Reciprocity
/// decays over time, but re-asking days after a deferral reads as
/// nagging — 30 days is the floor the next tier must also clear.
const SNOOZE_SECS: i64 = 30 * 86_400;

const KEY_OPT_OUT: &str = "donation_opt_out";
const KEY_LAST_MILESTONE: &str = "donation_last_milestone_secs";
const KEY_SNOOZE_UNTIL: &str = "donation_snooze_until";

/// Returned by [`prompt`] when a nudge should show. `None` from
/// [`prompt`] means show nothing.
#[derive(Debug, Clone, Serialize)]
pub struct DonationPrompt {
    /// The milestone tier (seconds) that triggered this nudge — one of
    /// [`MILESTONES_SECS`].
    pub milestone_secs: u64,
    /// Cumulative seconds saved so far — drives the value-quantified
    /// copy ("You've saved 1h of typing").
    pub saved_secs: f64,
}

/// Highest milestone the user has crossed but not yet acted on. Walks
/// the tiers high-to-low so a user who blew past several while away gets
/// the most impressive (highest) un-acted number rather than a stale
/// lower one.
fn next_milestone(total_saved_secs: f64, last_milestone_secs: u64) -> Option<u64> {
    MILESTONES_SECS
        .iter()
        .rev()
        .copied()
        .find(|&m| total_saved_secs >= m as f64 && m > last_milestone_secs)
}

/// Pure gating decision so the policy is unit-testable without a store.
/// Returns the milestone to show, or `None` to stay silent.
fn decide(
    opt_out: bool,
    last_milestone_secs: u64,
    snooze_until: Option<i64>,
    total_saved_secs: f64,
    now_secs: i64,
) -> Option<u64> {
    if opt_out {
        return None;
    }
    if let Some(until) = snooze_until {
        // A legitimate snooze is at most SNOOZE_SECS in the future. A
        // value beyond that is corrupt (or stems from a backwards clock
        // jump after the snooze was set) — ignore it so the nudge
        // self-heals instead of going silent forever, since past the
        // 50h tier there's no higher milestone left to ever clear it.
        let plausible = until <= now_secs.saturating_add(SNOOZE_SECS);
        if plausible && now_secs < until {
            return None;
        }
    }
    next_milestone(total_saved_secs, last_milestone_secs)
}

fn read_state<R: Runtime>(app: &AppHandle<R>) -> (bool, u64, Option<i64>) {
    let store = match app.store(STORE_FILE) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("donation: open store failed: {e}");
            return (false, 0, None);
        }
    };
    let opt_out = store
        .get(KEY_OPT_OUT)
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let last_milestone = store
        .get(KEY_LAST_MILESTONE)
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let snooze_until = store
        .get(KEY_SNOOZE_UNTIL)
        .and_then(|v| v.as_i64())
        .filter(|n| *n > 0);
    (opt_out, last_milestone, snooze_until)
}

/// Whether to surface the nudge right now, and for which milestone.
pub fn prompt<R: Runtime>(app: &AppHandle<R>) -> Option<DonationPrompt> {
    let (opt_out, last_milestone, snooze_until) = read_state(app);
    let total = stats::total_saved_secs(app);
    let now = chrono::Utc::now().timestamp();
    let milestone = decide(opt_out, last_milestone, snooze_until, total, now)?;
    // `saved_secs` is the live, retention-bounded total (it can dip below
    // a previously-acted tier once old records evict past RETENTION_DAYS);
    // it drives the copy and is intentionally separate from the gating
    // `milestone_secs`.
    Some(DonationPrompt {
        milestone_secs: milestone,
        saved_secs: total,
    })
}

/// "Donate" / "I already donated" — stop all future nudges. The passive
/// "Support Vox" row stays visible, so repeat giving is always one click
/// away; we just never interrupt with the milestone banner again.
pub fn opt_out<R: Runtime>(app: &AppHandle<R>) -> anyhow::Result<()> {
    let store = app
        .store(STORE_FILE)
        .map_err(|e| anyhow::anyhow!("open store: {e}"))?;
    store.set(KEY_OPT_OUT, serde_json::Value::Bool(true));
    store
        .save()
        .map_err(|e| anyhow::anyhow!("save store: {e}"))?;
    Ok(())
}

/// "Maybe later" — record the tier we just showed so it never re-fires,
/// and hold off any further nudge for at least [`SNOOZE_SECS`]. The next
/// (higher) tier can then fire once both its threshold is crossed and the
/// snooze has elapsed.
pub fn snooze<R: Runtime>(app: &AppHandle<R>, milestone_secs: u64) -> anyhow::Result<()> {
    let store = app
        .store(STORE_FILE)
        .map_err(|e| anyhow::anyhow!("open store: {e}"))?;
    store.set(KEY_LAST_MILESTONE, serde_json::Value::from(milestone_secs));
    let until = chrono::Utc::now().timestamp() + SNOOZE_SECS;
    store.set(KEY_SNOOZE_UNTIL, serde_json::Value::from(until));
    store
        .save()
        .map_err(|e| anyhow::anyhow!("save store: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAR_FUTURE: i64 = 9_999_999_999;

    #[test]
    fn no_nudge_below_first_milestone() {
        assert_eq!(decide(false, 0, None, 3_599.0, 0), None);
    }

    #[test]
    fn first_milestone_fires_at_one_hour() {
        assert_eq!(decide(false, 0, None, 3_600.0, 0), Some(3_600));
    }

    #[test]
    fn picks_highest_crossed_unacted_milestone() {
        // Past 10h but never acted → jump straight to the 10h ask
        // (the more impressive number), skipping the 1h tier.
        assert_eq!(decide(false, 0, None, 40_000.0, 0), Some(36_000));
        // Past 50h → top tier.
        assert_eq!(decide(false, 0, None, 200_000.0, 0), Some(180_000));
    }

    #[test]
    fn opt_out_silences_everything() {
        assert_eq!(decide(true, 0, None, 200_000.0, 0), None);
    }

    #[test]
    fn acted_tier_does_not_refire_until_next_crossed() {
        // Snoozed the 1h tier (last=3600); still below 10h → silent.
        assert_eq!(decide(false, 3_600, None, 30_000.0, FAR_FUTURE), None);
        // ...then the 10h tier is crossed → it fires.
        assert_eq!(decide(false, 3_600, None, 36_000.0, FAR_FUTURE), Some(36_000));
    }

    #[test]
    fn snooze_window_suppresses_then_releases() {
        // now < snooze_until → suppressed even though 10h is crossed.
        assert_eq!(decide(false, 3_600, Some(1_000), 36_000.0, 999), None);
        // now >= snooze_until → the 10h tier fires.
        assert_eq!(decide(false, 3_600, Some(1_000), 36_000.0, 1_000), Some(36_000));
    }

    #[test]
    fn top_tier_is_terminal() {
        // Acted on 50h → nothing higher exists; silent forever after.
        assert_eq!(decide(false, 180_000, None, 10_000_000.0, FAR_FUTURE), None);
    }

    #[test]
    fn implausible_future_snooze_self_heals() {
        // A snooze far beyond the 30-day floor (corrupt value or a
        // backwards clock jump) must NOT suppress forever — it's
        // ignored so the nudge recovers.
        let bogus = Some(SNOOZE_SECS + 10 * 86_400);
        assert_eq!(decide(false, 0, bogus, 3_600.0, 0), Some(3_600));
        // A snooze within the plausible window still suppresses.
        let legit = Some(SNOOZE_SECS - 1);
        assert_eq!(decide(false, 0, legit, 3_600.0, 0), None);
    }

    #[test]
    fn next_milestone_boundary_is_inclusive() {
        assert_eq!(next_milestone(3_600.0, 0), Some(3_600));
        assert_eq!(next_milestone(3_599.999, 0), None);
    }
}
