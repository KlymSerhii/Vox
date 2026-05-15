//! `gemma_http` runs out-of-process so a C++ assert in llama.cpp exits
//! the child rather than the menu-bar app; engine ID is still
//! `"gemma-local"` to preserve user intent across the move.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use tauri::{AppHandle, Manager, Wry};

#[cfg(all(target_os = "macos", feature = "apple-fm"))]
pub mod apple_fm;
#[cfg(all(target_os = "macos", feature = "apple-fm"))]
pub mod apple_fm_bridge;
pub mod gemma_http;
pub mod none;
// Parakeet bridge module compiles unconditionally so the Registry can
// reference its types from any cfg; the Swift FFI is only linked in
// when the parakeet feature + macOS arm64 target apply (see
// `parakeet_bridge::imp` cfg branches).
pub mod parakeet_bridge;
pub mod parakeet_local;
pub mod whisper_local;

/// Optional callback fired by an STT engine as it produces partial
/// segments during decode. `text` is the **cumulative** transcript so
/// far (engines must accumulate inside the impl, not pass deltas).
/// Called from whatever thread the engine is running decode on — keep
/// the closure cheap and `Send + Sync + 'static`.
pub type SegmentCallback = Box<dyn Fn(&str) + Send + Sync + 'static>;

/// Optional callback fired by an LLM engine as polish tokens stream in.
/// `text` is the **cumulative** polished output so far (engines
/// accumulate deltas internally). Same threading rules as
/// [`SegmentCallback`].
pub type ChunkCallback = Box<dyn Fn(&str) + Send + Sync + 'static>;

#[async_trait::async_trait]
pub trait SttEngine: Send + Sync {
    /// `wav` is 16 kHz mono PCM-16 (the shape `audio::encode_recorded`
    /// produces). `language` is a Whisper code or `None` for auto.
    /// `on_segment` (if Some) receives cumulative transcript text as
    /// segments arrive — used by the streaming pill.
    async fn transcribe(
        &self,
        wav: &[u8],
        language: Option<&str>,
        dictionary: &[String],
        on_segment: Option<SegmentCallback>,
    ) -> anyhow::Result<String>;

    #[allow(dead_code)]
    fn id(&self) -> &'static str;

    #[allow(dead_code)]
    fn label(&self) -> String;

    #[allow(dead_code)]
    fn requires_network(&self) -> bool;

    async fn prewarm(&self) -> anyhow::Result<()> {
        Ok(())
    }
}

#[async_trait::async_trait]
pub trait LlmEngine: Send + Sync {
    /// `enable_thinking` is the resolved per-session value (mode override
    /// > global setting). Engines that have no thinking-mode equivalent
    /// (Apple Intelligence, the no-op engine) ignore it.
    async fn cleanup(
        &self,
        transcript: &str,
        system_prompt: &str,
        dictionary: &[String],
        enable_thinking: bool,
    ) -> anyhow::Result<String>;

    /// Streaming variant of `cleanup` — fires `on_chunk` repeatedly with
    /// the cumulative output so far while the polish runs, then
    /// resolves with the final cleaned text. Default impl falls back
    /// to non-streaming `cleanup` and emits a single chunk on success
    /// so callers always get at least one cumulative-text update.
    async fn cleanup_stream(
        &self,
        transcript: &str,
        system_prompt: &str,
        dictionary: &[String],
        enable_thinking: bool,
        on_chunk: ChunkCallback,
    ) -> anyhow::Result<String> {
        let out = self
            .cleanup(transcript, system_prompt, dictionary, enable_thinking)
            .await?;
        on_chunk(&out);
        Ok(out)
    }

    fn id(&self) -> &'static str;
    #[allow(dead_code)]
    fn label(&self) -> String;
    #[allow(dead_code)]
    fn requires_network(&self) -> bool;
}

#[derive(Default, Clone)]
pub struct Registry {
    inner: Arc<Mutex<RegistryInner>>,
}

