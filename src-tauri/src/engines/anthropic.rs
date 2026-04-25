//! Cloud LLM cleanup — Claude Haiku 4.5. Opt-in fallback when the user
//! prefers cloud cleanup. The wire format is unchanged from the original
//! `llm.rs`; this is the trait-shaped wrapper.

use std::time::Duration;

use anyhow::{anyhow, Context};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Wry};

use crate::engines::LlmEngine;
use crate::settings;

const ANTHROPIC_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const MODEL: &str = "claude-haiku-4-5";
const MAX_TOKENS: u32 = 1024;
const TEMPERATURE: f32 = 0.2;
pub const TIMEOUT: Duration = Duration::from_secs(3);

const BASE_SYSTEM_PROMPT: &str = "You clean up voice-dictation transcripts. \
Return plain text only — no preface, no quotes, no explanations, no trailing notes.

Rules:
- Remove filler words (um, uh, like as filler, you know, sort of, kind of as filler).
- Apply punctuation and capitalization, including commas, periods, and question marks.
- Fix obvious mid-sentence self-corrections (\"go to the — I mean, the store\" → \"go to the store\").
- Preserve the speaker's register and word choice. Do not formalize casual speech. Do not reword intact sentences.
- Never add facts, names, links, or content the user did not say.
- Do not refuse, editorialize, or append closing remarks. If the input is gibberish or empty, return it unchanged.
- Output only the cleaned text.";

static HTTP: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .timeout(TIMEOUT)
        .https_only(true)
        .build()
        .expect("build reqwest client")
});

#[derive(Debug, thiserror::Error)]
pub enum AnthropicError {
    #[error("API key missing")]
    MissingKey,
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("Anthropic API error ({status}): {body}")]
    Api { status: u16, body: String },
    #[error("failed to parse response: {0}")]
    Parse(String),
    #[error("cleanup timed out")]
    Timeout,
    #[error("transcript too short")]
    TooShort,
}

#[derive(Deserialize)]
struct MessagesResponse {
    content: Vec<ContentBlock>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ContentBlock {
    Text { text: String },
    #[serde(other)]
    Other,
}

#[derive(Serialize)]
struct Message<'a> {
    role: &'a str,
    content: &'a str,
}

pub struct AnthropicEngine {
    app: AppHandle<Wry>,
}

impl AnthropicEngine {
    pub fn new(app: AppHandle<Wry>) -> Self {
        Self { app }
    }
}

#[async_trait::async_trait]
impl LlmEngine for AnthropicEngine {
    async fn cleanup(
        &self,
        transcript: &str,
        system_prompt: &str,
        dictionary: &[String],
    ) -> anyhow::Result<String> {
        let api_key = settings::get_anthropic_key(&self.app);
        if api_key.trim().is_empty() {
            return Err(anyhow!(AnthropicError::MissingKey));
        }
        let trimmed = transcript.trim();
        if trimmed.split_whitespace().count() < 2 {
            return Err(anyhow!(AnthropicError::TooShort));
        }

        let prompt = build_prompt(system_prompt, dictionary);

        let body = json!({
            "model": MODEL,
            "max_tokens": MAX_TOKENS,
            "temperature": TEMPERATURE,
            "system": prompt,
            "messages": [
                Message { role: "user", content: trimmed },
            ],
        });

        let resp = HTTP
            .post(ANTHROPIC_URL)
            .header("x-api-key", api_key.trim())
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    anyhow!(AnthropicError::Timeout)
                } else {
                    anyhow!(AnthropicError::Network(e))
                }
            })?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!(AnthropicError::Api {
                status: status.as_u16(),
                body,
            }));
        }

        let parsed: MessagesResponse =
            resp.json().await.context("Anthropic response parse")?;

        for block in parsed.content {
            if let ContentBlock::Text { text } = block {
                let out = text.trim().to_string();
                if out.is_empty() {
                    return Err(anyhow!(AnthropicError::Parse("empty content block".into())));
                }
                return Ok(out);
            }
        }
        Err(anyhow!(AnthropicError::Parse(
            "no text content in response".into()
        )))
    }

    fn id(&self) -> &'static str {
        "anthropic"
    }

    fn label(&self) -> String {
        "Claude Haiku 4.5 (cloud)".into()
    }

    fn requires_network(&self) -> bool {
        true
    }
}

/// Cheap key validation — POST a 1-token request.
pub async fn test_key(api_key: &str) -> Result<(), AnthropicError> {
    if api_key.trim().is_empty() {
        return Err(AnthropicError::MissingKey);
    }

    let body = json!({
        "model": MODEL,
        "max_tokens": 1,
        "messages": [ { "role": "user", "content": "." } ],
    });

    let resp = HTTP
        .post(ANTHROPIC_URL)
        .timeout(Duration::from_secs(10))
        .header("x-api-key", api_key.trim())
        .header("anthropic-version", ANTHROPIC_VERSION)
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(AnthropicError::Api {
            status: status.as_u16(),
            body,
        });
    }
    Ok(())
}

fn build_prompt(system_override: &str, dictionary: &[String]) -> String {
    let base = if system_override.trim().is_empty() {
        BASE_SYSTEM_PROMPT.to_string()
    } else {
        system_override.to_string()
    };
    let cleaned: Vec<&str> = dictionary
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if cleaned.is_empty() {
        base
    } else {
        format!(
            "{}\n\nKnown terms (preserve these exact spellings when phonetically close): {}.",
            base,
            cleaned.join(", ")
        )
    }
}
