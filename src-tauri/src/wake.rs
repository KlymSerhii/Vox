//! "Hey Vox" wake-phrase listener.
//!
//! When `Settings::wake_word_enabled` is on, a dedicated thread keeps a
//! low-cost microphone capture open while Vox is otherwise idle. A simple
//! energy VAD watches for a short utterance; when one ends, the buffered
//! audio is transcribed with the user's already-selected STT engine (the
//! same cached `Registry` instance the dictation pipeline uses, so no
//! extra model RAM) and fuzzy-matched against the wake phrase. On a match
//! the listener releases the mic, starts a normal dictation session via
//! `SessionManager::begin`, and then auto-stops the recording on trailing
//! silence so the whole flow stays hands-free.
//!
//! Privacy posture matches the rest of the app: the capture buffer lives
//! in RAM, holds at most a few seconds, and is dropped the moment it has
//! been inspected. Nothing is persisted and nothing leaves the device.
//!
//! The listener suspends itself — dropping the cpal stream so the OS mic
//! indicator releases — whenever a recording or pipeline is in flight,
//! when dictation / the wake toggle is off, or (macOS) when microphone
//! permission isn't granted. `refresh` re-snapshots the relevant settings
//! after any settings write; the thread also re-checks once a second so a
//! missed refresh can only delay a config change, never lose it.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex as StdMutex};
use std::thread;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};
use parking_lot::Mutex;
use tauri::{AppHandle, Manager, Runtime, Wry};

use crate::audio;
use crate::engines::Registry;
use crate::session::{BeginOutcome, SessionManager};
use crate::settings;

/// Main loop cadence. Every tick re-checks config / session state and
/// advances the VAD state machine by one step.
const TICK_MS: u64 = 100;
/// RMS evaluation window — the tail of the capture buffer inspected per
/// tick.
const RMS_WINDOW_MS: u64 = 120;
/// An utterance must contain at least this much voiced audio before a
/// detection pass runs; shorter blips (keyboard clatter, coughs) are
/// dropped without paying for a transcribe.
const MIN_VOICED_MS: u64 = 300;
/// Trailing quiet that ends an utterance and triggers the detection pass.
const END_SILENCE_MS: u64 = 450;
/// Voiced audio longer than this is decoded mid-stream — the wake phrase
/// is short, so anything longer is almost certainly not it, but decoding
/// keeps the buffer bounded and catches "hey vox, <command…>" styles.
const MAX_UTTERANCE_MS: u64 = 3200;
/// Pre-roll included before the detected voice onset so a soft first
/// syllable isn't clipped from the decode window.
const PREROLL_MS: u64 = 500;
/// Hard cap on the audio handed to the STT engine per detection pass.
const DETECT_WINDOW_MAX_MS: u64 = 3800;
/// Capture ring buffer capacity (device rate, interleaved).
const BUFFER_CAP_MS: u64 = 8_000;

/// Backoff after a failed stream open (device busy / vanished).
const OPEN_FAIL_BACKOFF: Duration = Duration::from_secs(5);
/// Backoff after a failed transcribe (model missing / engine error) so a
/// chatty room doesn't spin error logs.
const TRANSCRIBE_FAIL_BACKOFF: Duration = Duration::from_secs(20);
/// Minimum spacing between identical warn-level log lines.
const LOG_THROTTLE: Duration = Duration::from_secs(60);

/// Post-trigger auto-stop: `AudioHandle::current_rms` (already 0..=1
/// scaled + smoothed) above this counts as speech…
const SESSION_SPEECH_RMS: f32 = 0.07;
/// …and this much continuous quiet after speech ends the recording.
const SESSION_STOP_SILENCE_MS: u64 = 1500;
/// If the user never says anything after the wake phrase, stop anyway —
/// the session's own silence handling surfaces the friendly pill state.
const SESSION_NO_SPEECH_TIMEOUT_MS: u64 = 8000;

/// Settings snapshot the wake thread runs against. Rebuilt by
/// [`refresh`]; comparing snapshots is how the thread knows to reopen
/// the stream (device change) or drop a cached engine choice.
#[derive(Clone, Default, PartialEq)]
struct WakeConfig {
    /// `wake_word_enabled && dictation_enabled && onboarded`.
    enabled: bool,
    phrase: String,
    language: String,
    stt_engine: String,
    stt_model_id: String,
    device: Option<String>,
}

