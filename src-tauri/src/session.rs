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

/// How long a finalised session sticks around in the manager's map so
/// the popover briefly shows its terminal stage before it folds into
/// "RECENT".
const TERMINAL_LINGER_MS: u64 = 1500;

/// Toast duration for the saturated state.
const SATURATED_TOAST_MS: u32 = 1500;

/// Toast duration for the "pipeline still in flight" rejection. Slightly
/// shorter than the saturation toast — the user is mid-keystroke and we
/// just want a brief acknowledgement that the press was seen but ignored.
const BUSY_TOAST_MS: u32 = 1500;

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
    /// Resolved at session start: per-mode override falls back to the global `Settings` value.
    pub enable_thinking: bool,
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
    Saturated { in_flight: usize },
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
    /// Single-pipeline admission gate. `try_acquire` is the one
    /// admission point — a press while a previous pipeline is still
    /// in its post-recording stages (transcribe / polish / clipboard)
    /// gets `None` back and surfaces a brief pill toast. Release
    /// happens via the RAII guard parked on `SessionState::busy_guard`
    /// (Drop) plus eager `release()` calls in `cancel_session` and the
    /// audio-rollback branch of `begin`.
    pub busy_gate: BusyGate,
}

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
            busy_gate: BusyGate::new(),
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
                pill::show(app);
                pill::emit_toast(
                    app,
                    "Still finishing previous".into(),
                    BUSY_TOAST_MS,
                );
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
                return BeginOutcome::Saturated { in_flight };
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
            streaming_enabled: settings::load(app).streaming_pill_enabled,
            enable_thinking,
            busy_guard: Mutex::new(Some(busy_guard)),
        });

        // Insert into map BEFORE starting audio so the saturation
        // check is monotonic and the snapshot shows the new session
        // immediately. If audio fails we'll remove + reset below.
        self.sessions.lock().insert(new_sid, session.clone());
        *active = Some(new_sid);
        // Take ownership of the pill — replaces any previous session
        // whose pipeline is still running in the background, so the
        // pill always tracks the newest recording.
        *self.pill_owner.lock() = Some(new_sid);
        drop(active);

        // Arm the OS Escape grab now (and not at app launch) so
        // Escape only short-circuits to Vox while a dictation session
        // is in flight. The disarm fires from end_recording's drop
        // path or from cancel.
        crate::hotkey::arm_cancel(app);

        log::info!("▶ begin recording (sid={new_sid})");
        pill::show(app);
        pill::emit_state(app, PillState::Listening, None);
        emit_sessions_changed(app, self);

        if let Some(m) = active_mode.as_ref() {
            pill::emit_active_mode(app, m);
        }
        pill::emit_mode_suggestion(app, None);

        if prefs.auto_suggest_modes {
            spawn_suggestion_probe(app.clone(), prefs.active_mode_id.clone());
        }

        if let Err(e) = self.audio.start() {
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
            pill::emit_state(
                app,
                PillState::Error,
                Some("No mic access — grant in System Settings.".into()),
            );
            self.schedule_pill_hide_unless_taken(app, new_sid, 3000);
            emit_sessions_changed(app, self);
            return BeginOutcome::AudioError(e.to_string());
        }

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

        // Disarm Escape grab the moment recording stops; another
        // session may still be running but its cancel uses the
        // popover X (no global key grab). The next begin re-arms.
        crate::hotkey::disarm_cancel(&app);

        let session = match self.sessions.lock().get(&sid).cloned() {
            Some(s) => s,
            None => {
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
                // Drain the audio buffer regardless so the next start()
                // reaches `Some(cap)`-less state cleanly.
                let _ = manager.audio.stop_and_encode();
                Ok(None)
            } else {
                match manager.audio.stop_and_encode() {
                    Ok(buf) => Ok(Some(buf)),
                    Err(e) => Err(format!("{e}")),
                }
            };

            run_pipeline(app_clone, manager, session, wav_result).await;
        });
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

        // If this is the foreground recording, tear down the audio
        // capture and the Escape grab. The pipeline task picks up the
        // cancel flag and emits Canceled.
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
            // Drain the audio buffer so cpal's input stream is freed.
            let _ = self.audio.stop_and_encode();
            crate::hotkey::disarm_cancel(app);
            // Snap the pill back to compact in case streaming widened it.
            pill::set_wide(app, false);
            pill::emit_state(app, PillState::Canceled, None);
            self.schedule_pill_hide_unless_taken(app, id, 600);
        }

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

    /// Last-finished-wins clipboard write, serialised across pipelines.
    pub fn write_clipboard<R: Runtime>(
        self: &Arc<Self>,
        _app: &AppHandle<R>,
        _id: SessionId,
        text: &str,
    ) -> Result<(), String> {
        let _g = self.clipboard_lock.lock();
        inject::copy_to_clipboard(text).map_err(|e| e.to_string())
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
    }
    impl PipelineGuard {
        fn new(session: Arc<SessionState>) -> Self {
            Self { finalised: false, session }
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
        }
    }
    let mut _guard = PipelineGuard::new(session.clone());

    if session.is_canceled() {
        log::info!("session {id} canceled before pipeline ran");
        manager.set_stage(&app, id, Stage::Canceled, None, false);
        _guard.finalise();
        return;
    }

    let wav = match wav_result {
        Ok(Some(buf)) => buf,
        Ok(None) => {
            log::info!("session {id} too short, skipping STT");
            // Silent skip: pill goes back to hidden.
            pill::hide(&app);
            manager.sessions.lock().remove(&id);
            emit_sessions_changed(&app, &manager);
            _guard.finalise();
            return;
        }
        Err(e) => {
            log::error!("audio stop/encode failed: {e}");
            *session.error.lock() = Some(e);
            manager.set_stage(
                &app,
                id,
                Stage::Error,
                Some("Audio error.".into()),
                false,
            );
            _guard.finalise();
            return;
        }
    };

    if wav.len() < 44 + 16_000 * 2 / 4 {
        log::info!("recording too short ({} bytes), skipping STT", wav.len());
        pill::hide(&app);
        manager.sessions.lock().remove(&id);
        emit_sessions_changed(&app, &manager);
        _guard.finalise();
        return;
    }

    if wav_peak_pcm16(&wav) < SILENCE_PEAK_THRESHOLD {
        log::info!("session {id} silent buffer; skipping STT");
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
            manager.set_stage(
                &app,
                id,
                Stage::Error,
                Some("STT engine not configured. Open Settings → Models.".into()),
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
    let streaming_enabled = session.streaming_enabled;
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
        // Block on any in-flight speculative decode before starting
        // the authoritative pass — both share the same WhisperContext
        // and `WhisperState::full` is not safe to drive concurrently.
        // Awaiting the lock here is what realises the speculative
        // savings: the speculative pass already chewed through ~all of
        // the audio captured before the user released the hotkey, so
        // by the time this `lock()` returns the only audio still to
        // decode here is whatever was captured between the last
        // speculative snapshot and end-of-recording.
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
                    pill::hide(&app);
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
        manager.set_stage(&app, id, Stage::Canceled, None, false);
        _guard.finalise();
        return;
    }

    let mode_system_prompt = session.mode.system_prompt.clone();
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
    let app_for_copy = app.clone();
    let copy_result = tauri::async_runtime::spawn_blocking(move || {
        manager_for_copy.write_clipboard(&app_for_copy, id, &copy_text)
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

    let warning_flag = polish_warning.is_some();
    let entry = CompletedEntry {
        session_id: id,
        ts: chrono::Utc::now().timestamp(),
        text: cleaned.clone(),
        mode_id: session.mode.id.clone(),
        stage: Stage::Done,
        warning: warning_flag,
        error_message: None,
    };

    match copy_result {
        Ok(Ok(())) => {
            transcripts::push_completed(entry);
            // Legacy event keeps the existing tray-popover RECENT
            // logic working for any consumer that hasn't migrated.
            let _ = app.emit("transcripts:changed", transcripts::snapshot());

            // Snap the pill back to its compact width before set_stage
            // emits the Done state so the success treatment renders
            // at 144px.
            if stream_to_pill() {
                pill::set_wide(&app, false);
            }

            match polish_warning {
                Some(msg) => {
                    manager.set_stage(
                        &app,
                        id,
                        Stage::Done,
                        Some(msg.into()),
                        true,
                    );
                }
                None => {
                    manager.set_stage(
                        &app,
                        id,
                        Stage::Done,
                        Some("Copied".into()),
                        false,
                    );
                }
            }
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
                Some("Copy failed.".into()),
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
                Some("Copy failed.".into()),
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
    let body = wav.get(44..).unwrap_or(&[]);
    let mut peak = 0i16;
    for chunk in body.chunks_exact(2) {
        let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
        let abs = sample.saturating_abs();
        if abs > peak {
            peak = abs;
        }
    }
    peak as f32 / i16::MAX as f32
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
                    manager.end_recording(app);
                }
            }
        }
    });
}

