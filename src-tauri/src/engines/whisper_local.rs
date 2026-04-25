//! Local Whisper STT — `whisper-rs` 0.16 with Core ML + Metal.
//!
//! Loads the GGML model from the user-data cache on first use, keeps the
//! `WhisperContext` warm for repeat dictations. The Core ML encoder
//! (`*.mlmodelc`) is auto-discovered alongside the GGML file when present.
//!
//! Falls back to a clear `anyhow!` error if the model isn't installed —
//! the session orchestrator surfaces "Model not downloaded. Open
//! Settings → Models." in that case.

use std::path::PathBuf;
#[cfg(feature = "local-engines")]
use std::sync::Arc;

#[cfg(feature = "local-engines")]
use parking_lot::Mutex;
use tauri::{AppHandle, Wry};

use crate::engines::SttEngine;
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

#[async_trait::async_trait]
impl SttEngine for WhisperLocalEngine {
    async fn transcribe(
        &self,
        wav: &[u8],
        language: Option<&str>,
        dictionary: &[String],
    ) -> anyhow::Result<String> {
        let path = self
            .cache_path()
            .ok_or_else(|| self.fallback_error())?;

        let model_id = self.model_id.clone();
        let lang = language.map(|s| s.to_string());
        let dict = dictionary.to_vec();
        let wav_owned = wav.to_vec();

        // Decode + transcribe runs on a blocking thread — whisper-rs
        // inference is CPU/GPU-bound and `WhisperState::full` would
        // otherwise hold the async runtime.
        #[cfg(feature = "local-engines")]
        {
            let ctx_cell = self.ctx.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let ctx = ctx_cell.get_or_try_init::<_, anyhow::Error>(|| {
                    let params = whisper_rs::WhisperContextParameters::default();
                    let path_str = path
                        .to_str()
                        .ok_or_else(|| anyhow::anyhow!("model path is not UTF-8"))?;
                    log::info!("[whisper-local] loading model id={model_id} path={path_str}");
                    let ctx = whisper_rs::WhisperContext::new_with_params(path_str, params)
                        .map_err(|e| anyhow::anyhow!("whisper load: {e}"))?;
                    Ok(Arc::new(Mutex::new(ctx)))
                })?;

                let pcm = decode_wav_to_f32_mono_16k(&wav_owned)?;

                let mut state = ctx
                    .lock()
                    .create_state()
                    .map_err(|e| anyhow::anyhow!("whisper state: {e}"))?;

                let mut params =
                    whisper_rs::FullParams::new(whisper_rs::SamplingStrategy::Greedy {
                        best_of: 1,
                    });
                params.set_n_threads(num_cpus::get_physical().max(1) as i32);
                params.set_temperature(0.0);
                params.set_no_context(true);
                params.set_print_progress(false);
                params.set_print_realtime(false);
                params.set_print_special(false);
                params.set_print_timestamps(false);
                if let Some(l) = lang.as_deref() {
                    if l != "auto" {
                        params.set_language(Some(l));
                    }
                }
                let prompt;
                let cleaned: Vec<&str> = dict
                    .iter()
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .collect();
                if !cleaned.is_empty() {
                    prompt = cleaned.join(", ");
                    params.set_initial_prompt(&prompt);
                }

                state
                    .full(params, &pcm)
                    .map_err(|e| anyhow::anyhow!("whisper run: {e}"))?;

                let mut out = String::new();
                let segments = state.full_n_segments();
                for i in 0..segments {
                    if let Some(seg) = state.get_segment(i) {
                        match seg.to_str_lossy() {
                            Ok(s) => out.push_str(s.as_ref()),
                            Err(e) => log::warn!("whisper segment {i}: {e}"),
                        }
                    }
                }
                Ok(out.trim().to_string())
            })
            .await
            .map_err(|e| anyhow::anyhow!("whisper-local task panicked: {e}"))?
        }

        #[cfg(not(feature = "local-engines"))]
        {
            let _ = (path, model_id, lang, dict, wav_owned);
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
}

/// Decode a PCM-16 mono 16 kHz WAV (the shape `audio::stop_and_encode`
/// produces) into the f32 PCM whisper-rs expects. We don't trust a
/// foreign WAV — re-validate the spec and bail if it doesn't match.
#[cfg(feature = "local-engines")]
fn decode_wav_to_f32_mono_16k(wav: &[u8]) -> anyhow::Result<Vec<f32>> {
    use std::io::Cursor;
    let mut reader = hound::WavReader::new(Cursor::new(wav))
        .map_err(|e| anyhow::anyhow!("wav header: {e}"))?;
    let spec = reader.spec();
    if spec.channels != 1 {
        return Err(anyhow::anyhow!(
            "expected mono WAV, got {} channels",
            spec.channels
        ));
    }
    if spec.sample_rate != 16_000 {
        return Err(anyhow::anyhow!(
            "expected 16 kHz WAV, got {} Hz",
            spec.sample_rate
        ));
    }
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => reader
            .samples::<i16>()
            .map(|s| {
                s.map(|v| v as f32 / i16::MAX as f32)
                    .map_err(|e| anyhow::anyhow!("wav sample: {e}"))
            })
            .collect::<Result<_, _>>()?,
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .map(|s| s.map_err(|e| anyhow::anyhow!("wav sample: {e}")))
            .collect::<Result<_, _>>()?,
    };
    Ok(samples)
}
