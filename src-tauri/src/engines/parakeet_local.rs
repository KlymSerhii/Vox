//! NVIDIA Parakeet-TDT-0.6B-v3 STT engine.
//!
//! Engine ID: `"parakeet-local"`. On macOS aarch64 this delegates to the
//! Swift bridge in `parakeet_bridge`, which wraps FluidAudio's `AsrManager`
//! / `AsrModels` API (Core ML / ANE). On Windows it uses the `parakeet-rs`
//! Rust crate backed by ONNX Runtime with the DirectML execution provider
//! (DX12 GPU) and a CPU EP fallback.
//!
//! Both paths share:
//! - The same `SttEngine` trait surface.
//! - The same pre-warm OnceCell pattern used by `whisper_local`.
//! - Fail-open semantics: errors surface as `SttEngine::transcribe` `Err`,
//!   which `session.rs` turns into a friendly error pill.

use std::path::PathBuf;
#[cfg(any(
    all(target_os = "macos", target_arch = "aarch64", feature = "parakeet"),
    all(target_os = "windows", feature = "parakeet"),
))]
use std::sync::Arc;

#[cfg(all(target_os = "macos", target_arch = "aarch64", feature = "parakeet"))]
use parking_lot::Mutex;
use tauri::{AppHandle, Manager, Wry};

use crate::engines::{SegmentCallback, SttEngine};
use crate::models::{self, ModelKind};

#[cfg(feature = "local-engines")]
use crate::engines::whisper_local::decode_wav_to_f32_mono_16k;

use once_cell::sync::OnceCell;

// ── macOS-only constants ─────────────────────────────────────────────────────

/// Hardcoded to FluidAudio's `Repo.parakeetV3.folderName` (v0.14.3).
/// `AsrModels.load(from:)` reconstructs file paths from the parent of
/// the directory we hand it plus this folder name; if FluidAudio
/// renames the repo we'd need to update this in lockstep.
#[cfg(all(target_os = "macos", target_arch = "aarch64", feature = "parakeet"))]
const FLUIDAUDIO_REPO_FOLDER: &str = "parakeet-tdt-0.6b-v3-coreml";

/// Sentinel file the catalog downloader is expected to write last so
/// we can probe "fully populated" without enumerating every .mlmodelc.
#[cfg(all(target_os = "macos", target_arch = "aarch64", feature = "parakeet"))]
const FLUIDAUDIO_VOCAB_FILE: &str = "parakeet_vocab.json";

/// Transcription timeout forwarded to the Swift bridge.
#[cfg(all(target_os = "macos", target_arch = "aarch64", feature = "parakeet"))]
const TRANSCRIBE_TIMEOUT_SECS: u32 = 60;

// ── Windows ONNX backend ─────────────────────────────────────────────────────

/// Windows ONNX backend — parakeet-rs + ort + DirectML EP.
///
/// Gated on `cfg(all(target_os = "windows", feature = "parakeet"))` so
/// the macOS build never sees the `parakeet_rs` crate. All parakeet-rs
/// API calls live inside this module so the outer file stays clean.
#[cfg(all(target_os = "windows", feature = "parakeet"))]
mod onnx_imp {
    use anyhow::Result;
    use parakeet_rs::{ExecutionConfig, ExecutionProvider, ParakeetTDT, TimestampMode, Transcriber};
    use parking_lot::Mutex;
    use std::path::Path;
    use std::sync::Arc;

    /// Heap-allocated, shareable Parakeet session.
    pub type Loaded = Arc<Mutex<ParakeetTDT>>;

    /// Build a `ParakeetTDT` on the calling thread (must already be on a
    /// blocking task — `from_pretrained` does disk I/O and ONNX session
    /// init). The `ExecutionConfig` (and its internal `Rc<dyn Fn>`) is
    /// created here, consumed by `from_pretrained`, and not stored, so
    /// the resulting `Loaded` is `Send`.
    pub fn load_blocking(model_dir: &Path) -> Result<Loaded> {
        // DirectML EP with CPU fallback. The CPU EP is registered
        // as the second provider automatically by `parakeet-rs`'s
        // `apply_to_session_builder` when a GPU EP is selected.
        // Note: ExecutionConfig uses Rc internally (not Arc) so it is
        // !Send; build it here inside spawn_blocking, not on the async
        // side.
        //
        // `with_memory_pattern(false)` + `with_parallel_execution(false)`:
        // defensive settings for DirectML — disabling memory pattern bypasses
        // a known DirectML failure mode with dynamic shapes (mismatched
        // allocator pattern). `ExecutionConfig` is constructed and consumed
        // here on the same blocking task so its `!Send` Rc internals never
        // cross a thread boundary. The `?` converts `BuilderResult` via
        // `From<Error<SessionBuilder>> for Error<()>` which ort ships.
        let cfg = ExecutionConfig::new()
            .with_execution_provider(ExecutionProvider::DirectML)
            .with_custom_configure(|b| {
                Ok(b.with_memory_pattern(false)?.with_parallel_execution(false)?)
            });
        log::info!("parakeet-onnx: loading model from {}", model_dir.display());
        let model = ParakeetTDT::from_pretrained(model_dir, Some(cfg))
            .map_err(|e| anyhow::anyhow!("parakeet onnx load: {e}"))?;
        Ok(Arc::new(Mutex::new(model)))
    }