/// Pre-emptively decode the in-progress capture buffer while the user
/// is still speaking. Throttling rules:
///   * Don't start the first speculative pass until the buffer holds at
///     least [`SPEC_MIN_FIRST_SECS`] of audio (otherwise we burn cycles
///     on a tiny utterance the user is about to release the hotkey on).
///   * Each subsequent pass needs the buffer to have grown by at least
///     [`SPEC_MIN_DELTA_SECS`] vs the snapshot the previous pass started
///     from.
///   * Only one Whisper decode runs at a time; while one is in flight
///     the next iteration just polls. We don't queue — when the
///     in-flight pass finishes we re-evaluate against the *current*
///     buffer (so we always work on the latest data, not stale).
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
    const SPEC_MIN_FIRST_SECS: f32 = 2.0;
    /// Minimum new audio (vs the last snapshot we decoded) before we
    /// kick off another speculative pass.
    const SPEC_MIN_DELTA_SECS: f32 = 1.5;
    /// Polling cadence — small enough that cancel/stop reacts within
    /// ~one tick, large enough that we don't pin a CPU.
    const SPEC_POLL_MS: u64 = 100;
    /// Only run speculative decoding for whisper-local. Other STT
    /// engines either don't exist yet or wouldn't benefit from this
    /// shape of pre-emptive call.
    const ELIGIBLE_STT_ENGINE: &str = "whisper-local";

    if session.stt_engine_id != ELIGIBLE_STT_ENGINE {
        return;
    }

    // Battery-aware bail: speculative decoding runs full Whisper passes
    // on a 1.5s cadence while the user is still speaking, which is
    // measurable battery drain on laptops on the unplugged path. When
    // the Mac is unplugged we fall back to the original "decode entirely
    // after hotkey release" behaviour. AC adapter, UPS, and any desktop
    // (no battery at all) keep the streaming pass. Single check at
    // task-start is sufficient — session lifetime is short and a user
    // (un)plugging mid-utterance is a non-event.
    if mac_power::on_battery() {
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
        let streaming_enabled_at_begin = session.streaming_enabled;
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
                SPEC_MIN_FIRST_SECS
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
    let audio = manager.audio.clone();
    tauri::async_runtime::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_millis(50));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            if manager.foreground_id() != Some(id) {
                break;
            }
            let level = audio.current_rms();
            pill::emit_audio_level(&app, level);
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

const SATURATED_TOAST_MS_PUB: u32 = SATURATED_TOAST_MS;

/// Helper for the hotkey layer: emit a saturated toast through the pill.
pub fn emit_saturated_toast<R: Runtime>(app: &AppHandle<R>, in_flight: usize, max: usize) {
    pill::show(app);
    let msg = format!("Too many in flight ({in_flight}/{max})");
    pill::emit_toast(app, msg, SATURATED_TOAST_MS_PUB);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_manager() -> Arc<SessionManager> {
        let audio = AudioHandle::spawn();
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
            enable_thinking: false,
            busy_guard: Mutex::new(None),
        })
    }

    #[test]
    fn manager_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Arc<SessionManager>>();
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