/// Shared control block managed in Tauri state as `Arc<WakeControl>`.
pub struct WakeControl {
    cfg: Mutex<WakeConfig>,
    /// Bumped on every config change so the loop can cheaply detect that
    /// its open stream / VAD state is stale.
    generation: AtomicU64,
    /// Wakes the loop out of its idle wait when config changes.
    wakeup: (StdMutex<()>, Condvar),
}

impl WakeControl {
    fn new() -> Self {
        Self {
            cfg: Mutex::new(WakeConfig::default()),
            generation: AtomicU64::new(0),
            wakeup: (StdMutex::new(()), Condvar::new()),
        }
    }

    fn snapshot(&self) -> (WakeConfig, u64) {
        let cfg = self.cfg.lock().clone();
        (cfg, self.generation.load(Ordering::Acquire))
    }

    fn update(&self, cfg: WakeConfig) {
        {
            let mut guard = self.cfg.lock();
            if *guard == cfg {
                return;
            }
            *guard = cfg;
        }
        self.generation.fetch_add(1, Ordering::AcqRel);
        let (lock, cv) = &self.wakeup;
        let _unused = lock.lock();
        cv.notify_all();
    }

    /// Park for `timeout` or until a config change notifies.
    fn idle_wait(&self, timeout: Duration) {
        let (lock, cv) = &self.wakeup;
        if let Ok(guard) = lock.lock() {
            let _ = cv.wait_timeout(guard, timeout);
        } else {
            thread::sleep(timeout);
        }
    }
}

/// Spawn the wake listener thread and seed its config. Called once from
/// `lib.rs::run`'s setup hook, after `SessionManager` and the engine
/// `Registry` are managed.
pub fn install(app: &AppHandle<Wry>, manager: Arc<SessionManager>) {
    let control = Arc::new(WakeControl::new());
    app.manage(control.clone());
    refresh(app);

    let app = app.clone();
    if let Err(e) = thread::Builder::new()
        .name("vox-wake".into())
        .spawn(move || wake_thread(app, manager, control))
    {
        log::error!("wake: failed to spawn listener thread: {e}");
    }
}

/// Re-snapshot the wake-relevant settings into the control block. Cheap
/// and idempotent — call after any settings write that could affect the
/// listener (`set_settings`, dictation toggle, onboarding finish).
pub fn refresh<R: Runtime>(app: &AppHandle<R>) {
    let Some(control) = app.try_state::<Arc<WakeControl>>() else {
        return;
    };
    let s = settings::load(app);
    let phrase = {
        let trimmed = s.wake_phrase.trim();
        if trimmed.is_empty() {
            settings::default_wake_phrase()
        } else {
            trimmed.to_lowercase()
        }
    };
    control.update(WakeConfig {
        enabled: s.wake_word_enabled && s.dictation_enabled && s.onboarded,
        phrase,
        language: s.language,
        stt_engine: s.stt_engine,
        stt_model_id: s.stt_model_id,
        device: s.audio_input_device,
    });
}

// ---------------------------------------------------------------------------
// Listener thread
// ---------------------------------------------------------------------------

/// Open cpal capture owned by the wake thread. `cpal::Stream` is `!Send`,
/// so the whole lifecycle stays on this thread (mirroring `audio.rs`).
struct Listener {
    stream: cpal::Stream,
    samples: Arc<Mutex<Vec<f32>>>,
    input_sr: u32,
    input_channels: u16,
    /// Config generation this stream was opened under.
    generation: u64,
}

impl Listener {
    fn close(self) {
        audio::release_stream(self.stream);
    }
}

#[derive(Default)]
struct VadState {
    /// Exponential noise-floor estimate (raw RMS units).
    noise_floor: f32,
    voiced_ms: u64,
    silence_ms: u64,
}

impl VadState {
    fn reset(&mut self) {
        self.voiced_ms = 0;
        self.silence_ms = 0;
    }
}

