//! Time-saved tracking — backed by `tauri-plugin-store`.
//!
//! Records one entry per successful dictation session and exposes
//! aggregated rollups for the Stats UI. The "saved" figure is a model:
//! how long it would have taken the user to type the same transcript at
//! [`WPM`] words per minute, minus how long Vox actually took (capture
//! + STT + optional polish). Negative values clamp to zero so a long
//! polish on a short utterance doesn't generate negative-saved noise.

use chrono::{DateTime, Datelike, Days, Local, TimeZone};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Runtime};
use tauri_plugin_store::StoreExt;

const STORE_FILE: &str = "vox-stats.json";
const RECORDS_KEY: &str = "records";

/// Words-per-minute baseline used to model how long the user *would*
/// have spent typing this transcript. 50 wpm is the commonly cited
/// average sustained typing speed for non-touch-typists; tweak in one
/// place if we ever expose a "your typing speed" knob in Settings.
const WPM: f64 = 50.0;

/// Drop records older than this when writing. ~13 months — enough for
/// year-over-year comparisons in the UI without unbounded growth.
const RETENTION_DAYS: i64 = 400;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    pub ts: i64,
    pub saved_secs: f64,
    pub spoken_secs: f64,
    pub typed_secs: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    LastSession,
    Day,
    Week,
    Month,
    Year,
}

impl Period {
    pub fn parse(s: &str) -> Option<Period> {
        match s {
            "last_session" => Some(Period::LastSession),
            "day" => Some(Period::Day),
            "week" => Some(Period::Week),
            "month" => Some(Period::Month),
            "year" => Some(Period::Year),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TimeSavedStats {
    pub saved_seconds: f64,
    pub sessions: u32,
}

pub fn typed_seconds_for(transcript: &str) -> f64 {
    let words = transcript.split_whitespace().count();
    if words == 0 {
        return 0.0;
    }
    (words as f64 / WPM) * 60.0
}

pub fn saved_seconds_for(actual: f64, typed: f64) -> f64 {
    (typed - actual).max(0.0)
}

pub(crate) fn evict_old(records: &mut Vec<Record>, now_ts: i64) {
    let cutoff = now_ts - RETENTION_DAYS * 86_400;
    records.retain(|r| r.ts >= cutoff);
}

fn window_start(period: Period, now_local: DateTime<Local>) -> i64 {
    let today_midnight = Local
        .with_ymd_and_hms(
            now_local.year(),
            now_local.month(),
            now_local.day(),
            0,
            0,
            0,
        )
        .single()
        .unwrap_or(now_local);

    let days_back: u64 = match period {
        Period::Day => 0,
        Period::Week => 6,
        Period::Month => 29,
        Period::Year => 364,
        // LastSession is handled before window_start is reached.
        Period::LastSession => 0,
    };

    if days_back == 0 {
        today_midnight.timestamp()
    } else {
        today_midnight
            .checked_sub_days(Days::new(days_back))
            .unwrap_or(today_midnight)
            .timestamp()
    }
}

pub(crate) fn aggregate_in_memory(
    records: &[Record],
    period: Period,
    now_local: DateTime<Local>,
) -> TimeSavedStats {
    if period == Period::LastSession {
        return match records.iter().max_by_key(|r| r.ts) {
            Some(r) => TimeSavedStats { saved_seconds: r.saved_secs, sessions: 1 },
            None => TimeSavedStats::default(),
        };
    }
    let cutoff = window_start(period, now_local);
    let mut saved = 0.0_f64;
    let mut count: u32 = 0;
    for r in records {
        if r.ts >= cutoff {
            saved += r.saved_secs;
            count += 1;
        }
    }
    TimeSavedStats { saved_seconds: saved, sessions: count }
}

/// Cumulative `saved_secs` across every retained record — i.e. the
/// running "time saved since install", modulo the [`RETENTION_DAYS`]
/// eviction window. Unlike [`aggregate_in_memory`] this isn't bounded
/// to a calendar window; it's the figure [`crate::donation`] tests
/// against its milestone tiers.
pub(crate) fn total_saved_in_memory(records: &[Record]) -> f64 {
    records.iter().map(|r| r.saved_secs).sum()
}

fn load_records<R: Runtime>(app: &AppHandle<R>) -> Vec<Record> {
    let store = match app.store(STORE_FILE) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("stats: failed to open store: {e}");
            return Vec::new();
        }
    };
    let raw = match store.get(RECORDS_KEY) {
        Some(v) => v,
        None => return Vec::new(),
    };
    match serde_json::from_value::<Vec<Record>>(raw) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("stats: malformed records, resetting in memory: {e}");
            Vec::new()
        }
    }
}

