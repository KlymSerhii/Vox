//! NVIDIA Parakeet-TDT-0.6B-v3 STT via FluidAudio.
//!
//! Engine ID: `"parakeet-local"`. Calls into the Swift bridge in
//! `parakeet_bridge`, which wraps FluidAudio's `AsrManager` /
//! `AsrModels` API.
//!
//! Crash isolation: FluidAudio dispatches CoreML inference to the
//! system `aned` daemon, so a Parakeet model crash can't take the
//! menu-bar app down — same crash-isolation profile as Apple
//! Intelligence's Foundation Models. Model load + inference errors
//! return through the standard `SttEngine::transcribe` Err path; the
//! existing `session.rs` error surface paints a friendly pill.

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use tauri::{AppHandle, Manager, Wry};

use crate::engines::{parakeet_bridge, SegmentCallback, SttEngine};
use crate::models::{self, ModelKind};

#[cfg(feature = "local-engines")]
use crate::engines::whisper_local::decode_wav_to_f32_mono_16k;

use once_cell::sync::OnceCell;

/// Per-(engine, model_id) cell memoising the load result so a second
/// `prewarm()` or `transcribe()` short-circuits without re-asking the
/// Swift side. Mirrors the `OnceCell<Arc<Mutex<WhisperContext>>>` shape
/// in `whisper_local`.
type LoadOnce = OnceCell<Arc<Mutex<Result<(), String>>>>;

const TRANSCRIBE_TIMEOUT_SECS: u32 = 60;

/// Hardcoded to FluidAudio's `Repo.parakeetV3.folderName` (v0.14.3).
/// `AsrModels.load(from:)` reconstructs file paths from the parent of
/// the directory we hand it plus this folder name; if FluidAudio
/// renames the repo we'd need to update this in lockstep.
const FLUIDAUDIO_REPO_FOLDER: &str = "parakeet-tdt-0.6b-v3-coreml";

/// Sentinel file the catalog downloader is expected to write last so
/// we can probe "fully populated" without enumerating every .mlmodelc.
const FLUIDAUDIO_VOCAB_FILE: &str = "parakeet_vocab.json";

pub struct ParakeetLocalEngine {
    app: AppHandle<Wry>,
    model_id: String,
    loaded: LoadOnce,
}

impl ParakeetLocalEngine {
    pub fn new(app: AppHandle<Wry>, model_id: String) -> Self {
        Self {
            app,
            model_id,
            loaded: OnceCell::new(),
        }
    }

    /// Mirrors `models::root_dir` + `models::model_dir` without
    /// touching the private helpers — the model directory is derived
    /// the same way the catalog downloader writes to disk.
    fn model_dir(&self) -> Option<PathBuf> {
        let root = self
            .app
            .path()
            .app_data_dir()
            .ok()
            .map(|p| p.join("models"))
            .or_else(|| {
                dirs::data_dir().map(|p| p.join("app.vox.vox/models"))
            })?;
        Some(root.join("stt").join(&self.model_id))
    }

    /// Path passed to FluidAudio's `AsrModels.load(from:)`. FluidAudio
    /// strips the last component and re-appends `Repo.parakeetV3.folderName`
    /// (= `"parakeet-tdt-0.6b-v3-coreml"`) to find the .mlmodelc files,
    /// so we hand it `<model_dir>/parakeet-tdt-0.6b-v3-coreml`. The
    /// catalog downloader places the four .mlmodelc bundles + vocab
    /// under that subdirectory.
    fn fluid_audio_load_dir(&self) -> Option<PathBuf> {
        Some(self.model_dir()?.join(FLUIDAUDIO_REPO_FOLDER))
    }