fn wake_thread(app: AppHandle<Wry>, manager: Arc<SessionManager>, control: Arc<WakeControl>) {
    // Same rationale as `audio::audio_thread`: a panic must log loudly
    // instead of silently killing the feature, and we don't auto-respawn
    // to avoid a restart loop on a wedged audio stack.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        wake_thread_inner(app, manager, control)
    }));
    if let Err(payload) = result {
        let msg = if let Some(s) = payload.downcast_ref::<&'static str>() {
            (*s).to_string()
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else {
            "<non-string panic payload>".to_string()
        };
        log::error!(
            "wake: listener thread panicked: {msg}. The voice trigger is \
             offline until the next launch."
        );
    }
}

fn wake_thread_inner(
    app: AppHandle<Wry>,
    manager: Arc<SessionManager>,
    control: Arc<WakeControl>,
) {
    let mut listener: Option<Listener> = None;
    let mut vad = VadState::default();
    let mut backoff_until: Option<Instant> = None;
    let mut last_warn: Option<Instant> = None;

    let close_listener = |l: &mut Option<Listener>, vad: &mut VadState| {
        if let Some(active) = l.take() {
            active.close();
        }
        vad.reset();
    };

    loop {
        let (cfg, generation) = control.snapshot();

        if !cfg.enabled {
            close_listener(&mut listener, &mut vad);
            control.idle_wait(Duration::from_secs(1));
            continue;
        }

        // Never let the wake listener be the thing that triggers the
        // mic TCC prompt — onboarding owns that moment.
        if !mic_granted() {
            close_listener(&mut listener, &mut vad);
            control.idle_wait(Duration::from_secs(3));
            continue;
        }

        // Stay off the mic while any dictation session records or its
        // pipeline runs. The session's capture owns the device; the
        // busy_gate would reject a wake-triggered begin anyway.
        if manager.is_recording() || manager.has_active_pipeline() {
            close_listener(&mut listener, &mut vad);
            thread::sleep(Duration::from_millis(2 * TICK_MS));
            continue;
        }

        if let Some(until) = backoff_until {
            if Instant::now() < until {
                close_listener(&mut listener, &mut vad);
                control.idle_wait(Duration::from_millis(500));
                continue;
            }
            backoff_until = None;
        }

        // Config changed under an open stream (device / engine / phrase)
        // — reopen from scratch so we never listen on a stale device.
        if listener.as_ref().is_some_and(|l| l.generation != generation) {
            close_listener(&mut listener, &mut vad);
        }

        if listener.is_none() {
            match open_listener(cfg.device.as_deref(), generation) {
                Ok(l) => {
                    log::info!(
                        "wake: listening for \"{}\" ({} Hz, {} ch)",
                        cfg.phrase,
                        l.input_sr,
                        l.input_channels
                    );
                    listener = Some(l);
                    vad = VadState::default();
                }
                Err(e) => {
                    warn_throttled(&mut last_warn, &format!("wake: mic open failed: {e}"));
                    backoff_until = Some(Instant::now() + OPEN_FAIL_BACKOFF);
                    continue;
                }
            }
        }

        thread::sleep(Duration::from_millis(TICK_MS));

        let Some(active) = listener.as_ref() else {
            continue;
        };

        let utterance = advance_vad(active, &mut vad);
        if !utterance {
            continue;
        }

        // Snapshot the utterance window (plus pre-roll), then clear VAD
        // state so the same audio is never analysed twice.
        let window_ms =
            (vad.voiced_ms + vad.silence_ms + PREROLL_MS).min(DETECT_WINDOW_MAX_MS);
        let raw_tail = {
            let buf = active.samples.lock();
            let per_ms = (active.input_sr as u64 * active.input_channels as u64) / 1000;
            let want = (per_ms * window_ms) as usize;
            let start = buf.len().saturating_sub(want);
            buf[start..].to_vec()
        };
        let input_sr = active.input_sr;
        let input_channels = active.input_channels;
        vad.reset();
        active.samples.lock().clear();

        let pcm = audio::postprocess_to_pcm_mono_16k(&raw_tail, input_sr, input_channels);
        // Under half a second of audio can't contain the phrase.
        if pcm.len() < audio::TARGET_SR as usize / 2 {
            continue;
        }
        let wav = match audio::encode_wav_pcm16(&pcm, audio::TARGET_SR) {
            Ok(w) => w,
            Err(e) => {
                warn_throttled(&mut last_warn, &format!("wake: wav encode failed: {e}"));
                continue;
            }
        };

        match transcribe_window(&app, &cfg, &wav) {
            Ok(text) => {
                log::debug!("wake: heard {text:?}");
                if phrase_matches(&text, &cfg.phrase) {
                    log::info!(
                        "wake: trigger phrase \"{}\" detected (heard {text:?})",
                        cfg.phrase
                    );
                    // Release our stream BEFORE the session opens its own
                    // capture so the device handoff is clean.
                    close_listener(&mut listener, &mut vad);
                    start_wake_session(&app, &manager);
                }
            }
            Err(e) => {
                warn_throttled(
                    &mut last_warn,
                    &format!(
                        "wake: transcribe failed ({e}); voice trigger pausing \
                         for {}s",
                        TRANSCRIBE_FAIL_BACKOFF.as_secs()
                    ),
                );
                backoff_until = Some(Instant::now() + TRANSCRIBE_FAIL_BACKOFF);
            }
        }
    }
}

