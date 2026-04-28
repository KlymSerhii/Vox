//! Orchestrates the dictation pipeline: mic capture → STT → optional LLM
//! cleanup → clipboard. STT/LLM engines come from the per-app
//! `engines::Registry`; both are lazy-loaded and kept warm.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tauri::{AppHandle, Emitter, Manager, Runtime, Wry};

use crate::audio::AudioHandle;
use crate::engines::Registry;
use crate::modes::{self, ModeStore, VoiceMode};
use crate::pill::{self, PillState};
use crate::{inject, settings, transcripts};

const MAX_RECORD_SECS: u64 = 30 * 60;
const MIN_RECORD_MS: u128 = 300;
const SILENCE_PEAK_THRESHOLD: f32 = 0.003;

#[derive(Clone)]
pub struct SessionCtx {
    pub audio: AudioHandle,
    pub recording: Arc<AtomicBool>,
    pub session_id: Arc<AtomicU64>,
    started_at: Arc<Mutex<Option<Instant>>>,
    canceled: Arc<AtomicBool>,
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
    let prefs = settings::prefs(&app);
    if !prefs.dictation_enabled {
        log::info!("dictation is disabled; hotkey press ignored");
        return;
    }

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

    if let Some(mode) = app
        .try_state::<Arc<ModeStore>>()
        .map(|s| s.inner().clone())
        .and_then(|s| s.get(&prefs.active_mode_id).or_else(|| s.get("general")))
    {
        pill::emit_active_mode(&app, &mode);
    }
    pill::emit_mode_suggestion(&app, None);

    if prefs.auto_suggest_modes {
        spawn_suggestion_probe(app.clone(), prefs.active_mode_id.clone());
    }

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

    spawn_max_record_guard(app.clone(), ctx.clone(), MAX_RECORD_SECS);
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