    /// Run inference on the calling thread (must already be on a blocking
    /// task). `transcribe_samples` takes `&mut self` so we hold the Mutex
    /// for the duration of the decode.
    pub fn transcribe_blocking(loaded: &Loaded, pcm: Vec<f32>) -> Result<String> {
        // Mutex held for entire decode; matches FluidAudio's `aned`-serialised
        // behaviour on macOS. SessionManager prevents overlapping sessions in
        // practice.
        let mut guard = loaded.lock();
        // TDT models predict punctuation; `Sentences` matches the upstream
        // `parakeet-rs` recommendation. We discard the timing data — only
        // `.text` is used.
        let result = guard
            .transcribe_samples(pcm, 16_000, 1, Some(TimestampMode::Sentences))
            .map_err(|e| anyhow::anyhow!("parakeet onnx transcribe: {e}"))?;
        Ok(result.text)
    }
}

// ── Engine struct ────────────────────────────────────────────────────────────

/// OnceCell shape for the macOS path: tracks load success via
/// `Arc<Mutex<Result<(), String>>>`.
#[cfg(all(target_os = "macos", target_arch = "aarch64", feature = "parakeet"))]
type MacLoadOnce = OnceCell<Arc<Mutex<Result<(), String>>>>;

pub struct ParakeetLocalEngine {
    app: AppHandle<Wry>,
    model_id: String,

    /// Windows: memoises the loaded ONNX session. Wrapped in Arc so the
    /// pre-warm closure can hold a reference without cloning OnceCell
    /// (which doesn't implement Clone). Matches the `ctx: OnceCell<Arc<…>>`
    /// pattern in `whisper_local`.
    #[cfg(all(target_os = "windows", feature = "parakeet"))]
    onnx_loaded: Arc<OnceCell<onnx_imp::Loaded>>,

    /// macOS: records load success/failure from the Swift bridge.
    #[cfg(all(target_os = "macos", target_arch = "aarch64", feature = "parakeet"))]
    loaded: MacLoadOnce,
}

impl ParakeetLocalEngine {
    pub fn new(app: AppHandle<Wry>, model_id: String) -> Self {
        Self {
            app,
            model_id,
            #[cfg(all(target_os = "windows", feature = "parakeet"))]
            onnx_loaded: Arc::new(OnceCell::new()),
            #[cfg(all(target_os = "macos", target_arch = "aarch64", feature = "parakeet"))]
            loaded: OnceCell::new(),
        }
    }

    /// Mirrors `models::root_dir` + `models::model_dir` without touching
    /// private helpers. On both hosts the bundle root is
    /// `<app_data>/models/stt/<model_id>/`.
    fn model_dir(&self) -> Option<PathBuf> {
        let root = self
            .app
            .path()
            .app_data_dir()
            .ok()
            .map(|p| p.join("models"))
            .or_else(|| dirs::data_dir().map(|p| p.join("app.vox.vox/models")))?;
        Some(root.join("stt").join(&self.model_id))
    }

    // ── macOS helpers ──────────────────────────────────────────────────────

