//! Orchestrates the dictation pipeline: mic capture → STT → optional LLM
//! cleanup → clipboard. STT/LLM engines come from the per-app
//! `engines::Registry`; both are lazy-loaded and kept warm.
//!
//! Parallel sessions: `SessionManager` allows up to
//! [`MAX_CONCURRENT_SESSIONS`] in-flight pipelines. Only ONE recording is
//! active at any moment (cpal mic capture is single-instance hardware) —
//! "parallel" means a NEW recording can start while previous sessions are
//! still in their post-recording (transcribe / polish) stages. The pill
//! always shows the foreground (most-recent recording) session; older
//! ones live in the tray popover's IN PROGRESS list. Last-finished-wins
//! on the clipboard.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime, Wry};
use tokio::sync::Notify;

use crate::audio::AudioHandle;
use crate::engines::Registry;
use crate::mac_power;
use crate::modes::{self, CaseStyle, ModeStore, PunctuationStyle, VoiceMode};
use crate::pill::{self, PillState};
use crate::transcripts::{self, CompletedEntry, InProgressEntry, SessionsSnapshot};
use crate::{inject, settings};

const MAX_RECORD_SECS: u64 = 30 * 60;
const MIN_RECORD_MS: u128 = 300;
const SILENCE_PEAK_THRESHOLD: f32 = 0.003;

/// Maximum simultaneous in-flight pipelines (one active recording plus
/// up to two background polish/transcribe tasks). Hard cap — UI shows an
/// amber toast when a 4th attempt is made and refuses to start it.
pub const MAX_CONCURRENT_SESSIONS: usize = 3;

/// Max gap between the two cancel-hotkey presses for the opt-in
/// double-press cancel (`Settings::cancel_requires_double_press`). A
/// second press within this window confirms the cancel; a lone press
/// just arms the window and surfaces a pill hint. Tuned as a deliberate
/// "double tap" — long enough to read the hint and tap again, short
/// enough that two unrelated Escape presses rarely both land inside it.
pub const CANCEL_DOUBLE_PRESS_WINDOW_MS: u32 = 700;
const CANCEL_DOUBLE_PRESS_WINDOW: Duration =
    Duration::from_millis(CANCEL_DOUBLE_PRESS_WINDOW_MS as u64);

/// How long a finalised session sticks around in the manager's map so
/// the popover briefly shows its terminal stage before it folds into
/// "RECENT".
const TERMINAL_LINGER_MS: u64 = 1500;

/// Done-stage hint shown when the transcript is on the clipboard and the
/// user pastes it themselves. The paste key differs per platform — ⌘V on
/// macOS, Ctrl+V on Windows.
#[cfg(target_os = "macos")]
const PASTE_HINT: &str = "Copied — press \u{2318}V";
#[cfg(not(target_os = "macos"))]
const PASTE_HINT: &str = "Copied — press Ctrl+V";

/// Done-stage hint shown when polish succeeded but auto-paste didn't fire.
/// On macOS that almost always means Accessibility wasn't granted; on
/// Windows there is no such grant, so `SendInput` failing means the
/// foreground window rejected the injection (typically an elevated app).
#[cfg(target_os = "macos")]
const PASTE_FAILED_HINT: &str = "Copied — grant Accessibility for auto-paste";
#[cfg(not(target_os = "macos"))]
const PASTE_FAILED_HINT: &str = "Copied — paste didn't go through, press Ctrl+V";

/// How long an Error pill stays visible before auto-hiding. The Done
/// path schedules its own hide from `run_pipeline`'s tail with a custom
/// duration; Canceled is hidden ~600 ms after `cancel_session`. Error
/// is reached from several early-return paths in `run_pipeline` that
/// each skip the tail — without a central hide schedule the message
/// (e.g. "No audio detected — check your mic.") stays on screen until
/// the next dictation starts. Matches the warning-treatment Done
/// duration so the message can be read comfortably.
const ERROR_LINGER_MS: u64 = 3500;

/// Toast duration for the saturated state.
const SATURATED_TOAST_MS: u32 = 1500;

/// Toast duration for the "pipeline still in flight" rejection. Slightly
/// shorter than the saturation toast — the user is mid-keystroke and we
/// just want a brief acknowledgement that the press was seen but ignored.
const BUSY_TOAST_MS: u32 = 1500;

/// Pure decision for the opt-in double-press cancel. Given the stored
/// timestamp of a prior unconfirmed cancel press (if any), the current
/// instant, and the confirmation window, returns `(next_stored, fire)`:
/// `fire` is true when this press confirms the cancel; `next_stored` is
/// what to write back (`None` once it fires or hasn't armed, `Some(now)`
/// while a fresh window is open). A second press *after* the window
/// lapses simply re-arms rather than firing. Split out as a pure fn so
/// the timing logic is unit-testable without real clocks.
fn double_press_decision(
    prev: Option<Instant>,
    now: Instant,
    window: Duration,
) -> (Option<Instant>, bool) {
    match prev {
        Some(first) if now.duration_since(first) <= window => (None, true),
        _ => (Some(now), false),
    }
}

pub type SessionId = u64;

/// Binary admission gate for the dictation pipeline. While a previous
/// session is still in its post-recording stages (transcribe → polish →
/// clipboard) a new hotkey press is rejected so the second pipeline
/// can't double-up on the `WhisperContext` mutex or race the audio
/// device.
///
/// Implemented as `AtomicUsize` rather than `AtomicBool` so the same
/// type can later be retargeted to a cap-N variant (e.g. when the
/// shared-state hazards behind the gate get factored out) without
/// changing the public API. Today only `try_acquire` admits, and it
/// uses `compare_exchange(0, 1)` — strictly binary. Release happens
/// via the RAII [`BusyGuard`] on Drop, with an idempotent eager
/// [`BusyGuard::release`] for cancel and audio-rollback paths.
#[derive(Clone, Default)]
pub struct BusyGate {
    count: Arc<AtomicUsize>,
}

