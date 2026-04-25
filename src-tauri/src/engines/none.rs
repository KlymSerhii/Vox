//! No-op cleanup engine — selected when `llm_engine == "none"`. Returns
//! the input transcript unchanged. Lets us drop `cleanup_enabled` as a
//! dedicated bool: "off" is just `llm_engine = "none"`.

use crate::engines::LlmEngine;

pub struct NoOpEngine;

#[async_trait::async_trait]
impl LlmEngine for NoOpEngine {
    async fn cleanup(
        &self,
        transcript: &str,
        _system_prompt: &str,
        _dictionary: &[String],
    ) -> anyhow::Result<String> {
        Ok(transcript.trim().to_string())
    }

    fn id(&self) -> &'static str {
        "none"
    }

    fn label(&self) -> String {
        "Off (raw transcript)".into()
    }

    fn requires_network(&self) -> bool {
        false
    }
}