    /// Path passed to FluidAudio's `AsrModels.load(from:)`.
    #[cfg(all(target_os = "macos", target_arch = "aarch64", feature = "parakeet"))]
    fn fluid_audio_load_dir(&self) -> Option<PathBuf> {
        Some(self.model_dir()?.join(FLUIDAUDIO_REPO_FOLDER))
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64", feature = "parakeet"))]
    fn ensure_loaded(&self) -> Result<(), anyhow::Error> {
        use crate::engines::parakeet_bridge;
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

// ── SttEngine implementation ─────────────────────────────────────────────────

#[async_trait::async_trait]
impl SttEngine for ParakeetLocalEngine {
    async fn transcribe(
        &self,
        wav: &[u8],
        // Parakeet v3 auto-detects across its 25 supported languages and
        // exposes no per-call hint; the user-side language picker drives
        // engine choice rather than a runtime hint.
        _language: Option<&str>,
        // Parakeet has no biasing/prompt API in v1 — dictionary path will
        // land later via a vocab boosting layer.
        _dictionary: &[String],
        on_segment: Option<SegmentCallback>,
    ) -> anyhow::Result<String> {
        // ── macOS branch ───────────────────────────────────────────────────
        #[cfg(all(target_os = "macos", target_arch = "aarch64", feature = "parakeet"))]
        {
            self.ensure_loaded()?;

            let wav_owned = wav.to_vec();
            let model_id = self.model_id.clone();

            #[cfg(feature = "local-engines")]
            {
                let result =
                    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<String> {
                        use crate::engines::parakeet_bridge;
                        let pcm = decode_wav_to_f32_mono_16k(&wav_owned)?;
                        let started = std::time::Instant::now();
                        let text =
                            parakeet_bridge::transcribe_blocking(&pcm, TRANSCRIBE_TIMEOUT_SECS)
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

                if let Some(cb) = on_segment {
                    cb(&result);
                }
                return Ok(result);
            }

            #[cfg(not(feature = "local-engines"))]
            {
                let _ = (wav_owned, model_id, on_segment);
                return Err(anyhow::anyhow!(
                    "parakeet-local requires the local-engines feature for WAV decoding"
                ));
            }
        }

        // ── Windows ONNX branch ────────────────────────────────────────────
        #[cfg(all(target_os = "windows", feature = "parakeet"))]
        {
            let dir = self
                .model_dir()
                .ok_or_else(|| anyhow::anyhow!("could not resolve app data dir for parakeet model"))?;
            if !dir.join("vocab.txt").exists() {
                return Err(anyhow::anyhow!(
                    "parakeet model '{}' not downloaded — open Settings → Models",
                    self.model_id
                ));
            }

            let wav_owned = wav.to_vec();
            let model_id = self.model_id.clone();
            let cell = self.onnx_loaded.clone();

            #[cfg(feature = "local-engines")]
            {
                let result =
                    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<String> {
                        // get_or_try_init serialises concurrent first-loads so only
                        // one thread runs load_blocking (10-30 s of ONNX + DirectML
                        // init). Stable since once_cell 1.7 — no nightly needed.
                        let loaded = cell
                            .get_or_try_init(|| onnx_imp::load_blocking(&dir))?
                            .clone();

                        let pcm = decode_wav_to_f32_mono_16k(&wav_owned)?;
                        let started = std::time::Instant::now();
                        let text = onnx_imp::transcribe_blocking(&loaded, pcm)
                            .map_err(|e| anyhow::anyhow!("parakeet ({model_id}): {e}"))?;
                        log::info!(
                            "parakeet-onnx: transcribed → {} chars in {} ms",
                            text.len(),
                            started.elapsed().as_millis(),
                        );
                        Ok(text)
                    })
                    .await
                    .map_err(|e| anyhow::anyhow!("parakeet-onnx task panicked: {e}"))??;

                // Single end-of-stream paint so the streaming pill stays in
                // sync with engines that emit partials.
                if let Some(cb) = on_segment {
                    cb(&result);
                }
                return Ok(result);
            }

            #[cfg(not(feature = "local-engines"))]
            {
                let _ = (wav_owned, model_id, on_segment);
                return Err(anyhow::anyhow!(
                    "parakeet-local requires the local-engines feature for WAV decoding"
                ));
            }
        }

        // ── Fallback for non-macOS, non-Windows ────────────────────────────
        #[cfg(not(any(
            all(target_os = "macos", target_arch = "aarch64", feature = "parakeet"),
            all(target_os = "windows", feature = "parakeet")
        )))]
        {
            let _ = (wav, on_segment);
            Err(anyhow::anyhow!(
                "parakeet-local not available on this platform/build"
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
        // ── macOS branch ───────────────────────────────────────────────────
        #[cfg(all(target_os = "macos", target_arch = "aarch64", feature = "parakeet"))]
        {
            use crate::engines::parakeet_bridge;
            if !parakeet_bridge::is_available() {
                return Ok(());
            }
            let dir = match self.fluid_audio_load_dir() {
                Some(d) if d.join(FLUIDAUDIO_VOCAB_FILE).exists() => d,
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
                        log::warn!(
                            "parakeet-local: prewarm load failed in {} ms: {e}",
                            started.elapsed().as_millis()
                        );
                        *guard = Err(e);
                    }
                }
            })
            .await
            .map_err(|e| anyhow::anyhow!("parakeet prewarm task panicked: {e}"))?;
            return Ok(());
        }

        // ── Windows ONNX branch ────────────────────────────────────────────
        #[cfg(all(target_os = "windows", feature = "parakeet"))]
        {
            let dir = match self.model_dir() {
                Some(d) if d.join("vocab.txt").exists() => d,
                // Best-effort prewarm: silently skip if model not on disk yet.
                _ => return Ok(()),
            };
            let cell = self.onnx_loaded.clone();
            let model_id = self.model_id.clone();
            tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<()> {
                if cell.get().is_some() {
                    return Ok(());
                }
                let started = std::time::Instant::now();
                let loaded = onnx_imp::load_blocking(&dir)?;
                let _ = cell.set(loaded);
                log::info!(
                    "parakeet-onnx: prewarm complete in {} ms (model={model_id})",
                    started.elapsed().as_millis()
                );
                Ok(())
            })
            .await
            .map_err(|e| anyhow::anyhow!("parakeet prewarm task panicked: {e}"))??;
            return Ok(());
        }

        // ── Fallback ───────────────────────────────────────────────────────
        #[cfg(not(any(
            all(target_os = "macos", target_arch = "aarch64", feature = "parakeet"),
            all(target_os = "windows", feature = "parakeet")
        )))]
        Ok(())
    }
}
