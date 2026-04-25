//! In-memory ring buffer of the most recent cleaned transcripts.
//!
//! Nothing is persisted — cleared on app quit per the privacy model.

use std::collections::VecDeque;

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

const CAPACITY: usize = 5;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentTranscript {
    /// Unix epoch seconds.
    pub ts: i64,
    pub text: String,
}

static RING: Lazy<Mutex<VecDeque<RecentTranscript>>> =
    Lazy::new(|| Mutex::new(VecDeque::with_capacity(CAPACITY)));

pub fn push(text: &str) {
    let entry = RecentTranscript {
        ts: chrono::Utc::now().timestamp(),
        text: text.to_string(),
    };
    let mut ring = RING.lock();
    if ring.len() >= CAPACITY {
        ring.pop_back();
    }
    ring.push_front(entry);
}

pub fn snapshot() -> Vec<RecentTranscript> {
    let ring = RING.lock();
    ring.iter().cloned().collect()
}
