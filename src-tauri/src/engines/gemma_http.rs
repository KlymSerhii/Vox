//! HTTP-based LLM cleanup engine. Talks to the out-of-process
//! `llama-server` over loopback HTTP using the OpenAI chat-completions
//! shape.
//!
//! Replaces the old in-process `gemma_local.rs` (`llama-cpp-2`). The
//! whole point of the move is crash isolation: a C++ assert in
//! `llama-server` exits the child process with a non-zero status,
//! `inference_proc` notices, and either respawns transparently on the
//! next request or returns `Err` and lets `session.rs::run`'s
//! existing fail-open path ship the raw Whisper transcript. Either
//! way the menu-bar app stays alive.
//!
//! Engine ID is unchanged (`"gemma-local"`). Settings persist the
//! user's *intent* ("polish locally"), not the implementation
//! detail of how. No migration required.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Wry};

use crate::engines::LlmEngine;
use crate::inference_proc;

/// Wall-clock ceiling on the whole HTTP polish call, including a
/// possible respawn of the server. Sized for the worst case: a
/// 30-min dictation (~6.7k input tokens, ~7k output tokens) at the
/// measured ~52 tok/s on Apple Silicon = ~135 s generation + ~30 s
/// prompt eval + ~10 s margin. A short polish finishes in under
/// 2 s and is unaffected by this ceiling — it only matters for
/// the long-form path.
const HARD_TIMEOUT: Duration = Duration::from_secs(240);

/// Lower bound on `max_tokens` so a one-sentence polish always has
/// room to emit punctuation + the eos token.
const MIN_NEW_TOKENS: u32 = 256;

/// Upper bound on `max_tokens`. At ~52 tok/s this corresponds to
/// ~160 s of pure generation — past that we'd rather fail-open to
/// the raw transcript than wait. 30 min of speech is ~6.7k tokens
/// of input which polishes to ≤ that many output tokens, so 8192
/// covers the worst supported case with margin for structural
/// markup (numbered lists, paragraph breaks) the prompt below now
/// asks for.
const MAX_NEW_TOKENS_CAP: u32 = 8192;

/// Estimate `max_tokens` from input char count. Polish output is
/// bounded by input — cleanup is ≈ a subset of input — so we size
/// the cap proportionally instead of always asking llama-server
/// for a full 8k tokens regardless of input size. The 1.2× factor
/// pads for structural markup the rewritten prompt encourages
/// (numbered lists, paragraph breaks) without giving the model
/// license to ramble.
///
/// English BPE averages ~4 chars/token; the formula is therefore
/// `(input_chars / 4) * 1.2 + base`.
fn dynamic_max_tokens(input_chars: usize) -> i32 {
    let estimated_input = input_chars / 4;
    let dynamic = ((estimated_input * 12) / 10 + 128) as u32;
    dynamic.clamp(MIN_NEW_TOKENS, MAX_NEW_TOKENS_CAP) as i32
}

const DEFAULT_SYSTEM_PROMPT: &str = "You are a transcript cleaner. Output ONLY the cleaned text. Do not output reasoning, planning, commentary, prefaces, or quotes.