impl BusyGate {
    pub fn new() -> Self {
        Self {
            count: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Try to take the gate. Returns `Some(BusyGuard)` if the gate was
    /// idle (count was 0 → 1); returns `None` if a pipeline is already
    /// in flight. The returned guard decrements on Drop; release is
    /// idempotent so an explicit release in a cancel path won't
    /// double-decrement when the guard is later dropped.
    pub fn try_acquire(&self) -> Option<BusyGuard> {
        match self.count.compare_exchange(
            0,
            1,
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            Ok(_) => Some(BusyGuard {
                count: self.count.clone(),
                released: AtomicBool::new(false),
            }),
            Err(_) => None,
        }
    }

    #[cfg(test)]
    pub fn in_flight(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }
}

/// RAII handle that decrements the [`BusyGate`] on Drop. Idempotent
/// `release()` lets the cancel and audio-rollback paths free the gate
/// promptly without breaking the Drop-side decrement that fires when
/// the owning `SessionState` is finally dropped.
pub struct BusyGuard {
    count: Arc<AtomicUsize>,
    released: AtomicBool,
}

impl BusyGuard {
    pub fn release(&self) {
        if !self.released.swap(true, Ordering::SeqCst) {
            self.count.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        self.release();
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    Listening,
    Transcribing,
    Polishing,
    Done,
    Error,
    Canceled,
}

/// Outcome of [`SessionManager::deliver_transcript`]. The session
/// pipeline reads this to pick the right Done-stage message — and to
/// set the warning flag when the paste fell back to clipboard-only,
/// so the user notices that auto-paste didn't go through.
#[derive(Debug)]
pub enum DeliveryOutcome {
    /// Auto-paste was off. The transcript is on the clipboard; the
    /// user pastes manually. Matches Vox's default v1 behaviour.
    CopiedOnly,
    /// Auto-paste was on and the synthetic ⌘V was posted. A delayed
    /// clipboard restore may be running on a background thread.
    Pasted,
    /// Auto-paste was on but the ⌘V post failed (typically because
    /// Accessibility isn't granted). The transcript is still on the
    /// clipboard, so the user can paste manually — we surface a
    /// warning so they know to grant Accessibility or paste themselves.
    /// The specific reason is logged at the moment of failure; we don't
    /// carry it through to the UI because the toast is one-size-fits-all.
    PasteFailedCopiedOnly,
}

/// In-flight clipboard restore — tracked on the [`SessionManager`] so
/// a follow-up `deliver_transcript` (e.g. the user dictates again
/// before the 250 ms restore lands) can cancel the previous restore
/// and inherit its captured `prior_text`. Without this, two rapid
/// pipelines race: pipeline B would snapshot pipeline A's polished
/// text as if it were the user's clipboard, then schedule a "restore"
/// that writes A's polished text back, permanently clobbering the
/// user's true prior content.
struct RestoreToken {
    /// The clipboard text the user had BEFORE this pipeline (or before
    /// the first pipeline in a rapid chain) clobbered it. The restore
    /// thread writes this back when it fires.
    prior_text: String,
    /// Flipped to `true` when a subsequent pipeline takes responsibility
    /// for the chain's restore. The waiting thread checks this both
    /// before and after acquiring the pending-restore lock so a cancel
    /// that lands during the sleep window or during lock acquisition
    /// still wins.
    cancel: Arc<AtomicBool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModeSnapshot {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub system_prompt: String,
    pub punctuation: PunctuationStyle,
    pub case: CaseStyle,
    pub max_length_chars: Option<usize>,
    pub dictionary: Vec<String>,
    pub context_enabled: bool,
}

impl ModeSnapshot {
    fn from_voice_mode(m: &VoiceMode) -> Self {
        Self {
            id: m.id.clone(),
            name: m.name.clone(),
            icon: m.icon.clone(),
            system_prompt: m.system_prompt.clone(),
            punctuation: m.punctuation.clone(),
            case: m.case.clone(),
            max_length_chars: m.max_length_chars,
            dictionary: m.dictionary.clone(),
            context_enabled: m.context_enabled,
        }
    }

    fn fallback() -> Self {
        Self {
            id: "general".into(),
            name: "General".into(),
            icon: "💬".into(),
            system_prompt: String::new(),
            punctuation: PunctuationStyle::Standard,
            case: CaseStyle::Sentence,
            max_length_chars: None,
            dictionary: Vec::new(),
            context_enabled: true,
        }
    }

    fn apply_post_processing(&self, text: &str) -> String {
        // Snapshot-typed post-processing — delegates to the shared
        // free function in `modes` so the rule set lives in exactly
        // one place. The snapshot still owns its own copies of the
        // fields so a mid-session edit to the underlying mode can't
        // change post-processing for an in-flight pipeline.
        modes::apply_post_processing(
            &self.case,
            &self.punctuation,
            self.max_length_chars,
            text,
        )
    }
}

/// Per-session live state. `SessionState`s are kept in
/// `SessionManager::sessions` while in flight, and removed on a delay
/// after they reach a terminal `Stage` so the popover shows the
/// terminal label briefly before the row finalises into "RECENT".
pub struct SessionState {
    pub id: SessionId,
    pub stage: Mutex<Stage>,
    pub mode: ModeSnapshot,
    pub polishing_enabled: bool,
    pub stt_engine_id: String,
    pub stt_model_id: String,
    pub llm_engine_id: String,
    pub llm_model_id: String,
    pub language: String,
    pub dictionary: Vec<String>,
    pub context: String,
    pub cancel: Arc<AtomicBool>,
    pub started_at: Instant,
    pub started_at_unix: i64,
    pub transcript: Mutex<Option<String>>,
    pub result: Mutex<Option<String>>,
    pub error: Mutex<Option<String>>,
    pub warning: Mutex<bool>,
    /// Flips to `true` when the pipeline reaches any terminal state
    /// (Done, Error, Canceled, audio-too-short bail). Read by the
    /// max-record guard task to short-circuit its 30-min sleep so the
    /// guard's Arc<SessionManager> + Arc<SessionState> + AppHandle
    /// captures drop within ~one tick of session completion instead of
    /// being held for the full half-hour.
    pub completed: AtomicBool,
    /// Async wake-up source paired with `completed` so the guard task
    /// can `select!` between sleep and completion without polling.
    /// Notified exactly once when `completed` is first set to true.
    pub completion: Arc<Notify>,
    /// Held by the speculative streaming decoder while it has a
    /// `whisper_rs::WhisperState::full` call in flight. The
    /// authoritative final pass in `run_pipeline` acquires this lock
    /// before its own decode so the two passes never share a
    /// `WhisperContext` at the same time. (Whisper-rs's `WhisperState`
    /// is per-call but the underlying context is not safe to drive
    /// concurrently from multiple states.) Also serves as the gate the
    /// final pass waits on to "absorb" an in-flight speculative pass
    /// rather than killing it mid-decode.
    pub speculative_decode: Arc<tokio::sync::Mutex<()>>,
    /// Snapshot of the user's `streaming_pill_enabled` setting at
    /// session-begin time. Captured once so the speculative decoder
    /// and the final pipeline pass cannot disagree if the user toggles
    /// the setting mid-utterance. Both sites read this field instead
    /// of re-loading settings.
    pub streaming_enabled: bool,
    /// Snapshot of `Settings::live_transcript_enabled`. Captured at
    /// session start so a mid-utterance toggle can't swap the pill
    /// silhouette under the user. When true, the speculative decoder
    /// bypasses its battery-saver bail and `stream_to_pill` opens up
    /// the transcript-stream emit even if `streaming_enabled` is
    /// false — the opt-in implies the user wants the live caption.
    pub live_transcript_enabled: bool,
    /// Snapshot of `Settings::auto_paste_enabled`. Captured at session
    /// start so flipping the toggle mid-pipeline doesn't change
    /// delivery behaviour for an in-flight transcript. Read by
    /// `run_pipeline` at the clipboard-delivery step.
    pub auto_paste_enabled: bool,
    /// Resolved at session start: per-mode override falls back to the global `Settings` value.
    pub enable_thinking: bool,
    /// When true this session routes its transcript through the notch
    /// command interpreter (returns structured ops applied to the Notch
    /// List) instead of the polish + clipboard delivery path. Set in
    /// `begin` from either the one-shot notch-voice-hotkey flag or when
    /// the active mode is `notch-list`. macOS-only behaviour, but the
    /// flag itself is cross-platform so the field compiles everywhere.
    pub notch_command: bool,
    /// RAII handle for the `SessionManager::busy_gate`. Held by the
    /// session for its entire lifetime; the gate decrements when the
    /// last `Arc<SessionState>` is dropped (after the spawned
    /// pipeline task finishes and the terminal-cleanup linger removes
    /// the session from the manager's map). `cancel_session` and the
    /// audio-start failure path call `release()` explicitly so the
    /// next press is accepted promptly without waiting for the
    /// terminal linger or any pending Arc clones to drain. `release()`
    /// is idempotent — the eventual Drop is a no-op once it's been
    /// released early.
    pub busy_guard: Mutex<Option<BusyGuard>>,
}

impl SessionState {
    pub fn is_canceled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    /// Mark the session pipeline as finished and wake any waiter
    /// (currently the max-record guard task). Idempotent — multiple
    /// callers in different terminal paths are safe.
    pub fn mark_completed(&self) {
        if !self.completed.swap(true, Ordering::SeqCst) {
            self.completion.notify_waiters();
        }
    }

    fn current_stage(&self) -> Stage {
        *self.stage.lock()
    }

    fn snapshot_in_progress(&self) -> InProgressEntry {
        InProgressEntry {
            session_id: self.id,
            stage: *self.stage.lock(),
            mode_id: self.mode.id.clone(),
            mode_name: self.mode.name.clone(),
            mode_icon: self.mode.icon.clone(),
            started_at: self.started_at_unix,
        }
    }
}

#[derive(Debug, Serialize)]
pub enum BeginOutcome {
    Started(SessionId),
    Saturated { in_flight: usize, notch: bool },
    DictationDisabled,
    AlreadyRecording,
    AudioError(String),
    /// A previous session is still in its post-recording pipeline
    /// (transcribe / polish / clipboard) and a new press would race the
    /// shared `WhisperContext` mutex. Surface a brief pill toast and
    /// drop the press; the user retries once the pill clears.
    PipelineBusy,
}

pub struct SessionManager {
    pub audio: AudioHandle,
    pub sessions: Mutex<BTreeMap<SessionId, Arc<SessionState>>>,
    next_id: AtomicU64,
    active_recording: Mutex<Option<SessionId>>,
    /// Tracks which session owns the pill UI right now. Set on
    /// `begin` and replaced (not cleared) by the next begin — survives
    /// the end-of-recording → transcribe/polish handoff so the pill's
    /// "Transcribing.." stage label still flows from the right session.
    /// Streaming pill events are filtered through this same gate so
    /// background sessions never blast text into the foreground pill.
    pill_owner: Mutex<Option<SessionId>>,
    pub max_concurrent: usize,
    /// Serialises clipboard writes so two pipelines finishing at the
    /// same moment don't interleave their `Clipboard::set_text` calls.
    /// "Last-finished-wins" still holds — whichever lock acquisition
    /// runs second writes last.
    clipboard_lock: Mutex<()>,
    /// Tracks the in-flight auto-paste clipboard restore, if any. A
    /// rapid second pipeline cancels the previous restore (so its
    /// thread won't write stale data) and inherits the captured
    /// `prior_text` so the chain's eventual restore still puts the
    /// user's true pre-chain clipboard back. See [`RestoreToken`].
    pending_restore: Mutex<Option<RestoreToken>>,
    /// Single-pipeline admission gate. `try_acquire` is the one
    /// admission point — a press while a previous pipeline is still
    /// in its post-recording stages (transcribe / polish / clipboard)
    /// gets `None` back and surfaces a brief pill toast. Release
    /// happens via the RAII guard parked on `SessionState::busy_guard`
    /// (Drop) plus eager `release()` calls in `cancel_session` and the
    /// audio-rollback branch of `begin`.
    pub busy_gate: BusyGate,
    /// Timestamp of the first cancel-hotkey press while the opt-in
    /// double-press cancel is armed and a recording is live. `Some`
    /// means a confirmation window is open: a second press within
    /// [`CANCEL_DOUBLE_PRESS_WINDOW`] confirms the cancel. Reset at every
    /// `begin_recording`; only consulted by `request_cancel_foreground`.
    cancel_first_press_at: Mutex<Option<Instant>>,
    /// One-shot flag set by the notch-voice hotkey just before it calls
    /// `begin`. The next `begin` consumes it (swap to false) and flags the
    /// new session as a notch-command session. A one-shot flag (rather
    /// than a `begin` argument) keeps the hands-free toggle path — which
    /// can't pass extra args through the shared PTT closure — simple.
    notch_command_armed: AtomicBool,
    /// Staged destructive notch batch awaiting confirmation. Holds the
    /// fully-applied new doc plus the instant it was staged. The NEXT
    /// notch-voice hotkey press within [`NOTCH_CONFIRM_WINDOW`] commits
    /// it instead of starting a recording. Cleared on commit, on timeout,
    /// or by any non-destructive voice edit.
    pending_notch_destructive: Mutex<Option<(crate::notch_list::NotchListDoc, Instant)>>,
}

/// How long a staged destructive notch batch stays confirmable. A second
/// notch-voice press within this window commits it; later presses start a
/// fresh recording (the stale stage is dropped).
const NOTCH_CONFIRM_WINDOW: Duration = Duration::from_secs(8);

impl SessionManager {
    pub fn new(audio: AudioHandle) -> Self {
        Self {
            audio,
            sessions: Mutex::new(BTreeMap::new()),
            next_id: AtomicU64::new(0),
            active_recording: Mutex::new(None),
            pill_owner: Mutex::new(None),
            max_concurrent: MAX_CONCURRENT_SESSIONS,
            clipboard_lock: Mutex::new(()),
            pending_restore: Mutex::new(None),
            busy_gate: BusyGate::new(),
            cancel_first_press_at: Mutex::new(None),
            notch_command_armed: AtomicBool::new(false),
            pending_notch_destructive: Mutex::new(None),
        }
    }

    /// Arm the next `begin` to start a notch-command session. Called by
    /// the notch-voice hotkey closure immediately before `begin`.
    pub fn arm_notch_command(&self) {
        self.notch_command_armed.store(true, Ordering::SeqCst);
    }

    /// Stage a confirmed-pending destructive notch batch. The next
    /// notch-voice press within [`NOTCH_CONFIRM_WINDOW`] commits it.
    pub fn stage_notch_destructive(&self, doc: crate::notch_list::NotchListDoc) {
        *self.pending_notch_destructive.lock() = Some((doc, Instant::now()));
    }

    /// Drop any staged destructive batch (e.g. after a normal voice edit
    /// or once committed).
    pub fn clear_notch_destructive(&self) {
        *self.pending_notch_destructive.lock() = None;
    }

    /// If a fresh staged destructive batch exists, commit it to the notch
    /// store (with undo) and return `true`; the caller then skips starting
    /// a recording. A stale stage (older than [`NOTCH_CONFIRM_WINDOW`]) is
    /// dropped and `false` returned so the press starts a normal session.
    pub fn try_commit_pending_notch(&self, app: &AppHandle<Wry>) -> bool {
        let staged = self.pending_notch_destructive.lock().take();
        let Some((doc, at)) = staged else {
            return false;
        };
        if at.elapsed() > NOTCH_CONFIRM_WINDOW {
            return false;
        }
        let store = crate::notch_list::store(app);
        match store.commit_staged(doc) {
            Ok(()) => {
                crate::commands::emit_notch_list_changed(app);
                pill::emit_notch_phase(app, "idle", None);
                true
            }
            Err(e) => {
                log::warn!("notch confirm: commit failed: {e}");
                false
            }
        }
    }

    /// True when `id` is the session whose state changes should drive
    /// the pill UI right now. Streaming events use the same predicate.
    pub fn owns_pill_emit(&self, id: SessionId) -> bool {
        *self.pill_owner.lock() == Some(id)
    }

    pub fn foreground_id(&self) -> Option<SessionId> {
        *self.active_recording.lock()
    }

    pub fn is_recording(&self) -> bool {
        self.active_recording.lock().is_some()
    }

    #[allow(dead_code)]
    pub fn pipeline_in_flight_count(&self) -> usize {
        self.sessions.lock().len()
    }

    /// True iff there's any session that hasn't reached a terminal
    /// stage yet. Used by the mode-cycle hide-after to avoid hiding
    /// the pill while a session is still active.
    pub fn has_active_pipeline(&self) -> bool {
        let map = self.sessions.lock();
        map.values().any(|s| {
            !matches!(
                s.current_stage(),
                Stage::Done | Stage::Error | Stage::Canceled
            )
        })
    }

    pub fn snapshot(&self) -> SessionsSnapshot {
        let in_progress: Vec<InProgressEntry> = {
            let map = self.sessions.lock();
            // Show oldest-first so the order in the popover matches
            // the natural "earlier → later" reading direction.
            map.values().map(|s| s.snapshot_in_progress()).collect()
        };
        SessionsSnapshot {
            in_progress,
            recent: transcripts::snapshot_completed(),
        }
    }

    /// Begin a new recording. Returns the outcome — successful starts
    /// register the session, arm the cancel grab, show the pill, and
    /// kick off audio + level sampler tasks.
    pub fn begin(self: &Arc<Self>, app: &AppHandle<Wry>) -> BeginOutcome {
        // Consume the one-shot notch-command arm FIRST, before any
        // early-return below, so a failed/rejected begin can never leave
        // the flag set for a later normal dictation. Whether we end up
        // starting a session or not, this press's intent is now spent.
        let notch_armed = self.notch_command_armed.swap(false, Ordering::SeqCst);

        let prefs = settings::prefs(app);
        if !prefs.dictation_enabled {
            log::info!("dictation is disabled; hotkey press ignored");
            return BeginOutcome::DictationDisabled;
        }

        // Single-pipeline gate. A press while a previous session is
        // still running its post-recording stages (transcribe / polish /
        // clipboard) is rejected with a brief pill toast — without this
        // the second pipeline would contend for the shared
        // `WhisperContext` mutex held inside `spawn_blocking`, freezing
        // the app for the duration of the previous decode.
        let busy_guard = match self.busy_gate.try_acquire() {
            Some(g) => g,
            None => {
                log::warn!("ignoring hotkey: pipeline still in flight");
                if notch_armed {
                    // Notch press: its feedback lives in-notch, not in the
                    // pill. Route the busy rejection to the notch surface
                    // (auto-clears after ~4.5s) and skip the pill-only
                    // toast + state-restore entirely.
                    pill::emit_notch_phase(
                        app,
                        "error",
                        Some("Still finishing the previous list".into()),
                    );
                } else {
                    // No pill::show here on purpose: the pill is guaranteed
                    // visible because a pipeline is in flight (the busy_gate
                    // cap-1 IS the proof of life). Calling show would fire
                    // the frontend's `pill:show` listener which calls
                    // clearStreaming(), wiping the in-flight polish stream
                    // out from under the user.
                    pill::emit_toast(
                        app,
                        "Still finishing previous".into(),
                        BUSY_TOAST_MS,
                    );
                    // Restore the live pipeline stage on the pill as soon as
                    // the toast plays out. The frontend's toast handler
                    // (Pill.tsx) flips state to "saturated" on toast arrival
                    // and only schedules a clear of the toast MESSAGE — it
                    // never clears the state itself. Without this re-emit
                    // the pill stays stuck on the saturated warning until
                    // the pipeline's next stage transition fires a fresh
                    // pill:state event, which during a long polish run is
                    // typically not until Done.
                    self.schedule_pill_state_restore(app, u64::from(BUSY_TOAST_MS));
                }
                return BeginOutcome::PipelineBusy;
            }
        };

        // Saturation check: total in-flight (including any active
        // recording) must be < max_concurrent before we start a new
        // one. The busy_gate above is cap-1, so this branch is
        // currently unreachable; it stays as the cap-N saturation
        // policy so future work that flips the gate to cap-N (after
        // the shared-state hazards behind it get factored out) doesn't
        // have to re-derive when to emit the saturated toast.
        {
            let map = self.sessions.lock();
            if map.len() >= self.max_concurrent {
                let in_flight = map.len();
                log::info!(
                    "session: saturated ({in_flight}/{}) — rejecting new session",
                    self.max_concurrent
                );
                drop(busy_guard);
                return BeginOutcome::Saturated {
                    in_flight,
                    notch: notch_armed,
                };
            }
        }

        // The hardware-mic constraint: only one session may be
        // recording at any moment. If a recording is already in
        // flight, the PTT closure should call end_recording instead;
        // returning here is the race-loser case.
        let mut active = self.active_recording.lock();
        if active.is_some() {
            log::debug!("begin called while a recording is already active; ignored");
            drop(busy_guard);
            return BeginOutcome::AlreadyRecording;
        }

        let new_sid = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;

        // Snapshot mode at session start. Subsequent mode switches
        // never affect the in-flight transcript.
        let active_mode = resolve_active_mode(app, &prefs.active_mode_id);
        let mode_snap = active_mode
            .as_ref()
            .map(ModeSnapshot::from_voice_mode)
            .unwrap_or_else(ModeSnapshot::fallback);

        // Per-session resolved chain-of-thought toggle: mode override
        // wins over the global setting; `None` (the default for every
        // built-in mode) inherits.
        let enable_thinking = active_mode
            .as_ref()
            .and_then(|m| m.enable_thinking)
            .unwrap_or(prefs.enable_thinking);

        // Notch-command routing: the one-shot hotkey flag (consumed at the
        // top of `begin`), OR the dedicated `notch-list` voice mode being
        // active. Either path sends this session's transcript to the notch
        // interpreter instead of polish + clipboard.
        let notch_command = notch_armed || prefs.active_mode_id == "notch-list";

        let session = Arc::new(SessionState {
            id: new_sid,
            stage: Mutex::new(Stage::Listening),
            mode: mode_snap,
            polishing_enabled: prefs.polishing_enabled,
            stt_engine_id: prefs.stt_engine.clone(),
            stt_model_id: prefs.stt_model_id.clone(),
            llm_engine_id: if prefs.polishing_enabled {
                prefs.llm_engine.clone()
            } else {
                "none".into()
            },
            llm_model_id: prefs.llm_model_id.clone(),
            language: prefs.language.clone(),
            dictionary: prefs.dictionary.clone(),
            context: prefs.context.clone(),
            cancel: Arc::new(AtomicBool::new(false)),
            started_at: Instant::now(),
            started_at_unix: chrono::Utc::now().timestamp(),
            transcript: Mutex::new(None),
            result: Mutex::new(None),
            error: Mutex::new(None),
            warning: Mutex::new(false),
            completed: AtomicBool::new(false),
            completion: Arc::new(Notify::new()),
            speculative_decode: Arc::new(tokio::sync::Mutex::new(())),
            streaming_enabled: prefs.streaming_pill_enabled,
            live_transcript_enabled: prefs.live_transcript_enabled,
            auto_paste_enabled: prefs.auto_paste_enabled,
            enable_thinking,
            notch_command,
            busy_guard: Mutex::new(Some(busy_guard)),
        });

        // Insert into map BEFORE starting audio so the saturation
        // check is monotonic and the snapshot shows the new session
        // immediately. If audio fails we'll remove + reset below.
        self.sessions.lock().insert(new_sid, session.clone());
        *active = Some(new_sid);
        // Take ownership of the pill — replaces any previous session
        // whose pipeline is still running in the background, so the
        // pill always tracks the newest recording. A notch-voice (⌘L)
        // session deliberately does NOT claim the pill: its UI lives in
        // the notch, so leaving `pill_owner` untouched keeps every
        // `set_stage` pill emit a no-op for this session (and avoids
        // clobbering a real pill another in-flight session may own).
        if !notch_command {
            *self.pill_owner.lock() = Some(new_sid);
        }
        drop(active);

        // Arm the OS Escape grab now (and not at app launch) so
        // Escape only short-circuits to Vox while a dictation session
        // is in flight. The disarm fires from end_recording's drop
        // path or from cancel.
        crate::hotkey::arm_cancel(app);
        // Fresh session → discard any half-armed double-press window
        // left over from a previous recording.
        *self.cancel_first_press_at.lock() = None;

        log::info!("▶ begin recording (sid={new_sid})");
        // A notch-voice (⌘L) session NEVER shows the floating pill — its
        // listening UI lives in the notch instead. Drive the notch's
        // listening phase here and skip every pill::show / emit_state /
        // set_wide call below so the user never sees the pill for ⌘L.
        // The transcribing/compiling/done states for this session are
        // routed to the notch from `interpret_and_apply_notch`, and the
        // mic levels are mirrored to the notch by `spawn_audio_level_sampler`.
        if notch_command {
            pill::emit_notch_phase(app, "listening", None);
        } else {
            pill::show(app);
            pill::emit_state(app, PillState::Listening, None);
            // Live-transcript renders the listening pill in the wide layout
            // (header label + typing dots + companion chip) before the user
            // even releases the hotkey. The frontend's ResizeObserver pushes
            // the new size up via IPC, but the WebView2 → Tauri → DWM chain
            // on Windows runs slow enough that the OS window stays at the
            // compact 144 px frame for a visible frame or two and the pill
            // content clips at both sides. Widen here so the OS window
            // matches the layout the frontend already wants — idempotent
            // on macOS where the round-trip was fast enough to win the race.
            if prefs.live_transcript_enabled {
                pill::set_wide(app, true);
            }
        }
        emit_sessions_changed(app, self);

        if let Some(m) = active_mode.as_ref() {
            pill::emit_active_mode(app, m);
        }
        pill::emit_mode_suggestion(app, None);

        if prefs.auto_suggest_modes {
            spawn_suggestion_probe(app.clone(), prefs.active_mode_id.clone());
        }

        // Probe playback BEFORE opening the mic so the reading isn't
        // polluted by our own capture: on a combined Bluetooth headset
        // (one device carrying input + output) our mic stream can make
        // the output read "active" even in silence, which would let
        // pause mode post a media key that starts music from silence.
        // Skip the probe entirely when attenuation is off.
        let was_playing =
            prefs.audio_attenuation != "off" && crate::audio_attenuation::probe_playback();

        if let Err(e) = self.audio.start(prefs.audio_input_device.clone()) {
            log::error!("failed to start audio: {e}");
            // Roll back: release the busy gate, remove session, clear
            // active_recording, and tear down the cancel grab so Escape
            // works in other apps.
            if let Some(g) = session.busy_guard.lock().take() {
                g.release();
            }
            self.sessions.lock().remove(&new_sid);
            *self.active_recording.lock() = None;
            // Roll back pill ownership too if it's still pointing at
            // this session, otherwise leave whatever caller previously
            // claimed it intact.
            {
                let mut owner = self.pill_owner.lock();
                if *owner == Some(new_sid) {
                    *owner = None;
                }
            }
            crate::hotkey::disarm_cancel(app);
            // Branch on the typed error so a missing user-selected
            // mic shows "iPhone Microphone isn't connected" instead
            // of the generic "No mic access" line — the latter sends
            // the user to System Settings → Privacy when the real
            // fix is to reconnect the device or pick another input.
            // The catchall stays neutral on purpose: a `Cpal` error
            // spans TCC denial, sample-format mismatch, and CoreAudio
            // backend failures, so we can't safely pin one fix-it.
            let pill_msg = match &e {
                crate::audio::AudioError::ConfiguredDeviceUnavailable { name } => {
                    format!(
                        "'{name}' not connected — reconnect it or pick another mic in Settings."
                    )
                }
                crate::audio::AudioError::NoInputDevice => {
                    "No microphone available — connect one and try again.".into()
                }
                _ => "Microphone unavailable — check System Settings and try again.".into(),
            };
            // Route the mic-failure surface to the notch (not the pill)
            // for a notch-voice session so a ⌘L mic failure never flashes
            // the pill. The pill was never shown for this session, so its
            // hide schedule below is a harmless no-op either way.
            if notch_command {
                pill::emit_notch_phase(app, "error", Some(pill_msg));
            } else {
                pill::emit_state(app, PillState::Error, Some(pill_msg));
            }
            self.schedule_pill_hide_unless_taken(app, new_sid, 3000);
            emit_sessions_changed(app, self);
            return BeginOutcome::AudioError(e.to_string());
        }

        // Audio is recording — attenuate other audio (music/video) per
        // the user's setting. The primary restore is anchored to
        // mic-release in `end_recording`; `cancel_session` and
        // `PipelineGuard::drop` are idempotent backstops. Best-effort and
        // fully fail-open: a failure here never blocks the session. Placed
        // after the audio-start success so the error early-return above
        // can't leave audio ducked with no recording to restore it. The
        // playback state was probed above (before mic-open) to avoid a
        // combined-Bluetooth-device false-positive that could let pause
        // start music from silence.
        crate::audio_attenuation::begin(&prefs.audio_attenuation, was_playing);

        spawn_max_record_guard(app.clone(), self.clone(), session.clone(), MAX_RECORD_SECS);
        spawn_audio_level_sampler(app.clone(), self.clone(), session.clone());
        spawn_speculative_decoder(app.clone(), self.clone(), session.clone());

        BeginOutcome::Started(new_sid)
    }

    /// Called when the PTT hotkey toggles off (or hands-free fires
    /// the second press). Stops audio for the active recording and
    /// dispatches the post-recording pipeline as a tokio task. The
    /// next `begin` may run while this pipeline is still running.
    pub fn end_recording(self: &Arc<Self>, app: AppHandle<Wry>) {
        let sid = match self.active_recording.lock().take() {
            Some(s) => s,
            None => {
                log::debug!("end_recording called without an active recording; ignored");
                return;
            }
        };

        // Release the cpal input stream RIGHT NOW, on the calling
        // thread, before any further bookkeeping or task spawn. Without
        // this the mic stayed hot for the tokio scheduling latency of
        // the spawned task plus any pending `SnapshotWav` from the
        // speculative decoder — on top of macOS Control Center's own
        // ~few-second mic-indicator release delay. Calling `stop` here
        // makes that perceived latency a function of macOS alone.
        // Synchronous and short: the audio thread only has to drop the
        // stream; the heavier WAV encode runs in the spawned task below
        // via `encode_recorded`.
        if let Err(e) = self.audio.stop() {
            log::warn!("end_recording: audio.stop failed: {e}");
        }

        // Primary restore: un-attenuate other audio the moment the mic is
        // released — i.e. when the user stops dictating ("revert when
        // untriggered") — rather than waiting for the detached pipeline to
        // unwind in `PipelineGuard::drop`. This keeps `begin`↔`restore`
        // symmetric with the serialized recording lifecycle, so a new
        // dictation that `begin`s while the previous pipeline is still
        // transcribing/polishing can't race two snapshots over the one
        // global slot. Idempotent (take-based): the guard-drop and
        // cancel backstops below safely no-op if this already ran.
        crate::audio_attenuation::restore();

        // Escape stays grabbed past the end of recording — the user can
        // still cancel during the transcribe / polish stages. Disarm is
        // owned by `PipelineGuard::Drop` (covers success, all error
        // returns, the silent / too-short / silent-buffer skips, and
        // both cancel paths via the guard's tail call); `cancel_session`
        // also disarms eagerly on its foreground branch so the next
        // hotkey press doesn't have to wait for the pipeline to unwind.

        let session = match self.sessions.lock().get(&sid).cloned() {
            Some(s) => s,
            None => {
                // active_recording was cleared and the audio stream
                // dropped above, so this defensive branch can return
                // cleanly without leaking the mic. The stashed sample
                // buffer goes with the next `Start` (or the next
                // `encode_recorded`, whichever fires first).
                log::warn!("end_recording: session {sid} missing from map");
                return;
            }
        };

        let started = session.started_at;
        let too_short = started.elapsed().as_millis() < MIN_RECORD_MS;
        log::info!("■ end recording (sid={sid})");

        let manager = self.clone();
        let app_clone = app.clone();

        tauri::async_runtime::spawn(async move {
            let wav_result: Result<Option<Vec<u8>>, String> = if too_short {
                // Drain the stashed buffer so the next start()'s
                // `stashed = None` cleanup doesn't have to (cheap
                // either way — it's just a take()).
                let _ = manager.audio.encode_recorded();
                Ok(None)
            } else {
                match manager.audio.encode_recorded() {
                    Ok(buf) => Ok(Some(buf)),
                    Err(e) => Err(format!("{e}")),
                }
            };

            run_pipeline(app_clone, manager, session, wav_result).await;
        });
    }

    /// Entry point for the cancel hotkey (default Escape) while a session
    /// is in flight. With `require_double` off — the default — this is a
    /// straight pass-through to [`cancel_foreground`]. With it on, the
    /// *live recording* is guarded by a double-press: the first press
    /// arms a short confirmation window and surfaces a pill hint, and only
    /// a second press inside [`CANCEL_DOUBLE_PRESS_WINDOW`] actually
    /// cancels. This stops a stray Escape (e.g. dismissing a screenshot
    /// mid-dictation) from tearing the session down. The guard is scoped
    /// to recording only: once the user has stopped speaking and the
    /// pipeline is transcribing / polishing, a single press still aborts
    /// (the accidental-press risk is specific to the dictation phase, and
    /// `is_recording()` is false by then).
    pub fn request_cancel_foreground(
        self: &Arc<Self>,
        app: &AppHandle<Wry>,
        require_double: bool,
    ) {
        if !require_double || !self.is_recording() {
            self.cancel_foreground(app);
            return;
        }

        let fire = {
            let mut slot = self.cancel_first_press_at.lock();
            let (next, fire) =
                double_press_decision(*slot, Instant::now(), CANCEL_DOUBLE_PRESS_WINDOW);
            *slot = next;
            fire
        };

        if fire {
            self.cancel_foreground(app);
        } else {
            log::debug!("cancel: first press armed; awaiting confirm within window");
            // Name the actually-bound cancel key so the hint reads
            // "Press F13 again…" when the user has remapped it off Escape.
            let key = crate::settings::load(app).cancel_hotkey;
            crate::pill::emit_cancel_hint(app, CANCEL_DOUBLE_PRESS_WINDOW_MS, key);
        }
    }

    /// Cancel whichever session is the current foreground recording.
    /// Mapped to the Escape hotkey while a recording is in flight.
    pub fn cancel_foreground(self: &Arc<Self>, app: &AppHandle<Wry>) {
        let sid = match *self.active_recording.lock() {
            Some(s) => s,
            None => {
                // No active recording — but a session may still be
                // polishing in the background. Fall back to cancelling
                // any in-flight session so Escape isn't a no-op.
                let last = self
                    .sessions
                    .lock()
                    .values()
                    .map(|s| s.id)
                    .max();
                match last {
                    Some(s) => s,
                    None => {
                        log::debug!("cancel_foreground: no in-flight sessions");
                        return;
                    }
                }
            }
        };
        self.cancel_session(sid, app);
    }

    /// Cancel a specific session (popover X / cancel hotkey path).
    pub fn cancel_session(self: &Arc<Self>, id: SessionId, app: &AppHandle<Wry>) {
        // Release the cpal input stream FIRST if this cancel targets
        // the foreground recording, BEFORE the sessions-map lookup
        // below. That ordering means even a defensive "session missing
        // from map" early return can't leak the mic — every code path
        // out of `cancel_session` that observed `active_recording ==
        // Some(id)` has already dropped the stream by the time it
        // returns. The stream drop is cheap (audio thread just calls
        // `AudioOutputUnitStop` + `Dispose`); the WAV encode is
        // skipped entirely on the cancel path.
        let was_foreground = {
            let mut active = self.active_recording.lock();
            if active.as_ref() == Some(&id) {
                *active = None;
                true
            } else {
                false
            }
        };
        if was_foreground {
            if let Err(e) = self.audio.stop() {
                log::warn!("cancel_session({id}): audio.stop failed: {e}");
            }
            crate::hotkey::disarm_cancel(app);
            // Backstop restore for a cancel that aborts the recording
            // before `end_recording` runs its primary restore (and before
            // `run_pipeline` / `PipelineGuard` is ever constructed).
            // Idempotent: whichever restore runs first take()s the
            // snapshot, the rest no-op.
            crate::audio_attenuation::restore();
        }

        let session = match self.sessions.lock().get(&id).cloned() {
            Some(s) => s,
            None => {
                log::debug!("cancel_session({id}): not found (already finalized?)");
                return;
            }
        };
        session.cancel.store(true, Ordering::SeqCst);
        // Release the busy gate immediately so the next press is
        // accepted promptly. Idempotent — the SessionState's eventual
        // Drop is a no-op once released.
        if let Some(g) = session.busy_guard.lock().take() {
            g.release();
        }

        // A notch-voice session owns no pill — cancelling it drops the
        // notch back to idle rather than flashing the pill's Canceled
        // label. Escape IS armed for notch sessions (begin arms the grab
        // regardless), so this path is reachable when the user cancels a
        // ⌘L recording.
        if session.notch_command {
            pill::emit_notch_phase(app, "idle", None);
            self.set_stage(app, id, Stage::Canceled, None, false);
            return;
        }

        // Pill cleanup runs for both foreground (cancel during recording)
        // and background (cancel during transcribe / polish) cancels so
        // the pill always disappears a beat after Escape. Without the
        // hide schedule on the non-foreground path, the Canceled label
        // stayed on screen indefinitely — the pipeline's own cancel
        // branch paints the label and clears pill_owner, but nothing
        // schedules the OS window hide. set_wide(false) is also moved
        // out so a streaming-wide polish snaps to compact in the same
        // tick the Canceled label is painted, instead of waiting ~50 ms
        // for the pipeline's wait_for_cancel poll to fire.
        pill::set_wide(app, false);
        pill::emit_state(app, PillState::Canceled, None);
        self.schedule_pill_hide_unless_taken(app, id, 600);

        self.set_stage(app, id, Stage::Canceled, None, false);
    }

    /// Update a session's stage and emit pill state (only for the
    /// foreground session) plus a sessions:changed snapshot.
    pub fn set_stage<R: Runtime>(
        self: &Arc<Self>,
        app: &AppHandle<R>,
        id: SessionId,
        stage: Stage,
        message: Option<String>,
        warning: bool,
    ) {
        let session_present = {
            let map = self.sessions.lock();
            match map.get(&id) {
                Some(s) => {
                    *s.stage.lock() = stage;
                    if warning {
                        *s.warning.lock() = true;
                    }
                    true
                }
                None => false,
            }
        };
        if !session_present {
            return;
        }

        // Hold the pill_owner lock for the full read → decide → emit
        // span. Without this a concurrent begin() could replace the
        // owner between the check and the emit and we'd publish a
        // Done/Error pill state that overwrites the new session's
        // pill. The emit itself is just a Tauri event publish (cheap),
        // and the only thing the lock protects is a single Option<u64>,
        // so holding it across the emit is the simpler and safer path
        // than racing two sub-operations.
        {
            let owner_guard = self.pill_owner.lock();
            let owner_snapshot: Option<SessionId> = *owner_guard;
            let is_owner = owner_snapshot == Some(id);
            if is_owner {
                // This session owns the pill UI right now — drive both
                // the visible state and the background-count badge from
                // it. pill_owner persists across the end-of-recording
                // → polish boundary so the pill follows the most-recent
                // recording through its full lifecycle.
                let pill_state = match stage {
                    Stage::Listening => Some(PillState::Listening),
                    Stage::Transcribing => Some(PillState::Transcribing),
                    Stage::Polishing => Some(PillState::Polishing),
                    Stage::Done => Some(PillState::Done),
                    Stage::Error => Some(PillState::Error),
                    Stage::Canceled => Some(PillState::Canceled),
                };
                if let Some(s) = pill_state {
                    let bg = self.background_count_excluding_pill_owner_locked(owner_snapshot);
                    if warning {
                        pill::emit_state_warning_with_bg(app, s, message.clone(), bg);
                    } else {
                        pill::emit_state_with_bg(app, s, message.clone(), bg);
                    }
                }
            } else {
                // Background session — refresh the pill's background-count
                // badge so the user sees the queue grow/shrink. Only
                // re-emit when there's a foreground pill owner, otherwise
                // there's no pill to badge.
                if let Some(owner_id) = owner_snapshot {
                    if let Some(owner) = self.sessions.lock().get(&owner_id).cloned() {
                        let owner_pill = match owner.current_stage() {
                            Stage::Listening => PillState::Listening,
                            Stage::Transcribing => PillState::Transcribing,
                            Stage::Polishing => PillState::Polishing,
                            Stage::Done => PillState::Done,
                            Stage::Error => PillState::Error,
                            Stage::Canceled => PillState::Canceled,
                        };
                        let bg = self
                            .background_count_excluding_pill_owner_locked(owner_snapshot);
                        pill::emit_state_with_bg(app, owner_pill, None, bg);
                    }
                }
            }
            drop(owner_guard);
        }

        emit_sessions_changed(app, self);

        // Schedule terminal-state cleanup so the popover briefly
        // shows the terminal stage before the row folds away.
        if matches!(stage, Stage::Done | Stage::Error | Stage::Canceled) {
            // Wake the max-record guard so it drops its captured
            // Arc<SessionManager> + Arc<SessionState> + AppHandle
            // immediately instead of holding them for the rest of
            // the 30-minute safety cap. Look up the session under
            // the same lock cycle the cleanup uses.
            if let Some(s) = self.sessions.lock().get(&id).cloned() {
                s.mark_completed();
            }
            // Release pill ownership so the next begin() doesn't
            // inherit a stale pointer; the cleanup task will also
            // remove the session from the map after the linger.
            {
                let mut owner = self.pill_owner.lock();
                if *owner == Some(id) {
                    *owner = None;
                }
            }
            self.spawn_terminal_cleanup(app, id);
            // Error has no other code path that schedules the pill
            // hide — Done is hidden by `run_pipeline`'s tail, and
            // Canceled by `cancel_session`. Without this, Error
            // pills (silence, audio failure, STT failure, etc.) stay
            // on screen until the next dictation starts.
            if matches!(stage, Stage::Error) {
                self.schedule_pill_hide_unless_taken(app, id, ERROR_LINGER_MS);
            }
        }
    }

    /// Background-session count for the pill's queue badge.
    /// Counts every in-flight session except the current pill owner —
    /// matches the user-visible "this many other sessions are queued
    /// behind the one you're watching" mental model.
    #[allow(dead_code)]
    fn background_count_excluding_pill_owner(&self) -> u32 {
        let owner = *self.pill_owner.lock();
        self.background_count_excluding_pill_owner_locked(owner)
    }

    /// Same as [`background_count_excluding_pill_owner`] but the
    /// caller passes the owner snapshot — used inside `set_stage`
    /// where the pill_owner lock is already held. Calling the public
    /// helper would re-lock and deadlock with the held guard.
    fn background_count_excluding_pill_owner_locked(
        &self,
        owner: Option<SessionId>,
    ) -> u32 {
        let map = self.sessions.lock();
        let total = map.len();
        let bg = if let Some(owner_id) = owner {
            if map.contains_key(&owner_id) {
                total.saturating_sub(1)
            } else {
                total
            }
        } else {
            total
        };
        bg as u32
    }

    fn spawn_terminal_cleanup<R: Runtime>(
        self: &Arc<Self>,
        app: &AppHandle<R>,
        id: SessionId,
    ) {
        let manager = self.clone();
        let app_handle = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(TERMINAL_LINGER_MS)).await;
            let removed = manager.sessions.lock().remove(&id);
            if removed.is_some() {
                emit_sessions_changed(&app_handle, &manager);
            }
        });
    }

    /// Re-emit the pill owner's CURRENT stage after `ms` ms. Used by the
    /// `PipelineBusy` branch of `begin` to restore the live pipeline
    /// stage on the pill once the busy toast clears. Without it the
    /// frontend's toast handler leaves `state="saturated"` until the
    /// next pipeline stage transition fires a `pill:state` event —
    /// which during a long polishing run can be many seconds away, so
    /// the warning sign visibly outlives the busy event.
    ///
    /// The fired stage is read FRESH at fire time, not snapshotted at
    /// schedule time: if the pipeline has progressed transcribing →
    /// polishing while the toast was up, we restore to the live stage,
    /// not the one the user interrupted. Sessions that have already
    /// reached a terminal state (Done/Error/Canceled) are skipped — the
    /// terminal emit already painted the right treatment with the right
    /// message, and re-emitting from here would clobber that message.
    fn schedule_pill_state_restore<R: Runtime>(
        self: &Arc<Self>,
        app: &AppHandle<R>,
        ms: u64,
    ) {
        let manager = self.clone();
        let app_clone = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            // Hold pill_owner across the read-and-emit span, mirroring
            // set_stage's pattern (~line 730). Without this, a
            // concurrent set_stage(Done) for the same session could
            // interleave between our stage read and our emit, leaving
            // the frontend painted with a stale "polishing" treatment
            // on top of a Done that just emitted.
            let owner_guard = manager.pill_owner.lock();
            let owner_id = match *owner_guard {
                Some(id) => id,
                None => return,
            };
            let session = match manager.sessions.lock().get(&owner_id).cloned() {
                Some(s) => s,
                None => return,
            };
            let stage = session.current_stage();
            if !should_restore_pill_state(stage) {
                return;
            }
            let warning = *session.warning.lock();
            let pill_state = match stage {
                Stage::Listening => PillState::Listening,
                Stage::Transcribing => PillState::Transcribing,
                Stage::Polishing => PillState::Polishing,
                Stage::Done => PillState::Done,
                Stage::Error => PillState::Error,
                Stage::Canceled => PillState::Canceled,
            };
            let bg = manager
                .background_count_excluding_pill_owner_locked(Some(owner_id));
            if warning {
                pill::emit_state_warning_with_bg(&app_clone, pill_state, None, bg);
            } else {
                pill::emit_state_with_bg(&app_clone, pill_state, None, bg);
            }
            drop(owner_guard);
        });
    }

