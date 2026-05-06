//! Out-of-process LLM polish over loopback HTTP (OpenAI chat-completions
//! shape). A C++ assert in llama.cpp now exits the child rather than the
//! menu-bar app — `inference_proc` respawns or `session.rs` fail-opens
//! to the raw transcript. Engine ID stays `"gemma-local"` so settings
//! survive the move.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Wry};

use crate::engines::{ChunkCallback, LlmEngine};
use crate::inference_proc;
use futures_util::StreamExt;

// Sized for a 30-min dictation: ~135 s generation + ~30 s prompt eval +
// margin at the measured ~52 tok/s on Apple Silicon.
const HARD_TIMEOUT: Duration = Duration::from_secs(240);

// Gemma 4 is a thinking model: llama.cpp's `peg-gemma4` chat handler
// routes chain-of-thought into `reasoning_content` before any answer
// tokens land in `content`. Reasoning grows with input *complexity*
// (numbered lists, conditionals, multi-step instructions) and not
// just length — empirically a ~400-token input on a structuring task
// burns 1 200+ tokens of thinking. The previous 1024 floor truncated
// the completion mid-thought, leaving `content` empty and tripping
// the empty-content fail-open in `session.rs`. 2048 covers reasoning
// + a short reply for typical short utterances.
const MIN_NEW_TOKENS: u32 = 2048;

// ~160 s of generation at ~52 tok/s; past that we'd rather fail-open.
const MAX_NEW_TOKENS_CAP: u32 = 8192;

// Hard upper bound on the SSE accumulation buffer. A well-behaved
// server emits one complete event per ~few hundred bytes, capped by
// MAX_NEW_TOKENS_CAP × ~4 chars/token ≈ 32 KiB across the whole
// stream. 1 MiB leaves four orders of magnitude of headroom and still
// catches a runaway/malicious server that ships a single endless
// "data:" line with no `\n\n` event boundary.
const SSE_BUFFER_MAX_BYTES: usize = 1_048_576;

// English BPE ≈ 4 chars/token. Budget = reasoning + output + margin:
// - Reasoning: ~3× input. Gemma 4's chain-of-thought consistently
//   runs 2-4× the input length on cleanup-with-structuring tasks;
//   3× is the empirical median.
// - Output: 1.2× input. Cleaned text is roughly input length plus
//   structural markup (numbered lists, paragraph breaks).
// - Margin: 256 tokens for chat-template overhead and rounding.
//
// When `enable_thinking == false` the reasoning multiplier is wasted
// — Gemma 4 emits answer tokens directly into `content`. A tighter
// budget shortens the per-call timeout (computed downstream from
// `max_tokens`) so a wedged call fails-open in seconds, not minutes.
fn dynamic_max_tokens(input_chars: usize, enable_thinking: bool) -> i32 {
    let estimated_input = input_chars / 4;
    if enable_thinking {
        let reasoning_budget = estimated_input * 3;
        let output_budget = (estimated_input * 12) / 10;
        let dynamic = (reasoning_budget + output_budget + 256) as u32;
        dynamic.clamp(MIN_NEW_TOKENS, MAX_NEW_TOKENS_CAP) as i32
    } else {
        let budget = ((estimated_input as f64) * 1.5).round() as u32;
        budget.clamp(256, MAX_NEW_TOKENS_CAP) as i32
    }
}

// Per-call deadline: pessimistic 30 tok/s (acceptable buffer below the
// measured ~52 tok/s on Apple Silicon) + 30 s of prompt-eval slack,
// floored at 30 s and capped at HARD_TIMEOUT so a pathological
// max_tokens still can't wedge a session past 4 min.
fn timeout_for(max_tokens: i32) -> Duration {
    let secs = ((max_tokens as f64 / 30.0) + 30.0)
        .max(30.0)
        .min(HARD_TIMEOUT.as_secs() as f64) as u64;
    Duration::from_secs(secs)
}

