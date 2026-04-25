//! Local LLM cleanup — `llama-cpp-2` 0.1 with Metal.
//!
//! Loads a GGUF model on first use, keeps it warm for repeat dictations.
//! Hard 3 s wall-clock timeout per the existing fail-open pattern: if the
//! model takes longer than that, the session falls back to the raw
//! transcript.
//!
//! NB: the actual inference path is gated behind the `local-engines`
//! Cargo feature. Without that feature this engine compiles into a stub
//! that returns the input unchanged plus a debug log — useful for
//! cloud-only builds and CI `cargo check`.

#[cfg(feature = "local-engines")]
use std::sync::Arc;
#[cfg(feature = "local-engines")]
use std::time::Duration;

#[cfg(feature = "local-engines")]
use parking_lot::Mutex;
use tauri::{AppHandle, Wry};

use crate::engines::LlmEngine;
use crate::models::{self, ModelKind};

#[cfg(feature = "local-engines")]
use once_cell::sync::OnceCell;

#[cfg(feature = "local-engines")]
const HARD_TIMEOUT: Duration = Duration::from_secs(3);

const DEFAULT_SYSTEM_PROMPT: &str = "You are a transcript cleaner. Output ONLY the cleaned text. Do not add commentary, prefaces, or quotes.

Rules:
- Remove filler words (um, uh, you know, like, basically) only when they are filler — keep them when they're meaningful.
- Apply correct punctuation and capitalization.
- Fix obvious self-corrections (\"go to the — I mean — the store\" → \"go to the store\").
- Preserve the speaker's register: do not make casual text formal or vice versa.
- Never add content the speaker did not say.";

pub struct GemmaLocalEngine {
    app: AppHandle<Wry>,
    model_id: String,
    #[cfg(feature = "local-engines")]
    inner: OnceCell<Arc<Mutex<llama_cpp_2::model::LlamaModel>>>,
    #[cfg(feature = "local-engines")]
    backend: OnceCell<Arc<llama_cpp_2::llama_backend::LlamaBackend>>,
}

impl GemmaLocalEngine {
    pub fn new(app: AppHandle<Wry>, model_id: String) -> Self {
        Self {
            app,
            model_id,
            #[cfg(feature = "local-engines")]
            inner: OnceCell::new(),
            #[cfg(feature = "local-engines")]
            backend: OnceCell::new(),
        }
    }
}

#[async_trait::async_trait]
impl LlmEngine for GemmaLocalEngine {
    async fn cleanup(
        &self,
        transcript: &str,
        system_prompt: &str,
        dictionary: &[String],
    ) -> anyhow::Result<String> {
        let trimmed = transcript.trim();
        if trimmed.split_whitespace().count() < 2 {
            // Match the cloud path: skip cleanup on very short utterances.
            return Ok(trimmed.to_string());
        }

        let path = models::cached_model_path(&self.app, &self.model_id, ModelKind::Llm)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "model '{}' not downloaded — open Settings → Models",
                    self.model_id
                )
            })?;

        let prompt = build_prompt(system_prompt, trimmed, dictionary);

        // Run inference on a blocking thread with a hard 3 s timeout.
        let model_id = self.model_id.clone();
        let _ = (path, prompt, model_id);

        #[cfg(feature = "local-engines")]
        {
            let path_owned = models::cached_model_path(&self.app, &self.model_id, ModelKind::Llm)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "model '{}' not downloaded — open Settings → Models",
                        self.model_id
                    )
                })?;
            let prompt = build_prompt(system_prompt, trimmed, dictionary);
            let backend_cell = self.backend.clone();
            let model_cell = self.inner.clone();

            let handle = tauri::async_runtime::spawn_blocking(move || {
                let backend = backend_cell.get_or_try_init::<_, anyhow::Error>(|| {
                    let b = llama_cpp_2::llama_backend::LlamaBackend::init()
                        .map_err(|e| anyhow::anyhow!("llama backend init: {e}"))?;
                    Ok(Arc::new(b))
                })?;

                let model = model_cell.get_or_try_init::<_, anyhow::Error>(|| {
                    let mp = llama_cpp_2::model::params::LlamaModelParams::default();
                    let m = llama_cpp_2::model::LlamaModel::load_from_file(
                        backend, &path_owned, &mp,
                    )
                    .map_err(|e| anyhow::anyhow!("llama load: {e}"))?;
                    Ok(Arc::new(Mutex::new(m)))
                })?;

                run_completion(backend, model, &prompt)
            });

            let timed = tokio::time::timeout(HARD_TIMEOUT, handle).await;
            let result = match timed {
                Ok(Ok(r)) => r,
                Ok(Err(join_err)) => {
                    return Err(anyhow::anyhow!("gemma task panicked: {join_err}"));
                }
                Err(_) => {
                    return Err(anyhow::anyhow!("gemma cleanup timed out"));
                }
            };
            return result;
        }

        #[cfg(not(feature = "local-engines"))]
        {
            log::debug!(
                "[gemma-local] feature disabled — returning raw transcript for {}",
                self.model_id
            );
            Ok(trimmed.to_string())
        }
    }

    fn id(&self) -> &'static str {
        "gemma-local"
    }

    fn label(&self) -> String {
        format!("Local Gemma ({})", self.model_id)
    }

    fn requires_network(&self) -> bool {
        false
    }
}