#[derive(Default)]
struct RegistryInner {
    stt: HashMap<String, Arc<dyn SttEngine>>,
    llm: HashMap<String, Arc<dyn LlmEngine>>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn stt(
        &self,
        app: &AppHandle<Wry>,
        engine_id: &str,
        model_id: &str,
    ) -> Option<Arc<dyn SttEngine>> {
        // Key includes model_id so swapping models invalidates the cached engine.
        let key = format!("{engine_id}::{model_id}");
        {
            let guard = self.inner.lock();
            if let Some(existing) = guard.stt.get(&key) {
                return Some(existing.clone());
            }
        }

        let engine: Arc<dyn SttEngine> = match engine_id {
            "whisper-local" => Arc::new(whisper_local::WhisperLocalEngine::new(
                app.clone(),
                model_id.to_string(),
            )),
            "parakeet-local" => Arc::new(parakeet_local::ParakeetLocalEngine::new(
                app.clone(),
                model_id.to_string(),
            )),
            _ => return None,
        };

        let mut guard = self.inner.lock();
        guard.stt.insert(key, engine.clone());
        Some(engine)
    }

    pub fn llm(
        &self,
        app: &AppHandle<Wry>,
        engine_id: &str,
        model_id: &str,
    ) -> Option<Arc<dyn LlmEngine>> {
        let key = format!("{engine_id}::{model_id}");
        {
            let guard = self.inner.lock();
            if let Some(existing) = guard.llm.get(&key) {
                return Some(existing.clone());
            }
        }

        let engine: Arc<dyn LlmEngine> = match engine_id {
            "gemma-local" => Arc::new(gemma_http::GemmaHttpEngine::new(
                app.clone(),
                model_id.to_string(),
            )),
            #[cfg(all(target_os = "macos", feature = "apple-fm"))]
            "apple-fm" => Arc::new(apple_fm::AppleFmEngine::new()),
            "none" => Arc::new(none::NoOpEngine),
            _ => return None,
        };

        let mut guard = self.inner.lock();
        guard.llm.insert(key, engine.clone());
        Some(engine)
    }

    /// Drop any cached engine instance keyed on `model_id`. Used by
    /// `models::delete_model` so a cached Whisper context releases its
    /// mmap on the about-to-be-deleted file — without this the OS keeps
    /// the inode alive (disk space stays reserved) until the user
    /// quits Vox.
    pub fn drop_for_model(&self, model_id: &str) {
        let suffix = format!("::{model_id}");
        let mut guard = self.inner.lock();
        guard.stt.retain(|k, _| !k.ends_with(&suffix));
        guard.llm.retain(|k, _| !k.ends_with(&suffix));
        drop(guard);

        // The Parakeet engine's cached `AsrManager` lives in the Swift
        // `ModelsCache` actor, not in the Rust Registry — dropping the
        // Rust-side `Arc<ParakeetLocalEngine>` doesn't release it.
        // Without this evict, a delete + re-download cycle would keep
        // serving transcribes from the stale pre-delete in-memory
        // copy until the app restarts.
        if model_id == "parakeet-tdt-0.6b-v3" {
            parakeet_bridge::evict();
        }
    }
}

/// Fire-and-forget pre-warm. Idempotent — `WhisperLocalEngine::prewarm`'s
/// `OnceCell` short-circuits a second call.
pub fn prewarm_stt_in_background(app: &AppHandle<Wry>) {
    let s = crate::settings::load(app);
    if !s.onboarded || !s.dictation_enabled || s.stt_model_id.is_empty() {
        return;
    }
    let Some(registry) = app.try_state::<Registry>() else { return };
    let Some(engine) = registry.stt(app, &s.stt_engine, &s.stt_model_id) else { return };
    let model_id = s.stt_model_id.clone();
    tauri::async_runtime::spawn(async move {
        let started = std::time::Instant::now();
        log::info!("stt: pre-warming (model={model_id})");
        match engine.prewarm().await {
            Ok(_) => log::info!(
                "stt: pre-warm complete in {} ms",
                started.elapsed().as_millis()
            ),
            Err(e) => log::info!(
                "stt: pre-warm skipped after {} ms: {e}",
                started.elapsed().as_millis()
            ),
        }
    });
}
