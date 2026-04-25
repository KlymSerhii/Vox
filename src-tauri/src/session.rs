//! Orchestrates the dictation pipeline.
//!
//! Flow: hotkey down → show pill (Listening) → mic capture + emit levels.
//! Hotkey up → stop mic → STT → optional LLM cleanup → paste → Done pill.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tauri::{AppHandle, Runtime};

use crate::audio::AudioHandle;
use crate::pill::{self, PillState};
use crate::{inject, llm, settings, stt, transcripts};

/// Max continuous hold before we auto-stop (safety net).
const MAX_RECORD_SECS: u64 = 60;

/// Minimum recording duration we'll consider valid (misfire guard).
/// PRD §3.1: <300 ms is treated as a misfire.
const MIN_RECORD_MS: u128 = 300;

/// Threshold (peak amplitude in [0,1]) below which we treat the buffer as
/// silence and surface "No audio detected" instead of sending it to STT.
/// -50 dBFS ≈ 0.003.
const SILENCE_PEAK_THRESHOLD: f32 = 0.003;

#[derive(Clone)]
pub struct SessionCtx {
    pub audio: AudioHandle,
    pub recording: Arc<AtomicBool>,
    /// Monotonically incremented on each session; used to cancel stale work.
    pub session_id: Arc<AtomicU64>,
    /// When the current recording started (Some iff recording).
    started_at: Arc<Mutex<Option<Instant>>>,
    /// Per-session boolean flipped by `cancel()` while the current pipeline
    /// is in flight. Reset to false at the start of every new session so a
    /// stale cancel from a previous session can't bleed into a fresh one.
    canceled: Arc<AtomicBool>,
    /// True from begin_recording until the post-release pipeline has either
    /// pasted, errored, or been canceled. Lets `cancel()` distinguish "Esc
    /// during a real session" from "Esc with nothing in flight" so we don't
    /// flash a Canceled pill when there's nothing to cancel.
    pipeline_active: Arc<AtomicBool>,
}

impl SessionCtx {
    pub fn new(audio: AudioHandle) -> Self {
        Self {
            audio,
            recording: Arc::new(AtomicBool::new(false)),
            session_id: Arc::new(AtomicU64::new(0)),
            started_at: Arc::new(Mutex::new(None)),
            canceled: Arc::new(AtomicBool::new(false)),
            pipeline_active: Arc::new(AtomicBool::new(false)),
        }
    }

    #[allow(dead_code)]
    pub fn is_recording(&self) -> bool {
        self.recording.load(Ordering::SeqCst)
    }

    pub fn is_canceled(&self) -> bool {
        self.canceled.load(Ordering::SeqCst)
    }

    pub fn is_pipeline_active(&self) -> bool {
        self.pipeline_active.load(Ordering::SeqCst)
    }
}

pub fn begin_recording<R: Runtime>(app: AppHandle<R>, ctx: &SessionCtx) {
    // Cheap, secret-free pref read — the keychain is *not* hit on the
    // hotkey thread (latency budget per PRD §6.1).
    let prefs = settings::prefs(&app);
    if !prefs.dictation_enabled {
        log::info!("dictation is disabled; hotkey press ignored");
        return;
    }
    if !prefs.send_audio_to_cloud {
        log::info!("'send audio to cloud' is off; dictation is unavailable");
        pill::show(&app);
        pill::emit_state(
            &app,
            PillState::Error,
            Some("Cloud dictation is off — enable it in Privacy.".into()),
        );
        let app_arc = Arc::new(app.clone());
        pill::hide_after(&app_arc, 2500);
        return;
    }

    // Note: we do *not* notify_one() here. The cancel_notify is reserved for
    // explicit user cancels. Re-press supersession is observed by in-flight
    // tasks via the session_id mismatch checked at every await boundary —
    // they'll bail in at most one HTTP RTT, which is acceptable. Sending a
    // notify here would also leave a stale permit on the Notify that the
    // *new* session's pipeline could mistakenly consume as a cancel.

    if ctx.recording.swap(true, Ordering::SeqCst) {
        log::debug!("begin_recording called while already recording; ignored");
        return;
    }

    let new_sid = ctx.session_id.fetch_add(1, Ordering::SeqCst) + 1;
    ctx.canceled.store(false, Ordering::SeqCst);
    ctx.pipeline_active.store(true, Ordering::SeqCst);
    *ctx.started_at.lock() = Some(Instant::now());

    log::info!("▶ begin recording (sid={new_sid})");
    pill::show(&app);
    pill::emit_state(&app, PillState::Listening, None);

    if let Err(e) = ctx.audio.start() {
        log::error!("failed to start audio: {e}");
        ctx.recording.store(false, Ordering::SeqCst);
        ctx.pipeline_active.store(false, Ordering::SeqCst);
        *ctx.started_at.lock() = None;
        pill::emit_state(
            &app,
            PillState::Error,
            Some("No mic access — grant in System Settings.".into()),
        );
        let app_arc = Arc::new(app.clone());
        pill::hide_after(&app_arc, 3000);
        return;
    }

    // Start a safety-net watcher that auto-ends the session if the user
    // holds the key past MAX_RECORD_SECS.
    spawn_max_record_guard(app.clone(), ctx.clone(), MAX_RECORD_SECS);

    // Start an RMS sampler that emits pill:audio_level events at ~20 Hz.
    spawn_audio_level_sampler(app, ctx.clone());
}

