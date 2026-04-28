//! Engine trait surfaces for STT and LLM cleanup.
//!
//! Concrete engine implementations:
//! - [`whisper_local`] — `whisper-rs` 0.16 with Core ML + Metal,
//!   in-process. Stable, hasn't been the source of Vox crashes.
//! - [`gemma_http`] — talks to an out-of-process `llama-server`
//!   subprocess over loopback HTTP. Replaced the old in-process
//!   `gemma_local` (`llama-cpp-2`) on 2026-04-27 so a C++ assert in
//!   llama.cpp can no longer crash the menu-bar app — the assert
//!   exits the child, the supervisor respawns, and the user keeps
//!   working. Engine ID is unchanged (`"gemma-local"`) because the
//!   user's *intent* (polish locally) is unchanged.
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
use tauri::{AppHandle, Manager, Wry};

pub mod gemma_http;
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

    /// Optional pre-warm hook. Loads any heavy state (model weights,
    /// subprocess handles) into RAM so the first transcription doesn't
    /// pay the cold-load cost. Default impl is a no-op for engines
    /// that don't need it.
    async fn prewarm(&self) -> anyhow::Result<()> {
        Ok(())
    }
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
            "gemma-local" => Arc::new(gemma_http::GemmaHttpEngine::new(
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

/// Conditionally pre-warm the active STT engine in the background.
/// No-op when:
/// - the user hasn't finished onboarding, or
/// - dictation is disabled, or
/// - no model id is configured, or
/// - the engine id isn't recognised, or
/// - the engine's own prewarm returns a benign error (e.g. model not
///   downloaded — the real `transcribe()` path will surface it later).
///
/// Idempotent: spawning more than once is safe because
/// `WhisperLocalEngine::prewarm` is itself idempotent (the
/// `OnceCell::get_or_try_init` short-circuits on the second pass).
///
/// Fire-and-forget: the actual model load runs on a Tokio task so it
/// never delays the caller (UI startup, settings save, toggle flip).
pub fn prewarm_stt_in_background(app: &AppHandle<Wry>) {
    let s = crate::settings::load(app);
    if !s.onboarded || !s.dictation_enabled || s.stt_model_id.is_empty() {
        return;
    }
    let registry = match app.try_state::<Registry>() {
        Some(r) => r,
        None => return,
    };
    let engine = match registry.stt(app, &s.stt_engine, &s.stt_model_id) {
        Some(e) => e,
        None => return,
    };
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