fn save_records<R: Runtime>(app: &AppHandle<R>, records: &[Record]) -> anyhow::Result<()> {
    let store = app
        .store(STORE_FILE)
        .map_err(|e| anyhow::anyhow!("open stats store: {e}"))?;
    let value = serde_json::to_value(records)
        .map_err(|e| anyhow::anyhow!("serialize stats records: {e}"))?;
    store.set(RECORDS_KEY, value);
    store
        .save()
        .map_err(|e| anyhow::anyhow!("save stats store: {e}"))?;
    Ok(())
}

pub fn record_session<R: Runtime>(
    app: &AppHandle<R>,
    raw_transcript: &str,
    actual_seconds: f64,
    now_ts: i64,
) {
    let typed = typed_seconds_for(raw_transcript);
    let saved = saved_seconds_for(actual_seconds, typed);
    let record = Record {
        ts: now_ts,
        saved_secs: saved,
        spoken_secs: actual_seconds,
        typed_secs: typed,
    };

    let mut records = load_records(app);
    records.push(record);
    evict_old(&mut records, now_ts);

    if let Err(e) = save_records(app, &records) {
        log::warn!("stats: save failed: {e}");
        return;
    }

    log::debug!(
        "stats: recorded session saved={:.1}s actual={:.1}s typed={:.1}s",
        saved,
        actual_seconds,
        typed
    );

    if let Err(e) = app.emit("stats:changed", ()) {
        log::warn!("stats: emit stats:changed failed: {e}");
    }
}

pub fn aggregate<R: Runtime>(app: &AppHandle<R>, period: Period) -> TimeSavedStats {
    let records = load_records(app);
    aggregate_in_memory(&records, period, Local::now())
}