/// One VAD step: returns `true` when an utterance just finished (or grew
/// past [`MAX_UTTERANCE_MS`]) and a detection pass should run on the
/// buffered audio.
fn advance_vad(listener: &Listener, vad: &mut VadState) -> bool {
    let rms = {
        let buf = listener.samples.lock();
        let per_ms = (listener.input_sr as u64 * listener.input_channels as u64) / 1000;
        let want = (per_ms * RMS_WINDOW_MS) as usize;
        let start = buf.len().saturating_sub(want);
        tail_rms(&buf[start..])
    };

    // Adaptive threshold: track the noise floor while quiet so a fan or
    // hum doesn't read as permanent speech, with absolute clamps so a
    // dead-silent room still needs a real voice to trip.
    if vad.noise_floor == 0.0 {
        vad.noise_floor = rms.max(0.0005);
    }
    let voice_on = (vad.noise_floor * 3.5).clamp(0.010, 0.080);
    if rms < voice_on {
        vad.noise_floor = vad.noise_floor * 0.95 + rms * 0.05;
    }

    let voiced = rms >= voice_on;
    if vad.voiced_ms == 0 {
        if voiced {
            vad.voiced_ms = TICK_MS;
            vad.silence_ms = 0;
        }
        return false;
    }

    if voiced {
        vad.voiced_ms += TICK_MS;
        vad.silence_ms = 0;
    } else {
        vad.silence_ms += TICK_MS;
    }

    if vad.silence_ms >= END_SILENCE_MS {
        if vad.voiced_ms >= MIN_VOICED_MS {
            return true;
        }
        // Sub-threshold blip — discard without decoding.
        vad.reset();
        return false;
    }

    vad.voiced_ms >= MAX_UTTERANCE_MS
}

fn tail_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|s| s * s).sum();
    (sum_sq / samples.len() as f32).sqrt()
}

fn transcribe_window(
    app: &AppHandle<Wry>,
    cfg: &WakeConfig,
    wav: &[u8],
) -> anyhow::Result<String> {
    let registry = app
        .try_state::<Registry>()
        .ok_or_else(|| anyhow::anyhow!("engine registry not available"))?;
    let engine = registry
        .stt(app, &cfg.stt_engine, &cfg.stt_model_id)
        .ok_or_else(|| anyhow::anyhow!("unknown STT engine '{}'", cfg.stt_engine))?;
    let lang = (cfg.language != "auto").then_some(cfg.language.as_str());
    // Bias the decoder toward the phrase the same way dictionary terms
    // bias normal dictation — makes Whisper much likelier to emit "Vox"
    // instead of "fox"/"box".
    let bias = vec![cfg.phrase.clone()];
    // Dedicated thread, so blocking on the runtime here is fine.
    tauri::async_runtime::block_on(engine.transcribe(wav, lang, &bias, None))
}

