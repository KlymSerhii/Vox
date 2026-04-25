//! Cloud STT — Groq Whisper Large v3 Turbo. Opt-in fallback when the user
//! prefers cloud over local. Same wire-shape as the original Phase 1/2
//! implementation in `stt.rs`; the trait wrapper just routes calls.

use std::time::Duration;

use anyhow::{anyhow, Context};
use once_cell::sync::Lazy;
use reqwest::multipart::{Form, Part};
use tauri::{AppHandle, Wry};

use crate::engines::SttEngine;
use crate::settings;

const GROQ_TRANSCRIPTIONS_URL: &str =
    "https://api.groq.com/openai/v1/audio/transcriptions";
const GROQ_MODELS_URL: &str = "https://api.groq.com/openai/v1/models";
const WHISPER_MODEL: &str = "whisper-large-v3-turbo";

static HTTP: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .https_only(true)
        .build()
        .expect("build reqwest client")
});

#[derive(Debug, thiserror::Error)]
pub enum GroqError {
    #[error("API key missing — open Settings and paste a Groq API key")]
    MissingKey,
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("Groq API error ({status}): {body}")]
    Api { status: u16, body: String },
    #[error("failed to parse response: {0}")]
    Parse(String),
}

#[derive(serde::Deserialize)]
struct TranscriptionResponse {
    text: String,
}

pub struct GroqEngine {
    app: AppHandle<Wry>,
}

impl GroqEngine {
    pub fn new(app: AppHandle<Wry>) -> Self {
        Self { app }
    }
}

#[async_trait::async_trait]
impl SttEngine for GroqEngine {
    async fn transcribe(
        &self,
        wav: &[u8],
        _language: Option<&str>,
        dictionary: &[String],
    ) -> anyhow::Result<String> {
        let api_key = settings::get_groq_key(&self.app);
        if api_key.trim().is_empty() {
            return Err(anyhow!(GroqError::MissingKey));
        }

        let part = Part::bytes(wav.to_vec())
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .map_err(|e| anyhow!(GroqError::Parse(e.to_string())))?;

        let mut form = Form::new()
            .text("model", WHISPER_MODEL)
            .text("response_format", "json")
            .text("temperature", "0")
            .part("file", part);

        if let Some(prompt) = whisper_prompt(dictionary) {
            form = form.text("prompt", prompt);
        }

        let resp = HTTP
            .post(GROQ_TRANSCRIPTIONS_URL)
            .bearer_auth(api_key.trim())
            .multipart(form)
            .send()
            .await
            .context("Groq request")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!(GroqError::Api {
                status: status.as_u16(),
                body,
            }));
        }

        let parsed: TranscriptionResponse = resp
            .json()
            .await
            .map_err(|e| anyhow!(GroqError::Parse(e.to_string())))?;
        Ok(parsed.text.trim().to_string())
    }

    fn id(&self) -> &'static str {
        "groq"
    }

    fn label(&self) -> String {
        "Groq Whisper Large v3 Turbo (cloud)".into()
    }

    fn requires_network(&self) -> bool {
        true
    }
}

fn whisper_prompt(dictionary: &[String]) -> Option<String> {
    let cleaned: Vec<&str> = dictionary
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if cleaned.is_empty() {
        return None;
    }
    let joined = cleaned.join(", ");
    let bounded: String = if joined.chars().count() > 700 {
        joined.chars().take(700).collect()
    } else {
        joined
    };
    Some(format!("Names and terms used: {bounded}."))
}

/// Cheap key validation — kept for the Settings → Test button.
pub async fn test_key(api_key: &str) -> Result<(), GroqError> {
    if api_key.trim().is_empty() {
        return Err(GroqError::MissingKey);
    }

    let resp = HTTP
        .get(GROQ_MODELS_URL)
        .timeout(Duration::from_secs(10))
        .bearer_auth(api_key.trim())
        .send()
        .await?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(GroqError::Api {
            status: status.as_u16(),
            body,
        });
    }
    Ok(())
}