pub fn end_recording<R: Runtime>(app: AppHandle<R>, ctx: SessionCtx) {
    if !ctx.recording.swap(false, Ordering::SeqCst) {
        log::debug!("end_recording called without an active recording; ignored");
        return;
    }
    let started = ctx.started_at.lock().take();
    let my_sid = ctx.session_id.load(Ordering::SeqCst);
    log::info!("■ end recording (sid={my_sid})");

    let too_short = started
        .map(|t| t.elapsed().as_millis() < MIN_RECORD_MS)
        .unwrap_or(false);

    let app_for_pill = app.clone();

    // Move stop + upload + paste off the hotkey callback thread.
    tauri::async_runtime::spawn(async move {
        // Drop guard clears `pipeline_active` no matter which return path
        // the task takes (cancel, stale, error, or success). This lets
        // `cancel()` distinguish "Esc with a real session in flight" from
        // "Esc with nothing happening" so we don't flash a Canceled pill
        // when the user presses Esc with no session running.
        let _pipeline_guard = PipelineGuard::new(&ctx, my_sid);

        // Always fetch audio to flush it, even if we'll discard it.
        let wav_result = ctx.audio.stop_and_encode();

        // Bail-out helper — used at every async boundary in the pipeline so
        // a cancel or a fresh-press takes effect within ~one I/O round trip.
        let stale = || ctx.session_id.load(Ordering::SeqCst) != my_sid;

        if ctx.is_canceled() {
            log::info!("session {my_sid} canceled; discarding audio");
            pill::emit_state(&app_for_pill, PillState::Canceled, None);
            hide_after_session(&app_for_pill, &ctx, my_sid, 600);
            return;
        }
        if stale() {
            log::info!("session {my_sid} superseded; abandoning silently");
            return;
        }

        if too_short {
            log::info!("recording too short (<{MIN_RECORD_MS} ms), skipping");
            pill::hide(&app_for_pill);
            return;
        }

        let wav = match wav_result {
            Ok(w) => w,
            Err(e) => {
                log::error!("audio stop/encode failed: {e}");
                pill::emit_state(
                    &app_for_pill,
                    PillState::Error,
                    Some("Audio error.".into()),
                );
                hide_after_session(&app_for_pill, &ctx, my_sid, 2500);
                return;
            }
        };

        if wav.len() < 44 + 16_000 * 2 / 4 {
            log::info!("recording too short ({} bytes), skipping STT", wav.len());
            pill::hide(&app_for_pill);
            return;
        }

        // PRD §3.2: if the buffer is silent, surface "No audio detected"
        // rather than burning an STT request.
        if wav_peak_pcm16(&wav) < SILENCE_PEAK_THRESHOLD {
            log::info!("silent buffer; skipping STT");
            pill::emit_state(
                &app_for_pill,
                PillState::Error,
                Some("No audio detected — check your mic.".into()),
            );
            hide_after_session(&app_for_pill, &ctx, my_sid, 2500);
            return;
        }

        // Read full settings (incl. keychain keys) only here — off the hotkey
        // thread.
        let groq_key = settings::get_groq_key(&app_for_pill);
        let anthropic_key = settings::get_anthropic_key(&app_for_pill);
        let prefs = settings::prefs(&app_for_pill);

        pill::emit_state(&app_for_pill, PillState::Transcribing, None);

        let transcript = {
            let stt_fut = stt::transcribe_wav(wav, &groq_key, &prefs.dictionary);
            // Race the HTTP call against a 50 ms-tick cancel watcher. The
            // watcher wakes the select whenever `canceled` flips or the
            // session id moves, so cancel takes effect within ~50 ms even
            // if the HTTP call would otherwise hang for the full timeout.
            tokio::select! {
                biased;
                () = wait_for_cancel(&ctx, my_sid) => {
                    log::info!("STT canceled / superseded; dropping request");
                    if ctx.is_canceled() && !stale() {
                        pill::emit_state(&app_for_pill, PillState::Canceled, None);
                        hide_after_session(&app_for_pill, &ctx, my_sid, 600);
                    }
                    return;
                }
                result = stt_fut => match result {
                    Ok(text) if !text.trim().is_empty() => text,
                    Ok(_) => {
                        log::info!("empty transcript, nothing to paste");
                        pill::hide(&app_for_pill);
                        return;
                    }
                    Err(e) => {
                        log::error!("STT failed: {e}");
                        pill::emit_state(
                            &app_for_pill,
                            PillState::Error,
                            Some(friendly_stt_error(&e)),
                        );
                        hide_after_session(&app_for_pill, &ctx, my_sid, 3000);
                        return;
                    }
                }
            }
        };

        if ctx.is_canceled() || stale() {
            log::info!("post-STT cancel/supersede; aborting before LLM");
            if ctx.is_canceled() && !stale() {
                pill::emit_state(&app_for_pill, PillState::Canceled, None);
                hide_after_session(&app_for_pill, &ctx, my_sid, 600);
            }
            return;
        }

        // Decide whether to run LLM cleanup.
        let cleaned = if prefs.cleanup_enabled && !anthropic_key.trim().is_empty() {
            pill::emit_state(&app_for_pill, PillState::Polishing, None);
            let llm_fut = llm::cleanup(&transcript, &anthropic_key, &prefs.dictionary);
            tokio::select! {
                biased;
                () = wait_for_cancel(&ctx, my_sid) => {
                    log::info!("LLM canceled / superseded; dropping request");
                    if ctx.is_canceled() && !stale() {
                        pill::emit_state(&app_for_pill, PillState::Canceled, None);
                        hide_after_session(&app_for_pill, &ctx, my_sid, 600);
                    }
                    return;
                }
                result = llm_fut => match result {
                    Ok(t) => t,
                    Err(llm::LlmError::TooShort) => transcript.clone(),
                    Err(e) => {
                        log::warn!("LLM cleanup failed: {e}; falling back to raw transcript");
                        transcript.clone()
                    }
                }
            }
        } else {
            transcript.clone()
        };

        if ctx.is_canceled() || stale() {
            log::info!("post-LLM cancel/supersede; aborting before paste");
            if ctx.is_canceled() && !stale() {
                pill::emit_state(&app_for_pill, PillState::Canceled, None);
                hide_after_session(&app_for_pill, &ctx, my_sid, 600);
            }
            return;
        }

        log::info!("pasting {} chars (sid={my_sid})", cleaned.len());

        // Record before paste (paste may fail but we still want the history).
        transcripts::push(&cleaned);

        let t = cleaned.clone();
        let paste_result =
            tauri::async_runtime::spawn_blocking(move || inject::paste_text(&t)).await;

        // Last cancel check — by now the user has the text on their
        // clipboard regardless, but suppress the Done state if we were
        // canceled at the last moment.
        if ctx.is_canceled() || stale() {
            return;
        }

        match paste_result {
            Ok(Ok(())) => {
                pill::emit_state(&app_for_pill, PillState::Done, None);
                hide_after_session(&app_for_pill, &ctx, my_sid, 700);
            }
            Ok(Err(e)) => {
                log::error!("paste failed: {e}");
                pill::emit_state(
                    &app_for_pill,
                    PillState::Error,
                    Some("Copied — press Cmd+V to paste.".into()),
                );
                hide_after_session(&app_for_pill, &ctx, my_sid, 3000);
            }
            Err(e) => {
                log::error!("paste task panicked: {e}");
                pill::emit_state(&app_for_pill, PillState::Error, Some("Paste error.".into()));
                hide_after_session(&app_for_pill, &ctx, my_sid, 2500);
            }
        }
    });
}