    /// Delayed pill-hide that respects ownership. Sleeps `ms`, then hides
    /// the pill IFF `pill_owner` is `None` or still equal to
    /// `expected_owner`. A newer session that has claimed the pill in
    /// the meantime is left alone — its own pipeline tail will hide the
    /// pill when it's done. Replaces the previous unconditional
    /// `pill::hide` schedules everywhere a fresh recording can
    /// legitimately race the linger window; without this guard the
    /// previous session's stale timer hides the pill the user just
    /// brought up by pressing the hotkey again.
    fn schedule_pill_hide_unless_taken<R: Runtime>(
        self: &Arc<Self>,
        app: &AppHandle<R>,
        expected_owner: SessionId,
        ms: u64,
    ) {
        let manager = self.clone();
        let app_clone = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            // Bounded race: a `begin()` that lands between the lock
            // release here and the `pill::hide` below would lose its
            // pill briefly, but that begin's own `pill::show` +
            // `emit_state(Listening)` re-asserts within the next event
            // loop tick. Holding the lock across `pill::hide` is worse
            // (`app.emit_to` does real work).
            let owner = *manager.pill_owner.lock();
            if should_hide_pill(owner, expected_owner) {
                pill::hide(&app_clone);
            }
        });
    }

    /// Deliver the polished transcript to the user. When `auto_paste`
    /// is false this is the legacy "copy and let the user press ⌘V"
    /// path; when true, we capture (or inherit) the user's prior
    /// clipboard text, write the transcript, post a synthetic ⌘V at
    /// the HID event tap, and schedule a delayed restore of that
    /// prior text so the clipboard isn't permanently clobbered.
    ///
    /// **Chained pipelines.** If a second dictation lands before the
    /// previous restore fires (250 ms window), the new call cancels
    /// the previous restore and inherits its captured `prior_text` —
    /// otherwise the new pipeline would snapshot the previous
    /// transcript as if it were the user's clipboard, and the
    /// "restore" would write the previous transcript back, losing the
    /// user's true pre-chain content. See [`RestoreToken`].
    ///
    /// Returns a [`DeliveryOutcome`] describing what actually
    /// happened, used by the caller to pick the right Done-stage pill
    /// message. A `Pasted` outcome guarantees the clipboard was
    /// written; the restore (if any) runs asynchronously on a fresh
    /// thread.
    pub fn deliver_transcript(
        self: &Arc<Self>,
        id: SessionId,
        text: &str,
        auto_paste: bool,
    ) -> Result<DeliveryOutcome, String> {
        // Phase 1 — claim the previous chain's prior (cancelling its
        // restore so its thread won't fire) or snapshot fresh, then
        // write polished. Both locks held so a concurrent invocation
        // can't slip in between.
        let prior_text: Option<String> = {
            let mut pending = self.pending_restore.lock();
            let _clip = self.clipboard_lock.lock();
            let prior = if auto_paste {
                match pending.take() {
                    Some(prev) => {
                        prev.cancel.store(true, Ordering::SeqCst);
                        Some(prev.prior_text)
                    }
                    None => inject::snapshot_clipboard_text(),
                }
            } else {
                None
            };
            inject::copy_to_clipboard(text).map_err(|e| e.to_string())?;
            prior
        };

        if !auto_paste {
            return Ok(DeliveryOutcome::CopiedOnly);
        }

        // Phase 2 — ⌘V at the HID tap. Outside both locks since it
        // doesn't touch the pasteboard.
        #[cfg(target_os = "macos")]
        let paste_result = crate::mac_paste::post_cmd_v().map_err(|e| e.to_string());
        #[cfg(target_os = "windows")]
        let paste_result = crate::win_paste::post_ctrl_v().map_err(|e| e.to_string());
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let paste_result: Result<(), String> = Err("auto-paste unsupported on this OS".into());

        match paste_result {
            Ok(()) => {
                // Phase 3 — register a new pending restore and spawn
                // the thread. 250 ms is enough for Slack, Notion, VS
                // Code, browsers, and Terminal to consume the paste;
                // a clipboard manager polling every few hundred ms
                // can still catch the transient state, but the
                // user-visible end state is their prior clipboard.
                if let Some(prior) = prior_text {
                    let cancel = Arc::new(AtomicBool::new(false));
                    *self.pending_restore.lock() = Some(RestoreToken {
                        prior_text: prior.clone(),
                        cancel: cancel.clone(),
                    });
                    let manager = self.clone();
                    if let Err(e) = std::thread::Builder::new()
                        .name("vox-clipboard-restore".into())
                        .spawn(move || {
                            std::thread::sleep(Duration::from_millis(250));
                            if cancel.load(Ordering::SeqCst) {
                                return;
                            }
                            let mut pending = manager.pending_restore.lock();
                            // Re-check after acquiring the lock — a
                            // newer pipeline may have flipped cancel
                            // while we were waiting.
                            if cancel.load(Ordering::SeqCst) {
                                return;
                            }
                            let _clip = manager.clipboard_lock.lock();
                            if let Err(e) = inject::copy_to_clipboard(&prior) {
                                log::warn!(
                                    "clipboard restore (sid={id}) failed: {e}"
                                );
                            }
                            *pending = None;
                        })
                    {
                        log::warn!(
                            "failed to spawn clipboard-restore thread (sid={id}): {e}"
                        );
                    }
                }
                Ok(DeliveryOutcome::Pasted)
            }
            Err(reason) => {
                log::warn!("auto-paste (sid={id}) failed: {reason}");
                Ok(DeliveryOutcome::PasteFailedCopiedOnly)
            }
        }
    }

    #[allow(dead_code)]
    pub fn mark_canceled<R: Runtime>(self: &Arc<Self>, app: &AppHandle<R>, id: SessionId) {
        self.set_stage(app, id, Stage::Canceled, None, false);
    }
}

