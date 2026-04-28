//! `gemma_http` runs out-of-process so a C++ assert in llama.cpp exits
//! the child rather than the menu-bar app; engine ID is still
//! `"gemma-local"` to preserve user intent across the move.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use tauri::{AppHandle, Manager, Wry};

pub mod gemma_http;
pub mod none;
pub mod whisper_local;

#[async_trait::async_trait]
pub trait SttEngine: Send + Sync {
    /// `wav` is 16 kHz mono PCM-16 (the shape `audio::stop_and_encode`
    /// produces). `language` is a Whisper code or `None` for auto.
    async fn transcribe(
        &self,
        wav: &[u8],
        language: Option<&str>,
        dictionary: &[String],
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
