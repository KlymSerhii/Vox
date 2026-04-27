//! Engine trait surfaces for STT and LLM cleanup.
//!
//! There are concrete engine implementations under this module:
//! - [`whisper_local`] — `whisper-rs` 0.16 with Core ML + Metal.
//! - [`gemma_local`] — `llama-cpp-2` 0.1 with Metal (Gemma 4 / Phi-4-mini).
//! - [`none`] — no-op cleanup engine that returns its input unchanged.
//!
//! 2026-04-26: cloud engines (Groq STT, Anthropic LLM) were removed so
//! the app is fully on-device — no transcript or audio ever leaves the
//! Mac. The registry only resolves local engine ids; any stale cloud id
//! coming from settings is migrated to its local equivalent before it
//! reaches this layer (see `settings::load`).
//!
//! The session orchestrator picks the active engine instances at runtime
//! from the engine registry (see [`Registry`]). Engines are lazy-loaded
//! and kept warm.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use tauri::{AppHandle, Wry};

pub mod gemma_local;
pub mod none;
pub mod whisper_local;

/// Speech-to-text engine.
#[async_trait::async_trait]
pub trait SttEngine: Send + Sync {
    /// Transcribe a 16 kHz mono PCM-16 WAV byte buffer (the shape produced
    /// by `audio::stop_and_encode`). `language` is a Whisper code (`"en"`,
    /// `"es"`, …) or `None` for auto-detect. `dictionary` biases output
    /// toward proper-noun spellings.
    async fn transcribe(
        &self,
        wav: &[u8],
        language: Option<&str>,
        dictionary: &[String],
    ) -> anyhow::Result<String>;

    /// Stable id used in settings (currently always `"whisper-local"`).
    #[allow(dead_code)]
    fn id(&self) -> &'static str;

    /// Human-readable label for UI.
    #[allow(dead_code)]
    fn label(&self) -> String;

    /// True if this engine sends data over the network.
    #[allow(dead_code)]
    fn requires_network(&self) -> bool;
}

/// LLM cleanup engine. The optional `system_prompt` is reserved for the
/// upcoming Voice Modes feature: when empty the engine bakes its own
/// default cleanup prompt; when non-empty, that prompt overrides ours.
/// All callers pass `""` for v1 — Voice Modes will pass a mode-specific
/// prompt later.
#[async_trait::async_trait]
pub trait LlmEngine: Send + Sync {
    async fn cleanup(
        &self,
        transcript: &str,
        system_prompt: &str,
        dictionary: &[String],
    ) -> anyhow::Result<String>;

    fn id(&self) -> &'static str;
    #[allow(dead_code)]
    fn label(&self) -> String;
    #[allow(dead_code)]
    fn requires_network(&self) -> bool;
}

/// Lazy, process-wide engine registry. Built at startup; engines are
/// instantiated on first use and cached in this map.
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

    /// Resolve (or lazy-build) the STT engine for the given settings
    /// `stt_engine` value. Returns `None` if the id is unknown.
    pub fn stt(
        &self,
        app: &AppHandle<Wry>,
        engine_id: &str,
        model_id: &str,
    ) -> Option<Arc<dyn SttEngine>> {
        // Cache key combines engine id + model id so swapping models does
        // not return a stale loaded model.
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
            "gemma-local" => Arc::new(gemma_local::GemmaLocalEngine::new(
                app.clone(),
                model_id.to_string(),
            )),
            "none" => Arc::new(none::NoOpEngine),
            _ => return None,
        };

        let mut guard = self.inner.lock();
        guard.llm.insert(key, engine.clone());
        Some(engine)
    }
}
