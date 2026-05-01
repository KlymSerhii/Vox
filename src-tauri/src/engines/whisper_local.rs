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
    let params = whisper_rs::WhisperContextParameters::default();
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
                if let Some(l) = lang.as_deref().filter(|l| *l != "auto") {
                    params.set_language(Some(l));
                }
                let prompt;
                let cleaned: Vec<&str> =
                    dict.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
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
                    .full(params, &pcm)
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
#[cfg(feature = "local-engines")]
fn decode_wav_to_f32_mono_16k(wav: &[u8]) -> anyhow::Result<Vec<f32>> {
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