/// Cumulative time saved across all retained records (see
/// [`total_saved_in_memory`]). Read by [`crate::donation`] to decide
/// which milestone nudge, if any, to surface.
pub fn total_saved_secs<R: Runtime>(app: &AppHandle<R>) -> f64 {
    total_saved_in_memory(&load_records(app))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn typed_seconds_for_basic() {
        let fifty = (1..=50)
            .map(|n| format!("w{n}"))
            .collect::<Vec<_>>()
            .join(" ");
        assert!((typed_seconds_for(&fifty) - 60.0).abs() < 1e-9);

        let hundred = (1..=100)
            .map(|n| format!("w{n}"))
            .collect::<Vec<_>>()
            .join(" ");
        assert!((typed_seconds_for(&hundred) - 120.0).abs() < 1e-9);

        assert_eq!(typed_seconds_for(""), 0.0);
        assert_eq!(typed_seconds_for("   "), 0.0);

        let three = typed_seconds_for("one two three");
        assert!((three - 3.6).abs() < 1e-9, "got {three}");
    }

    #[test]
    fn saved_seconds_for_clamps_at_zero() {
        assert_eq!(saved_seconds_for(30.0, 10.0), 0.0);
        assert!((saved_seconds_for(30.0, 120.0) - 90.0).abs() < 1e-9);
        assert_eq!(saved_seconds_for(0.0, 0.0), 0.0);
    }

    fn rec(ts: i64, saved: f64) -> Record {
        Record { ts, saved_secs: saved, spoken_secs: 0.0, typed_secs: 0.0 }
    }

    #[test]
    fn aggregation_windows_correctly() {
        let now_local = Local
            .with_ymd_and_hms(2026, 5, 5, 12, 0, 0)
            .single()
            .expect("valid local time");
        let now_ts = now_local.timestamp();

        let one_hour_ago = now_ts - 3600;
        let six_days_ago = now_ts - 6 * 86_400;
        let twenty_nine_days_ago = now_ts - 29 * 86_400;
        let hundred_days_ago = now_ts - 100 * 86_400;
        let four_hundred_days_ago = now_ts - 400 * 86_400;
        // Exactly today's midnight — guards against a `>=` → `>` regression
        // in the aggregation predicate.
        let today_midnight_ts = Local
            .with_ymd_and_hms(2026, 5, 5, 0, 0, 0)
            .single()
            .expect("valid local time")
            .timestamp();

        let records = vec![
            rec(one_hour_ago, 10.0),
            rec(six_days_ago, 20.0),
            rec(twenty_nine_days_ago, 40.0),
            rec(hundred_days_ago, 80.0),
            rec(four_hundred_days_ago, 160.0),
            rec(today_midnight_ts, 5.0),
        ];

        // Day window — 1h-ago record + the on-the-edge midnight record.
        let day = aggregate_in_memory(&records, Period::Day, now_local);
        assert_eq!(day.sessions, 2);
        assert!((day.saved_seconds - 15.0).abs() < 1e-9);

        // Week — 1h-ago + 6 days ago (window starts 6 calendar days before
        // today's midnight; six_days_ago = now - 6*86400 falls inside).
        let week = aggregate_in_memory(&records, Period::Week, now_local);
        assert_eq!(week.sessions, 3);
        assert!((week.saved_seconds - 35.0).abs() < 1e-9);

        // Month — adds the 29-days-ago record.
        let month = aggregate_in_memory(&records, Period::Month, now_local);
        assert_eq!(month.sessions, 4);
        assert!((month.saved_seconds - 75.0).abs() < 1e-9);

        // Year — 1h, 6d, 29d, 100d, plus the midnight edge record.
        // The 400-days-ago record is outside.
        let year = aggregate_in_memory(&records, Period::Year, now_local);
        assert_eq!(year.sessions, 5);
        assert!((year.saved_seconds - 155.0).abs() < 1e-9);
    }

    #[test]
    fn eviction_drops_old_records() {
        let now_ts: i64 = 1_700_000_000;
        let cutoff_inclusive = now_ts - RETENTION_DAYS * 86_400;
        let mut records = vec![
            rec(now_ts - 86_400, 1.0),
            rec(now_ts - 100 * 86_400, 2.0),
            rec(cutoff_inclusive, 3.0),
            rec(cutoff_inclusive - 1, 4.0),
            rec(now_ts - 1000 * 86_400, 5.0),
        ];

        evict_old(&mut records, now_ts);
        assert_eq!(records.len(), 3);

        let saved: Vec<f64> = records.iter().map(|r| r.saved_secs).collect();
        assert!(saved.contains(&1.0));
        assert!(saved.contains(&2.0));
        assert!(saved.contains(&3.0));
        assert!(!saved.contains(&4.0));
        assert!(!saved.contains(&5.0));
    }

    #[test]
    fn total_saved_sums_every_record() {
        assert_eq!(total_saved_in_memory(&[]), 0.0);
        let records = vec![rec(1, 10.0), rec(2, 20.5), rec(3, 0.0), rec(4, 90.0)];
        assert!((total_saved_in_memory(&records) - 120.5).abs() < 1e-9);
    }

    #[test]
    fn period_parse_strict_lowercase() {
        assert_eq!(Period::parse("day"), Some(Period::Day));
        assert_eq!(Period::parse("week"), Some(Period::Week));
        assert_eq!(Period::parse("month"), Some(Period::Month));
        assert_eq!(Period::parse("year"), Some(Period::Year));
        assert_eq!(Period::parse("last_session"), Some(Period::LastSession));
        assert_eq!(Period::parse("Day"), None);
        assert_eq!(Period::parse("DAY"), None);
        assert_eq!(Period::parse("hour"), None);
        assert_eq!(Period::parse("last-session"), None);
        assert_eq!(Period::parse(""), None);
    }

    #[test]
    fn last_session_returns_most_recent_record() {
        let now_local = Local
            .with_ymd_and_hms(2026, 5, 5, 12, 0, 0)
            .single()
            .expect("valid local time");
        let now_ts = now_local.timestamp();

        // Empty records → zero stats.
        let empty = aggregate_in_memory(&[], Period::LastSession, now_local);
        assert_eq!(empty.sessions, 0);
        assert_eq!(empty.saved_seconds, 0.0);

        // Out-of-order timestamps — must pick the largest ts, not the
        // last vector slot.
        let records = vec![
            rec(now_ts - 60, 5.0),
            rec(now_ts - 10, 99.0),
            rec(now_ts - 30, 7.0),
        ];
        let last = aggregate_in_memory(&records, Period::LastSession, now_local);
        assert_eq!(last.sessions, 1);
        assert!((last.saved_seconds - 99.0).abs() < 1e-9);
    }
}