/// Begin a session in response to the wake phrase, then watch the live
/// RMS meter and stop the recording after trailing silence so the flow
/// is fully hands-free. Pressing the hotkey mid-session still works —
/// `end_recording` is a no-op for whoever loses that race, and the
/// monitor exits as soon as the session stops being the foreground one.
fn start_wake_session(app: &AppHandle<Wry>, manager: &Arc<SessionManager>) {
    let sid = match manager.begin(app) {
        BeginOutcome::Started(sid) => sid,
        other => {
            log::info!("wake: session not started ({other:?})");
            return;
        }
    };

    let mut speech_seen = false;
    let mut silence_ms: u64 = 0;
    let mut waited_ms: u64 = 0;
    loop {
        thread::sleep(Duration::from_millis(TICK_MS));
        if manager.foreground_id() != Some(sid) {
            // User (or the max-record guard) already ended/canceled it.
            return;
        }
        let rms = manager.audio.current_rms();
        if rms >= SESSION_SPEECH_RMS {
            speech_seen = true;
            silence_ms = 0;
        } else {
            silence_ms += TICK_MS;
        }
        waited_ms += TICK_MS;

        let stop = (speech_seen && silence_ms >= SESSION_STOP_SILENCE_MS)
            || (!speech_seen && waited_ms >= SESSION_NO_SPEECH_TIMEOUT_MS);
        if stop {
            log::info!(
                "wake: auto-stopping session (speech_seen={speech_seen}, \
                 silence={silence_ms}ms)"
            );
            manager.end_recording(app.clone());
            return;
        }
    }
}

fn warn_throttled(last: &mut Option<Instant>, msg: &str) {
    let now = Instant::now();
    if last.map_or(true, |t| now.duration_since(t) >= LOG_THROTTLE) {
        log::warn!("{msg}");
        *last = Some(now);
    } else {
        log::debug!("{msg}");
    }
}

#[cfg(target_os = "macos")]
fn mic_granted() -> bool {
    crate::mac_perm::authorization_status().is_granted()
}

#[cfg(not(target_os = "macos"))]
fn mic_granted() -> bool {
    // No TCC-style gate on Windows; the capture either works or errors
    // through the normal open path.
    true
}

fn open_listener(
    device_name: Option<&str>,
    generation: u64,
) -> Result<Listener, audio::AudioError> {
    let host = cpal::default_host();
    let device = audio::resolve_input_device(&host, device_name)?;
    let default_cfg = device
        .default_input_config()
        .map_err(|e| audio::AudioError::Cpal(e.to_string()))?;
    let input_sr = default_cfg.sample_rate().0;
    let input_channels = default_cfg.channels();
    let sample_format = default_cfg.sample_format();
    let config: StreamConfig = default_cfg.into();

    let samples: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let cap = (input_sr as u64 * input_channels as u64 * BUFFER_CAP_MS / 1000) as usize;

    let err_fn = |err| log::warn!("wake: stream error: {err}");
    let make_push = || {
        let samples_cb = Arc::clone(&samples);
        move |converted: Vec<f32>| {
            let mut buf = samples_cb.lock();
            buf.extend_from_slice(&converted);
            if buf.len() > cap {
                let overflow = buf.len() - cap;
                buf.drain(0..overflow);
            }
        }
    };
    let stream = match sample_format {
        SampleFormat::F32 => {
            let push = make_push();
            device.build_input_stream(
                &config,
                move |data: &[f32], _| push(data.to_vec()),
                err_fn,
                None,
            )
        }
        SampleFormat::I16 => {
            let push = make_push();
            device.build_input_stream(
                &config,
                move |data: &[i16], _| {
                    push(data.iter().map(|&s| s as f32 / i16::MAX as f32).collect())
                },
                err_fn,
                None,
            )
        }
        SampleFormat::U16 => {
            let push = make_push();
            device.build_input_stream(
                &config,
                move |data: &[u16], _| {
                    push(
                        data.iter()
                            .map(|&s| (s as f32 - 32_768.0) / 32_768.0)
                            .collect(),
                    )
                },
                err_fn,
                None,
            )
        }
        other => {
            return Err(audio::AudioError::Cpal(format!(
                "unsupported sample format: {other:?}"
            )))
        }
    }
    .map_err(|e| audio::AudioError::Cpal(e.to_string()))?;

    stream
        .play()
        .map_err(|e| audio::AudioError::Cpal(e.to_string()))?;

    Ok(Listener {
        stream,
        samples,
        input_sr,
        input_channels,
        generation,
    })
}

// ---------------------------------------------------------------------------
// Phrase matching
// ---------------------------------------------------------------------------