pub const DEFAULT_SYSTEM_PROMPT: &str = "You clean up and structure voice-dictation transcripts. Output ONLY the cleaned text. Do not output reasoning, planning, commentary, prefaces, or quotes.

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
            // Loopback only — a healthy llama-server accepts inside a
            // few ms. A 5 s connect timeout means a hung TCP handshake
            // (e.g. supervisor died mid-spawn but the kernel still has
            // a half-open listener) fails fast instead of holding the
            // user-visible 240 s request timeout. The request-level
            // timeout above stays HARD_TIMEOUT so a real generation
            // gets the full budget.
            .connect_timeout(Duration::from_secs(5))
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
    cache_prompt: bool,
    /// Per-request OpenAI-compatible field that llama.cpp (b8953+)
    /// forwards into Jinja chat-template rendering. Gemma 4's template
    /// reads `enable_thinking` from here; omitted entirely when the
    /// caller didn't ask for it so the server's default chat-template
    /// behaviour is preserved.
    #[serde(skip_serializing_if = "Option::is_none")]
    chat_template_kwargs: Option<serde_json::Value>,
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

// `server_dead` flags whether a respawn could plausibly fix the
// failure. `true` only for outcomes that imply the child process is
// gone (TCP connect refused). Timeouts, HTTP-status errors, and
// parse errors all leave `server_dead = false` so the caller skips
// the cold-start retry that would just compound the user-visible
// latency.
struct AttemptError {
    err: anyhow::Error,
    server_dead: bool,
}

// Streaming variants — llama.cpp's OpenAI-compatible SSE shape.
#[derive(Deserialize)]
struct StreamChunk {
    #[serde(default)]
    choices: Vec<StreamChoice>,
}

#[derive(Deserialize)]
struct StreamChoice {
    #[serde(default)]
    delta: StreamDelta,
}

#[derive(Deserialize, Default)]
struct StreamDelta {
    #[serde(default)]
    content: Option<String>,
}