    fn ensure_loaded(&self) -> Result<(), anyhow::Error> {
        if !parakeet_bridge::is_available() {
            return Err(anyhow::anyhow!(
                "parakeet bridge not built into this binary (rebuild with --features parakeet on macOS arm64)"
            ));
        }
        let descriptor = models::descriptor_for(&self.model_id).ok_or_else(|| {
            anyhow::anyhow!("parakeet model '{}' not in catalog", self.model_id)
        })?;
        if descriptor.kind != ModelKind::Stt {
            return Err(anyhow::anyhow!(
                "model '{}' is not an STT model",
                self.model_id
            ));
        }
        let dir = self
            .fluid_audio_load_dir()
            .ok_or_else(|| anyhow::anyhow!("could not resolve app data dir for parakeet model"))?;
        // Probe the vocab file rather than the whole .mlmodelc tree —
        // it's the smallest required artifact and the catalog
        // downloader is expected to write it last so a partial download
        // doesn't trip a false-positive load attempt.
        if !dir.join(FLUIDAUDIO_VOCAB_FILE).exists() {
            return Err(anyhow::anyhow!(
                "parakeet model '{}' not downloaded — open Settings → Models",
                self.model_id
            ));
        }

        let cell = self
            .loaded
            .get_or_init(|| Arc::new(Mutex::new(Err("uninitialized".into()))));
        let mut guard = cell.lock();
        if guard.is_ok() {
            return Ok(());
        }
        match parakeet_bridge::load(&dir) {
            Ok(()) => {
                *guard = Ok(());
                Ok(())
            }
            Err(e) => {
                let msg = format!(
                    "parakeet load failed for '{}' at {}: {}",
                    self.model_id,
                    dir.display(),
                    e
                );
                *guard = Err(msg.clone());
                Err(anyhow::anyhow!(msg))
            }
        }
    }
}

#[async_trait::async_trait]
impl SttEngine for ParakeetLocalEngine {
    async fn transcribe(
        &self,
        wav: &[u8],
        // Parakeet v3 auto-detects across its 25 supported languages and
        // exposes no per-call hint; the user-side language picker drives
        // engine choice rather than a runtime hint.
        _language: Option<&str>,
        // Parakeet has no biasing/prompt API in v1 — the dictionary
        // path will land later via FluidAudio's vocab boosting layer.
        _dictionary: &[String],
        on_segment: Option<SegmentCallback>,
    ) -> anyhow::Result<String> {
        self.ensure_loaded()?;

        let wav_owned = wav.to_vec();
        let model_id = self.model_id.clone();

        #[cfg(feature = "local-engines")]
        {
            let result = tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<String> {
                let pcm = decode_wav_to_f32_mono_16k(&wav_owned)?;
                let started = std::time::Instant::now();
                let text = parakeet_bridge::transcribe_blocking(&pcm, TRANSCRIBE_TIMEOUT_SECS)
                    .map_err(|e| anyhow::anyhow!("parakeet ({model_id}): {e}"))?;
                log::info!(
                    "parakeet-local: transcribed {} samples → {} chars in {} ms",
                    pcm.len(),
                    text.len(),
                    started.elapsed().as_millis(),
                );
                Ok(text)
            })
            .await
            .map_err(|e| anyhow::anyhow!("parakeet-local task panicked: {e}"))??;

            // Single end-of-stream paint so the streaming pill stays in
            // sync with engines that emit partials, even though
            // Parakeet v1 in-process emits nothing mid-decode.
            if let Some(cb) = on_segment {
                cb(&result);
            }
            Ok(result)
        }

        #[cfg(not(feature = "local-engines"))]
        {
            let _ = (wav_owned, model_id, on_segment);
            Err(anyhow::anyhow!(
                "parakeet-local requires the local-engines feature for WAV decoding"
            ))
        }
    }

    fn id(&self) -> &'static str {
        "parakeet-local"
    }

    fn label(&self) -> String {
        "Parakeet (NVIDIA, on-device)".into()
    }

    fn requires_network(&self) -> bool {
        false
    }

    async fn prewarm(&self) -> anyhow::Result<()> {
        if !parakeet_bridge::is_available() {
            return Ok(());
        }
        let dir = match self.fluid_audio_load_dir() {
            Some(d) if d.join(FLUIDAUDIO_VOCAB_FILE).exists() => d,
            // Best-effort prewarm: silently skip if not downloaded yet.
            _ => return Ok(()),
        };
        let cell = self
            .loaded
            .get_or_init(|| Arc::new(Mutex::new(Err("uninitialized".into()))));
        let cell = cell.clone();
        let model_id = self.model_id.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let mut guard = cell.lock();
            if guard.is_ok() {
                return;
            }
            let started = std::time::Instant::now();
            match parakeet_bridge::load(&dir) {
                Ok(()) => {
                    *guard = Ok(());
                    log::info!(
                        "parakeet-local: prewarm load complete in {} ms (model={model_id})",
                        started.elapsed().as_millis()
                    );
                }
                Err(e) => {
                    log::info!(
                        "parakeet-local: prewarm load failed in {} ms: {e}",
                        started.elapsed().as_millis()
                    );
                    *guard = Err(e);
                }
            }
        })
        .await
        .map_err(|e| anyhow::anyhow!("parakeet prewarm task panicked: {e}"))?;
        Ok(())
    }
}