/// User triggered the cancel hotkey. Aborts whatever phase we're in:
/// recording, transcribing, polishing, or just-before-paste.
pub fn cancel<R: Runtime>(app: AppHandle<R>, ctx: SessionCtx) {
    // No-op if there's nothing in flight. Without this guard, pressing the
    // cancel hotkey with no session running would flash a stray "Canceled"
    // pill, since the global hotkey listener fires unconditionally.
    if !ctx.is_pipeline_active() {
        log::debug!("cancel: no active pipeline; ignoring");
        return;
    }

    // Flip the flag — any in-flight pipeline observes it within one cancel
    // tick (50 ms) via `wait_for_cancel`.
    ctx.canceled.store(true, Ordering::SeqCst);

    if ctx.recording.load(Ordering::SeqCst) {
        // Mid-recording: end_recording will see canceled=true and emit the
        // Canceled pill while dropping the audio.
        end_recording(app, ctx);
    } else {
        // Mid-pipeline: the in-flight task's wait_for_cancel poll will
        // observe `canceled=true` and drop the HTTP future. Flash the pill
        // for instant feedback.
        pill::emit_state(&app, PillState::Canceled, None);
        let app_arc = Arc::new(app.clone());
        pill::hide_after(&app_arc, 600);
    }
}

/// RAII guard that clears `pipeline_active` when the post-release pipeline
/// task ends, however it ends — but only if our session id is still the
/// active one. A re-press supersedes the old session and starts a new
/// pipeline; the *old* task's guard must not clobber the new task's flag.
struct PipelineGuard {
    flag: Arc<AtomicBool>,
    session_id: Arc<AtomicU64>,
    sid: u64,
}