    tauri::async_runtime::spawn(async move {
        let _pipeline_guard = PipelineGuard::new(&ctx, my_sid);

        let wav_result = ctx.audio.stop_and_encode();

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

        // Snapshot prefs ONCE per utterance — engines, mode, language and
        // dictionary all derive from this read. Adding a second
        // `settings::prefs` call below would let a mid-flight mode switch
        // pin a different prompt to a transcript recorded under the old mode.
        let prefs = settings::prefs(&app_for_pill);

        let app_wry: AppHandle<Wry> = match try_into_wry(&app_for_pill) {
            Some(a) => a,
            None => {
                log::error!("session: not running on Wry — engine registry unavailable");
                pill::emit_state(
                    &app_for_pill,
                    PillState::Error,
                    Some("Internal error.".into()),
                );
                hide_after_session(&app_for_pill, &ctx, my_sid, 2500);
                return;
            }
        };

        let registry = match app_wry.try_state::<Registry>() {
            Some(r) => r.inner().clone(),
            None => {
                log::error!("engine registry missing — onboarding never finished?");
                pill::emit_state(
                    &app_for_pill,
                    PillState::Error,
                    Some("Engines not initialised — restart Vox.".into()),
                );
                hide_after_session(&app_for_pill, &ctx, my_sid, 2500);
                return;
            }
        };

        let stt_engine = match registry.stt(&app_wry, &prefs.stt_engine, &prefs.stt_model_id) {
            Some(e) => e,
            None => {
                pill::emit_state(
                    &app_for_pill,
                    PillState::Error,
                    Some("STT engine not configured. Open Settings → Models.".into()),
                );
                hide_after_session(&app_for_pill, &ctx, my_sid, 3000);
                return;
            }
        };

        let llm_engine = if prefs.polishing_enabled {
            registry.llm(&app_wry, &prefs.llm_engine, &prefs.llm_model_id)
        } else {
            registry.llm(&app_wry, "none", "")
        };

        let active_mode = resolve_active_mode(&app_wry, &prefs.active_mode_id);
        if let Some(m) = active_mode.as_ref() {
            pill::emit_active_mode(&app_for_pill, m);
        }

        pill::emit_state(&app_for_pill, PillState::Transcribing, None);

        let language = (prefs.language != "auto").then_some(prefs.language.as_str());

        let transcript = {
            let stt_fut = stt_engine.transcribe(&wav, language, &prefs.dictionary);
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
                        log::info!("empty transcript, nothing to copy");
                        pill::hide(&app_for_pill);
                        return;
                    }
                    Err(e) => {
                        log::error!("STT failed: {e}");
                        pill::emit_state(
                            &app_for_pill,
                            PillState::Error,
                            Some(friendly_stt_error(&e, &prefs.stt_engine)),
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

        let (mode_system_prompt, merged_dictionary) = match active_mode.as_ref() {
            Some(m) => (m.system_prompt.clone(), merge_dictionaries(&prefs.dictionary, &m.dictionary)),
            None => (String::new(), prefs.dictionary.clone()),
        };

        let cleaned = match llm_engine {
            Some(engine) if engine.id() != "none" => {
                pill::emit_state(&app_for_pill, PillState::Polishing, None);
                let llm_fut = engine.cleanup(
                    &transcript,
                    &mode_system_prompt,
                    &merged_dictionary,
                );
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
                        Ok(t) if !t.trim().is_empty() => t,
                        Ok(_) => {
                            log::info!(
                                "LLM polish returned empty content; falling back to raw transcript"
                            );
                            transcript.clone()
                        }
                        Err(e) => {
                            log::warn!("LLM cleanup failed: {e}; falling back to raw transcript");
                            transcript.clone()
                        }
                    }
                }
            }
            _ => transcript.clone(),
        };

        let cleaned = match active_mode.as_ref() {
            Some(m) => m.apply_post_processing(&cleaned),
            None => cleaned,
        };

        if ctx.is_canceled() || stale() {
            log::info!("post-LLM cancel/supersede; aborting before copy");
            if ctx.is_canceled() && !stale() {
                pill::emit_state(&app_for_pill, PillState::Canceled, None);
                hide_after_session(&app_for_pill, &ctx, my_sid, 600);
            }
            return;
        }

        log::info!("copying {} chars (sid={my_sid})", cleaned.len());

        transcripts::push(&cleaned);
        if let Err(e) = app_for_pill.emit("transcripts:changed", transcripts::snapshot()) {
            log::warn!("failed to emit transcripts:changed: {e}");
        }

        let t = cleaned.clone();
        let copy_result =
            tauri::async_runtime::spawn_blocking(move || inject::copy_to_clipboard(&t)).await;

        if ctx.is_canceled() || stale() {
            return;
        }

        match copy_result {
            Ok(Ok(())) => {
                pill::emit_state(
                    &app_for_pill,
                    PillState::Done,
                    Some("Copied — press ⌘V to paste.".into()),
                );
                hide_after_session(&app_for_pill, &ctx, my_sid, 1800);
            }
            Ok(Err(e)) => {
                log::error!("copy failed: {e}");
                pill::emit_state(
                    &app_for_pill,
                    PillState::Error,
                    Some("Copy failed.".into()),
                );
                hide_after_session(&app_for_pill, &ctx, my_sid, 3000);
            }
            Err(e) => {
                log::error!("copy task panicked: {e}");
                pill::emit_state(&app_for_pill, PillState::Error, Some("Copy failed.".into()));
                hide_after_session(&app_for_pill, &ctx, my_sid, 2500);
            }
        }
    });
}

pub fn cancel<R: Runtime>(app: AppHandle<R>, ctx: SessionCtx) {
    if !ctx.is_pipeline_active() {
        log::debug!("cancel: no active pipeline; ignoring");
        return;
    }

    ctx.canceled.store(true, Ordering::SeqCst);

    if ctx.recording.load(Ordering::SeqCst) {
        end_recording(app, ctx);
    } else {
        pill::emit_state(&app, PillState::Canceled, None);
        let app_arc = Arc::new(app.clone());
        pill::hide_after(&app_arc, 600);
    }
}

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

async fn wait_for_cancel(ctx: &SessionCtx, my_sid: u64) {
    loop {
        if ctx.session_id.load(Ordering::SeqCst) != my_sid
            || ctx.canceled.load(Ordering::SeqCst)
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

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
            log::warn!("recording exceeded {max_secs}s safety cap — auto-stopping");
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

/// Downcast a generic `AppHandle<R>` to the production `Wry` runtime.
/// Returns `None` for non-Wry runtimes (only relevant for headless tests).
fn try_into_wry<R: Runtime>(app: &AppHandle<R>) -> Option<AppHandle<Wry>> {
    use std::any::Any;
    let any: &dyn Any = app as &dyn Any;
    any.downcast_ref::<AppHandle<Wry>>().cloned()
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