Rules:
1. Remove filler (\"um\", \"uh\", \"you know\", \"like\", \"basically\", \"I mean\") when used as filler. Keep them when they carry meaning (\"I mean it.\").
2. Apply standard English punctuation and capitalization. For long passages, group related ideas into paragraphs separated by blank lines.
3. Fix mid-sentence self-corrections.
   - \"go to the — I mean — the store\" → \"go to the store\"
   - \"we ship Friday, no Thursday\" → \"we ship Thursday\"
4. Drop content the speaker explicitly retracts — including everything before the retraction marker.
   - \"X. Actually no, disregard that. Y.\" → \"Y.\"
   - \"Send it to John. Wait, scratch that, send it to Mary.\" → \"Send it to Mary.\"
   - \"Forget what I just said. Z.\" → \"Z.\"
5. When the speaker enumerates items or steps, format them as a numbered list. Only do this when the enumeration is explicit (\"first… second… third…\", \"the three things are…\", \"step one… step two…\"). Leave flowing narrative as prose.
6. Preserve the speaker's register. Do not formalize casual speech or vice versa.
7. Never add content, facts, names, or numbers the speaker did not say.

Examples:
- \"uh so like the meeting is um at three pm\" → \"The meeting is at 3 PM.\"
- \"I'm not using the model, no, disregard that, I am using the model\" → \"I am using the model.\"
- \"first we ship the build then we run tests and finally we deploy\" →
   \"1. Ship the build.\n2. Run tests.\n3. Deploy.\"";

pub struct GemmaHttpEngine {
    app: AppHandle<Wry>,
    model_id: String,
    client: reqwest::Client,
}

impl GemmaHttpEngine {
    pub fn new(app: AppHandle<Wry>, model_id: String) -> Self {
        // One client per engine instance so we benefit from
        // connection reuse across rapid-fire dictations. Keepalive
        // matters on macOS — opening a fresh socket per request
        // adds ~10 ms which compounds across short utterances.
        let client = reqwest::Client::builder()
            .timeout(HARD_TIMEOUT)
            // No pooling cap surprises here: we only ever talk to
            // the single loopback server, but be explicit so a
            // future move to multi-server doesn't silently throttle.
            .pool_max_idle_per_host(2)
            .build()
            .unwrap_or_else(|e| {
                log::warn!("gemma-http: client build failed ({e}); using default");
                reqwest::Client::new()
            });
        Self { app, model_id, client }
    }
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    max_tokens: i32,
    temperature: f32,
    stream: bool,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatChoiceMessage,
}

#[derive(Deserialize)]
struct ChatChoiceMessage {
    content: String,
}

#[async_trait::async_trait]
impl LlmEngine for GemmaHttpEngine {
    async fn cleanup(
        &self,
        transcript: &str,
        system_prompt: &str,
        dictionary: &[String],
    ) -> anyhow::Result<String> {
        let trimmed = transcript.trim();
        if trimmed.split_whitespace().count() < 2 {
            // Match the legacy local-engine semantics: skip cleanup
            // on near-empty utterances so the user sees their words
            // immediately instead of paying a roundtrip for nothing.
            return Ok(trimmed.to_string());
        }

        let started = Instant::now();
        log::info!(
            "gemma-http: polishing {} chars with model={}",
            trimmed.len(),
            self.model_id
        );

        let request_once = || async {
            let server = inference_proc::ensure_running(&self.app, &self.model_id).await?;
            self.post_cleanup(&server, system_prompt, dictionary, trimmed).await
        };

        // Single retry on the supervisor: if the cached child is
        // stale (assert during the previous request, OOM-kill, etc.)
        // the first attempt fails, the supervisor's `is_alive` check
        // notices, and the second attempt spawns fresh. A second
        // failure is a real problem and bubbles up as Err — at that
        // point session.rs falls open to the raw transcript.
        let result = match request_once().await {
            Ok(s) => Ok(s),
            Err(first) => {
                log::warn!(
                    "gemma-http: first request failed ({first}); dropping handle and retrying once"
                );
                inference_proc::drop_managed(&self.app);
                request_once().await.map_err(|second| {
                    anyhow::anyhow!("gemma-http retry failed: {first} | {second}")
                })
            }
        };

        match &result {
            Ok(out) => log::info!(
                "gemma-http: polished {}→{} chars in {} ms",
                trimmed.len(),
                out.len(),
                started.elapsed().as_millis()
            ),
            Err(e) => log::warn!(
                "gemma-http: polish FAILED in {} ms — session will fail-open to raw transcript: {e}",
                started.elapsed().as_millis()
            ),
        }
        result
    }

    fn id(&self) -> &'static str {
        "gemma-local"
    }

    fn label(&self) -> String {
        format!("Gemma local HTTP ({})", self.model_id)
    }

    fn requires_network(&self) -> bool {
        // The server runs on 127.0.0.1, never reaches a public
        // network. Audit-friendly answer is the same as the old
        // in-process engine.
        false
    }
}

impl GemmaHttpEngine {
    async fn post_cleanup(
        &self,
        server: &inference_proc::RunningServer,
        system_prompt: &str,
        dictionary: &[String],
        transcript: &str,
    ) -> anyhow::Result<String> {
        let url = format!("http://127.0.0.1:{}/v1/chat/completions", server.port);
        let max_tokens = dynamic_max_tokens(transcript.len());
        log::debug!(
            "gemma-http: dynamic max_tokens={max_tokens} for {} input chars",
            transcript.len()
        );
        let body = ChatRequest {
            // The server doesn't actually validate this against the
            // loaded GGUF — it just echoes it in responses — but
            // we set it for log clarity.
            model: &server.model_id,
            messages: vec![
                ChatMessage {
                    role: "system",
                    content: build_system_message(system_prompt, dictionary),
                },
                ChatMessage {
                    role: "user",
                    content: format!("Clean this transcript:\n{transcript}"),
                },
            ],
            max_tokens,
            // Low temperature: the cleanup task is closer to
            // editing than generation. We want determinism so the
            // user can re-dictate the same sentence and get the
            // same polish.
            temperature: 0.2,
            stream: false,
        };

        let resp = self
            .client
            .post(&url)
            .bearer_auth(&server.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("llama-server POST: {e}"))?;

        let status = resp.status();
        if !status.is_success() {
            let snippet = resp
                .text()
                .await
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect::<String>();
            return Err(anyhow::anyhow!(
                "llama-server HTTP {status}: {snippet}"
            ));
        }

        let parsed: ChatResponse = resp
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("parse chat response: {e}"))?;
        let content = parsed
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .ok_or_else(|| anyhow::anyhow!("empty choices in chat response"))?;
        Ok(content.trim().to_string())
    }
}

