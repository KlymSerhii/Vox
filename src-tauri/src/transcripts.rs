//! In-memory ring buffer of recent transcripts; cleared on quit.
//!
//! Two record shapes coexist:
//! - `RecentTranscript` — the legacy `{ ts, text }` shape consumed by
//!   the old `get_recent_transcripts` IPC + the existing tray popover.
//! - `CompletedEntry` — the parallel-sessions shape with mode + stage
//!   + warning info. The popover's IN PROGRESS section needs the
//!   richer record; legacy callers project the simple one out via
//!   [`snapshot_legacy`].
//!
//! `InProgressEntry` and `SessionsSnapshot` are re-exported so other
//! modules don't import from session.rs (which depends on this one).

use std::collections::VecDeque;

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::session::Stage;

const CAPACITY: usize = 5;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentTranscript {
    /// Unix epoch seconds.
    pub ts: i64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct InProgressEntry {
    pub session_id: u64,
    pub stage: Stage,
    pub mode_id: String,
    pub mode_name: String,
    pub mode_icon: String,
    pub started_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompletedEntry {
    pub session_id: u64,
    pub ts: i64,
    pub text: String,
    pub mode_id: String,
    pub stage: Stage,
    pub warning: bool,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionsSnapshot {
    pub in_progress: Vec<InProgressEntry>,
    pub recent: Vec<CompletedEntry>,
}

static RING: Lazy<Mutex<VecDeque<CompletedEntry>>> =
    Lazy::new(|| Mutex::new(VecDeque::with_capacity(CAPACITY)));

/// Legacy push used by the pre-parallel pipeline. Synthesises a
/// CompletedEntry with default mode info. Kept so any consumer that
/// hasn't migrated keeps working.
#[allow(dead_code)]
pub fn push(text: &str) {
    let entry = CompletedEntry {
        session_id: 0,
        ts: chrono::Utc::now().timestamp(),
        text: text.to_string(),
        mode_id: String::new(),
        stage: Stage::Done,
        warning: false,
        error_message: None,
    };
    push_completed(entry);
}

pub fn push_completed(entry: CompletedEntry) {
    let mut ring = RING.lock();
    if ring.len() >= CAPACITY {
        ring.pop_back();
    }
    ring.push_front(entry);
}

/// Legacy snapshot — projects CompletedEntry to the older RecentTranscript
/// shape so existing IPC callers don't break.
pub fn snapshot() -> Vec<RecentTranscript> {
    let ring = RING.lock();
    ring.iter()
        .map(|e| RecentTranscript {
            ts: e.ts,
            text: e.text.clone(),
        })
        .collect()
}

pub fn snapshot_completed() -> Vec<CompletedEntry> {
    let ring = RING.lock();
    ring.iter().cloned().collect()
}
