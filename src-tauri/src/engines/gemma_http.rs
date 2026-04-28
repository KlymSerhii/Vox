//! Out-of-process LLM polish over loopback HTTP (OpenAI chat-completions
//! shape). A C++ assert in llama.cpp now exits the child rather than the
//! menu-bar app — `inference_proc` respawns or `session.rs` fail-opens
//! to the raw transcript. Engine ID stays `"gemma-local"` so settings
//! survive the move.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Wry};

use crate::engines::LlmEngine;
use crate::inference_proc;

// Sized for a 30-min dictation: ~135 s generation + ~30 s prompt eval +
// margin at the measured ~52 tok/s on Apple Silicon.
const HARD_TIMEOUT: Duration = Duration::from_secs(240);

// Gemma 4 is a thinking model: llama.cpp's `peg-gemma4` chat handler
// routes ~250-300 tokens of chain-of-thought into `reasoning_content`
// before any answer tokens land in `content`. The floor must therefore
// cover *thinking + answer*, not just answer. 256 was tight enough that
// short inputs (e.g. 100 chars) had thinking eat the entire budget,
// leaving content empty and finish_reason=length. 1024 leaves roughly
// 3× headroom for the model's internal reasoning plus a short reply.
const MIN_NEW_TOKENS: u32 = 1024;

// ~160 s of generation at ~52 tok/s; past that we'd rather fail-open.
const MAX_NEW_TOKENS_CAP: u32 = 8192;

// English BPE ≈ 4 chars/token; 1.2× pads for structural markup
// (numbered lists, paragraph breaks) the prompt encourages.
fn dynamic_max_tokens(input_chars: usize) -> i32 {
    let estimated_input = input_chars / 4;
    let dynamic = ((estimated_input * 12) / 10 + 128) as u32;
    dynamic.clamp(MIN_NEW_TOKENS, MAX_NEW_TOKENS_CAP) as i32
}

