//! Notch List: a single-file JSON to-do store at
//! `<app_data_dir>/notch-list.json`, mirroring the `modes.rs` file-store
//! pattern (lazy in-memory cache, invalidate on every mutation). Holds an
//! ordered list of active items plus an archive of completed/removed ones.
//!
//! The voice-command path (see `session.rs::interpret_and_apply_notch`)
//! reuses the user's selected LLM engine with a command-interpretation
//! prompt that returns the [`NotchCommandResponse`] JSON parsed here, then
//! applies the resulting [`NotchOp`]s atomically via [`apply_ops`].
//!
//! Whole module is cross-platform (it's pure data + file I/O); the notch
//! *window* is macOS-only (`notch.rs`). On non-macOS the store still
//! exists and round-trips so settings stay OS-agnostic.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime};

/// Hard cap on the active list length so a runaway voice batch can't grow
/// the file unbounded. Matches the "~200" guidance in the plan.
const MAX_ITEMS: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    High,
    Med,
    Low,
}

impl Priority {
    /// Sort rank: High sorts first (smaller rank). Used to order the
    /// active list by (priority desc, order asc).
    fn rank(self) -> u8 {
        match self {
            Priority::High => 0,
            Priority::Med => 1,
            Priority::Low => 2,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotchItem {
    pub id: String,
    pub text: String,
    pub priority: Priority,
    pub order: u32,
    pub done: bool,
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotchListDoc {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub items: Vec<NotchItem>,
    #[serde(default)]
    pub archive: Vec<NotchItem>,
}

fn default_version() -> u32 {
    1
}

impl Default for NotchListDoc {
    fn default() -> Self {
        Self {
            version: 1,
            items: Vec::new(),
            archive: Vec::new(),
        }
    }
}

impl NotchListDoc {
    /// Active items sorted by (priority desc, order asc), capped at `n`.
    /// `done` items never appear here — `mark_done` archives them, but we
    /// filter defensively in case an external edit left a done item in
    /// `items`.
    pub fn top_n(&self, n: usize) -> Vec<NotchItem> {
        let mut active: Vec<NotchItem> =
            self.items.iter().filter(|i| !i.done).cloned().collect();
        active.sort_by(|a, b| {
            a.priority
                .rank()
                .cmp(&b.priority.rank())
                .then(a.order.cmp(&b.order))
        });
        active.truncate(n);
        active
    }

    /// Re-pack `order` densely following the current sorted display order
    /// so reorders and removals keep a clean sequence. Packs starting at
    /// `1` (not `0`) so a later "move to top" can pick `min - 1` without
    /// the band-min-is-0 case saturating into a tie with the current top
    /// item (which a stable sort would otherwise resolve the wrong way).
    fn normalize_order(&mut self) {
        let mut sorted: Vec<&mut NotchItem> = self.items.iter_mut().collect();
        sorted.sort_by(|a, b| {
            a.priority
                .rank()
                .cmp(&b.priority.rank())
                .then(a.order.cmp(&b.order))
        });
        for (idx, item) in sorted.into_iter().enumerate() {
            item.order = idx as u32 + 1;
        }
    }
}

/// True if `id` is safe as data we generate/accept. Mirrors
/// `modes::is_safe_mode_id` (`[a-z0-9-]{1,64}`, no edge dashes). The notch
/// store isn't path-keyed by id (it's one file), but we keep ids
/// constrained so a hand-edited file or a voice op can't smuggle absurd
/// ids and so they stay URL/event-payload-safe.
#[allow(dead_code)]
pub fn is_safe_item_id(id: &str) -> bool {
    if id.is_empty() || id.len() > 64 {
        return false;
    }
    if id.starts_with('-') || id.ends_with('-') {
        return false;
    }
    id.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Monotonic counter folded into generated ids so two adds in the same
/// second never collide (timestamps are 1 s granular).
static ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Dependency-free id generator: `item-<unix_millis>-<counter>`, lowercased
/// and slug-safe. Always passes [`is_safe_item_id`].
fn gen_id() -> String {
    let n = ID_COUNTER.fetch_add(1, Ordering::SeqCst);
    let ts = Utc::now().timestamp_millis().max(0) as u64;
    format!("item-{ts}-{n}")
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

/// Process-wide cache of the on-disk doc, rebuilt lazily and invalidated
/// on every mutation — same shape as `modes::ModeStore`.
#[derive(Default)]
pub struct NotchStore {
    inner: RwLock<Option<NotchListDoc>>,
    path: RwLock<Option<PathBuf>>,
    /// Single-level undo snapshot taken before the last applied voice
    /// batch (or any mutation that opts to snapshot). Powers
    /// `notch_list_undo`.
    last_undo: RwLock<Option<NotchListDoc>>,
}

impl NotchStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn init_with_path(&self, path: PathBuf) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("create notch-list dir")?;
        }
        *self.path.write() = Some(path.clone());
        if !path.exists() {
            write_doc_file(&path, &NotchListDoc::default())?;
        }
        *self.inner.write() = None;
        Ok(())
    }

    fn path_or_err(&self) -> Result<PathBuf> {
        self.path
            .read()
            .clone()
            .ok_or_else(|| anyhow!("NotchStore not initialised"))
    }

    /// Read the current doc (cached). Returns a default empty doc rather
    /// than erroring on a missing/corrupt file so the UI degrades to an
    /// empty list instead of failing.
    pub fn read(&self) -> NotchListDoc {
        if let Some(doc) = self.inner.read().clone() {
            return doc;
        }
        let doc = match self.path_or_err() {
            Ok(p) => read_doc_file(&p).unwrap_or_default(),
            Err(_) => NotchListDoc::default(),
        };
        *self.inner.write() = Some(doc.clone());
        doc
    }

    /// Persist `doc` (pretty JSON) and refresh the cache.
    fn write(&self, doc: &NotchListDoc) -> Result<()> {
        let path = self.path_or_err()?;
        write_doc_file(&path, doc)?;
        *self.inner.write() = Some(doc.clone());
        Ok(())
    }

    #[allow(dead_code)]
    pub fn top_n(&self, n: usize) -> Vec<NotchItem> {
        self.read().top_n(n)
    }

    /// Snapshot the current doc for undo, then store `next` and persist.
    fn commit_with_undo(&self, prev: NotchListDoc, next: NotchListDoc) -> Result<()> {
        self.write(&next)?;
        *self.last_undo.write() = Some(prev);
        Ok(())
    }

    // --- mutations -------------------------------------------------------

    /// Mark an active item done: set `done`/`completed_at` and move it to
    /// the archive so the active list reflows. No-op error if not found.
    pub fn mark_done(&self, id: &str) -> Result<()> {
        let prev = self.read();
        let mut doc = prev.clone();
        let idx = doc
            .items
            .iter()
            .position(|i| i.id == id)
            .ok_or_else(|| anyhow!("item not found"))?;
        let mut item = doc.items.remove(idx);
        item.done = true;
        item.completed_at = Some(Utc::now().timestamp());
        doc.archive.insert(0, item);
        doc.normalize_order();
        self.commit_with_undo(prev, doc)
    }

    /// Add a new item. `position` controls placement among same-priority
    /// items; `None` appends at the bottom of its priority band.
    pub fn add(
        &self,
        text: &str,
        priority: Priority,
        position: Option<Position>,
    ) -> Result<NotchItem> {
        let prev = self.read();
        let mut doc = prev.clone();
        let item = build_add(&mut doc, text, priority, position)?;
        let created = item.clone();
        doc.normalize_order();
        self.commit_with_undo(prev, doc)?;
        Ok(created)
    }

    /// Remove an active item → archive (not deleted, so it can be
    /// restored).
    pub fn remove(&self, id: &str) -> Result<()> {
        let prev = self.read();
        let mut doc = prev.clone();
        let idx = doc
            .items
            .iter()
            .position(|i| i.id == id)
            .ok_or_else(|| anyhow!("item not found"))?;
        let item = doc.items.remove(idx);
        doc.archive.insert(0, item);
        doc.normalize_order();
        self.commit_with_undo(prev, doc)
    }

    pub fn edit(&self, id: &str, text: &str) -> Result<()> {
        let text = text.trim();
        if text.is_empty() {
            return Err(anyhow!("text is empty"));
        }
        let prev = self.read();
        let mut doc = prev.clone();
        let item = doc
            .items
            .iter_mut()
            .find(|i| i.id == id)
            .ok_or_else(|| anyhow!("item not found"))?;
        item.text = text.to_string();
        self.commit_with_undo(prev, doc)
    }

    pub fn reprioritize(&self, id: &str, priority: Priority) -> Result<()> {
        let prev = self.read();
        let mut doc = prev.clone();
        {
            let item = doc
                .items
                .iter_mut()
                .find(|i| i.id == id)
                .ok_or_else(|| anyhow!("item not found"))?;
            item.priority = priority;
        }
        doc.normalize_order();
        self.commit_with_undo(prev, doc)
    }

    /// Reorder active items to match `ordered_ids` (front = top). Ids not
    /// present keep their relative order after the listed ones. Priority
    /// bands are preserved — the explicit order wins within the existing
    /// sort, so callers should pass the full display order.
    pub fn reorder(&self, ordered_ids: &[String]) -> Result<()> {
        let prev = self.read();
        let mut doc = prev.clone();
        apply_reorder(&mut doc, ordered_ids);
        self.commit_with_undo(prev, doc)
    }

    /// Restore an archived item back into the active list.
    pub fn restore(&self, id: &str) -> Result<()> {
        let prev = self.read();
        let mut doc = prev.clone();
        let idx = doc
            .archive
            .iter()
            .position(|i| i.id == id)
            .ok_or_else(|| anyhow!("archived item not found"))?;
        if doc.items.len() >= MAX_ITEMS {
            return Err(anyhow!("list is full"));
        }
        let mut item = doc.archive.remove(idx);
        item.done = false;
        item.completed_at = None;
        let next_order = doc.items.iter().map(|i| i.order).max().map(|m| m + 1).unwrap_or(0);
        item.order = next_order;
        doc.items.push(item);
        doc.normalize_order();
        self.commit_with_undo(prev, doc)
    }

    /// Drop every archived (completed/removed) item.
    pub fn clear_completed(&self) -> Result<()> {
        let prev = self.read();
        let mut doc = prev.clone();
        doc.archive.clear();
        self.commit_with_undo(prev, doc)
    }

    /// Apply a parsed voice-command batch atomically. Resolves targets
    /// against the current doc, applies all ops to a clone, validates,
    /// then commits. The returned [`NotchApplyResult`] carries a summary
    /// and a `staged_destructive` flag when `confirm_destructive` is on
    /// and the batch contained a Remove/clear (the caller surfaces a
    /// confirmation toast instead of mutating in that case).
    pub fn apply_voice_ops(
        &self,
        ops: Vec<NotchOp>,
        confirm_destructive: bool,
    ) -> Result<NotchApplyResult> {
        let prev = self.read();
        let result = apply_ops(&prev, &ops, confirm_destructive)?;
        if !result.staged_destructive {
            self.commit_with_undo(prev, result.doc.clone())?;
        }
        Ok(result)
    }

    /// Commit a previously-staged doc (e.g. a confirmed destructive voice
    /// batch) as the new list, snapshotting the current state for undo.
    /// Used by the confirm-destructive gesture in `session.rs`.
    pub fn commit_staged(&self, next: NotchListDoc) -> Result<()> {
        let prev = self.read();
        self.commit_with_undo(prev, next)
    }

    /// Whether an undo snapshot is currently available. Reserved for a
    /// future "disable the Undo button when nothing to undo" gate (the v1
    /// Settings button is always-enabled and reports "Nothing to undo");
    /// kept test-covered so the snapshot bookkeeping stays honest.
    #[allow(dead_code)]
    pub fn can_undo(&self) -> bool {
        self.last_undo.read().is_some()
    }

    /// Restore the pre-last-batch snapshot, if any. Returns false when
    /// there's nothing to undo.
    pub fn undo(&self) -> Result<bool> {
        let snapshot = self.last_undo.write().take();
        match snapshot {
            Some(doc) => {
                self.write(&doc)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }
}

/// Build + push a new item into `doc.items`, returning a clone. Shared by
/// the store `add` and the `apply_ops` Add arm so id generation /
/// validation / capping lives in one place. Does NOT normalize order —
/// the caller does that once after a batch.
fn build_add(
    doc: &mut NotchListDoc,
    text: &str,
    priority: Priority,
    position: Option<Position>,
) -> Result<NotchItem> {
    let text = text.trim();
    if text.is_empty() {
        return Err(anyhow!("text is empty"));
    }
    if doc.items.len() >= MAX_ITEMS {
        return Err(anyhow!("list is full"));
    }
    let now = Utc::now().timestamp();
    // Place at top or bottom of the same-priority band by picking an order
    // value just below the band min (Top) or just above the band max
    // (Bottom). normalize_order re-packs to dense indices afterwards.
    let band_orders: Vec<u32> = doc
        .items
        .iter()
        .filter(|i| i.priority == priority)
        .map(|i| i.order)
        .collect();
    let order = match position {
        Some(Position::Top) => band_orders.iter().min().copied().unwrap_or(0).saturating_sub(1),
        // Bottom / After(idx) / None all append within the band for v1;
        // After is treated as "after the targeted slot" which, post
        // normalize, lands below the band. Keeping it simple per the plan.
        _ => band_orders.iter().max().copied().map(|m| m + 1).unwrap_or(0),
    };
    let item = NotchItem {
        id: gen_id(),
        text: text.to_string(),
        priority,
        order,
        done: false,
        created_at: now,
        completed_at: None,
    };
    doc.items.push(item.clone());
    Ok(item)
}

fn apply_reorder(doc: &mut NotchListDoc, ordered_ids: &[String]) {
    // Assign ascending order to listed ids first (in the given order),
    // then everything else after, preserving its prior relative order.
    let mut next: u32 = 0;
    for id in ordered_ids {
        if let Some(item) = doc.items.iter_mut().find(|i| &i.id == id) {
            item.order = next;
            next += 1;
        }
    }
    let listed: std::collections::HashSet<&String> = ordered_ids.iter().collect();
    let mut rest: Vec<&mut NotchItem> = doc
        .items
        .iter_mut()
        .filter(|i| !listed.contains(&i.id))
        .collect();
    rest.sort_by_key(|i| i.order);
    for item in rest {
        item.order = next;
        next += 1;
    }
}

// ---------------------------------------------------------------------------
// Voice-command schema
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Deserialize)]
pub struct NotchCommandResponse {
    #[serde(default)]
    pub ops: Vec<NotchOp>,
    #[serde(default)]
    pub needs_clarification: Option<String>,
    #[serde(default)]
    pub confidence: f32,
}

/// What [`crate::session`]'s notch interpreter should do with a parsed
/// response. Pulled out as a pure decision so the natural-language gating
/// is unit-testable without a live model.
#[derive(Clone, Debug, PartialEq)]
pub enum NotchAction {
    /// The model couldn't act and asked a short disambiguating question;
    /// surface it to the user.
    Clarify(String),
    /// At least one op should be applied.
    Apply,
    /// Nothing actionable was heard.
    Nothing,
}

impl NotchCommandResponse {
    /// Decide how to act on this response.
    ///
    /// Natural-language editing means *adding is the default*, and a
    /// rambling, multi-sentence brain-dump should still produce tasks — so
    /// any non-empty `ops` list is applied regardless of `confidence`
    /// (every op is reversible: adds/edits/completes via undo, removes via
    /// the confirm gesture). Gating ops behind a confidence floor used to
    /// silently drop perfectly good brain-dumps; `confidence` is now
    /// advisory (logging) only. A clarifying question is surfaced only when
    /// the model produced no ops AND gave a real question — never as a
    /// catch-all for conversational speech.
    pub fn action(&self) -> NotchAction {
        if !self.ops.is_empty() {
            return NotchAction::Apply;
        }
        if let Some(q) = self.needs_clarification.as_deref() {
            let q = q.trim();
            if !q.is_empty() {
                return NotchAction::Clarify(q.to_string());
            }
        }
        NotchAction::Nothing
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum NotchOp {
    Add {
        text: String,
        #[serde(default)]
        priority: Option<Priority>,
        #[serde(default)]
        position: Option<Position>,
    },
    Complete {
        target: Target,
    },
    Remove {
        target: Target,
    },
    Reorder {
        target: Target,
        position: Position,
    },
    Edit {
        target: Target,
        text: String,
    },
    Reprioritize {
        target: Target,
        priority: Priority,
    },
}

/// A reference to an existing item. Resolution order: explicit id →
/// 1-based index into the sorted active list → fuzzy text match.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Target {
    Index(u32),
    Id(String),
    Text(String),
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Position {
    Top,
    Bottom,
    /// After a 1-based index in the sorted active list. Kept simple per
    /// the plan (index only, not an arbitrary target).
    After(u32),
}

/// Outcome of [`apply_ops`].
#[derive(Clone, Debug)]
pub struct NotchApplyResult {
    /// The new doc after applying every op (already validated). This is
    /// the fully-applied result even when `staged_destructive` is true —
    /// the caller stages it and commits on confirmation.
    pub doc: NotchListDoc,
    pub added: u32,
    pub completed: u32,
    pub removed: u32,
    pub edited: u32,
    pub reprioritized: u32,
    pub reordered: u32,
    /// True when `confirm_destructive` was on and the batch contained a
    /// Remove. The caller should NOT auto-commit `doc` — instead stage it
    /// and surface a confirmation gesture (the store leaves the on-disk
    /// list untouched until the caller commits).
    pub staged_destructive: bool,
}

impl NotchApplyResult {
    /// Terse one-line summary for a pill toast, e.g. "Added 1, completed 2".
    pub fn summary(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.added > 0 {
            parts.push(format!("added {}", self.added));
        }
        if self.completed > 0 {
            parts.push(format!("completed {}", self.completed));
        }
        if self.removed > 0 {
            parts.push(format!("removed {}", self.removed));
        }
        if self.edited > 0 {
            parts.push(format!("edited {}", self.edited));
        }
        if self.reprioritized > 0 {
            parts.push(format!("reprioritized {}", self.reprioritized));
        }
        if self.reordered > 0 {
            parts.push(format!("reordered {}", self.reordered));
        }
        if parts.is_empty() {
            return "No changes".to_string();
        }
        // Capitalize first letter.
        let joined = parts.join(", ");
        let mut chars = joined.chars();
        match chars.next() {
            Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
            None => joined,
        }
    }
}

/// Resolve a [`Target`] to an item id against `doc`. Order: id → 1-based
/// index into the sorted active list → fuzzy (case-insensitive substring,
/// else nearest by token overlap) text match. Returns `None` when nothing
/// matches.
///
/// Note on the `Id`/`Text` arms: `Target` is `#[serde(untagged)]`, so any
/// JSON string deserializes into `Id` (the first string variant) — the
/// model can't actually produce a `Text` target over the wire. We bridge
/// that by falling back to a fuzzy text match when an `Id` string doesn't
/// match a real item id, which is what lets the LLM reference items by
/// their text (e.g. "the dentist task"). The standalone `Text` variant
/// stays for programmatic callers / tests.
fn resolve_target(doc: &NotchListDoc, target: &Target) -> Option<String> {
    // Use the FULL sorted active list (not truncated to top_n) so voice
    // references like "the 6th one" still resolve.
    let sorted = doc.top_n(usize::MAX);
    match target {
        Target::Id(id) => sorted
            .iter()
            .find(|i| &i.id == id)
            .map(|i| i.id.clone())
            .or_else(|| fuzzy_find(&sorted, id)),
        Target::Index(idx) => {
            if *idx == 0 {
                return None;
            }
            sorted.get((*idx - 1) as usize).map(|i| i.id.clone())
        }
        Target::Text(needle) => fuzzy_find(&sorted, needle),
    }
}

/// Case-insensitive substring match first; on a miss, the item with the
/// most shared lowercase tokens (must share at least one).
fn fuzzy_find(items: &[NotchItem], needle: &str) -> Option<String> {
    let needle_lc = needle.trim().to_lowercase();
    if needle_lc.is_empty() {
        return None;
    }
    if let Some(item) = items
        .iter()
        .find(|i| i.text.to_lowercase().contains(&needle_lc))
    {
        return Some(item.id.clone());
    }
    let needle_tokens: std::collections::HashSet<&str> =
        needle_lc.split_whitespace().collect();
    let mut best: Option<(usize, &NotchItem)> = None;
    for item in items {
        let text_lc = item.text.to_lowercase();
        let overlap = text_lc
            .split_whitespace()
            .filter(|t| needle_tokens.contains(t))
            .count();
        if overlap > 0 && best.map(|(b, _)| overlap > b).unwrap_or(true) {
            best = Some((overlap, item));
        }
    }
    best.map(|(_, i)| i.id.clone())
}

/// Apply `ops` to a CLONE of `doc`. Every Target is resolved up front
/// against the *original* doc snapshot so indices in the batch refer to
/// the list the speaker saw, not a mid-batch reshuffled one. An
/// unresolvable target is a hard error (the caller fails open with a
/// toast). Returns the fully-applied new doc + a summary. When
/// `confirm_destructive` is on and the batch contains a Remove,
/// `staged_destructive` is set so the caller stages `doc` rather than
/// committing it (nothing is persisted by this function either way).
pub fn apply_ops(
    doc: &NotchListDoc,
    ops: &[NotchOp],
    confirm_destructive: bool,
) -> Result<NotchApplyResult> {
    let has_destructive = ops.iter().any(|o| matches!(o, NotchOp::Remove { .. }));

    // Pre-resolve every target against the original snapshot.
    let mut resolved: Vec<Option<String>> = Vec::with_capacity(ops.len());
    for op in ops {
        let target = match op {
            NotchOp::Complete { target }
            | NotchOp::Remove { target }
            | NotchOp::Reorder { target, .. }
            | NotchOp::Edit { target, .. }
            | NotchOp::Reprioritize { target, .. } => Some(target),
            NotchOp::Add { .. } => None,
        };
        match target {
            Some(t) => {
                let id = resolve_target(doc, t)
                    .ok_or_else(|| anyhow!("could not find the item referenced by the command"))?;
                resolved.push(Some(id));
            }
            None => resolved.push(None),
        }
    }

    let mut next = doc.clone();
    let mut r = NotchApplyResult {
        doc: NotchListDoc::default(),
        added: 0,
        completed: 0,
        removed: 0,
        edited: 0,
        reprioritized: 0,
        reordered: 0,
        staged_destructive: false,
    };

    for (op, id) in ops.iter().zip(resolved.iter()) {
        match op {
            NotchOp::Add {
                text,
                priority,
                position,
            } => {
                build_add(
                    &mut next,
                    text,
                    priority.unwrap_or(Priority::Med),
                    *position,
                )?;
                r.added += 1;
            }
            NotchOp::Complete { .. } => {
                let id = id.as_ref().expect("complete resolved");
                if let Some(idx) = next.items.iter().position(|i| &i.id == id) {
                    let mut item = next.items.remove(idx);
                    item.done = true;
                    item.completed_at = Some(Utc::now().timestamp());
                    next.archive.insert(0, item);
                    r.completed += 1;
                }
            }
            NotchOp::Remove { .. } => {
                let id = id.as_ref().expect("remove resolved");
                if let Some(idx) = next.items.iter().position(|i| &i.id == id) {
                    let item = next.items.remove(idx);
                    next.archive.insert(0, item);
                    r.removed += 1;
                }
            }
            NotchOp::Edit { text, .. } => {
                let id = id.as_ref().expect("edit resolved");
                let text = text.trim();
                if text.is_empty() {
                    return Err(anyhow!("edit text is empty"));
                }
                if let Some(item) = next.items.iter_mut().find(|i| &i.id == id) {
                    item.text = text.to_string();
                    r.edited += 1;
                }
            }
            NotchOp::Reprioritize { priority, .. } => {
                let id = id.as_ref().expect("reprioritize resolved");
                if let Some(item) = next.items.iter_mut().find(|i| &i.id == id) {
                    item.priority = *priority;
                    r.reprioritized += 1;
                }
            }
            NotchOp::Reorder { position, .. } => {
                let id = id.as_ref().expect("reorder resolved");
                apply_reorder_one(&mut next, id, *position);
                r.reordered += 1;
            }
        }
    }

    next.normalize_order();
    if next.items.len() > MAX_ITEMS {
        return Err(anyhow!("list would exceed the maximum size"));
    }
    r.doc = next;
    // `r.doc` is always the FULLY-APPLIED new doc. When the batch was
    // destructive and confirmation is on, we flag it so the caller stages
    // `r.doc` for a confirm gesture instead of committing it now.
    r.staged_destructive = confirm_destructive && has_destructive;
    Ok(r)
}

/// Move a single item to Top/Bottom of its priority band, or after a
/// 1-based index. Implemented by nudging its `order` then letting
/// `normalize_order` re-pack.
fn apply_reorder_one(doc: &mut NotchListDoc, id: &str, position: Position) {
    let priority = match doc.items.iter().find(|i| i.id == id) {
        Some(i) => i.priority,
        None => return,
    };
    let band_orders: Vec<u32> = doc
        .items
        .iter()
        .filter(|i| i.priority == priority && i.id != id)
        .map(|i| i.order)
        .collect();
    let new_order = match position {
        Position::Top => band_orders.iter().min().copied().unwrap_or(0).saturating_sub(1),
        Position::Bottom => band_orders.iter().max().copied().map(|m| m + 1).unwrap_or(0),
        Position::After(idx) => {
            // Insert just after the order of the item currently at 1-based
            // `idx` in the sorted active list; fall back to bottom.
            let sorted = doc.top_n(usize::MAX);
            sorted
                .get((idx.saturating_sub(1)) as usize)
                .map(|i| i.order.saturating_add(1))
                .unwrap_or_else(|| band_orders.iter().max().copied().map(|m| m + 1).unwrap_or(0))
        }
    };
    if let Some(item) = doc.items.iter_mut().find(|i| i.id == id) {
        // Multiply existing orders by 2 conceptually isn't needed because
        // normalize re-packs; the relative position from the float-ish
        // nudge is enough since Top uses min-1 and Bottom uses max+1.
        item.order = new_order;
    }
}

/// Strip ```json fences and grab the substring from the first `{` to the
/// last `}`, then `serde_json::from_str`. Returns `None` on any failure
/// (caller fails open). Mirrors the fence-strip + brace-slice the plan
/// describes.
pub fn parse_command_response(raw: &str) -> Option<NotchCommandResponse> {
    let trimmed = raw.trim();
    // Strip a leading ```json / ``` fence and a trailing ``` fence if
    // present, then fall through to the brace-slice which is robust to
    // any leftover prose.
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    if end < start {
        return None;
    }
    let slice = &trimmed[start..=end];
    serde_json::from_str::<NotchCommandResponse>(slice).ok()
}

// ---------------------------------------------------------------------------
// File I/O
// ---------------------------------------------------------------------------

pub fn notch_list_path<R: Runtime>(app: &AppHandle<R>) -> PathBuf {
    if let Ok(p) = app.path().app_data_dir() {
        return p.join("notch-list.json");
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("com.vox.app/notch-list.json")
}

fn write_doc_file(path: &Path, doc: &NotchListDoc) -> Result<()> {
    let json = serde_json::to_string_pretty(doc)?;
    fs::write(path, json).with_context(|| format!("write notch-list file {:?}", path))?;
    Ok(())
}

fn read_doc_file(path: &Path) -> Result<NotchListDoc> {
    let json = fs::read_to_string(path)
        .with_context(|| format!("read notch-list file {:?}", path))?;
    let doc: NotchListDoc =
        serde_json::from_str(&json).with_context(|| format!("parse notch-list file {:?}", path))?;
    Ok(doc)
}

// ---------------------------------------------------------------------------
// AppHandle convenience wrappers
// ---------------------------------------------------------------------------

pub fn store<R: Runtime>(app: &AppHandle<R>) -> std::sync::Arc<NotchStore> {
    app.state::<std::sync::Arc<NotchStore>>().inner().clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct ScratchFile {
        path: PathBuf,
    }
    impl ScratchFile {
        fn new() -> Self {
            static N: AtomicU64 = AtomicU64::new(0);
            let n = N.fetch_add(1, Ordering::SeqCst);
            let pid = std::process::id();
            let dir = std::env::temp_dir().join(format!("vox-notch-test-{pid}-{n}"));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self {
                path: dir.join("notch-list.json"),
            }
        }
    }
    impl Drop for ScratchFile {
        fn drop(&mut self) {
            if let Some(parent) = self.path.parent() {
                let _ = fs::remove_dir_all(parent);
            }
        }
    }

    fn fresh_store() -> (NotchStore, ScratchFile) {
        let f = ScratchFile::new();
        let store = NotchStore::new();
        store.init_with_path(f.path.clone()).unwrap();
        (store, f)
    }

    #[test]
    fn add_and_top_n_orders_by_priority_then_order() {
        let (store, _f) = fresh_store();
        store.add("low task", Priority::Low, None).unwrap();
        store.add("high task", Priority::High, None).unwrap();
        store.add("med task", Priority::Med, None).unwrap();
        let top = store.top_n(5);
        assert_eq!(top.len(), 3);
        assert_eq!(top[0].text, "high task");
        assert_eq!(top[1].text, "med task");
        assert_eq!(top[2].text, "low task");
        // Orders are densely packed (1-based) following display order.
        let mut orders: Vec<u32> = top.iter().map(|i| i.order).collect();
        orders.sort();
        assert_eq!(orders, vec![1, 2, 3]);
    }

    #[test]
    fn add_rejects_empty_text() {
        let (store, _f) = fresh_store();
        assert!(store.add("   ", Priority::Med, None).is_err());
    }

    #[test]
    fn mark_done_archives_and_reflows() {
        let (store, _f) = fresh_store();
        let a = store.add("first", Priority::High, None).unwrap();
        store.add("second", Priority::High, None).unwrap();
        store.mark_done(&a.id).unwrap();
        let doc = store.read();
        assert_eq!(doc.items.len(), 1);
        assert_eq!(doc.items[0].text, "second");
        assert_eq!(doc.archive.len(), 1);
        assert!(doc.archive[0].done);
        assert!(doc.archive[0].completed_at.is_some());
        // Reflowed: the surviving item is order 1 (1-based dense packing).
        assert_eq!(doc.items[0].order, 1);
    }

    #[test]
    fn remove_then_restore_roundtrips() {
        let (store, _f) = fresh_store();
        let a = store.add("task", Priority::Med, None).unwrap();
        store.remove(&a.id).unwrap();
        assert_eq!(store.read().items.len(), 0);
        assert_eq!(store.read().archive.len(), 1);
        store.restore(&a.id).unwrap();
        let doc = store.read();
        assert_eq!(doc.items.len(), 1);
        assert_eq!(doc.archive.len(), 0);
        assert!(!doc.items[0].done);
        assert!(doc.items[0].completed_at.is_none());
    }

    #[test]
    fn reorder_respects_explicit_order() {
        let (store, _f) = fresh_store();
        let a = store.add("a", Priority::Med, None).unwrap();
        let b = store.add("b", Priority::Med, None).unwrap();
        let c = store.add("c", Priority::Med, None).unwrap();
        store
            .reorder(&[c.id.clone(), a.id.clone(), b.id.clone()])
            .unwrap();
        let top = store.top_n(5);
        assert_eq!(top.iter().map(|i| i.text.as_str()).collect::<Vec<_>>(), vec!["c", "a", "b"]);
    }

    #[test]
    fn reprioritize_moves_item_up() {
        let (store, _f) = fresh_store();
        store.add("a", Priority::Low, None).unwrap();
        let b = store.add("b", Priority::Low, None).unwrap();
        store.reprioritize(&b.id, Priority::High).unwrap();
        let top = store.top_n(5);
        assert_eq!(top[0].text, "b");
    }

    #[test]
    fn clear_completed_empties_archive() {
        let (store, _f) = fresh_store();
        let a = store.add("a", Priority::Med, None).unwrap();
        store.mark_done(&a.id).unwrap();
        assert_eq!(store.read().archive.len(), 1);
        store.clear_completed().unwrap();
        assert_eq!(store.read().archive.len(), 0);
    }

    #[test]
    fn persistence_roundtrips_across_store_instances() {
        let f = ScratchFile::new();
        {
            let store = NotchStore::new();
            store.init_with_path(f.path.clone()).unwrap();
            store.add("persisted", Priority::High, None).unwrap();
        }
        let store2 = NotchStore::new();
        store2.init_with_path(f.path.clone()).unwrap();
        let doc = store2.read();
        assert_eq!(doc.items.len(), 1);
        assert_eq!(doc.items[0].text, "persisted");
    }

    #[test]
    fn parse_command_response_strips_json_fence() {
        let raw = "```json\n{\"ops\": [{\"op\": \"add\", \"text\": \"buy milk\"}], \"confidence\": 0.9}\n```";
        let parsed = parse_command_response(raw).expect("parses fenced json");
        assert_eq!(parsed.ops.len(), 1);
        assert!((parsed.confidence - 0.9).abs() < 1e-6);
        match &parsed.ops[0] {
            NotchOp::Add { text, .. } => assert_eq!(text, "buy milk"),
            _ => panic!("expected add"),
        }
    }

    #[test]
    fn parse_command_response_handles_prose_around_object() {
        let raw = "Sure! Here is the result: {\"ops\": [], \"needs_clarification\": \"which one?\"} hope that helps";
        let parsed = parse_command_response(raw).expect("parses with prose");
        assert!(parsed.ops.is_empty());
        assert_eq!(parsed.needs_clarification.as_deref(), Some("which one?"));
    }

    #[test]
    fn parse_command_response_returns_none_on_garbage() {
        assert!(parse_command_response("not json at all").is_none());
        assert!(parse_command_response("").is_none());
        assert!(parse_command_response("}{").is_none());
    }

    #[test]
    fn apply_ops_resolves_target_by_index() {
        let mut doc = NotchListDoc::default();
        build_add(&mut doc, "first", Priority::High, None).unwrap();
        build_add(&mut doc, "second", Priority::High, None).unwrap();
        doc.normalize_order();
        // Complete the 2nd item (1-based).
        let ops = vec![NotchOp::Complete {
            target: Target::Index(2),
        }];
        let r = apply_ops(&doc, &ops, false).unwrap();
        assert_eq!(r.completed, 1);
        assert_eq!(r.doc.items.len(), 1);
        assert_eq!(r.doc.items[0].text, "first");
    }

    #[test]
    fn apply_ops_resolves_target_by_text_fuzzy() {
        let mut doc = NotchListDoc::default();
        build_add(&mut doc, "Call the dentist tomorrow", Priority::Med, None).unwrap();
        build_add(&mut doc, "Email Bob", Priority::Med, None).unwrap();
        doc.normalize_order();
        let ops = vec![NotchOp::Complete {
            target: Target::Text("dentist".into()),
        }];
        let r = apply_ops(&doc, &ops, false).unwrap();
        assert_eq!(r.completed, 1);
        assert!(r.doc.items.iter().all(|i| !i.text.contains("dentist")));
    }

    #[test]
    fn apply_ops_unresolvable_target_errors() {
        let doc = NotchListDoc::default();
        let ops = vec![NotchOp::Complete {
            target: Target::Index(1),
        }];
        assert!(apply_ops(&doc, &ops, false).is_err());
    }

    #[test]
    fn apply_ops_stages_destructive_when_confirm_on() {
        let mut doc = NotchListDoc::default();
        build_add(&mut doc, "task", Priority::Med, None).unwrap();
        doc.normalize_order();
        let ops = vec![NotchOp::Remove {
            target: Target::Index(1),
        }];
        let r = apply_ops(&doc, &ops, true).unwrap();
        assert!(r.staged_destructive);
        // `r.doc` now carries the FULLY-APPLIED result (item removed),
        // ready for the caller to stage and commit on confirmation. The
        // on-disk list is only mutated when the caller commits.
        assert_eq!(r.doc.items.len(), 0);
        assert_eq!(r.doc.archive.len(), 1);
        assert_eq!(r.removed, 1);
    }

    #[test]
    fn apply_ops_applies_destructive_when_confirm_off() {
        let mut doc = NotchListDoc::default();
        build_add(&mut doc, "task", Priority::Med, None).unwrap();
        doc.normalize_order();
        let ops = vec![NotchOp::Remove {
            target: Target::Index(1),
        }];
        let r = apply_ops(&doc, &ops, false).unwrap();
        assert!(!r.staged_destructive);
        assert_eq!(r.removed, 1);
        assert_eq!(r.doc.items.len(), 0);
        assert_eq!(r.doc.archive.len(), 1);
    }

    #[test]
    fn apply_voice_ops_undo_restores_previous() {
        let (store, _f) = fresh_store();
        store.add("keep", Priority::Med, None).unwrap();
        let ops = vec![NotchOp::Add {
            text: "added by voice".into(),
            priority: None,
            position: None,
        }];
        store.apply_voice_ops(ops, true).unwrap();
        assert_eq!(store.read().items.len(), 2);
        assert!(store.undo().unwrap());
        assert_eq!(store.read().items.len(), 1);
        // Nothing left to undo.
        assert!(!store.undo().unwrap());
    }

    #[test]
    fn apply_ops_resolves_text_via_id_string_fallback() {
        // Over the wire a string target deserializes into Target::Id
        // (untagged enum), so the LLM's "the dentist task" arrives as
        // Id("...the dentist..."). resolve_target must still fuzzy-match.
        let mut doc = NotchListDoc::default();
        build_add(&mut doc, "Call the dentist tomorrow", Priority::Med, None).unwrap();
        build_add(&mut doc, "Email Bob", Priority::Med, None).unwrap();
        doc.normalize_order();
        let ops = vec![NotchOp::Complete {
            target: Target::Id("dentist".into()),
        }];
        let r = apply_ops(&doc, &ops, false).unwrap();
        assert_eq!(r.completed, 1);
        assert!(r.doc.items.iter().all(|i| !i.text.contains("dentist")));
    }

    #[test]
    fn commit_staged_persists_and_snapshots_for_undo() {
        let (store, _f) = fresh_store();
        store.add("keep", Priority::Med, None).unwrap();
        // A staged destructive batch is just the fully-applied doc the
        // confirm gesture will commit.
        let ops = vec![NotchOp::Remove {
            target: Target::Index(1),
        }];
        let staged = apply_ops(&store.read(), &ops, true).unwrap();
        assert!(staged.staged_destructive);
        store.commit_staged(staged.doc).unwrap();
        assert_eq!(store.read().items.len(), 0);
        assert!(store.can_undo());
        assert!(store.undo().unwrap());
        assert_eq!(store.read().items.len(), 1);
    }

    #[test]
    fn can_undo_reports_snapshot_presence() {
        let (store, _f) = fresh_store();
        assert!(!store.can_undo());
        store.add("a", Priority::Med, None).unwrap();
        assert!(store.can_undo());
        store.undo().unwrap();
        assert!(!store.can_undo());
    }

    #[test]
    fn is_safe_item_id_matches_slug_shape() {
        assert!(is_safe_item_id("item-123-4"));
        assert!(!is_safe_item_id(""));
        assert!(!is_safe_item_id("-bad"));
        assert!(!is_safe_item_id("bad-"));
        assert!(!is_safe_item_id("../x"));
        assert!(is_safe_item_id(&gen_id()));
    }

    #[test]
    fn summary_reads_naturally() {
        let r = NotchApplyResult {
            doc: NotchListDoc::default(),
            added: 1,
            completed: 2,
            removed: 0,
            edited: 0,
            reprioritized: 0,
            reordered: 0,
            staged_destructive: false,
        };
        assert_eq!(r.summary(), "Added 1, completed 2");
    }

    #[test]
    fn action_applies_any_nonempty_ops_ignoring_confidence() {
        // The regression that made natural speech vanish: a multi-item
        // brain-dump the model returned with low confidence must STILL be
        // applied. Adding is the default and every add is undoable.
        let resp = NotchCommandResponse {
            ops: vec![
                NotchOp::Add {
                    text: "Finish the alarm features".into(),
                    priority: Some(Priority::High),
                    position: None,
                },
                NotchOp::Add {
                    text: "Apply for Apple review".into(),
                    priority: None,
                    position: None,
                },
            ],
            needs_clarification: None,
            confidence: 0.1,
        };
        assert_eq!(resp.action(), NotchAction::Apply);
    }

    #[test]
    fn action_prefers_acting_over_a_tacked_on_question() {
        // If the model both proposed ops AND tacked on a question, act —
        // never strand the user's tasks behind a clarification prompt.
        let resp = NotchCommandResponse {
            ops: vec![NotchOp::Add {
                text: "Buy milk".into(),
                priority: None,
                position: None,
            }],
            needs_clarification: Some("anything else?".into()),
            confidence: 0.2,
        };
        assert_eq!(resp.action(), NotchAction::Apply);
    }

    #[test]
    fn action_clarifies_only_with_no_ops_and_a_real_question() {
        let resp = NotchCommandResponse {
            ops: vec![],
            needs_clarification: Some("Which task did you mean?".into()),
            confidence: 0.4,
        };
        assert_eq!(
            resp.action(),
            NotchAction::Clarify("Which task did you mean?".into())
        );
    }

    #[test]
    fn action_nothing_when_empty_with_blank_or_absent_question() {
        let blank = NotchCommandResponse {
            ops: vec![],
            needs_clarification: Some("   ".into()),
            confidence: 0.9,
        };
        assert_eq!(blank.action(), NotchAction::Nothing);
        let absent = NotchCommandResponse {
            ops: vec![],
            needs_clarification: None,
            confidence: 0.9,
        };
        assert_eq!(absent.action(), NotchAction::Nothing);
    }
}
