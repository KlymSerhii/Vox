use std::time::Duration;

use once_cell::sync::Lazy;
use reqwest::multipart::{Form, Part};

/// One reqwest client per process. Reuses TLS connections to api.groq.com
/// across requests (saves ~100 ms RTT on every call after the first).
static HTTP: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .https_only(true)
        .build()
        .expect("build reqwest client")
});

const GROQ_TRANSCRIPTIONS_URL: &str =
    "https://api.groq.com/openai/v1/audio/transcriptions";
const GROQ_MODELS_URL: &str = "https://api.groq.com/openai/v1/models";
const WHISPER_MODEL: &str = "whisper-large-v3-turbo";

#[derive(Debug, thiserror::Error)]
pub enum SttError {
    #[error("API key missing — open Settings and paste a Groq API key")]
    MissingKey,
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("Groq API error ({status}): {body}")]
    Api { status: u16, body: String },
    #[error("failed to parse response: {0}")]
    Parse(String),
}

#[derive(Debug, serde::Deserialize)]
struct TranscriptionResponse {
    text: String,
}

/// POST the WAV bytes to Groq and return the transcript text.
///
/// `dictionary` terms are passed to Whisper as a prompt to bias decoding toward
/// proper-noun spellings (PRD §4.5). This affects raw STT, complementing the
/// LLM-side dictionary application in [`crate::llm::cleanup`].
pub async fn transcribe_wav(
    wav_bytes: Vec<u8>,
    api_key: &str,
    dictionary: &[String],
) -> Result<String, SttError> {
    if api_key.trim().is_empty() {
        return Err(SttError::MissingKey);
    }

    let client = &*HTTP;

    let part = Part::bytes(wav_bytes)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| SttError::Parse(e.to_string()))?;

    let mut form = Form::new()
        .text("model", WHISPER_MODEL)
        .text("response_format", "json")
        .text("temperature", "0")
        .part("file", part);

    if let Some(prompt) = whisper_prompt(dictionary) {
        form = form.text("prompt", prompt);
    }

    let resp = client
        .post(GROQ_TRANSCRIPTIONS_URL)
        .bearer_auth(api_key.trim())
        .multipart(form)
        .send()
        .await?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(SttError::Api {
            status: status.as_u16(),
            body,
        });
    }

    let parsed: TranscriptionResponse = resp
        .json()
        .await
        .map_err(|e| SttError::Parse(e.to_string()))?;
    Ok(parsed.text.trim().to_string())
}

/// Build the optional Whisper bias prompt from dictionary terms.
///
/// Whisper accepts up to 224 tokens of prompt; we conservatively cap by
/// character count. Returns None when the dictionary is empty (no prompt
/// header).
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
    // ~896 char ≈ 224 tokens upper bound. We leave some headroom.
    let bounded = if joined.chars().count() > 700 {
        joined.chars().take(700).collect()
    } else {
        joined
    };
    Some(format!("Names and terms used: {bounded}."))
}

/// Hit Groq's models endpoint as a cheap validation of the API key.
pub async fn test_key(api_key: &str) -> Result<(), SttError> {
    if api_key.trim().is_empty() {
        return Err(SttError::MissingKey);
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
        return Err(SttError::Api {
            status: status.as_u16(),
            body,
        });
    }
    Ok(())
}