fn build_prompt(system_override: &str, transcript: &str, dictionary: &[String]) -> String {
    let system = if system_override.trim().is_empty() {
        DEFAULT_SYSTEM_PROMPT.to_string()
    } else {
        system_override.to_string()
    };
    let cleaned: Vec<&str> = dictionary
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    let dict_block = if cleaned.is_empty() {
        String::new()
    } else {
        format!(
            "\n\nPreserve these proper nouns and acronyms exactly as written: {}.",
            cleaned.join(", ")
        )
    };

    // Use the Gemma instruction-tuned chat template. The exact tags are
    // `<start_of_turn>user … <end_of_turn>\n<start_of_turn>model …`.
    format!(
        "<start_of_turn>user\n{system}{dict_block}\n\nClean this transcript:\n{transcript}<end_of_turn>\n<start_of_turn>model\n"
    )
}

#[cfg(feature = "local-engines")]
fn run_completion(
    backend: &llama_cpp_2::llama_backend::LlamaBackend,
    model_lock: &Arc<Mutex<llama_cpp_2::model::LlamaModel>>,
    prompt: &str,
) -> anyhow::Result<String> {
    use llama_cpp_2::context::params::LlamaContextParams;
    use llama_cpp_2::llama_batch::LlamaBatch;
    use llama_cpp_2::model::AddBos;
    use std::num::NonZeroU32;

    let model = model_lock.lock();

    let n_ctx = NonZeroU32::new(2048).unwrap();
    let cparams = LlamaContextParams::default().with_n_ctx(Some(n_ctx));
    let mut ctx = model
        .new_context(backend, cparams)
        .map_err(|e| anyhow::anyhow!("llama context: {e}"))?;

    let tokens = model
        .str_to_token(prompt, AddBos::Always)
        .map_err(|e| anyhow::anyhow!("tokenize: {e}"))?;

    let mut batch = LlamaBatch::new(2048, 1);
    let last_idx = tokens.len() as i32 - 1;
    for (i, tok) in tokens.iter().enumerate() {
        batch
            .add(*tok, i as i32, &[0], i as i32 == last_idx)
            .map_err(|e| anyhow::anyhow!("batch add: {e}"))?;
    }

    ctx.decode(&mut batch)
        .map_err(|e| anyhow::anyhow!("decode prompt: {e}"))?;

    let mut out = String::new();
    let mut n_cur = batch.n_tokens();
    let max_new = 1024i32;
    let mut decoder = encoding_rs::UTF_8.new_decoder();

    // Simple greedy decoder. We pull the post-decode candidate
    // distribution from the context, pick the argmax token, append the
    // detokenized piece, then feed the chosen token back into the
    // batch. Sampling sophistication (temperature, top-p) can come in a
    // follow-up — for the cleanup task greedy + the tightly scoped
    // system prompt is competitive.
    for _ in 0..max_new {
        let mut best_tok: Option<llama_cpp_2::token::LlamaToken> = None;
        let mut best_logit: f32 = f32::NEG_INFINITY;
        for c in ctx.candidates() {
            if c.logit() > best_logit {
                best_logit = c.logit();
                best_tok = Some(c.id());
            }
        }
        let Some(tok) = best_tok else { break };
        if model.is_eog_token(tok) {
            break;
        }
        let piece = model
            .token_to_piece(tok, &mut decoder, false, None)
            .unwrap_or_default();
        out.push_str(&piece);
        if out.contains("<end_of_turn>") {
            out = out.replace("<end_of_turn>", "");
            break;
        }
        batch.clear();
        batch
            .add(tok, n_cur, &[0], true)
            .map_err(|e| anyhow::anyhow!("batch add gen: {e}"))?;
        n_cur += 1;
        ctx.decode(&mut batch)
            .map_err(|e| anyhow::anyhow!("decode gen: {e}"))?;
    }

    Ok(out.trim().to_string())
}