const DEFAULT_SYSTEM_PROMPT: &str = "You clean up and structure voice-dictation transcripts. Output ONLY the cleaned text. Do not output reasoning, planning, commentary, prefaces, or quotes.

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
5. When the speaker enumerates two or more items, actions, or steps, render them as a numbered list. Triggers include:
   - Ordinal markers: \"first… second… third…\", \"one is… two is…\".
   - Step markers: \"step one… step two…\".
   - Enumeration phrases: \"the three things are…\", \"a couple of points:\".
   - Conversational connectors that introduce a sequence: \"first of all… and then…\", \"to start… after that…\", \"I need you to X and then Y\", \"we should do A and also B\".
   Preserve any prose lead-in before the list (\"I need you to:\\n1. …\\n2. …\"). Single statements and flowing narrative stay as prose.
6. Preserve the speaker's register. Do not formalize casual speech or vice versa.
7. Never add content, facts, names, or numbers the speaker did not say.

Examples:
- \"uh so like the meeting is um at three pm\" → \"The meeting is at 3 PM.\"
- \"I'm not using the model, no, disregard that, I am using the model\" → \"I am using the model.\"
- \"first we ship the build then we run tests and finally we deploy\" →
   \"1. Ship the build.\n2. Run tests.\n3. Deploy.\"
- \"I need you to first of all add tests and then also verify the build passes\" →
   \"I need you to:\n1. Add tests.\n2. Verify the build passes.\"
- \"so everything looks fine but now I need you to first of all add tests and then also to verify that the build passes appropriately\" →
   \"Everything looks fine. I need you to:\n1. Add tests.\n2. Verify that the build passes appropriately.\"";

pub struct GemmaHttpEngine {
    app: AppHandle<Wry>,
    model_id: String,
    client: reqwest::Client,
}

impl GemmaHttpEngine {
    pub fn new(app: AppHandle<Wry>, model_id: String) -> Self {
        // Reuse one client across dictations — fresh-socket cost is
        // ~10 ms on macOS and compounds across short utterances.
        let client = reqwest::Client::builder()
            .timeout(HARD_TIMEOUT)
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
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatChoiceMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ChatChoiceMessage {
    content: String,
    /// Some llama.cpp builds (and OpenAI-compatible reasoning models) split
    /// chain-of-thought into a separate field. If `content` is empty but
    /// this is non-empty, the model is in thinking mode and the answer is
    /// being discarded.
    #[serde(default)]
    reasoning_content: Option<String>,
}

#[derive(Deserialize, Default)]
struct Usage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
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

        // One retry: a stale child (post-assert / OOM-kill) fails first,
        // the supervisor's is_alive check trips, second attempt spawns fresh.
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
            Ok(out) => {
                log::info!(
                    "gemma-http: polished {}→{} chars in {} ms",
                    trimmed.len(),
                    out.len(),
                    started.elapsed().as_millis()
                );
                if out.trim().is_empty() && trimmed.len() > 20 {
                    log::warn!(
                        "gemma-http: polish returned EMPTY content for {}-char input; session will fail-open to raw transcript",
                        trimmed.len()
                    );
                }
            }
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
        // 127.0.0.1 only — never leaves the Mac.
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

        let system_msg = build_system_message(system_prompt, dictionary);
        // Fenced block keeps content like "Hey Claude, …" from being
        // parsed as an instruction directed at the model.
        let user_msg = format!(
            "Clean this transcript. Output only the cleaned text.\n\n```\n{transcript}\n```"
        );

        log::debug!(
            "gemma-http: request → model={}, max_tokens={}, temp=0.2, system_chars={}, user_chars={}",
            server.model_id,
            max_tokens,
            system_msg.len(),
            user_msg.len(),
        );
        log::debug!("gemma-http: system prompt:\n{system_msg}");
        log::debug!("gemma-http: user message:\n{user_msg}");

        let body = ChatRequest {
            model: &server.model_id,
            messages: vec![
                ChatMessage { role: "system", content: system_msg },
                ChatMessage { role: "user", content: user_msg },
            ],
            max_tokens,
            // Cleanup is editing, not generation — keep it deterministic.
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
        // Read the raw body once so we can log it on parse failure or
        // empty-content cases.
        let body_text = resp
            .text()
            .await
            .map_err(|e| anyhow::anyhow!("read response body: {e}"))?;

        if !status.is_success() {
            let snippet: String = body_text.chars().take(200).collect();
            return Err(anyhow::anyhow!("llama-server HTTP {status}: {snippet}"));
        }

        let parsed: ChatResponse = serde_json::from_str(&body_text).map_err(|e| {
            let snippet: String = body_text.chars().take(500).collect();
            anyhow::anyhow!("parse chat response: {e}; body={snippet}")
        })?;

        let choice = parsed.choices.into_iter().next().ok_or_else(|| {
            let snippet: String = body_text.chars().take(500).collect();
            anyhow::anyhow!("empty choices in chat response; body={snippet}")
        })?;

        let finish_reason = choice.finish_reason.unwrap_or_else(|| "(none)".into());
        let content = choice.message.content;
        let reasoning = choice.message.reasoning_content.unwrap_or_default();
        let usage = parsed.usage.unwrap_or_default();

        log::info!(
            "gemma-http: response — finish_reason={}, prompt_tokens={}, completion_tokens={}, content_chars={}, reasoning_chars={}",
            finish_reason,
            usage.prompt_tokens,
            usage.completion_tokens,
            content.len(),
            reasoning.len(),
        );

        let trimmed_content = content.trim().to_string();
        if trimmed_content.is_empty() {
            // Single most useful diagnostic: what did llama-server
            // actually send back? Dump the whole JSON body unredacted —
            // this only happens on the failure path, and the body is
            // small (just envelope + empty content).
            log::warn!(
                "gemma-http: empty content (input={} chars, finish_reason={}, completion_tokens={}). Raw response body:\n{}",
                transcript.len(),
                finish_reason,
                usage.completion_tokens,
                body_text,
            );
            if !reasoning.trim().is_empty() {
                log::warn!(
                    "gemma-http: reasoning_content was non-empty ({} chars) while content was empty — model is in thinking mode and the answer is being discarded. First 300 chars of reasoning:\n{}",
                    reasoning.len(),
                    reasoning.chars().take(300).collect::<String>(),
                );
            }
        }

        Ok(trimmed_content)
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
        assert_eq!(dynamic_max_tokens(5), MIN_NEW_TOKENS as i32);
        assert_eq!(dynamic_max_tokens(0), MIN_NEW_TOKENS as i32);
    }

    #[test]
    fn dynamic_max_tokens_scales_for_30min_audio() {
        // 30 min × 150 wpm × ~5 chars/word ≈ 22.5k chars input.
        let cap = dynamic_max_tokens(22_500);
        assert!(cap >= 6_000, "30-min polish needs at least 6k output tokens, got {cap}");
        assert!(cap <= MAX_NEW_TOKENS_CAP as i32);
    }

    #[test]
    fn dynamic_max_tokens_caps_pathological_inputs() {
        assert_eq!(dynamic_max_tokens(200_000), MAX_NEW_TOKENS_CAP as i32);
    }
}