#[async_trait::async_trait]
impl LlmEngine for GemmaHttpEngine {
    async fn cleanup(
        &self,
        transcript: &str,
        system_prompt: &str,
        dictionary: &[String],
        enable_thinking: bool,
    ) -> anyhow::Result<String> {
        let trimmed = transcript.trim();
        if trimmed.split_whitespace().count() < 2 {
            return Ok(trimmed.to_string());
        }

        let started = Instant::now();
        log::info!(
            "gemma-http: polishing {} chars with model={} (thinking={})",
            trimmed.len(),
            self.model_id,
            enable_thinking,
        );

        let request_once = || async {
            let server = inference_proc::ensure_running(&self.app, &self.model_id)
                .await
                .map_err(|e| AttemptError { err: e, server_dead: false })?;
            self.post_cleanup(&server, system_prompt, dictionary, trimmed, enable_thinking).await
        };

        // Retry only when the child is actually gone: a TCP connect
        // refused on the first attempt means the supervisor's cached
        // handle is stale (post-assert / OOM-kill), and a fresh spawn
        // is the fix. Timeouts, HTTP-status, and parse errors mean the
        // server is alive — retrying just pays the cold-start cost
        // again before failing the same way.
        let result = match request_once().await {
            Ok(s) => Ok(s),
            Err(first) if first.server_dead => {
                log::warn!(
                    "gemma-http: first request failed ({}) and server appears dead; respawning and retrying once",
                    first.err
                );
                inference_proc::drop_managed(&self.app);
                request_once().await.map_err(|second| {
                    anyhow::anyhow!("gemma-http retry failed: {} | {}", first.err, second.err)
                })
            }
            Err(first) => Err(first.err),
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

    async fn cleanup_stream(
        &self,
        transcript: &str,
        system_prompt: &str,
        dictionary: &[String],
        enable_thinking: bool,
        on_chunk: ChunkCallback,
    ) -> anyhow::Result<String> {
        let trimmed = transcript.trim();
        if trimmed.split_whitespace().count() < 2 {
            return Ok(trimmed.to_string());
        }

        let started = Instant::now();
        log::info!(
            "gemma-http: streaming-polish {} chars with model={} (thinking={})",
            trimmed.len(),
            self.model_id,
            enable_thinking,
        );

        let request_once = || async {
            let server = inference_proc::ensure_running(&self.app, &self.model_id)
                .await
                .map_err(|e| AttemptError { err: e, server_dead: false })?;
            self.post_cleanup_stream(
                &server,
                system_prompt,
                dictionary,
                trimmed,
                enable_thinking,
                &on_chunk,
            )
            .await
        };

        // Mirror cleanup()'s respawn-only-on-actual-death policy:
        // retrying after a timeout pays full cold-start before failing
        // the same way, multiplying the user-visible wait.
        let result = match request_once().await {
            Ok(s) => Ok(s),
            Err(first) if first.server_dead => {
                log::warn!(
                    "gemma-http: first stream request failed ({}) and server appears dead; respawning and retrying once",
                    first.err
                );
                inference_proc::drop_managed(&self.app);
                request_once().await.map_err(|second| {
                    anyhow::anyhow!("gemma-http stream retry failed: {} | {}", first.err, second.err)
                })
            }
            Err(first) => Err(first.err),
        };

        match &result {
            Ok(out) => log::info!(
                "gemma-http: streamed polish {}→{} chars in {} ms",
                trimmed.len(),
                out.len(),
                started.elapsed().as_millis()
            ),
            Err(e) => log::warn!(
                "gemma-http: stream polish FAILED in {} ms: {e}",
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
        enable_thinking: bool,
    ) -> Result<String, AttemptError> {
        let url = format!("http://127.0.0.1:{}/v1/chat/completions", server.port);
        let max_tokens = dynamic_max_tokens(transcript.len(), enable_thinking);
        let deadline = timeout_for(max_tokens);

        let system_msg = build_system_message(system_prompt, dictionary);
        // Fenced block keeps content like "Hey Claude, …" from being
        // parsed as an instruction directed at the model.
        let user_msg = format!(
            "Clean this transcript. Output only the cleaned text.\n\n```\n{transcript}\n```"
        );

        log::debug!(
            "gemma-http: request → model={}, max_tokens={}, temp=0.2, thinking={}, system_chars={}, user_chars={}, deadline={}s",
            server.model_id,
            max_tokens,
            enable_thinking,
            system_msg.len(),
            user_msg.len(),
            deadline.as_secs(),
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
            cache_prompt: true,
            chat_template_kwargs: Some(serde_json::json!({
                "enable_thinking": enable_thinking,
            })),
        };

        let send_fut = self
            .client
            .post(&url)
            .bearer_auth(&server.api_key)
            .json(&body)
            .send();

        let resp = match tokio::time::timeout(deadline, send_fut).await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                let dead = e.is_connect();
                return Err(AttemptError {
                    err: anyhow::anyhow!("llama-server POST: {e}"),
                    server_dead: dead,
                });
            }
            Err(_) => {
                return Err(AttemptError {
                    err: anyhow::anyhow!(
                        "llama-server POST timed out after {}s",
                        deadline.as_secs()
                    ),
                    server_dead: false,
                });
            }
        };

        let status = resp.status();
        let body_text = match tokio::time::timeout(deadline, resp.text()).await {
            Ok(Ok(t)) => t,
            Ok(Err(e)) => {
                return Err(AttemptError {
                    err: anyhow::anyhow!("read response body: {e}"),
                    server_dead: false,
                });
            }
            Err(_) => {
                return Err(AttemptError {
                    err: anyhow::anyhow!(
                        "read response body timed out after {}s",
                        deadline.as_secs()
                    ),
                    server_dead: false,
                });
            }
        };

        if !status.is_success() {
            let snippet: String = body_text.chars().take(200).collect();
            return Err(AttemptError {
                err: anyhow::anyhow!("llama-server HTTP {status}: {snippet}"),
                server_dead: false,
            });
        }

        let parsed: ChatResponse = serde_json::from_str(&body_text).map_err(|e| {
            let snippet: String = body_text.chars().take(500).collect();
            AttemptError {
                err: anyhow::anyhow!("parse chat response: {e}; body={snippet}"),
                server_dead: false,
            }
        })?;

        let choice = parsed.choices.into_iter().next().ok_or_else(|| {
            let snippet: String = body_text.chars().take(500).collect();
            AttemptError {
                err: anyhow::anyhow!("empty choices in chat response; body={snippet}"),
                server_dead: false,
            }
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

impl GemmaHttpEngine {
    async fn post_cleanup_stream(
        &self,
        server: &inference_proc::RunningServer,
        system_prompt: &str,
        dictionary: &[String],
        transcript: &str,
        enable_thinking: bool,
        on_chunk: &ChunkCallback,
    ) -> Result<String, AttemptError> {
        let started = Instant::now();
        let url = format!("http://127.0.0.1:{}/v1/chat/completions", server.port);
        let max_tokens = dynamic_max_tokens(transcript.len(), enable_thinking);
        let deadline = timeout_for(max_tokens);

        let system_msg = build_system_message(system_prompt, dictionary);
        let user_msg = format!(
            "Clean this transcript. Output only the cleaned text.\n\n```\n{transcript}\n```"
        );

        let body = ChatRequest {
            model: &server.model_id,
            messages: vec![
                ChatMessage { role: "system", content: system_msg },
                ChatMessage { role: "user", content: user_msg },
            ],
            max_tokens,
            temperature: 0.2,
            stream: true,
            cache_prompt: true,
            chat_template_kwargs: Some(serde_json::json!({
                "enable_thinking": enable_thinking,
            })),
        };

        let send_fut = self
            .client
            .post(&url)
            .bearer_auth(&server.api_key)
            .json(&body)
            .send();

        let resp = match tokio::time::timeout(deadline, send_fut).await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                let dead = e.is_connect();
                return Err(AttemptError {
                    err: anyhow::anyhow!("llama-server stream POST: {e}"),
                    server_dead: dead,
                });
            }
            Err(_) => {
                return Err(AttemptError {
                    err: anyhow::anyhow!(
                        "llama-server stream POST timed out after {}s",
                        deadline.as_secs()
                    ),
                    server_dead: false,
                });
            }
        };

        let status = resp.status();
        if !status.is_success() {
            let snippet: String = resp
                .text()
                .await
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect();
            return Err(AttemptError {
                err: anyhow::anyhow!("llama-server HTTP {status}: {snippet}"),
                server_dead: false,
            });
        }

        let stream_fut = async {
            let mut acc = String::new();
            let mut buf: Vec<u8> = Vec::new();
            let mut stream = resp.bytes_stream();
            // One-shot TTFT log so a slow prompt-eval stage shows up in
            // the timeline alongside the existing total-time log. Set the
            // first time content lands in `acc`.
            let mut first_chunk_at: Option<Instant> = None;

            while let Some(next) = stream.next().await {
                let bytes = next.map_err(|e| AttemptError {
                    err: anyhow::anyhow!("stream read: {e}"),
                    server_dead: false,
                })?;
                buf.extend_from_slice(&bytes);
                if buf.len() > SSE_BUFFER_MAX_BYTES {
                    return Err(AttemptError {
                        err: anyhow::anyhow!(
                            "SSE buffer exceeded {SSE_BUFFER_MAX_BYTES} bytes without an event boundary"
                        ),
                        server_dead: false,
                    });
                }

                // SSE events end at a blank line — \n\n in the byte stream.
                // Drain complete events from the buffer one at a time so a
                // partial trailing event stays buffered for the next chunk.
                loop {
                    let Some(pos) = find_double_newline(&buf) else {
                        break;
                    };
                    let event = buf.drain(..pos + 2).collect::<Vec<u8>>();
                    let event_len = event.len().saturating_sub(2);
                    let event_str = match std::str::from_utf8(&event[..event_len]) {
                        Ok(s) => s,
                        Err(_) => continue,
                    };
                    if let Some(stop) = parse_sse_event(event_str, &mut acc) {
                        if stop {
                            // [DONE] sentinel — final chunk delivered, exit
                            // both the inner and outer loops cleanly.
                            on_chunk(&acc);
                            return Ok(acc.trim().to_string());
                        }
                        on_chunk(&acc);
                        if first_chunk_at.is_none() && !acc.is_empty() {
                            first_chunk_at = Some(Instant::now());
                            log::debug!(
                                "gemma-http: first polish chunk at {} ms (acc={} chars)",
                                started.elapsed().as_millis(),
                                acc.len(),
                            );
                        }
                    }
                }
            }

            // Stream ended without a [DONE] marker — flush whatever we
            // accumulated and treat as best-effort success.
            on_chunk(&acc);
            Ok(acc.trim().to_string())
        };

        match tokio::time::timeout(deadline, stream_fut).await {
            Ok(r) => r,
            Err(_) => Err(AttemptError {
                err: anyhow::anyhow!(
                    "llama-server stream timed out after {}s",
                    deadline.as_secs()
                ),
                server_dead: false,
            }),
        }
    }
}

/// Find the byte index of the first `\n\n` sequence in `buf`, if any.
/// SSE-friendly: handles `\r\n\r\n` by collapsing CR before this is
/// called (we just trim CRs in the parser instead).
fn find_double_newline(buf: &[u8]) -> Option<usize> {
    if buf.len() < 2 {
        return None;
    }
    for i in 0..buf.len() - 1 {
        if buf[i] == b'\n' && buf[i + 1] == b'\n' {
            return Some(i);
        }
    }
    None
}

/// Parse one SSE event block. Appends any decoded `delta.content` to
/// `acc` (cumulative), returns:
/// - `Some(true)` on a `data: [DONE]` sentinel (caller stops the stream)
/// - `Some(false)` if `acc` was extended (caller should fire on_chunk)
/// - `None` if the event was a comment / heartbeat / non-data line
fn parse_sse_event(event: &str, acc: &mut String) -> Option<bool> {
    let mut extended = false;
    for raw_line in event.split('\n') {
        let line = raw_line.trim_end_matches('\r');
        // SSE comments start with ":" — heartbeat from llama.cpp.
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        // Only "data:" lines are interesting for the chat-completions stream.
        let Some(payload) = line.strip_prefix("data:") else {
            continue;
        };
        let payload = payload.trim_start();
        if payload == "[DONE]" {
            return Some(true);
        }
        // llama.cpp occasionally emits `{}` heartbeats and chunks
        // without `delta.content` (role-only first chunk). Tolerate both.
        let chunk: StreamChunk = match serde_json::from_str(payload) {
            Ok(c) => c,
            Err(e) => {
                log::warn!("gemma-http: bad SSE chunk JSON ({e}); raw={payload}");
                continue;
            }
        };
        for choice in chunk.choices {
            if let Some(piece) = choice.delta.content {
                if !piece.is_empty() {
                    acc.push_str(&piece);
                    extended = true;
                }
            }
        }
    }
    if extended {
        Some(false)
    } else {
        None
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
        assert_eq!(dynamic_max_tokens(5, true), MIN_NEW_TOKENS as i32);
        assert_eq!(dynamic_max_tokens(0, true), MIN_NEW_TOKENS as i32);
    }

    #[test]
    fn dynamic_max_tokens_covers_thinking_for_medium_inputs() {
        // Regression guard for the silent fail-open we hit in
        // production: a 1600-char (~400-token) transcript consumed
        // ~1200 tokens on Gemma 4's reasoning_content before the
        // answer began, while max_tokens was clamped at 1024. The
        // model emitted reasoning exclusively, content came back
        // empty, and the session fell through to copying the raw
        // Whisper transcript untouched. The budget must now leave
        // room for thinking + a polished reply.
        let cap = dynamic_max_tokens(1_600, true);
        assert!(
            cap >= 1_800,
            "1600-char input must budget for thinking + answer, got {cap}"
        );
    }

    #[test]
    fn dynamic_max_tokens_scales_for_30min_audio() {
        // 30 min × 150 wpm × ~5 chars/word ≈ 22.5k chars input.
        let cap = dynamic_max_tokens(22_500, true);
        assert!(cap >= 6_000, "30-min polish needs at least 6k output tokens, got {cap}");
        assert!(cap <= MAX_NEW_TOKENS_CAP as i32);
    }

    #[test]
    fn dynamic_max_tokens_caps_pathological_inputs() {
        assert_eq!(dynamic_max_tokens(200_000, true), MAX_NEW_TOKENS_CAP as i32);
    }

    #[test]
    fn sse_parser_accumulates_cumulative_content() {
        // Walk hand-rolled SSE bytes through the parsing helpers exactly
        // the way `post_cleanup_stream` does, asserting that each event
        // adds to a single cumulative buffer (the contract the pill UI
        // relies on).
        let mut buf: Vec<u8> = Vec::new();
        let raw =
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n\
             data: {\"choices\":[{\"delta\":{\"content\":\" world\"}}]}\n\n\
             data: [DONE]\n\n";
        buf.extend_from_slice(raw.as_bytes());

        let mut acc = String::new();
        let mut emissions: Vec<String> = Vec::new();
        let mut saw_done = false;

        loop {
            let Some(pos) = find_double_newline(&buf) else { break };
            let event = buf.drain(..pos + 2).collect::<Vec<u8>>();
            let event_len = event.len().saturating_sub(2);
            let event_str = std::str::from_utf8(&event[..event_len]).unwrap();
            match parse_sse_event(event_str, &mut acc) {
                Some(true) => {
                    saw_done = true;
                    emissions.push(acc.clone());
                    break;
                }
                Some(false) => emissions.push(acc.clone()),
                None => {}
            }
        }

        assert!(saw_done, "[DONE] sentinel must terminate the stream");
        assert_eq!(emissions, vec!["Hello".to_string(), "Hello world".to_string(), "Hello world".to_string()]);
        assert_eq!(acc, "Hello world");
    }

    #[test]
    fn sse_parser_handles_split_chunks() {
        // Real wire shape: a single SSE event arrives split across two
        // network reads. The parser's outer drain loop must keep the
        // partial trailing event buffered until the second read
        // completes the `\n\n` boundary, then emit the cumulative
        // content exactly once.
        let mut buf: Vec<u8> = Vec::new();
        let mut acc = String::new();
        let mut emissions: Vec<String> = Vec::new();

        // First half: the data line is complete but the terminating
        // `\n\n` is split.
        buf.extend_from_slice(
            b"data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n",
        );
        // No double-newline yet → no emission this round.
        if let Some(pos) = find_double_newline(&buf) {
            let event = buf.drain(..pos + 2).collect::<Vec<u8>>();
            let event_len = event.len().saturating_sub(2);
            let event_str = std::str::from_utf8(&event[..event_len]).unwrap();
            if let Some(_stop) = parse_sse_event(event_str, &mut acc) {
                emissions.push(acc.clone());
            }
        }
        assert!(emissions.is_empty(), "no emit before \\n\\n boundary");
        assert_eq!(acc, "");

        // Second half: completes the first event AND introduces a
        // second event. The parser must drain BOTH events from the
        // buffer in a single outer-pass.
        buf.extend_from_slice(
            b"\ndata: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\n",
        );
        loop {
            let Some(pos) = find_double_newline(&buf) else { break };
            let event = buf.drain(..pos + 2).collect::<Vec<u8>>();
            let event_len = event.len().saturating_sub(2);
            let event_str = std::str::from_utf8(&event[..event_len]).unwrap();
            match parse_sse_event(event_str, &mut acc) {
                Some(true) => {
                    emissions.push(acc.clone());
                    break;
                }
                Some(false) => emissions.push(acc.clone()),
                None => {}
            }
        }

        assert_eq!(emissions, vec!["Hel".to_string(), "Hello".to_string()]);
        assert_eq!(acc, "Hello");
    }

    #[test]
    fn sse_buffer_max_bytes_constant_is_sane() {
        // Cheap consistency check: the cap must be positive and big
        // enough to hold any plausible single event from a
        // well-behaved server (a complete chat-completion delta is
        // hundreds of bytes at worst).
        assert!(SSE_BUFFER_MAX_BYTES >= 64 * 1024);
        assert!(SSE_BUFFER_MAX_BYTES <= 16 * 1024 * 1024);
    }

    #[test]
    fn sse_parser_caps_buffer_at_max_bytes() {
        // Simulates a pathological/malicious server emitting bytes
        // without `\n\n`: the buffer grows unbounded until it crosses
        // SSE_BUFFER_MAX_BYTES, at which point post_cleanup_stream
        // returns Err. Test mirrors the production check predicate
        // directly (the production code is `if buf.len() >
        // SSE_BUFFER_MAX_BYTES { return Err(...) }`).
        let mut buf: Vec<u8> = Vec::new();
        let chunk = vec![b'X'; 64 * 1024];
        let mut tripped = false;
        for _ in 0..(SSE_BUFFER_MAX_BYTES / chunk.len() + 2) {
            buf.extend_from_slice(&chunk);
            if buf.len() > SSE_BUFFER_MAX_BYTES {
                tripped = true;
                break;
            }
            // No `\n\n` ever appended → find_double_newline is None
            // and no events drain.
            assert!(find_double_newline(&buf).is_none());
        }
        assert!(
            tripped,
            "buffer must cross SSE_BUFFER_MAX_BYTES when no boundary arrives"
        );
        // The buffer at the moment of trip is just past the cap.
        assert!(buf.len() > SSE_BUFFER_MAX_BYTES);
    }

    #[test]
    fn sse_parser_tolerates_heartbeat_and_role_chunks() {
        // llama.cpp emits a leading `delta: {"role": "assistant"}` chunk
        // and may inject `:` heartbeat comments between events; neither
        // should advance the cumulative buffer or crash the parser.
        let mut acc = String::new();
        // Role-only first chunk: no content field.
        assert!(parse_sse_event(
            "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}",
            &mut acc,
        ).is_none());
        assert_eq!(acc, "");
        // Heartbeat / comment lines.
        assert!(parse_sse_event(": ping", &mut acc).is_none());
        assert!(parse_sse_event("", &mut acc).is_none());
        // Real chunk.
        assert_eq!(
            parse_sse_event(
                "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}",
                &mut acc,
            ),
            Some(false)
        );
        assert_eq!(acc, "hi");
        // Done sentinel.
        assert_eq!(parse_sse_event("data: [DONE]", &mut acc), Some(true));
    }
}