fn emit_sessions_changed<R: Runtime>(app: &AppHandle<R>, manager: &SessionManager) {
    let snap = manager.snapshot();
    let _ = app.emit("sessions:changed", &snap);
}

/// Owner-check predicate used by [`SessionManager::schedule_pill_hide_unless_taken`].
/// Hide the pill only when no one owns it, or when the previous owner
/// (the one who scheduled the hide) is still the owner — never trample
/// a newer session that has already claimed the pill.
fn should_hide_pill(owner: Option<SessionId>, expected: SessionId) -> bool {
    owner.is_none() || owner == Some(expected)
}

/// Predicate used by [`SessionManager::schedule_pill_state_restore`] to
/// decide whether to re-emit the live stage. Terminal stages already
/// painted the right treatment (with the right message) when set_stage
/// fired them; re-emitting from the restore path would clobber that
/// message with a `None`. Restore only the in-flight stages.
fn should_restore_pill_state(stage: Stage) -> bool {
    !matches!(stage, Stage::Done | Stage::Error | Stage::Canceled)
}

/// Pipeline body. Lifted out of the old `end_recording` async block —
/// reads everything from the per-session snapshot rather than the
/// global settings store, so a mid-flight settings change cannot pin
/// a different prompt to a transcript that's already been recorded.
async fn run_pipeline(
    app: AppHandle<Wry>,
    manager: Arc<SessionManager>,
    session: Arc<SessionState>,
    wav_result: Result<Option<Vec<u8>>, String>,
) {
    let id = session.id;
    // A notch-voice (⌘L) session never touches the pill — its UI lives in
    // the notch. Captured once here so every bail-out path below can route
    // its surface to the notch (`notch:listening`) and skip the pill
    // hide/show calls that would otherwise dismiss or flash a pill the
    // user never saw (and that another in-flight session may own).
    let notch_command = session.notch_command;

    // Guard so the cancel grab disarm + map cleanup happens on every
    // exit path. We use an explicit RAII guard rather than scattering
    // returns because there are many bail-out points.
    //
    // Drop also signals pipeline completion to the max-record guard
    // task so it can drop its captured Arcs immediately. This covers
    // paths that skip set_stage entirely (silent / empty-buffer
    // skips).
    struct PipelineGuard {
        finalised: bool,
        session: Arc<SessionState>,
        app: AppHandle<Wry>,
    }
    impl PipelineGuard {
        fn new(session: Arc<SessionState>, app: AppHandle<Wry>) -> Self {
            Self { finalised: false, session, app }
        }
        fn finalise(&mut self) {
            self.finalised = true;
        }
    }
    impl Drop for PipelineGuard {
        fn drop(&mut self) {
            // Idempotent — set_stage's terminal-state path may have
            // already marked completion. Either order is fine.
            self.session.mark_completed();
            // Release the busy gate on every pipeline exit path
            // (success, STT error, polish error, cancel). Idempotent —
            // cancel_session and the audio-rollback path may have
            // already released it. Done before the Arc<SessionState>
            // drops at end of scope so the next hotkey press is
            // accepted with no extra latency.
            if let Some(g) = self.session.busy_guard.lock().take() {
                g.release();
            }
            // Tear down the OS Escape grab now that the pipeline has
            // reached a terminal state. Idempotent — cancel_session may
            // already have called disarm. Doing this here (instead of
            // in end_recording, where it lived previously) is what lets
            // Escape interrupt the transcription / polishing stages —
            // the grab survives from begin() through to pipeline
            // finalisation.
            crate::hotkey::disarm_cancel(&self.app);
            // Backstop restore for any audio we attenuated at recording
            // start. The primary restore now fires at mic-release in
            // `end_recording`; this guard-drop covers the only paths that
            // skip it — a pipeline-phase panic that unwinds through Drop.
            // Idempotent: if the mic-release (or a foreground cancel)
            // already restored, this take()s an empty snapshot and no-ops.
            // Panic-safe — runs from Drop.
            crate::audio_attenuation::restore();
        }
    }
    let mut _guard = PipelineGuard::new(session.clone(), app.clone());

    if session.is_canceled() {
        log::info!("session {id} canceled before pipeline ran");
        if notch_command {
            // A cancelled notch session leaves the list untouched — drop
            // the notch back to idle rather than flashing a pill.
            pill::emit_notch_phase(&app, "idle", None);
        }
        manager.set_stage(&app, id, Stage::Canceled, None, false);
        _guard.finalise();
        return;
    }

    let wav = match wav_result {
        Ok(Some(buf)) => buf,
        Ok(None) => {
            log::info!("session {id} too short, skipping STT");
            if notch_command {
                // Silent skip → notch idle; never hide the pill (which a
                // concurrent normal session may own).
                pill::emit_notch_phase(&app, "idle", None);
            } else {
                // Silent skip: pill goes back to hidden.
                pill::hide(&app);
            }
            manager.sessions.lock().remove(&id);
            emit_sessions_changed(&app, &manager);
            _guard.finalise();
            return;
        }
        Err(e) => {
            log::error!("audio stop/encode failed: {e}");
            *session.error.lock() = Some(e);
            if notch_command {
                pill::emit_notch_phase(&app, "error", Some("Audio error — try again".into()));
            }
            manager.set_stage(
                &app,
                id,
                Stage::Error,
                Some("Audio error — try again".into()),
                false,
            );
            _guard.finalise();
            return;
        }
    };

    if wav.len() < 44 + 16_000 * 2 / 4 {
        log::info!("recording too short ({} bytes), skipping STT", wav.len());
        if notch_command {
            pill::emit_notch_phase(&app, "idle", None);
        } else {
            pill::hide(&app);
        }
        manager.sessions.lock().remove(&id);
        emit_sessions_changed(&app, &manager);
        _guard.finalise();
        return;
    }

    if wav_peak_pcm16(&wav) < SILENCE_PEAK_THRESHOLD {
        log::info!("session {id} silent buffer; skipping STT");
        if notch_command {
            pill::emit_notch_phase(&app, "error", Some("No audio detected — check your mic.".into()));
        }
        manager.set_stage(
            &app,
            id,
            Stage::Error,
            Some("No audio detected — check your mic.".into()),
            false,
        );
        _guard.finalise();
        return;
    }

    let registry = match app.try_state::<Registry>() {
        Some(r) => r.inner().clone(),
        None => {
            log::error!("engine registry missing — onboarding never finished?");
            if notch_command {
                pill::emit_notch_phase(&app, "error", Some("Engines not initialised — restart Vox.".into()));
            }
            manager.set_stage(
                &app,
                id,
                Stage::Error,
                Some("Engines not initialised — restart Vox.".into()),
                false,
            );
            _guard.finalise();
            return;
        }
    };

    let stt_engine = match registry.stt(&app, &session.stt_engine_id, &session.stt_model_id) {
        Some(e) => e,
        None => {
            if notch_command {
                pill::emit_notch_phase(
                    &app,
                    "error",
                    Some("STT engine not configured — open Settings → Models".into()),
                );
            }
            manager.set_stage(
                &app,
                id,
                Stage::Error,
                Some("STT engine not configured — open Settings → Models".into()),
                false,
            );
            _guard.finalise();
            return;
        }
    };

    let llm_engine = if session.polishing_enabled {
        registry.llm(&app, &session.llm_engine_id, &session.llm_model_id)
    } else {
        registry.llm(&app, "none", "")
    };

    // Streaming mode is opt-in (Settings → General + tray popover).
    // The setting was captured once at session-begin time on
    // `SessionState::streaming_enabled` so the speculative decoder
    // task and the final pipeline pass can't disagree if the user
    // toggles the toggle mid-utterance. The pill-owner check is
    // recomputed at every emit site below, so a session that loses
    // ownership (e.g. a newer recording started) stops painting into
    // someone else's pill mid-stream.
    //
    // `live_transcript_enabled` is the listening-phase opt-in; if
    // the user asked for a live caption they want it through
    // completion, so either flag opens the post-release transcript
    // stream.
    let streaming_enabled = session.streaming_enabled || session.live_transcript_enabled;
    let owns_pill = manager.owns_pill_emit(id);
    // Closure used at every emit site so an in-flight session that
    // gets pre-empted by a newer recording stops writing to the pill.
    // `streaming_enabled` is the user toggle, `owns_pill_emit(id)` is
    // the live ownership predicate.
    let stream_to_pill = || streaming_enabled && manager.owns_pill_emit(id);

    // Widen the pill before we start emitting cumulative segment text
    // so the layout grow doesn't kick in while the first words appear.
    if stream_to_pill() {
        pill::set_wide(&app, true);
    }

    manager.set_stage(&app, id, Stage::Transcribing, None, false);

    let language: Option<&str> = if session.language != "auto" {
        Some(session.language.as_str())
    } else {
        None
    };

    let stt_callback: Option<crate::engines::SegmentCallback> = if stream_to_pill() {
        let app_for_cb = app.clone();
        let sid = id;
        // Re-check ownership inside the callback because Whisper
        // segments can keep arriving after a newer session has stolen
        // the pill — without this gate the partials would render in
        // the new session's pill.
        let manager_for_cb = manager.clone();
        Some(Box::new(move |text: &str| {
            if !manager_for_cb.owns_pill_emit(sid) {
                return;
            }
            pill::emit_transcript_stream(&app_for_cb, sid, text.to_string(), false);
        }))
    } else {
        None
    };

    let transcript = {
        // Block on any in-flight speculative decode before starting the
        // authoritative pass — both share the same engine instance
        // (WhisperContext / Parakeet AsrManager) and neither is safe to
        // drive concurrently, so the lock serialises them.
        //
        // For whisper-local this wait also *pays for itself*: the
        // speculative pass primes the WhisperContext cache, so by the
        // time the lock is free the only audio left to decode is the
        // sliver captured after the last speculative snapshot.
        //
        // For parakeet-local there's no such cache — the final pass
        // re-decodes the whole buffer regardless — so if a live-caption
        // pass is in flight at release we simply wait it out (bounded by
        // one Parakeet decode; the length-aware cadence above keeps that
        // bounded on long takes). Acceptable for an opt-in caption; the
        // alternative (cancelling an uninterruptible in-flight decode)
        // isn't available.
        let speculative_lock = session.speculative_decode.clone();
        let dict = session.dictionary.clone();
        let stt_engine_for_call = stt_engine.clone();
        let session_for_cancel = session.clone();
        let language_owned: Option<String> = language.map(|s| s.to_string());
        let wav_for_stt = wav;
        let stt_fut = async move {
            let _g = tokio::select! {
                biased;
                () = wait_for_cancel(session_for_cancel.clone()) => {
                    return Err(anyhow::anyhow!("canceled before final STT could start"));
                }
                guard = speculative_lock.lock_owned() => guard,
            };
            stt_engine_for_call
                .transcribe(&wav_for_stt, language_owned.as_deref(), &dict, stt_callback)
                .await
        };
        tokio::select! {
            biased;
            () = wait_for_cancel(session.clone()) => {
                log::info!("STT canceled; dropping request (sid={id})");
                if stream_to_pill() {
                    pill::set_wide(&app, false);
                }
                if notch_command {
                    pill::emit_notch_phase(&app, "idle", None);
                }
                manager.set_stage(&app, id, Stage::Canceled, None, false);
                _guard.finalise();
                return;
            }
            result = stt_fut => match result {
                Ok(text) if !text.trim().is_empty() => text,
                Ok(_) => {
                    log::info!("empty transcript, nothing to copy (sid={id})");
                    if stream_to_pill() {
                        pill::set_wide(&app, false);
                    }
                    if notch_command {
                        // Nothing was said — drop the notch back to idle
                        // rather than hiding a pill that was never shown.
                        pill::emit_notch_phase(&app, "idle", None);
                    } else {
                        pill::hide(&app);
                    }
                    manager.sessions.lock().remove(&id);
                    emit_sessions_changed(&app, &manager);
                    _guard.finalise();
                    return;
                }
                Err(e) => {
                    log::error!("STT failed (sid={id}): {e}");
                    if stream_to_pill() {
                        pill::set_wide(&app, false);
                    }
                    if notch_command {
                        pill::emit_notch_phase(
                            &app,
                            "error",
                            Some(friendly_stt_error(&e, &session.stt_engine_id)),
                        );
                    }
                    *session.error.lock() = Some(format!("{e}"));
                    manager.set_stage(
                        &app,
                        id,
                        Stage::Error,
                        Some(friendly_stt_error(&e, &session.stt_engine_id)),
                        false,
                    );
                    _guard.finalise();
                    return;
                }
            }
        }
    };

    *session.transcript.lock() = Some(transcript.clone());

    // Final cumulative transcript — flushes the pill's rate limiter.
    if stream_to_pill() {
        pill::emit_transcript_stream(&app, id, transcript.clone(), true);
    }

    if session.is_canceled() {
        log::info!("post-STT cancel (sid={id})");
        if stream_to_pill() {
            pill::set_wide(&app, false);
        }
        if notch_command {
            pill::emit_notch_phase(&app, "idle", None);
        }
        manager.set_stage(&app, id, Stage::Canceled, None, false);
        _guard.finalise();
        return;
    }

    // Notch-command fork: this session was started via the notch voice
    // hotkey (or with the `notch-list` mode active). Interpret the
    // transcript as a list command and apply the resulting ops to the
    // Notch List — NO polish, NO clipboard delivery. Fail-open on any
    // error (the list stays unchanged; the outcome surfaces in the notch
    // via `notch:listening`, and any list change rides `notch-list:changed`).
    // The pill is never involved: a notch session owns no pill, so
    // `stream_to_pill()` and `owns_pill` are both false here — there is
    // nothing to widen or hide. `interpret_and_apply_notch` emits every
    // notch phase (compiling → idle/error) itself.
    if notch_command {
        interpret_and_apply_notch(&app, &manager, &session, &transcript, llm_engine).await;
        _guard.finalise();
        return;
    }

    let ctx_prompt = with_user_context(
        &session.mode.system_prompt,
        &session.context,
        session.mode.context_enabled,
    );
    let mode_system_prompt = with_language_directive(&ctx_prompt, &session.language);
    let merged_dictionary = merge_dictionaries(&session.dictionary, &session.mode.dictionary);

    let mut polish_warning: Option<&'static str> = None;
    let polish_engine_active = llm_engine
        .as_ref()
        .map(|e| e.id() != "none")
        .unwrap_or(false);

    let cleaned = match llm_engine {
        Some(engine) if engine.id() != "none" => {
            manager.set_stage(&app, id, Stage::Polishing, None, false);
            let llm_fut: std::pin::Pin<
                Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send>,
            > = if stream_to_pill() {
                let app_for_cb = app.clone();
                let sid = id;
                // Same ownership re-check as the STT segment callback:
                // a polish chunk arriving after a newer session stole
                // the pill must NOT paint into the new pill.
                let manager_for_cb = manager.clone();
                let cb: crate::engines::ChunkCallback = Box::new(move |text: &str| {
                    if !manager_for_cb.owns_pill_emit(sid) {
                        return;
                    }
                    pill::emit_polish_stream(&app_for_cb, sid, text.to_string(), false);
                });
                let engine = engine.clone();
                let sys = mode_system_prompt.clone();
                let dict = merged_dictionary.clone();
                let t = transcript.clone();
                let thinking = session.enable_thinking;
                Box::pin(async move {
                    engine.cleanup_stream(&t, &sys, &dict, thinking, cb).await
                })
            } else {
                let engine = engine.clone();
                let sys = mode_system_prompt.clone();
                let dict = merged_dictionary.clone();
                let t = transcript.clone();
                let thinking = session.enable_thinking;
                Box::pin(async move { engine.cleanup(&t, &sys, &dict, thinking).await })
            };
            tokio::select! {
                biased;
                () = wait_for_cancel(session.clone()) => {
                    log::info!("LLM canceled (sid={id})");
                    if stream_to_pill() {
                        pill::set_wide(&app, false);
                    }
                    manager.set_stage(&app, id, Stage::Canceled, None, false);
                    _guard.finalise();
                    return;
                }
                result = llm_fut => match result {
                    Ok(t) if !t.trim().is_empty() => t,
                    Ok(_) => {
                        log::info!(
                            "LLM polish returned empty content; falling back to raw transcript (sid={id})"
                        );
                        polish_warning = Some("Raw copy");
                        // Fail-open: push the raw transcript into the
                        // pill's polish slot so the partial polish UI
                        // collapses to the unmodified text we're about
                        // to copy.
                        if stream_to_pill() {
                            pill::emit_polish_stream(&app, id, transcript.clone(), true);
                        }
                        transcript.clone()
                    }
                    Err(e) => {
                        log::warn!(
                            "LLM cleanup failed (sid={id}): {e}; falling back to raw transcript"
                        );
                        polish_warning = Some("Raw copy");
                        if stream_to_pill() {
                            pill::emit_polish_stream(&app, id, transcript.clone(), true);
                        }
                        transcript.clone()
                    }
                }
            }
        }
        _ => transcript.clone(),
    };

    let cleaned = session.mode.apply_post_processing(&cleaned);

    // Final polish stream flush — emit so the rate limiter snaps to
    // the post-processed text immediately. Skip when polish is off
    // (we already emitted the final transcript stream above) or on
    // the fail-open paths (which already flushed the raw transcript).
    if stream_to_pill() && polish_warning.is_none() && polish_engine_active {
        pill::emit_polish_stream(&app, id, cleaned.clone(), true);
    }

    if session.is_canceled() {
        log::info!("post-LLM cancel (sid={id})");
        if stream_to_pill() {
            pill::set_wide(&app, false);
        }
        manager.set_stage(&app, id, Stage::Canceled, None, false);
        _guard.finalise();
        return;
    }

    log::info!("copying {} chars (sid={id})", cleaned.len());

    *session.result.lock() = Some(cleaned.clone());

    let copy_text = cleaned.clone();
    let manager_for_copy = manager.clone();
    let auto_paste = session.auto_paste_enabled;
    let copy_result = tauri::async_runtime::spawn_blocking(move || {
        manager_for_copy.deliver_transcript(id, &copy_text, auto_paste)
    })
    .await;

    if session.is_canceled() {
        if stream_to_pill() {
            pill::set_wide(&app, false);
        }
        manager.set_stage(&app, id, Stage::Canceled, None, false);
        _guard.finalise();
        return;
    }

    // Promote a paste-failure to a warning so the user notices that
    // auto-paste didn't go through (and that they need to ⌘V manually
    // / grant Accessibility). The reason was already logged inside
    // `deliver_transcript`; here we just need the boolean. A
    // successful paste leaves the warning state unchanged — only
    // polish_warning flips it.
    let paste_failed = matches!(
        &copy_result,
        Ok(Ok(DeliveryOutcome::PasteFailedCopiedOnly))
    );

    let warning_flag = polish_warning.is_some() || paste_failed;
    let entry = CompletedEntry {
        session_id: id,
        ts: chrono::Utc::now().timestamp(),
        text: cleaned.clone(),
        // Keep the pre-polish Whisper transcript around so a user who
        // re-polishes this entry via the tray popover can start from
        // the original raw text rather than the already-cleaned output.
        // Skipped when polish was disabled / fell open (cleaned IS the
        // raw transcript already, no value in duplicating it).
        raw_text: if polish_engine_active && polish_warning.is_none() {
            Some(transcript.clone())
        } else {
            None
        },
        mode_id: session.mode.id.clone(),
        stage: Stage::Done,
        warning: warning_flag,
        error_message: None,
    };

    match copy_result {
        Ok(Ok(outcome)) => {
            transcripts::push_completed(entry);
            // Stats record covers both polish-succeeded and the
            // LLM fail-open path — both arms above set `cleaned` to
            // either the polished or raw transcript and fall through
            // to this same successful-copy branch. A failed/panicked
            // copy skips this; cancel/STT-error/silent paths return
            // early before reaching here.
            {
                let actual_seconds = session.started_at.elapsed().as_secs_f64();
                let now_ts = chrono::Utc::now().timestamp();
                if let Some(t) = session.transcript.lock().as_deref() {
                    crate::stats::record_session(&app, t, actual_seconds, now_ts);
                } else {
                    log::warn!(
                        "stats: success branch reached with no transcript captured (sid={id}); skipping record"
                    );
                }
            }
            // Surface the milestone-donation window if this session just
            // pushed cumulative time-saved past a tier. No-op otherwise;
            // fires off-main-thread-safely and marks the tier shown so it
            // never re-appears.
            crate::donation_window::maybe_show(&app);
            // Same post-delivery moment for the update announcement: if a
            // background check has already found a new version, surface it
            // now rather than from the check timer, where a focus grab
            // could collide with the auto-paste ⌘V into the focused field.
            crate::update_window::maybe_show(&app);
            // Legacy event keeps the existing tray-popover RECENT
            // logic working for any consumer that hasn't migrated.
            let _ = app.emit("transcripts:changed", transcripts::snapshot());

            // Snap the pill back to its compact width before set_stage
            // emits the Done state so the success treatment renders
            // at 144px.
            if stream_to_pill() {
                pill::set_wide(&app, false);
            }

            // Done-stage message picks the worst outcome:
            //   1. polish_warning — polish failed and we fell open to
            //      the raw transcript. The user almost certainly cares
            //      more about losing the polish than the delivery mode.
            //   2. paste_failed — polish succeeded but auto-paste
            //      didn't (typically Accessibility missing). The
            //      transcript IS on the clipboard, so we nudge with
            //      "grant Accessibility" while making it clear they
            //      can still ⌘V manually right now.
            //   3. clean success — pick the message that matches the
            //      actual delivery: "Inserted" when we pasted for them,
            //      the legacy "Copied — press ⌘V" otherwise.
            let (message, warn) = if let Some(msg) = polish_warning {
                (msg.into(), true)
            } else if paste_failed {
                (PASTE_FAILED_HINT.into(), true)
            } else {
                match outcome {
                    DeliveryOutcome::Pasted => ("Inserted".into(), false),
                    DeliveryOutcome::CopiedOnly => (PASTE_HINT.into(), false),
                    // Already handled by the `paste_failed` branch
                    // above; the arm exists for exhaustiveness.
                    DeliveryOutcome::PasteFailedCopiedOnly => (PASTE_HINT.into(), true),
                }
            };
            manager.set_stage(&app, id, Stage::Done, Some(message), warn);
        }
        Ok(Err(e)) => {
            log::error!("copy failed (sid={id}): {e}");
            *session.error.lock() = Some(e);
            if stream_to_pill() {
                pill::set_wide(&app, false);
            }
            manager.set_stage(
                &app,
                id,
                Stage::Error,
                Some("Copy failed — try again".into()),
                false,
            );
        }
        Err(e) => {
            log::error!("copy task panicked (sid={id}): {e}");
            if stream_to_pill() {
                pill::set_wide(&app, false);
            }
            manager.set_stage(
                &app,
                id,
                Stage::Error,
                Some("Copy failed — try again".into()),
                false,
            );
        }
    }

    // Auto-hide the pill after a Done state if this session owned the
    // pill at any point in the pipeline (it just released ownership in
    // set_stage right before the Done emission). The `owns_pill` flag
    // captured up at the start of run_pipeline is the right anchor —
    // foreground_id() has been None ever since end_recording cleared it.
    if owns_pill {
        let hide_ms = if warning_flag { 3500 } else { 1800 };
        manager.schedule_pill_hide_unless_taken(&app, id, hide_ms);
    }

    _guard.finalise();
}