impl PipelineGuard {
    fn new(ctx: &SessionCtx, sid: u64) -> Self {
        Self {
            flag: ctx.pipeline_active.clone(),
            session_id: ctx.session_id.clone(),
            sid,
        }
    }
}

impl Drop for PipelineGuard {
    fn drop(&mut self) {
        if self.session_id.load(Ordering::SeqCst) == self.sid {
            self.flag.store(false, Ordering::SeqCst);
        }
    }
}

/// Future that resolves when the active session changes (re-press) or a
/// cancel is signaled for our session id. Used to race against in-flight
/// HTTP calls so cancel takes effect quickly without a per-session async
/// channel allocation.
async fn wait_for_cancel(ctx: &SessionCtx, my_sid: u64) {
    loop {
        // Check first so a cancel that fired before we were polled is
        // observed immediately.
        if ctx.session_id.load(Ordering::SeqCst) != my_sid
            || ctx.canceled.load(Ordering::SeqCst)
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Schedule a `pill::hide` after `ms`, but only if our session id is still
/// the active one when the timer fires. Avoids hiding a fresh pill if the
/// user re-pressed the hotkey during the timeout.
fn hide_after_session<R: Runtime>(
    app: &AppHandle<R>,
    ctx: &SessionCtx,
    sid: u64,
    ms: u64,
) {
    let app_clone = app.clone();
    let session_id = ctx.session_id.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(ms)).await;
        if session_id.load(Ordering::SeqCst) == sid {
            pill::hide(&app_clone);
        }
    });
}

/// Approximate peak amplitude over a PCM16 WAV byte buffer (after the 44-byte
/// header). Returns a value in [0, 1]. Cheap — used only as a "is this all
/// silence?" check.
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

fn friendly_stt_error(e: &stt::SttError) -> String {
    match e {
        stt::SttError::MissingKey => "Add your Groq key in Settings.".into(),
        stt::SttError::Network(_) => "Offline — Vox needs internet.".into(),
        stt::SttError::Api { status, .. } if *status == 401 => "Groq key rejected.".into(),
        stt::SttError::Api { .. } => "Transcription failed.".into(),
        stt::SttError::Parse(_) => "Unexpected response from Groq.".into(),
    }
}

fn spawn_max_record_guard<R: Runtime>(
    app: AppHandle<R>,
    ctx: SessionCtx,
    max_secs: u64,
) {
    let sid = ctx.session_id.load(Ordering::SeqCst);
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(max_secs)).await;
        if ctx.session_id.load(Ordering::SeqCst) == sid
            && ctx.recording.load(Ordering::SeqCst)
        {
            log::warn!("hotkey held >{max_secs}s — auto-stopping");
            end_recording(app, ctx);
        }
    });
}

fn spawn_audio_level_sampler<R: Runtime>(app: AppHandle<R>, ctx: SessionCtx) {
    let sid = ctx.session_id.load(Ordering::SeqCst);
    let audio = ctx.audio.clone();
    tauri::async_runtime::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_millis(50));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            if ctx.session_id.load(Ordering::SeqCst) != sid
                || !ctx.recording.load(Ordering::SeqCst)
            {
                break;
            }
            let level = audio.current_rms();
            pill::emit_audio_level(&app, level);
        }
    });
}
