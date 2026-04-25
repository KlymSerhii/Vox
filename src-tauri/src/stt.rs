use std::time::Duration;

use reqwest::multipart::{Form, Part};

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
pub async fn transcribe_wav(wav_bytes: Vec<u8>, api_key: &str) -> Result<String, SttError> {
    if api_key.trim().is_empty() {
        return Err(SttError::MissingKey);
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()?;

    let part = Part::bytes(wav_bytes)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| SttError::Parse(e.to_string()))?;

    let form = Form::new()
        .text("model", WHISPER_MODEL)
        .text("response_format", "json")
        .text("temperature", "0")
        .part("file", part);

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

/// Hit Groq's models endpoint as a cheap validation of the API key.
pub async fn test_key(api_key: &str) -> Result<(), SttError> {
    if api_key.trim().is_empty() {
        return Err(SttError::MissingKey);
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;

    let resp = client
        .get(GROQ_MODELS_URL)
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