fn build_system_message(system_override: &str, dictionary: &[String]) -> String {
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
    if cleaned.is_empty() {
        return system;
    }
    format!(
        "{system}\n\nPreserve these proper nouns and acronyms exactly as written: {}.",
        cleaned.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_system_message_appends_dictionary() {
        let s = build_system_message("base prompt", &["Anthropic".into(), "Vox".into()]);
        assert!(s.contains("base prompt"));
        assert!(s.contains("Anthropic"));
        assert!(s.contains("Vox"));
    }

    #[test]
    fn build_system_message_omits_empty_dictionary() {
        let s = build_system_message("base prompt", &[]);
        assert_eq!(s, "base prompt");
    }

    #[test]
    fn build_system_message_uses_default_for_empty_override() {
        let s = build_system_message("   ", &[]);
        assert_eq!(s, DEFAULT_SYSTEM_PROMPT);
    }

    #[test]
    fn dynamic_max_tokens_floors_short_inputs() {
        // A 5-char input would be ~1 token of estimated input,
        // which the formula would size to 129 tokens. Floor to
        // MIN_NEW_TOKENS so even a one-word polish has room for
        // the eos sentinel.
        assert_eq!(dynamic_max_tokens(5), MIN_NEW_TOKENS as i32);
        assert_eq!(dynamic_max_tokens(0), MIN_NEW_TOKENS as i32);
    }

    #[test]
    fn dynamic_max_tokens_scales_for_30min_audio() {
        // 30 min × 150 wpm × ~5 chars/word ≈ 22.5k chars input.
        // The cap should be in the multi-thousand range so the
        // model has room to emit the polished version.
        let cap = dynamic_max_tokens(22_500);
        assert!(
            cap >= 6_000,
            "30-min polish needs at least 6k output tokens, got {cap}"
        );
        assert!(
            cap <= MAX_NEW_TOKENS_CAP as i32,
            "must respect the runaway-generation ceiling"
        );
    }

    #[test]
    fn dynamic_max_tokens_caps_pathological_inputs() {
        // A 200k-char paste shouldn't unlock unbounded generation.
        assert_eq!(dynamic_max_tokens(200_000), MAX_NEW_TOKENS_CAP as i32);
    }
}
