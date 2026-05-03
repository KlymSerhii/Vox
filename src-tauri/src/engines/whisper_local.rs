use std::path::{Path, PathBuf};
#[cfg(feature = "local-engines")]
use std::sync::Arc;

#[cfg(feature = "local-engines")]
use parking_lot::Mutex;
use tauri::{AppHandle, Wry};

use crate::engines::{SegmentCallback, SttEngine};
use crate::models::{self, ModelKind};

#[cfg(feature = "local-engines")]
use once_cell::sync::OnceCell;

pub struct WhisperLocalEngine {
    app: AppHandle<Wry>,
    model_id: String,
    #[cfg(feature = "local-engines")]
    ctx: OnceCell<Arc<Mutex<whisper_rs::WhisperContext>>>,
}

impl WhisperLocalEngine {
    pub fn new(app: AppHandle<Wry>, model_id: String) -> Self {
        Self {
            app,
            model_id,
            #[cfg(feature = "local-engines")]
            ctx: OnceCell::new(),
        }
    }

    fn cache_path(&self) -> Option<PathBuf> {
        models::cached_model_path(&self.app, &self.model_id, ModelKind::Stt)
    }

    fn fallback_error(&self) -> anyhow::Error {
        anyhow::anyhow!(
            "model '{}' not downloaded — open Settings → Models",
            self.model_id
        )
    }
}

#[cfg(feature = "local-engines")]
fn load_context_blocking(
    model_id: &str,
    path: &Path,
) -> anyhow::Result<Arc<Mutex<whisper_rs::WhisperContext>>> {
    let mut params = whisper_rs::WhisperContextParameters::default();
    // flash-attn cuts decoder memory bandwidth on Metal/CoreML.
    params.flash_attn(true);
    let path_str = path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("model path is not UTF-8"))?;
    log::info!("[whisper-local] loading model id={model_id} path={path_str}");
    let ctx = whisper_rs::WhisperContext::new_with_params(path_str, params)
        .map_err(|e| anyhow::anyhow!("whisper load: {e}"))?;
    Ok(Arc::new(Mutex::new(ctx)))
}

#[async_trait::async_trait]
impl SttEngine for WhisperLocalEngine {
    async fn transcribe(
        &self,
        wav: &[u8],
        language: Option<&str>,
        dictionary: &[String],
        on_segment: Option<SegmentCallback>,
    ) -> anyhow::Result<String> {
        let path = self.cache_path().ok_or_else(|| self.fallback_error())?;

        let model_id = self.model_id.clone();
        let lang = language.map(|s| s.to_string());
        let dict = dictionary.to_vec();
        let wav_owned = wav.to_vec();

        // Inference is CPU/GPU-bound; `WhisperState::full` would otherwise
        // pin the async runtime.
        #[cfg(feature = "local-engines")]
        {
            let ctx_cell = self.ctx.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let ctx = ctx_cell.get_or_try_init::<_, anyhow::Error>(|| {
                    load_context_blocking(&model_id, &path)
                })?;
                let pcm = decode_wav_to_f32_mono_16k(&wav_owned)?;
                run_whisper_full(ctx, &model_id, &pcm, lang.as_deref(), &dict, on_segment)
            })
            .await
            .map_err(|e| anyhow::anyhow!("whisper-local task panicked: {e}"))?
        }

        #[cfg(not(feature = "local-engines"))]
        {
            let _ = (path, model_id, lang, dict, wav_owned, on_segment);
            Err(anyhow::anyhow!(
                "whisper-local engine compiled out — rebuild with --features local-engines"
            ))
        }
    }

    fn id(&self) -> &'static str {
        "whisper-local"
    }

    fn label(&self) -> String {
        format!("Local Whisper ({})", self.model_id)
    }

    fn requires_network(&self) -> bool {
        false
    }

    async fn prewarm(&self) -> anyhow::Result<()> {
        let Some(path) = self.cache_path() else {
            return Ok(());
        };

        #[cfg(feature = "local-engines")]
        {
            let ctx_cell = self.ctx.clone();
            let model_id = self.model_id.clone();
            tauri::async_runtime::spawn_blocking(move || {
                ctx_cell.get_or_try_init::<_, anyhow::Error>(|| {
                    load_context_blocking(&model_id, &path)
                })?;
                Ok::<_, anyhow::Error>(())
            })
            .await
            .map_err(|e| anyhow::anyhow!("whisper prewarm task panicked: {e}"))?
        }

        #[cfg(not(feature = "local-engines"))]
        {
            let _ = path;
            Ok(())
        }
    }
}