async fn wait_for_cancel(session: Arc<SessionState>) {
    loop {
        if session.cancel.load(Ordering::SeqCst) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn wav_peak_pcm16(wav: &[u8]) -> f32 {
    // Parse via hound rather than assuming a fixed 44-byte header — WAV
    // files with extra chunks (`LIST`, `bext`, etc.) push the data offset
    // past 44 bytes, so the old slice-based approach would walk into
    // metadata as if it were samples and false-positive on silence.
    // On any parse error we treat the buffer as silent (returning 0.0)
    // so the session still completes via the silent-buffer pill path
    // rather than crashing the pipeline.
    let mut reader = match hound::WavReader::new(std::io::Cursor::new(wav)) {
        Ok(r) => r,
        Err(e) => {
            log::warn!("wav_peak_pcm16: header parse failed ({e}); treating as silent");
            return 0.0;
        }
    };
    let spec = reader.spec();
    let mut peak: f32 = 0.0;
    match spec.sample_format {
        hound::SampleFormat::Int => {
            for s in reader.samples::<i16>() {
                if let Ok(v) = s {
                    let mag = (v as f32).abs() / i16::MAX as f32;
                    if mag > peak {
                        peak = mag;
                    }
                }
            }
        }
        hound::SampleFormat::Float => {
            for s in reader.samples::<f32>() {
                if let Ok(v) = s {
                    let mag = v.abs();
                    if mag > peak {
                        peak = mag;
                    }
                }
            }
        }
    }
    peak.min(1.0)
}

/// System prompt for the notch voice interpreter. The model must output
/// ONLY a JSON object matching [`crate::notch_list::NotchCommandResponse`].
///
/// Framed around *natural, conversational speech*, not memorised commands:
/// people talk to the list the way they'd talk to a friend ("my first
/// priority is to ship the app, then I need to call the dentist…") and the
/// model's job is to understand the meaning and capture it — adding is the
/// default. The worked examples carry most of the weight here; keep the
/// multi-item brain-dump example, since that is the exact case that used to
/// fail.
const NOTCH_COMMAND_PROMPT: &str = "You turn what someone says out loud into edits for their personal to-do list (the \"Notch List\"). They are talking the way they would talk to a friend — there are NO special command words and they should never have to learn any. Understand what they mean and capture it as list edits.

You are given the CURRENT LIST as JSON (each item has a 1-based \"index\", its \"text\", and \"priority\") and a transcript of WHAT THE USER SAID.

Output ONLY a single JSON object — no prose, no markdown fences, no explanation — with this exact shape:
{
  \"ops\": [ ... ],
  \"needs_clarification\": null,
  \"confidence\": 0.0
}

Each op in \"ops\" is one of:
- {\"op\": \"add\", \"text\": \"...\", \"priority\": \"high\"|\"med\"|\"low\" (optional), \"position\": \"top\"|\"bottom\" (optional)}
- {\"op\": \"complete\", \"target\": <target>}
- {\"op\": \"remove\", \"target\": <target>}
- {\"op\": \"edit\", \"target\": <target>, \"text\": \"...\"}
- {\"op\": \"reprioritize\", \"target\": <target>, \"priority\": \"high\"|\"med\"|\"low\"}
- {\"op\": \"reorder\", \"target\": <target>, \"position\": \"top\"|\"bottom\"}

A <target> is a 1-based integer index into the current list (use this when they point at a position: \"the second one\", \"number three\", \"the last one\") OR a string matching the item's text (\"the dentist one\").

How to understand natural speech:
- ADDING IS THE DEFAULT. Whenever someone mentions something they want to do, need to do, plan to do, or should remember — even as plain narration like \"my first priority is to finish the alarm app\", \"then I need to apply for review\", \"after that I want to release Vox\", or \"don't let me forget to call the dentist\" — create one \"add\" op for each distinct task, in the order they said them.
- Write each task as a short, clear action and DROP the filler around it (\"my first priority is to\", \"I need to\", \"then\", \"after that\", \"I want to\", \"I should\"). E.g. \"then I really need to call the dentist about my tooth\" → text \"Call the dentist about my tooth\".
- Keep the speaker's own language and wording. Do NOT translate.
- Infer priority from their words: clear importance/urgency (\"most important\", \"first priority\", \"top priority\", \"critical\", \"urgent\", \"asap\", \"right away\") → \"high\"; clearly can-wait (\"eventually\", \"someday\", \"at some point\", \"whenever\", \"low priority\", \"no rush\") → \"low\"; otherwise leave priority out.
- RESOLVE BACK-REFERENCES: when a task points back at something said earlier in the SAME utterance with a vague phrase (\"that application\", \"it\", \"the same one\", \"that bug\", \"there\"), rewrite it to name the concrete thing from the earlier sentence. E.g. \"fix all issues on the alarm application, then apply for Apple review for that application\" → the second task becomes \"Apply for Apple review for the alarm application\". Only substitute when exactly ONE earlier noun is an unambiguous antecedent; if it is unclear or there are several candidates, keep the user's words as spoken. NEVER invent a noun that was not said.
- They can also change EXISTING items: finishing/checking off → \"complete\"; deleting/scratching/never-mind → \"remove\"; rewording → \"edit\"; making something more/less important → \"reprioritize\"; moving up/down → \"reorder\". Resolve \"the second one\", \"that dentist task\", \"the last one\" against the current list (prefer an index when they name a position).
- One utterance can mix several edits (add two things AND finish one) — emit an op for each.

When NOT to act:
- Set \"needs_clarification\" to a short question (and return empty \"ops\") ONLY when they clearly meant to change an EXISTING item but you genuinely cannot tell which one. NEVER ask just because the speech is long, casual, or rambling — prefer adding.
- If they truly said nothing actionable, return an empty \"ops\" array.

Set \"confidence\" between 0 and 1. Output the JSON object and nothing else.

Examples:
CURRENT LIST: []
SAID: \"Okay so my first priority is to finish all the new features for the alarm app, then I need to apply for Apple review, after that I want to release the Vox update, and finally fix the alarm bugs.\"
OUTPUT: {\"ops\":[{\"op\":\"add\",\"text\":\"Finish all new features for the alarm app\",\"priority\":\"high\"},{\"op\":\"add\",\"text\":\"Apply for Apple review\"},{\"op\":\"add\",\"text\":\"Release the Vox update\"},{\"op\":\"add\",\"text\":\"Fix the alarm bugs\"}],\"needs_clarification\":null,\"confidence\":0.95}

CURRENT LIST: [{\"index\":1,\"text\":\"Email Bob\",\"priority\":\"med\"},{\"index\":2,\"text\":\"Call the dentist\",\"priority\":\"med\"}]
SAID: \"I finished emailing Bob, and bump the dentist one up to high priority.\"
OUTPUT: {\"ops\":[{\"op\":\"complete\",\"target\":1},{\"op\":\"reprioritize\",\"target\":\"dentist\",\"priority\":\"high\"}],\"needs_clarification\":null,\"confidence\":0.9}

CURRENT LIST: []
SAID: \"Fix all the issues on the alarm application, then apply for Apple review for that application.\"
OUTPUT: {\"ops\":[{\"op\":\"add\",\"text\":\"Fix all the issues on the alarm application\"},{\"op\":\"add\",\"text\":\"Apply for Apple review for the alarm application\"}],\"needs_clarification\":null,\"confidence\":0.95}";

/// True for a bare spoken "undo" command — "undo", "undo that", "undo
/// last", "undo last edit/change" — after lowercasing, trimming, and
/// stripping trailing punctuation. Used to short-circuit the notch path
/// to the store undo with no LLM round-trip.
fn is_undo_phrase(transcript: &str) -> bool {
    let t = transcript
        .trim()
        .trim_end_matches(['.', '!', '?', ',', ' '])
        .to_lowercase();
    matches!(
        t.as_str(),
        "undo" | "undo that" | "undo last" | "undo last edit" | "undo last change"
    )
}

/// Interpret `transcript` as a Notch List command using the user's LLM
/// engine, then apply the ops. Fail-open: any failure (no engine, parse
/// miss, low confidence, clarification needed, apply error) leaves the
/// list unchanged and surfaces a brief Done/Error pill. Never touches the
/// clipboard.
async fn interpret_and_apply_notch(
    app: &AppHandle<Wry>,
    manager: &Arc<SessionManager>,
    session: &Arc<SessionState>,
    transcript: &str,
    llm_engine: Option<Arc<dyn crate::engines::LlmEngine>>,
) {
    let id = session.id;
    let store = crate::notch_list::store(app);

    // Deterministic "undo" shortcut: a bare "undo" / "undo that" / "undo
    // last [edit]" skips the LLM round-trip entirely and restores the
    // pre-last-batch snapshot. Cheap, offline, and instant. The visible
    // result flows to the notch (idle on success, error otherwise); the
    // list change itself rides `notch-list:changed`. `set_stage` still
    // runs so the tray popover's IN PROGRESS snapshot stays consistent,
    // but it no longer drives the pill (a notch session owns no pill).
    if is_undo_phrase(transcript) {
        match store.undo() {
            Ok(true) => {
                crate::commands::emit_notch_list_changed(app);
                pill::emit_notch_phase(app, "idle", None);
                manager.set_stage(app, id, Stage::Done, Some("Undone".into()), false);
            }
            Ok(false) => {
                pill::emit_notch_phase(app, "idle", None);
                manager.set_stage(app, id, Stage::Done, Some("Nothing to undo".into()), true);
            }
            Err(e) => {
                log::warn!("notch command (sid={id}): undo failed: {e}");
                pill::emit_notch_phase(app, "error", Some("Undo failed".into()));
                manager.set_stage(app, id, Stage::Error, Some("Undo failed".into()), false);
            }
        }
        return;
    }

    // The notch interpreter needs a real LLM. When polishing is off /
    // the engine is the no-op, there's nothing to interpret with — fail
    // open with a clear message routed to the notch (never the pill).
    let engine = match llm_engine {
        Some(e) if e.id() != "none" => e,
        _ => {
            log::warn!("notch command (sid={id}): no LLM engine available");
            pill::emit_notch_phase(
                app,
                "error",
                Some("Enable polishing to use voice list editing".into()),
            );
            manager.set_stage(
                app,
                id,
                Stage::Error,
                Some("Enable polishing to use voice list editing".into()),
                false,
            );
            return;
        }
    };

    // Compiling = the LLM interpret call is about to run. Drives the
    // notch's "thinking" chrome in place of the pill's Polishing stage.
    pill::emit_notch_phase(app, "compiling", None);
    manager.set_stage(app, id, Stage::Polishing, None, false);

    // Give the model a slim, explicitly-indexed view of the active list
    // (1-based index + text + priority) instead of the raw on-disk items.
    // The interpreter references items by index or text — it never needs
    // ids, timestamps, or internal order — and the explicit "index" field
    // makes "the second one" resolve reliably. The index here matches
    // `resolve_target`'s 1-based index into the same sorted active list.
    let doc = store.read();
    let active = doc.top_n(usize::MAX);
    let slim: Vec<serde_json::Value> = active
        .iter()
        .enumerate()
        .map(|(i, item)| {
            serde_json::json!({
                "index": i + 1,
                "text": item.text,
                "priority": item.priority,
            })
        })
        .collect();
    let list_json = serde_json::to_string(&slim).unwrap_or_else(|_| "[]".into());
    let payload = format!(
        "CURRENT LIST (JSON):\n{}\n\nWHAT THE USER SAID:\n{}",
        list_json, transcript
    );

    let raw = match engine
        .cleanup_structured(&payload, NOTCH_COMMAND_PROMPT, &[], false)
        .await
    {
        Ok(t) => t,
        Err(e) => {
            log::warn!("notch command (sid={id}): LLM failed: {e}");
            pill::emit_notch_phase(app, "error", Some("Didn't catch that — try again".into()));
            manager.set_stage(
                app,
                id,
                Stage::Error,
                Some("Didn't catch that — try again".into()),
                false,
            );
            return;
        }
    };

    let parsed = match crate::notch_list::parse_command_response(&raw) {
        Some(p) => p,
        None => {
            log::warn!("notch command (sid={id}): could not parse LLM JSON: {raw}");
            pill::emit_notch_phase(app, "error", Some("Didn't catch that — try again".into()));
            manager.set_stage(
                app,
                id,
                Stage::Error,
                Some("Didn't catch that — try again".into()),
                false,
            );
            return;
        }
    };

    // Decide what to do with the interpreter's response. Non-empty ops are
    // always applied — pulling tasks out of natural, conversational speech
    // is the whole point, and every op is reversible (adds/edits/completes
    // via undo, removes via the confirm gesture). We deliberately do NOT
    // gate ops behind `confidence`; that floor used to silently drop
    // perfectly good multi-item brain-dumps. A clarifying question only
    // surfaces when the model produced no ops and asked a real one.
    match parsed.action() {
        crate::notch_list::NotchAction::Clarify(question) => {
            log::info!("notch command (sid={id}): needs clarification: {question}");
            // A clarifying question is a soft "couldn't act" outcome — the
            // notch surfaces it as an error-phase message so the user sees
            // what to disambiguate. The list is unchanged.
            pill::emit_notch_phase(app, "error", Some(question.clone()));
            manager.set_stage(app, id, Stage::Done, Some(question), true);
            return;
        }
        crate::notch_list::NotchAction::Nothing => {
            log::info!(
                "notch command (sid={id}): nothing actionable (confidence {:.2})",
                parsed.confidence
            );
            pill::emit_notch_phase(
                app,
                "error",
                Some("Didn't catch any tasks — try again".into()),
            );
            manager.set_stage(
                app,
                id,
                Stage::Done,
                Some("Didn't catch any tasks — try again".into()),
                true,
            );
            return;
        }
        crate::notch_list::NotchAction::Apply => {
            log::debug!(
                "notch command (sid={id}): applying {} op(s) (confidence {:.2})",
                parsed.ops.len(),
                parsed.confidence
            );
        }
    }

    let confirm = settings::prefs(app).notch_list_confirm_destructive;
    match store.apply_voice_ops(parsed.ops, confirm) {
        Ok(result) if result.staged_destructive => {
            // Destructive batch: stage the fully-applied doc and ask the
            // user to confirm by pressing the notch-voice hotkey again
            // within NOTCH_CONFIRM_WINDOW (handled in the hotkey closure
            // via `try_commit_pending_notch`). Nothing is persisted yet.
            manager.stage_notch_destructive(result.doc);
            // The notch list itself hasn't changed (nothing persisted), so
            // we use an error-phase message to carry the confirm prompt to
            // the notch UI rather than `notch-list:changed`.
            pill::emit_notch_phase(
                app,
                "error",
                Some("Press your Notch List hotkey again to confirm removal".into()),
            );
            manager.set_stage(
                app,
                id,
                Stage::Done,
                Some("Press your Notch List hotkey again to confirm removal".into()),
                true,
            );
        }
        Ok(result) => {
            // A non-destructive edit invalidates any pending destructive
            // confirmation so a later confirm press can't apply a stale,
            // now-superseded batch. The actual list change flows through
            // `notch-list:changed`; the notch returns to idle.
            manager.clear_notch_destructive();
            crate::commands::emit_notch_list_changed(app);
            pill::emit_notch_phase(app, "idle", None);
            manager.set_stage(app, id, Stage::Done, Some(result.summary()), false);
        }
        Err(e) => {
            log::warn!("notch command (sid={id}): apply failed: {e}");
            pill::emit_notch_phase(app, "error", Some("Couldn't find that item".into()));
            manager.set_stage(
                app,
                id,
                Stage::Error,
                Some("Couldn't find that item".into()),
                false,
            );
        }
    }
}

fn friendly_stt_error(e: &anyhow::Error, _engine_id: &str) -> String {
    let s = format!("{e}");
    if s.contains("not downloaded") {
        "Local model not downloaded — open Settings → Models.".into()
    } else if s.contains("compiled out") {
        "Local engines not enabled in this build.".into()
    } else {
        "Transcription failed.".into()
    }
}

fn spawn_max_record_guard(
    app: AppHandle<Wry>,
    manager: Arc<SessionManager>,
    session: Arc<SessionState>,
    max_secs: u64,
) {
    let id = session.id;
    tauri::async_runtime::spawn(async move {
        // Race the safety-cap sleep against pipeline completion. The
        // pipeline notifies on every terminal exit (set_stage + the
        // few bail-out paths that bypass it), so a typical 5-30s
        // session drops the guard's captured Arcs immediately rather
        // than holding them for the full 30 minutes. The select races
        // a sleep and the Notify; whichever wakes first wins.
        let completion = session.completion.clone();
        // Fast-path: completion already happened before the spawn was
        // scheduled. Notify::notified() against a notify_waiters()
        // that already fired would block forever; the AtomicBool
        // covers that gap.
        if session.completed.load(Ordering::SeqCst) {
            return;
        }
        tokio::select! {
            biased;
            _ = completion.notified() => {
                // Pipeline finished before the cap; nothing to do.
            }
            _ = tokio::time::sleep(Duration::from_secs(max_secs)) => {
                // Only auto-stop if this session is still the active
                // recording (a subsequent recording wouldn't have
                // replaced it because mic capture is single-instance).
                if manager.foreground_id() == Some(id) {
                    log::warn!(
                        "recording exceeded {max_secs}s safety cap — auto-stopping (sid={id})"
                    );
                    // Surface the cap to the user — without this the
                    // pipeline silently transitions to Transcribing
                    // and the user has no idea the safety guard
                    // fired. The transcript still gets copied (via
                    // the regular end_recording → run_pipeline path).
                    if session.notch_command {
                        // Notch session: its UI lives in the notch, not
                        // the pill, so route the cap notice there.
                        pill::emit_notch_phase(
                            &app,
                            "error",
                            Some("Stopped after 30 min".into()),
                        );
                    } else {
                        pill::emit_toast(
                            &app,
                            "Stopped after 30 min — paste or discard".into(),
                            4000,
                        );
                    }
                    manager.end_recording(app);
                }
            }
        }
    });
}

/// Pre-emptively decode the in-progress capture buffer while the user
/// is still speaking. Two purposes: prime the engine cache so the final
/// pass is cheaper (whisper-local), and — when `live_transcript_enabled`
/// is on — paint a live caption into the pill as the user speaks.
///
/// Engine eligibility differs (see the match in the body): whisper-local
/// always speculates when streaming is on; parakeet-local only when the
/// live caption is requested, because it has no cache to prime and emits
/// no mid-decode partials, so a speculative pass is pure overhead unless
/// the caption needs it.
///
/// Throttling rules:
///   * Don't start the first pass until the buffer holds at least
///     [`SPEC_MIN_FIRST_SECS`] of audio (or [`SPEC_MIN_FIRST_SECS_LIVE`]
///     on the live-caption path) — otherwise we burn cycles on a tiny
///     utterance the user is about to release the hotkey on.
///   * Each subsequent pass needs the buffer to have grown by a minimum
///     delta vs the previous snapshot: [`SPEC_MIN_DELTA_SECS`] normally,
///     or a length-aware gap on the live path (`SPEC_LIVE_DELTA_GROWTH`
///     × buffer length, floored at [`SPEC_MIN_DELTA_SECS_LIVE`]) so a
///     no-incremental-API engine re-decoding the whole buffer doesn't
///     pin a core on a long take.
///   * Only one decode runs at a time; while one is in flight the next
///     iteration just polls. We don't queue — when the in-flight pass
///     finishes we re-evaluate against the *current* buffer.
///   * Stops as soon as the recording ends (`foreground_id() != Some(id)`)
///     or the session is canceled. The final authoritative pass in
///     `run_pipeline` will run independently against the final buffer.
///
/// Cancel semantics: whisper-rs's `state.full()` is a synchronous,
/// uninterruptible call. When the user hits Escape mid-decode the loop
/// can't kill the in-flight pass — but the segment callback re-checks
/// `owns_pill_emit(id)` every fire (which has already flipped to false
/// by the time the cancel reaches us), so no partials leak. When the
/// blocking decode returns naturally, the next iteration sees
/// `is_canceled()` and bails. Worst case: ~one decode of CPU stays warm
/// after Escape. The pipeline's *final* pass is gated on the cancel
/// flag separately so cancel does not pay for a redundant final run.
fn spawn_speculative_decoder(
    app: AppHandle<Wry>,
    manager: Arc<SessionManager>,
    session: Arc<SessionState>,
) {
    /// Don't speculate on anything shorter than this — small utterances
    /// finalise faster than a speculative pass would complete anyway.
    /// Live-transcript users explicitly asked for a caption *while*
    /// they speak, so the threshold drops so the first chunk shows up
    /// within ~1 s instead of making them wait two seconds staring at
    /// typing dots before any text appears.
    const SPEC_MIN_FIRST_SECS: f32 = 2.0;
    const SPEC_MIN_FIRST_SECS_LIVE: f32 = 0.8;
    /// Minimum new audio (vs the last snapshot we decoded) before we
    /// kick off another speculative pass. Tighter cadence on the
    /// live-caption path so the visible text trails actual speech by
    /// a fraction of a second instead of 1.5 s.
    const SPEC_MIN_DELTA_SECS: f32 = 1.5;
    const SPEC_MIN_DELTA_SECS_LIVE: f32 = 0.5;
    /// Fraction of the current buffer length used as the live-caption
    /// re-decode gap once a take runs long. Engines with no incremental
    /// API re-decode the whole buffer each pass, so the gap grows with
    /// duration (gap = buffer_secs × this, floored at the LIVE delta) to
    /// keep total decode work ~linear instead of quadratic. 0.15 ≈ a
    /// 1.5 s gap at 10 s and ~4.5 s at 30 s.
    const SPEC_LIVE_DELTA_GROWTH: f32 = 0.15;
    /// Polling cadence — small enough that cancel/stop reacts within
    /// ~one tick, large enough that we don't pin a CPU.
    const SPEC_POLL_MS: u64 = 100;

    // Engine eligibility.
    //
    // `whisper-local` benefits from speculative decoding two ways: it
    // primes the WhisperContext cache so the final pass is cheaper, AND
    // it emits per-segment partials that feed the streaming pill. So we
    // always spin it up when streaming is on.
    //
    // `parakeet-local` is different. It's already fast enough (~96 ms
    // for a 4 s clip) that priming the final pass is moot, and it emits
    // NO mid-decode partials — `transcribe` returns the whole string in
    // one shot. The only way to paint text *before* the hotkey is
    // released is to re-run the engine against the growing audio
    // snapshot and surface each cumulative result. That's pure overhead
    // unless the user actually asked for the live caption, so we gate it
    // on `live_transcript_enabled` rather than running it for the
    // post-release reveal (which is instant for Parakeet anyway).
    let eligible = match session.stt_engine_id.as_str() {
        "whisper-local" => true,
        "parakeet-local" => session.live_transcript_enabled,
        _ => false,
    };
    if !eligible {
        return;
    }

    // Battery-aware bail: speculative decoding runs full transcription
    // passes on a recurring cadence while the user is still speaking,
    // which is measurable battery drain on laptops on the unplugged
    // path. When the Mac is unplugged we fall back to the original
    // "decode entirely after hotkey release" behaviour. AC adapter,
    // UPS, and any desktop (no battery at all) keep the streaming pass.
    // Single check at task-start is sufficient — session lifetime is
    // short and a user (un)plugging mid-utterance is a non-event.
    //
    // `live_transcript_enabled` overrides the bail: the user has
    // explicitly asked to see a caption while speaking, so silently
    // disabling the underlying decoder would defeat the toggle. (The
    // length-aware cadence below keeps the on-battery cost bounded on
    // long takes.)
    if mac_power::on_battery() && !session.live_transcript_enabled {
        log::info!("speculative decode skipped: on battery");
        return;
    }

    let id = session.id;
    let audio = manager.audio.clone();

    tauri::async_runtime::spawn(async move {
        let mut last_decoded_secs: f32 = 0.0;
        // Read from the session-scoped snapshot rather than re-loading
        // settings here — both this task and `run_pipeline`'s final
        // pass must agree on the same value, and a settings reload
        // after a mid-utterance toggle would cause them to diverge.
        // Either streaming-pill or live-transcript implies the user
        // wants segment text painted into the pill. The pill UI
        // itself gates *when* to reveal it (listening vs post-
        // release); the backend just needs to hand the data over.
        let streaming_enabled_at_begin =
            session.streaming_enabled || session.live_transcript_enabled;
        let mut ticker = tokio::time::interval(Duration::from_millis(SPEC_POLL_MS));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            ticker.tick().await;

            // Stop conditions: recording ended (the active_recording
            // slot was cleared by `end_recording`/`cancel_session`) or
            // the session was canceled. Either way bail — the final
            // pass will run on its own.
            if manager.foreground_id() != Some(id) || session.is_canceled() {
                log::debug!("speculative: stop signal (sid={id})");
                return;
            }

            // The audio handle's `snapshot_wav` is synchronous (it
            // round-trips through a std mpsc to the audio thread), so
            // we hop onto a blocking task to avoid pinning a tokio
            // worker. The work itself is cheap — buffer clone + WAV
            // encode — typically sub-millisecond, but `recv()` is a
            // blocking call so we shouldn't run it on a runtime worker.
            let audio_for_snap = audio.clone();
            let snap_result = tokio::task::spawn_blocking(move || {
                audio_for_snap.snapshot_wav()
            })
            .await;
            let snap = match snap_result {
                Ok(Ok(Some(s))) => s,
                Ok(Ok(None)) => continue,
                Ok(Err(e)) => {
                    log::warn!("speculative: snapshot_wav failed (sid={id}): {e}");
                    return;
                }
                Err(e) => {
                    log::warn!("speculative: snapshot task panicked (sid={id}): {e}");
                    return;
                }
            };

            let delta = snap.duration_secs - last_decoded_secs;
            let need = if last_decoded_secs == 0.0 {
                if session.live_transcript_enabled {
                    SPEC_MIN_FIRST_SECS_LIVE
                } else {
                    SPEC_MIN_FIRST_SECS
                }
            } else if session.live_transcript_enabled {
                // Length-aware cadence for the live caption. Engines with
                // no incremental API (Parakeet) re-decode the WHOLE
                // growing buffer every pass, so a fixed 0.5s gap pins a
                // core on a long take — the per-pass cost climbs with
                // duration until passes run back-to-back. Stretch the gap
                // as the buffer grows so total work stays roughly linear
                // instead of quadratic: ~0.5s early, widening past that
                // once the take runs long. Keeps the caption responsive
                // when it matters (the first several seconds) without
                // continuously re-decoding a 30s clip.
                (snap.duration_secs * SPEC_LIVE_DELTA_GROWTH)
                    .max(SPEC_MIN_DELTA_SECS_LIVE)
            } else {
                SPEC_MIN_DELTA_SECS
            };
            if delta < need {
                continue;
            }

            // Pull the engine through the registry. It's almost
            // certainly cached by `prewarm_stt_in_background`, but if
            // not the first call here will trigger a load the same
            // way the final pass would.
            let registry = match app.try_state::<Registry>() {
                Some(r) => r.inner().clone(),
                None => return,
            };
            let stt_engine = match registry.stt(&app, &session.stt_engine_id, &session.stt_model_id)
            {
                Some(e) => e,
                None => {
                    // Engine not configured — let the final pass
                    // surface the error.
                    return;
                }
            };

            let language: Option<String> = if session.language != "auto" {
                Some(session.language.clone())
            } else {
                None
            };
            let dictionary = session.dictionary.clone();

            // Fire only if streaming-pill emits are wanted by the user.
            // The optimisation is independent of the UI flag — even with
            // streaming disabled the speculative pass primes Whisper's
            // model cache and saves wall-clock for the final pass —
            // BUT the only thing the speculative pass produces is the
            // segment-callback stream, which is exactly the data we'd
            // be discarding if streaming is off. So we still run the
            // pass to keep the cache warm, just without a callback.
            let app_for_cb = app.clone();
            let manager_for_cb = manager.clone();
            let session_for_cb = session.clone();
            let stt_callback: Option<crate::engines::SegmentCallback> =
                if streaming_enabled_at_begin {
                    Some(Box::new(move |text: &str| {
                        // Re-check ownership inside the callback so a
                        // newer recording stealing the pill doesn't see
                        // partials from this older session. Also bail
                        // on cancel: whisper-rs's `state.full` is
                        // uninterruptible, so once Escape is pressed
                        // the in-flight pass keeps firing segments
                        // until it returns naturally — without this
                        // gate it would paint partials into a pill the
                        // user has already dismissed.
                        if !manager_for_cb.owns_pill_emit(id)
                            || session_for_cb.is_canceled()
                        {
                            return;
                        }
                        // `final=false` — the final pass emits the
                        // authoritative cumulative text with `final=true`
                        // when it completes.
                        pill::emit_transcript_stream(
                            &app_for_cb,
                            id,
                            text.to_string(),
                            false,
                        );
                    }))
                } else {
                    None
                };

            // The pill was widened by `run_pipeline`'s session-begin
            // path; the speculative task no longer drives UI state to
            // avoid racing two `set_wide(true)` calls (one from this
            // task before it took the speculative lock, one from
            // `run_pipeline` itself). Emits below still flow through
            // `pill::emit_transcript_stream`, which is purely a data
            // event and does not move the OS window.

            // Acquire the speculative lock. The final pass holds this
            // same mutex while it runs, so a hotkey-release that fires
            // mid-speculation won't race two whisper decodes against
            // the same WhisperContext. If the session has been
            // canceled while we waited, bail out before doing the
            // expensive call.
            let guard = session.speculative_decode.clone().lock_owned().await;
            if manager.foreground_id() != Some(id) || session.is_canceled() {
                drop(guard);
                return;
            }

            log::debug!(
                "speculative: decoding {:.1}s (sid={id})",
                snap.duration_secs
            );
            let started = Instant::now();
            let lang_ref = language.as_deref();
            let result = stt_engine
                .transcribe(&snap.wav, lang_ref, &dictionary, stt_callback)
                .await;
            drop(guard);

            match result {
                Ok(text) => {
                    log::debug!(
                        "speculative: decoded {:.1}s in {} ms → {} chars (sid={id})",
                        snap.duration_secs,
                        started.elapsed().as_millis(),
                        text.len()
                    );
                    last_decoded_secs = snap.duration_secs;
                }
                Err(e) => {
                    // Fail-open: the final pass will retry the same
                    // engine call against the full buffer. We log a
                    // warning and keep iterating in case a transient
                    // error clears.
                    log::warn!("speculative: decode failed (sid={id}): {e}");
                    last_decoded_secs = snap.duration_secs;
                }
            }
        }
    });
}

fn spawn_audio_level_sampler(
    app: AppHandle<Wry>,
    manager: Arc<SessionManager>,
    session: Arc<SessionState>,
) {
    let id = session.id;
    let notch_command = session.notch_command;
    let audio = manager.audio.clone();
    tauri::async_runtime::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_millis(50));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // Sliding history of recent RMS samples for the notch waveform.
        // The notch wants a full fixed-length window per tick (so it can
        // paint a stable equalizer without buffering), whereas the pill
        // consumes one scalar per tick. Only allocated/maintained for a
        // notch session.
        let mut window: Vec<f32> = if notch_command {
            vec![0.0; pill::NOTCH_LEVELS_LEN]
        } else {
            Vec::new()
        };
        loop {
            ticker.tick().await;
            if manager.foreground_id() != Some(id) {
                break;
            }
            let level = audio.current_rms();
            if notch_command {
                // Slide the newest sample into the right edge so the
                // waveform scrolls left-to-right like a live equalizer.
                window.rotate_left(1);
                if let Some(last) = window.last_mut() {
                    *last = level.clamp(0.0, 1.0);
                }
                pill::emit_notch_levels(&app, &window);
            } else {
                pill::emit_audio_level(&app, level);
            }
        }
    });
}

fn resolve_active_mode<R: Runtime>(app: &AppHandle<R>, id: &str) -> Option<VoiceMode> {
    let store = app.try_state::<Arc<ModeStore>>()?;
    let store = store.inner().clone();
    store
        .get(id)
        .or_else(|| store.get("general"))
        .or_else(|| modes::builtins().into_iter().next())
}

fn spawn_suggestion_probe<R: Runtime>(app: AppHandle<R>, active_mode_id: String) {
    tauri::async_runtime::spawn_blocking(move || {
        let Some((bundle_id, title)) = crate::frontmost::detect() else {
            return;
        };
        let Some(store) = app.try_state::<Arc<ModeStore>>().map(|s| s.inner().clone()) else {
            return;
        };
        let suggestion = match store.suggest_for(Some(&bundle_id), title.as_deref()) {
            Some(id) if id != active_mode_id => store.get(&id),
            _ => None,
        };
        pill::emit_mode_suggestion(&app, suggestion);
    });
}

fn merge_dictionaries(global: &[String], mode: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    global
        .iter()
        .chain(mode.iter())
        .filter_map(|term| {
            let trimmed = term.trim();
            (!trimmed.is_empty() && seen.insert(trimmed.to_lowercase()))
                .then(|| trimmed.to_string())
        })
        .collect()
}

/// Map an ISO 639-1 language code (the values the Settings language
/// picker stores) to its English name. Covers the finite set the UI
/// offers — Parakeet's 25 languages plus the common non-Parakeet ones
/// (see `LANGUAGE_OPTIONS` in the frontend). Returns `None` for any
/// code we don't name, so an exotic / legacy value falls back to the
/// prompt's own detect-and-match rule rather than feeding the model a
/// bare code it might misread.
fn language_name(code: &str) -> Option<&'static str> {
    Some(match code {
        "bg" => "Bulgarian",
        "hr" => "Croatian",
        "cs" => "Czech",
        "da" => "Danish",
        "nl" => "Dutch",
        "en" => "English",
        "et" => "Estonian",
        "fi" => "Finnish",
        "fr" => "French",
        "de" => "German",
        "el" => "Greek",
        "hu" => "Hungarian",
        "it" => "Italian",
        "lv" => "Latvian",
        "lt" => "Lithuanian",
        "mt" => "Maltese",
        "pl" => "Polish",
        "pt" => "Portuguese",
        "ro" => "Romanian",
        "sk" => "Slovak",
        "sl" => "Slovenian",
        "es" => "Spanish",
        "sv" => "Swedish",
        "ru" => "Russian",
        "uk" => "Ukrainian",
        "zh" => "Chinese",
        "ja" => "Japanese",
        "ko" => "Korean",
        "ar" => "Arabic",
        "hi" => "Hindi",
        "tr" => "Turkish",
        "vi" => "Vietnamese",
        "th" => "Thai",
        _ => return None,
    })
}

/// Append an explicit "respond in <language>" directive to a system
/// prompt when the user has pinned a specific spoken language. The
/// prompts already tell the model to detect and match the transcript's
/// language; this is reinforcement for the pinned case (short or
/// code-switched utterances where detection is shakier). A no-op for
/// `auto` / empty / unrecognised codes — those rely on detection alone.
fn with_language_directive(system_prompt: &str, language: &str) -> String {
    let code = language.trim();
    if code.is_empty() || code == "auto" {
        return system_prompt.to_string();
    }
    match language_name(code) {
        Some(name) => format!(
            "{system_prompt}\n\nThe speaker is dictating in {name}. Write the entire output in {name}. Do not translate to any other language."
        ),
        None => system_prompt.to_string(),
    }
}

/// Appends the user's free-form context to the cleanup system prompt when
/// the active mode opts in and the context is non-empty. Language-agnostic
/// — it adds background only; the language directive (applied after) still
/// governs output language.
fn with_user_context(prompt: &str, context: &str, enabled: bool) -> String {
    let trimmed = context.trim();
    if !enabled || trimmed.is_empty() {
        return prompt.to_string();
    }
    format!(
        "{prompt}\n\nBackground about the speaker (use it to inform word choice, names, and intent; do not repeat it verbatim in the output):\n{trimmed}"
    )
}

/// The exact LLM system message the next dictation would send for the
/// user's currently-selected Voice Mode: the active mode's prompt run
/// through the language directive, plus the merged global + mode
/// dictionary. Used by the llama-server warmup so its `cache_prompt`
/// prime shares a real prefix with the first real polish, rather than a
/// throwaway prompt the cache can't reuse. Mirrors `run_pipeline`'s
/// construction exactly. Falls back to the `general` built-in when no
/// `ModeStore` is registered yet (launch race), and to an empty string
/// only if even the built-ins are somehow unavailable.
pub fn active_polish_system_message<R: Runtime>(app: &AppHandle<R>) -> String {
    let prefs = settings::prefs(app);
    let mode = match resolve_active_mode(app, &prefs.active_mode_id)
        .or_else(|| modes::builtins().into_iter().next())
    {
        Some(m) => m,
        None => return String::new(),
    };
    let ctx_prompt = with_user_context(&mode.system_prompt, &prefs.context, mode.context_enabled);
    let system_prompt = with_language_directive(&ctx_prompt, &prefs.language);
    let dictionary = merge_dictionaries(&prefs.dictionary, &mode.dictionary);
    crate::engines::gemma_http::build_system_message(&system_prompt, &dictionary)
}

/// Result of [`polish_with_mode`]. Mirrors the fail-open shape of the
/// live pipeline: on LLM error the raw input is returned and `warning`
/// is set so the caller can surface a "Raw copy" hint to the user.
pub struct PolishOutcome {
    /// Post-processed text. Always non-empty unless the caller passed
    /// in an empty string (we don't synthesise content out of nothing).
    pub text: String,
    /// True when the LLM step failed (or returned empty) and we fell
    /// back to the raw input.
    pub warning: bool,
}

/// Standalone polish step — same engine path, same dictionary merge,
/// same mode post-processing as `run_pipeline`, but free of the
/// `SessionState` / pill / clipboard plumbing so the
/// `re_polish_transcript` IPC handler can reuse it.
///
/// Behaviour matches the live pipeline exactly:
/// - When `llm_engine_id` is `"none"` or the registry returns `None`
///   for the engine, the raw input goes through post-processing
///   unchanged (and `warning` is false — that's by design, mirroring
///   `polish_engine_active == false`).
/// - LLM errors are fail-open: the raw input is post-processed and
///   `warning` is set so the caller can surface the same "Raw copy"
///   treatment used by the live pipeline.
/// - Mode post-processing (`case` / `punctuation` / `max_length_chars`)
///   runs on the final text regardless of which branch produced it,
///   so the user sees a consistent shape across the dictation path
///   and the re-polish path.
pub async fn polish_with_mode(
    app: AppHandle<Wry>,
    transcript: String,
    mode: &VoiceMode,
    llm_engine_id: String,
    llm_model_id: String,
    global_dictionary: Vec<String>,
    context: &str,
    global_enable_thinking: bool,
    language: String,
) -> PolishOutcome {
    let mut mode_snapshot = ModeSnapshot::from_voice_mode(mode);
    let ctx_prompt =
        with_user_context(&mode_snapshot.system_prompt, context, mode_snapshot.context_enabled);
    mode_snapshot.system_prompt = with_language_directive(&ctx_prompt, &language);
    let merged_dictionary = merge_dictionaries(&global_dictionary, &mode_snapshot.dictionary);
    // Per-mode override of `enable_thinking` wins; None inherits the
    // global. Matches the resolution in `SessionManager::begin`.
    let enable_thinking = mode.enable_thinking.unwrap_or(global_enable_thinking);

    let registry = match app.try_state::<Registry>() {
        Some(r) => r.inner().clone(),
        None => {
            log::warn!("polish_with_mode: engine registry missing; returning raw transcript");
            return PolishOutcome {
                text: mode_snapshot.apply_post_processing(&transcript),
                warning: false,
            };
        }
    };

    let cleaned = match registry.llm(&app, &llm_engine_id, &llm_model_id) {
        Some(engine) if engine.id() != "none" => {
            match engine
                .cleanup(
                    &transcript,
                    &mode_snapshot.system_prompt,
                    &merged_dictionary,
                    enable_thinking,
                )
                .await
            {
                Ok(t) if !t.trim().is_empty() => (t, false),
                Ok(_) => {
                    log::info!(
                        "polish_with_mode: LLM returned empty content; falling back to raw"
                    );
                    (transcript.clone(), true)
                }
                Err(e) => {
                    log::warn!(
                        "polish_with_mode: LLM cleanup failed: {e}; falling back to raw"
                    );
                    (transcript.clone(), true)
                }
            }
        }
        _ => (transcript.clone(), false),
    };

    let (raw_polished, warning) = cleaned;
    PolishOutcome {
        text: mode_snapshot.apply_post_processing(&raw_polished),
        warning,
    }
}

const SATURATED_TOAST_MS_PUB: u32 = SATURATED_TOAST_MS;

/// Helper for the hotkey layer: emit a saturated toast through the pill.
pub fn emit_saturated_toast<R: Runtime>(app: &AppHandle<R>, _in_flight: usize, _max: usize) {
    pill::show(app);
    // Plain English instead of jargon. Capped at <40 chars so it fits
    // the pill width comfortably across all states.
    let msg = "One at a time — try again in a moment".to_string();
    pill::emit_toast(app, msg, SATURATED_TOAST_MS_PUB);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_undo_phrase_matches_spoken_undo_variants() {
        assert!(is_undo_phrase("undo"));
        assert!(is_undo_phrase("Undo."));
        assert!(is_undo_phrase("  undo that!  "));
        assert!(is_undo_phrase("Undo last"));
        assert!(is_undo_phrase("undo last edit"));
        assert!(is_undo_phrase("undo last change"));
        // Not a bare undo command — should fall through to the LLM.
        assert!(!is_undo_phrase("undo the dentist task"));
        assert!(!is_undo_phrase("add undo to the list"));
        assert!(!is_undo_phrase(""));
    }

    #[test]
    fn double_press_first_press_arms_does_not_fire() {
        let now = Instant::now();
        let (next, fire) =
            double_press_decision(None, now, CANCEL_DOUBLE_PRESS_WINDOW);
        assert!(!fire, "a lone first press must not cancel");
        assert_eq!(next, Some(now), "the window must be armed at `now`");
    }

    #[test]
    fn double_press_second_press_within_window_fires() {
        let now = Instant::now();
        let first = now - Duration::from_millis(200);
        let (next, fire) =
            double_press_decision(Some(first), now, CANCEL_DOUBLE_PRESS_WINDOW);
        assert!(fire, "a second press inside the window must cancel");
        assert_eq!(next, None, "firing clears the armed window");
    }

    #[test]
    fn double_press_second_press_after_window_rearms() {
        let now = Instant::now();
        let first = now - (CANCEL_DOUBLE_PRESS_WINDOW + Duration::from_millis(50));
        let (next, fire) =
            double_press_decision(Some(first), now, CANCEL_DOUBLE_PRESS_WINDOW);
        assert!(!fire, "a press after the window lapses re-arms, not cancels");
        assert_eq!(next, Some(now), "the new press opens a fresh window");
    }

    #[test]
    fn double_press_exactly_at_window_edge_fires() {
        let now = Instant::now();
        let first = now - CANCEL_DOUBLE_PRESS_WINDOW;
        let (_, fire) =
            double_press_decision(Some(first), now, CANCEL_DOUBLE_PRESS_WINDOW);
        assert!(fire, "the window bound is inclusive (<=)");
    }

    fn fresh_manager() -> Arc<SessionManager> {
        let audio = AudioHandle::spawn().expect("spawn audio thread for tests");
        Arc::new(SessionManager::new(audio))
    }

    fn fresh_session(id: SessionId) -> Arc<SessionState> {
        Arc::new(SessionState {
            id,
            stage: Mutex::new(Stage::Listening),
            mode: ModeSnapshot::fallback(),
            polishing_enabled: false,
            stt_engine_id: "whisper-local".into(),
            stt_model_id: "test".into(),
            llm_engine_id: "none".into(),
            llm_model_id: "test".into(),
            language: "auto".into(),
            dictionary: Vec::new(),
            context: String::new(),
            cancel: Arc::new(AtomicBool::new(false)),
            started_at: Instant::now(),
            started_at_unix: 0,
            transcript: Mutex::new(None),
            result: Mutex::new(None),
            error: Mutex::new(None),
            warning: Mutex::new(false),
            completed: AtomicBool::new(false),
            completion: Arc::new(Notify::new()),
            speculative_decode: Arc::new(tokio::sync::Mutex::new(())),
            streaming_enabled: false,
            live_transcript_enabled: false,
            auto_paste_enabled: false,
            enable_thinking: false,
            notch_command: false,
            busy_guard: Mutex::new(None),
        })
    }

    #[test]
    fn manager_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Arc<SessionManager>>();
    }

    #[test]
    fn language_directive_appends_for_pinned_language() {
        let out = with_language_directive("BASE PROMPT", "de");
        assert!(out.starts_with("BASE PROMPT"));
        assert!(out.contains("German"));
        assert!(out.to_lowercase().contains("do not translate"));
    }

    #[test]
    fn language_directive_noop_for_auto_and_empty() {
        // `auto` (the default) and blanks rely on the prompt's own
        // detect-and-match rule — no pinned directive appended.
        assert_eq!(with_language_directive("BASE", "auto"), "BASE");
        assert_eq!(with_language_directive("BASE", ""), "BASE");
        assert_eq!(with_language_directive("BASE", "   "), "BASE");
    }

    #[test]
    fn language_directive_noop_for_unknown_code() {
        // An exotic / legacy code we don't name falls back to detection
        // rather than feeding the model a bare code it might misread.
        assert_eq!(with_language_directive("BASE", "xx"), "BASE");
    }

    #[test]
    fn with_user_context_appends_when_enabled_and_nonempty() {
        let out = with_user_context("BASE PROMPT", "I am a backend engineer.", true);
        assert!(out.starts_with("BASE PROMPT"));
        assert!(out.contains("I am a backend engineer."));
    }

    #[test]
    fn with_user_context_omits_when_disabled() {
        // Mode opted out → the prompt is returned untouched even with a
        // non-empty context.
        assert_eq!(
            with_user_context("BASE", "I am a backend engineer.", false),
            "BASE"
        );
    }

    #[test]
    fn with_user_context_omits_when_empty() {
        // Enabled but no context (blank or whitespace-only) → no-op.
        assert_eq!(with_user_context("BASE", "", true), "BASE");
        assert_eq!(with_user_context("BASE", "   ", true), "BASE");
    }

    #[test]
    fn begin_increments_id_and_inserts() {
        let mgr = fresh_manager();
        // Direct manipulation: simulate begin's bookkeeping without
        // an AppHandle. We're validating the in-memory shape, not
        // the IPC side-effects.
        let s1 = fresh_session(1);
        mgr.sessions.lock().insert(s1.id, s1);
        let s2 = fresh_session(2);
        mgr.sessions.lock().insert(s2.id, s2);
        assert_eq!(mgr.pipeline_in_flight_count(), 2);
        let snap = mgr.snapshot();
        assert_eq!(snap.in_progress.len(), 2);
        assert_eq!(snap.in_progress[0].session_id, 1);
        assert_eq!(snap.in_progress[1].session_id, 2);
    }

    #[test]
    fn begin_rejects_when_saturated() {
        let mgr = fresh_manager();
        for i in 1..=MAX_CONCURRENT_SESSIONS as u64 {
            mgr.sessions.lock().insert(i, fresh_session(i));
        }
        assert_eq!(mgr.pipeline_in_flight_count(), MAX_CONCURRENT_SESSIONS);
        // The saturation predicate that begin() uses:
        let saturated = mgr.sessions.lock().len() >= mgr.max_concurrent;
        assert!(saturated, "manager should reject 4th session at the cap");
    }

    #[test]
    fn cancel_foreground_marks_only_active_recording() {
        let mgr = fresh_manager();
        let s1 = fresh_session(1);
        let s2 = fresh_session(2);
        mgr.sessions.lock().insert(s1.id, s1.clone());
        mgr.sessions.lock().insert(s2.id, s2.clone());
        *mgr.active_recording.lock() = Some(2);

        // Simulate the cancel_foreground flow's session.cancel.store
        // step (without an AppHandle). We assert that only the
        // foreground session's flag is set.
        let target = *mgr.active_recording.lock();
        if let Some(id) = target {
            if let Some(s) = mgr.sessions.lock().get(&id).cloned() {
                s.cancel.store(true, Ordering::SeqCst);
            }
        }
        assert!(!s1.cancel.load(Ordering::SeqCst));
        assert!(s2.cancel.load(Ordering::SeqCst));
    }

    #[test]
    fn cancel_session_targets_specific_id() {
        let mgr = fresh_manager();
        let s1 = fresh_session(1);
        let s2 = fresh_session(2);
        let s3 = fresh_session(3);
        mgr.sessions.lock().insert(s1.id, s1.clone());
        mgr.sessions.lock().insert(s2.id, s2.clone());
        mgr.sessions.lock().insert(s3.id, s3.clone());

        // Mirror the cancel_session predicate without an AppHandle.
        let target = 2;
        if let Some(s) = mgr.sessions.lock().get(&target).cloned() {
            s.cancel.store(true, Ordering::SeqCst);
        }
        assert!(!s1.cancel.load(Ordering::SeqCst));
        assert!(s2.cancel.load(Ordering::SeqCst));
        assert!(!s3.cancel.load(Ordering::SeqCst));
    }

    #[test]
    fn active_recording_clears_on_end_recording() {
        let mgr = fresh_manager();
        *mgr.active_recording.lock() = Some(7);
        assert_eq!(mgr.foreground_id(), Some(7));
        // end_recording's first step is to .take() the active_recording slot.
        let taken = mgr.active_recording.lock().take();
        assert_eq!(taken, Some(7));
        assert_eq!(mgr.foreground_id(), None);
    }

    #[test]
    fn snapshot_includes_recent_completed() {
        let mgr = fresh_manager();
        // The snapshot's `recent` field should mirror
        // transcripts::snapshot_completed() — push one and verify.
        let entry = CompletedEntry {
            session_id: 99,
            ts: 1,
            text: "hello".into(),
            raw_text: None,
            mode_id: "general".into(),
            stage: Stage::Done,
            warning: false,
            error_message: None,
        };
        transcripts::push_completed(entry);
        let snap = mgr.snapshot();
        // Don't assert exact length: the ring buffer is process-wide
        // and other tests may have populated it. Just verify our
        // entry made it into the recent list.
        assert!(snap.recent.iter().any(|e| e.session_id == 99));
    }

    #[test]
    fn mode_snapshot_post_processing_lower_case() {
        let snap = ModeSnapshot {
            id: "x".into(),
            name: "x".into(),
            icon: "x".into(),
            system_prompt: String::new(),
            punctuation: PunctuationStyle::Standard,
            case: CaseStyle::Lower,
            max_length_chars: None,
            dictionary: Vec::new(),
            context_enabled: true,
        };
        assert_eq!(snap.apply_post_processing("Hello WORLD"), "hello world");
    }

    #[test]
    fn mode_snapshot_post_processing_truncates() {
        // ModeSnapshot now delegates to modes::apply_post_processing,
        // which uses the sentence-boundary-aware truncate_to_cap (cap
        // is the OUTPUT char count, including a trailing ellipsis when
        // the source has no sentence boundary to cut at).
        let snap = ModeSnapshot {
            id: "x".into(),
            name: "x".into(),
            icon: "x".into(),
            system_prompt: String::new(),
            punctuation: PunctuationStyle::Standard,
            case: CaseStyle::Sentence,
            max_length_chars: Some(5),
            dictionary: Vec::new(),
            context_enabled: true,
        };
        let out = snap.apply_post_processing("abcdefghij");
        assert!(out.chars().count() <= 5, "output {out:?} exceeds cap of 5");
        assert!(out.ends_with('\u{2026}'));
    }

    #[test]
    fn mode_snapshot_post_processing_truncates_at_sentence_boundary() {
        // Sentence boundary fits inside the cap → cut there cleanly,
        // no ellipsis. Mirrors the modes::truncate_to_cap test path.
        let snap = ModeSnapshot {
            id: "x".into(),
            name: "x".into(),
            icon: "x".into(),
            system_prompt: String::new(),
            punctuation: PunctuationStyle::Standard,
            case: CaseStyle::Sentence,
            max_length_chars: Some(20),
            dictionary: Vec::new(),
            context_enabled: true,
        };
        let out = snap.apply_post_processing("Hello world. This is a longer second sentence.");
        assert_eq!(out, "Hello world.");
    }

    #[test]
    fn pill_owner_steal_blocks_old_session_emits() {
        // Regression guard for the streaming-bleed bug: when session B
        // begins while session A's pipeline is still polishing, A's
        // emit-time `owns_pill_emit(A.id)` must flip to false the
        // moment B claims the pill_owner slot. The pipeline reads this
        // predicate at every emit site so the bleed is impossible.
        let mgr = fresh_manager();
        let a = fresh_session(1);
        let b = fresh_session(2);
        mgr.sessions.lock().insert(a.id, a.clone());

        // Session A claims the pill (mirrors the begin() path).
        *mgr.pill_owner.lock() = Some(a.id);
        assert!(mgr.owns_pill_emit(a.id));
        assert!(!mgr.owns_pill_emit(b.id));

        // Session B begins → steals the pill_owner slot.
        mgr.sessions.lock().insert(b.id, b.clone());
        *mgr.pill_owner.lock() = Some(b.id);

        // Now any in-flight emit from A is gated off, B's are gated on.
        assert!(!mgr.owns_pill_emit(a.id));
        assert!(mgr.owns_pill_emit(b.id));

        // After B finalises and releases ownership, neither session
        // should drive the pill (no stale pointer for the next begin).
        *mgr.pill_owner.lock() = None;
        assert!(!mgr.owns_pill_emit(a.id));
        assert!(!mgr.owns_pill_emit(b.id));
    }

    #[test]
    fn pill_hide_yields_to_newer_owner() {
        // The owner-check predicate that gates schedule_pill_hide_unless_taken's
        // delayed hide. A stale timer scheduled by the previous session must
        // not hide the pill once a newer session has claimed ownership.
        let prev: SessionId = 1;
        let next: SessionId = 2;

        // No owner: the previous session's timer is the only one in flight,
        // so hiding is safe.
        assert!(should_hide_pill(None, prev));

        // Same owner: previous session is still the rightful owner — hide.
        assert!(should_hide_pill(Some(prev), prev));

        // Different (newer) owner: a fresh recording has claimed the pill;
        // the stale timer must NOT hide it.
        assert!(!should_hide_pill(Some(next), prev));
    }

    #[test]
    fn pill_state_restore_skips_terminal_stages() {
        // The post-busy-toast restore re-emits the live stage so the
        // pill drops the saturated/warning treatment. Terminal stages
        // already painted their own treatment with the right message;
        // re-emitting from here would clobber it with a None message.
        assert!(should_restore_pill_state(Stage::Listening));
        assert!(should_restore_pill_state(Stage::Transcribing));
        assert!(should_restore_pill_state(Stage::Polishing));
        assert!(!should_restore_pill_state(Stage::Done));
        assert!(!should_restore_pill_state(Stage::Error));
        assert!(!should_restore_pill_state(Stage::Canceled));
    }

    #[test]
    fn max_record_guard_drops_after_session_complete() {
        // The max-record guard captures Arc<SessionState>; without
        // the completion-Notify wakeup it would hold the Arc for the
        // full 30-min cap. We mark_completed() before the guard's
        // sleep elapses and assert the guard's strong reference is
        // released within a tick.
        let session = fresh_session(42);
        let strong_before = Arc::strong_count(&session);

        // Spin up a worker that waits on the same Notify the guard
        // would. We assert the wait-and-drop sequence in lockstep
        // with the production guard, without needing a tokio runtime
        // for a 30-minute sleep.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("tokio rt");
        let session_for_block = session.clone();
        rt.block_on(async move {
            let captured = session_for_block.clone();
            let completion = captured.completion.clone();
            // captured is moved into the spawned task — mirrors the
            // production guard which captures the Arc by move.
            let task = tokio::spawn(async move {
                if captured.completed.load(Ordering::SeqCst) {
                    return;
                }
                tokio::select! {
                    biased;
                    _ = completion.notified() => {}
                    _ = tokio::time::sleep(Duration::from_secs(60)) => {
                        panic!("guard slept to deadline; mark_completed never woke it");
                    }
                }
            });

            // Signal completion before the guard's sleep elapses.
            tokio::time::sleep(Duration::from_millis(10)).await;
            session_for_block.mark_completed();
            task.await.expect("guard task must complete");
        });

        // After the spawned task drops its Arc, only `session` itself
        // holds the strong reference — strong_count must return to
        // its starting value.
        assert_eq!(
            Arc::strong_count(&session),
            strong_before,
            "guard task must drop its Arc<SessionState> when completion fires"
        );
    }

    #[test]
    fn busy_gate_acquire_blocks_second_attempt() {
        // Gate starts idle: first acquire succeeds, second is rejected.
        // Dropping the first guard returns the gate to idle so the next
        // acquire succeeds again. Explicit release() before drop is a
        // no-op on the second invocation (idempotency check).
        let gate = BusyGate::new();
        assert_eq!(gate.in_flight(), 0);

        let g1 = gate.try_acquire().expect("first acquire must succeed");
        assert_eq!(gate.in_flight(), 1);

        assert!(
            gate.try_acquire().is_none(),
            "second acquire while first is held must return None"
        );
        assert_eq!(
            gate.in_flight(),
            1,
            "rejected acquire must not bump the counter"
        );

        // Explicit release before Drop — gate goes back to idle.
        g1.release();
        assert_eq!(gate.in_flight(), 0);

        // Drop the (already-released) guard: still idempotent, count
        // doesn't underflow.
        drop(g1);
        assert_eq!(gate.in_flight(), 0);

        // Next acquire works.
        let g2 = gate.try_acquire().expect("post-release acquire must succeed");
        drop(g2);
        assert_eq!(gate.in_flight(), 0);
    }

    #[test]
    fn busy_gate_drop_decrements() {
        // The Drop side of the RAII pattern releases the gate when the
        // guard is dropped without an explicit release. Used by the
        // success-path PipelineGuard at the tail of run_pipeline.
        let gate = BusyGate::new();
        {
            let _g = gate.try_acquire().expect("acquire");
            assert_eq!(gate.in_flight(), 1);
            assert!(gate.try_acquire().is_none());
        }
        assert_eq!(
            gate.in_flight(),
            0,
            "guard's Drop must decrement the gate"
        );
        assert!(gate.try_acquire().is_some(), "gate must accept after drop");
    }

    #[test]
    fn manager_busy_gate_rejects_concurrent_acquire() {
        // Lifecycle test on the manager-owned gate: catches the
        // regression where a future refactor forgets to wire
        // busy_guard into SessionState (the gate would never get
        // populated) or adds an early-return between gate-acquire and
        // SessionState construction without dropping the guard (the
        // gate would stay locked forever and every subsequent press
        // would be rejected).
        let mgr = fresh_manager();
        assert_eq!(mgr.busy_gate.in_flight(), 0);

        // First acquire admits.
        let g1 = mgr
            .busy_gate
            .try_acquire()
            .expect("first manager-gate acquire must succeed on idle gate");
        assert_eq!(mgr.busy_gate.in_flight(), 1);

        // Concurrent acquire is rejected and does not bump the counter.
        assert!(
            mgr.busy_gate.try_acquire().is_none(),
            "concurrent acquire on the manager's busy_gate must return None"
        );
        assert_eq!(
            mgr.busy_gate.in_flight(),
            1,
            "rejected acquire must not bump the counter"
        );

        // Eager release (mirrors the cancel_session / audio-rollback
        // path) frees the gate without waiting for Drop.
        g1.release();
        assert_eq!(mgr.busy_gate.in_flight(), 0);
        let g2 = mgr
            .busy_gate
            .try_acquire()
            .expect("post-release acquire must succeed");
        assert_eq!(mgr.busy_gate.in_flight(), 1);

        // Drop-only release: leaving scope without an explicit
        // release() must also return the gate to idle (the
        // success-path PipelineGuard relies on this).
        drop(g2);
        assert_eq!(
            mgr.busy_gate.in_flight(),
            0,
            "guard's Drop must decrement the manager's busy_gate"
        );
        assert!(
            mgr.busy_gate.try_acquire().is_some(),
            "manager's gate must admit again after the previous guard was dropped"
        );
    }

    #[test]
    fn mark_completed_is_idempotent() {
        // Multiple terminal paths in run_pipeline can both call
        // mark_completed (e.g. set_stage's terminal block plus the
        // PipelineGuard's Drop). The second call must not panic the
        // Notify nor double-fire waiters.
        let s = fresh_session(1);
        s.mark_completed();
        s.mark_completed();
        s.mark_completed();
        assert!(s.completed.load(Ordering::SeqCst));
    }
}