/// Lowercased alphanumeric word tokens — strips the punctuation and
/// capitalisation Whisper likes to add ("Hey, Vox!").
fn normalize_tokens(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_lowercase())
        .collect()
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        curr[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

/// Per-token tolerance: short tokens get one edit ("vox" → "fox"),
/// longer ones two. Tokens under three characters must match exactly —
/// at that length one edit reaches half the alphabet.
fn token_matches(heard: &str, want: &str) -> bool {
    if heard == want {
        return true;
    }
    let len = heard.chars().count().max(want.chars().count());
    let allowed = if len >= 7 {
        2
    } else if len >= 3 {
        1
    } else {
        0
    };
    levenshtein(heard, want) <= allowed
}

/// True when `transcript` contains the wake `phrase`, tolerating the
/// near-homophones STT produces for invented words ("hey fox",
/// "hey box") and run-together output ("heyvox").
fn phrase_matches(transcript: &str, phrase: &str) -> bool {
    let want = normalize_tokens(phrase);
    if want.is_empty() {
        return false;
    }
    let heard = normalize_tokens(transcript);
    if heard.is_empty() {
        return false;
    }

    // Token-aligned sliding window.
    if heard.len() >= want.len() {
        for window in heard.windows(want.len()) {
            if window.iter().zip(&want).all(|(h, w)| token_matches(h, w)) {
                return true;
            }
        }
    }

    // Joined fallback for merged/split tokenisation: compare contiguous
    // runs of heard tokens, concatenated, against the concatenated
    // phrase with a length-proportional edit budget.
    let want_joined: String = want.concat();
    // Round the length-proportional budget up: "heyvox" (6 chars) needs
    // 2 edits to absorb a trailing-consonant mishear like "hey voks".
    let budget = want_joined.chars().count().div_ceil(4).max(1);
    let max_run = want.len() + 1;
    for run_len in 1..=max_run.min(heard.len()) {
        for run in heard.windows(run_len) {
            let joined: String = run.concat();
            // Skip wildly mismatched lengths before paying for the DP.
            if joined.chars().count().abs_diff(want_joined.chars().count()) > budget {
                continue;
            }
            if levenshtein(&joined, &want_joined) <= budget {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_exact_phrase() {
        assert!(phrase_matches("Hey Vox", "hey vox"));
        assert!(phrase_matches("hey vox", "hey vox"));
    }

    #[test]
    fn matches_with_whisper_punctuation() {
        assert!(phrase_matches("Hey, Vox!", "hey vox"));
        assert!(phrase_matches(" Hey Vox. ", "hey vox"));
    }

    #[test]
    fn matches_near_homophones() {
        assert!(phrase_matches("hey fox", "hey vox"));
        assert!(phrase_matches("Hey Box.", "hey vox"));
        assert!(phrase_matches("hey voks", "hey vox"));
    }

    #[test]
    fn matches_run_together_tokens() {
        assert!(phrase_matches("heyvox", "hey vox"));
        assert!(phrase_matches("Heyvox!", "hey vox"));
    }

    #[test]
    fn matches_inside_longer_utterance() {
        assert!(phrase_matches("um hey vox can you hear me", "hey vox"));
    }

    #[test]
    fn rejects_unrelated_speech() {
        assert!(!phrase_matches("what time is it", "hey vox"));
        assert!(!phrase_matches("hello there", "hey vox"));
        assert!(!phrase_matches("hey everyone", "hey vox"));
        assert!(!phrase_matches("they fixed it", "hey vox"));
    }

    #[test]
    fn rejects_empty_inputs() {
        assert!(!phrase_matches("", "hey vox"));
        assert!(!phrase_matches("hey vox", ""));
        assert!(!phrase_matches("...", "hey vox"));
    }

    #[test]
    fn short_tokens_require_exact_match() {
        // "ok" must not fuzz onto "go" / "so".
        assert!(!token_matches("go", "ok"));
        assert!(token_matches("ok", "ok"));
    }

    #[test]
    fn levenshtein_basics() {
        assert_eq!(levenshtein("", ""), 0);
        assert_eq!(levenshtein("a", ""), 1);
        assert_eq!(levenshtein("vox", "fox"), 1);
        assert_eq!(levenshtein("vox", "voice"), 3);
    }

    #[test]
    fn custom_phrase_matches() {
        assert!(phrase_matches("Okay computer.", "okay computer"));
        assert!(phrase_matches("OK computer", "okay computer"));
        assert!(!phrase_matches("okay then", "okay computer"));
    }
}