// Re-validate spec defensively even though we produced the WAV ourselves.
// `pub(crate)` because parakeet_local reuses this — both engines consume
// the same 16 kHz mono PCM contract from `audio::stop_and_encode`.
#[cfg(feature = "local-engines")]
pub(crate) fn decode_wav_to_f32_mono_16k(wav: &[u8]) -> anyhow::Result<Vec<f32>> {
    use std::io::Cursor;
    let mut reader = hound::WavReader::new(Cursor::new(wav))
        .map_err(|e| anyhow::anyhow!("wav header: {e}"))?;
    let spec = reader.spec();
    if spec.channels != 1 {
        return Err(anyhow::anyhow!("expected mono WAV, got {} channels", spec.channels));
    }
    if spec.sample_rate != 16_000 {
        return Err(anyhow::anyhow!("expected 16 kHz WAV, got {} Hz", spec.sample_rate));
    }
    match spec.sample_format {
        hound::SampleFormat::Int => reader
            .samples::<i16>()
            .map(|s| {
                s.map(|v| v as f32 / i16::MAX as f32)
                    .map_err(|e| anyhow::anyhow!("wav sample: {e}"))
            })
            .collect(),
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .map(|s| s.map_err(|e| anyhow::anyhow!("wav sample: {e}")))
            .collect(),
    }
}

/// Shared body of every whisper-rs `state.full()` we run — both the
/// final authoritative pass (from [`SttEngine::transcribe`]) and the
/// speculative streaming passes (which also reach this through the
/// same `SttEngine::transcribe` entry point) use this. Caller is
/// responsible for spawning onto a blocking task; this function blocks
/// for the full decode. Callers must serialise concurrent invocations
/// on the same `Arc<Mutex<WhisperContext>>` — `state.full` mutates a
/// fresh `WhisperState` each call but the underlying context's KV
/// caches are not safe to drive in parallel.
#[cfg(feature = "local-engines")]
fn run_whisper_full(
    ctx: &Arc<Mutex<whisper_rs::WhisperContext>>,
    model_id: &str,
    pcm: &[f32],
    lang: Option<&str>,
    dict: &[String],
    on_segment: Option<SegmentCallback>,
) -> anyhow::Result<String> {
    let mut state = ctx
        .lock()
        .create_state()
        .map_err(|e| anyhow::anyhow!("whisper state: {e}"))?;

    let mut params = whisper_rs::FullParams::new(whisper_rs::SamplingStrategy::Greedy {
        best_of: 1,
    });
    params.set_n_threads(num_cpus::get_physical().max(1) as i32);
    params.set_temperature(0.0);
    params.set_no_context(true);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_special(false);
    params.set_print_timestamps(false);
    // English-only models can never decode anything else; skipping
    // lang-detect saves ~1 s on the first decoder pass.
    let is_english_only_model = model_id.contains("-en-");
    let auto_lang = lang.map(|l| l == "auto").unwrap_or(true);
    let effective_lang: Option<&str> = if auto_lang && is_english_only_model {
        Some("en")
    } else {
        lang.filter(|l| *l != "auto")
    };
    if let Some(l) = effective_lang {
        params.set_language(Some(l));
    }

    // Encoder cost scales with audio_ctx; default 1500 = full 30 s mel.
    // Clamp floor at 500 (~10 s of context): whisper.cpp's encoder
    // positional embeddings have been reported to truncate and corrupt
    // short-clip output at audio_ctx values below ~256, so 500 stays
    // comfortably above that while still leaving meaningful encoder
    // speedup for 5–10 s clips compared to the 1500-position default.
    let audio_ctx = (((pcm.len() / 16000) * 50) + 100).clamp(500, 1500) as i32;
    params.set_audio_ctx(audio_ctx);

    // Streaming speculative passes need per-segment callbacks for clips
    // that span multiple segments; only collapse to one segment for
    // short utterances where a single segment is the natural shape.
    // Gate on the absence of a segment callback: the speculative path
    // always passes Some(callback) and must stay multi-segment so the
    // pill paints partial transcripts as decoding progresses.
    if pcm.len() < 16_000 * 25 && on_segment.is_none() {
        params.set_single_segment(true);
    }
    let prompt;
    let cleaned: Vec<&str> = dict.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    if !cleaned.is_empty() {
        prompt = cleaned.join(", ");
        params.set_initial_prompt(&prompt);
    }

    // Hook the streaming callback if the caller asked for one.
    // whisper-rs's safe wrapper accumulates `n_new` segments
    // each invocation and hands us their decoded text — we
    // append to a shared accumulator and pass the cumulative
    // string up to the user closure.
    if let Some(cb) = on_segment {
        let acc = std::sync::Arc::new(parking_lot::Mutex::new(String::new()));
        let acc_for_cb = acc.clone();
        params.set_segment_callback_safe_lossy(
            move |seg: whisper_rs::SegmentCallbackData| {
                let mut g = acc_for_cb.lock();
                g.push_str(&seg.text);
                cb(g.as_str());
            },
        );
    }

    state
        .full(params, pcm)
        .map_err(|e| anyhow::anyhow!("whisper run: {e}"))?;

    let mut out = String::new();
    for i in 0..state.full_n_segments() {
        if let Some(seg) = state.get_segment(i) {
            match seg.to_str_lossy() {
                Ok(s) => out.push_str(s.as_ref()),
                Err(e) => log::warn!("whisper segment {i}: {e}"),
            }
        }
    }
    Ok(out.trim().to_string())
}
