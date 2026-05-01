//! Apple Intelligence (Foundation Models) polishing engine.
//!
//! Engine ID: `"apple-fm"`. Calls into the Swift bridge in
//! `apple_fm_bridge`, which uses `LanguageModelSession` from
//! Apple's `FoundationModels` framework.
//!
//! Failure modes that fall back to the raw transcript via the existing
//! `session.rs` fail-open path:
//! - macOS < 26 (`OsTooOld`)
//! - M1+ Mac without Apple Intelligence support (`DeviceNotEligible`)
//! - User has Apple Intelligence disabled (`IntelligenceNotEnabled`)
//! - Model still downloading on first launch (`ModelNotReady`)
//! - Apple's safety guardrails reject the dictation as a `GenerationError`
//!   — the engine returns an error, the orchestrator copies the raw
//!   transcript instead.

use std::time::Instant;

use std::sync::Arc;

use parking_lot::Mutex;

use crate::engines::apple_fm_bridge;
use crate::engines::{ChunkCallback, LlmEngine};

// Apple's on-device Foundation Model is a ~3B model tuned for short
// rewrites. It does NOT follow long bulleted instruction lists as
// reliably as Gemma 4 E4B, and its default "Apple house style" leans
// verbose — without an output-length cap it will routinely produce a
// polished transcript longer than the dictation, against rule #7.
//
// `dynamic_max_tokens` enforces a hard ceiling at ~1.2× the input so
// the model physically cannot run away. The +96 token slack covers
// paragraph breaks and structural markup the prompt asks for; the 256
// floor handles tiny inputs where 1.2× would be 0.

const TIMEOUT_SECS: u32 = 90;

/// English BPE ≈ 4 chars/token. Cleaned dictation should be ≤ input
/// length plus a small allowance for added structure (paragraph breaks,
/// numbered list punctuation).
fn dynamic_max_tokens(input_chars: usize) -> i32 {
    let estimated_input = input_chars / 4;
    let cap = (estimated_input * 12) / 10 + 96;
    cap.clamp(256, 8192) as i32
}

/// Reinforcement appended after every system prompt (built-in or
/// user-supplied) before it goes to the Foundation Model. Apple's small
/// model needs the conciseness/format rules in a *terminal* position
/// because instruction-following degrades on long preambles. Keep this
/// short and absolute — bulleted directives perform better than prose
/// here.
const APPLE_FM_REINFORCEMENT: &str = "

CRITICAL OUTPUT CONSTRAINTS (highest priority — these override any conflicting instructions above):
- Output MUST be the cleaned transcript text and nothing else. No preface, no quotes, no commentary, no \"Here is\" or \"Sure,\".
- Output length MUST be shorter than or equal to the input length. Do not expand, elaborate, or rephrase for politeness.
- For multi-topic content, separate distinct ideas with a single blank line (paragraph break).
- For enumerations of 2+ items, format as a numbered list (\"1. ... 2. ...\").
- Never add facts, names, dates, or details the speaker did not say.";

const DEFAULT_SYSTEM_PROMPT: &str = "You clean up and structure voice-dictation transcripts. Output ONLY the cleaned text. Do not output reasoning, planning, commentary, prefaces, or quotes.

Rules:
1. Remove filler (\"um\", \"uh\", \"you know\", \"like\", \"basically\", \"I mean\") when used as filler. Keep them when they carry meaning (\"I mean it.\").
2. Apply standard English punctuation and capitalization. For long passages, group related ideas into paragraphs separated by blank lines.
3. Fix mid-sentence self-corrections.
4. Drop content the speaker explicitly retracts — including everything before the retraction marker.
5. When the speaker enumerates two or more items, render them as a numbered list.
6. Preserve the speaker's register. Do not formalize casual speech or vice versa.
7. Never add content, facts, names, or numbers the speaker did not say.";

pub struct AppleFmEngine;

impl AppleFmEngine {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl LlmEngine for AppleFmEngine {
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

        // Probe before spawning — `DeviceNotEligible` / `OsTooOld` fail
        // fast without paying the blocking-thread tax.
        match apple_fm_bridge::availability() {
            apple_fm_bridge::AvailabilityCode::Available => {}
            other => {
                return Err(anyhow::anyhow!(
                    "Apple Intelligence unavailable: {}",
                    availability_message(other)
                ));
            }
        }

        let started = Instant::now();
        let max_tokens = dynamic_max_tokens(trimmed.len());
        let system = build_system_prompt(system_prompt, dictionary);
        let transcript_owned = trimmed.to_string();

        log::info!(
            "apple-fm: polishing {} chars, system_prompt={} chars, max_tokens={}",
            trimmed.len(),
            system.len(),
            max_tokens
        );
        log::debug!("apple-fm: full system prompt:\n{system}");
        log::debug!("apple-fm: transcript:\n{transcript_owned}");

        // The Swift bridge blocks on a `DispatchSemaphore`, so it must
        // not run on a tokio worker — `spawn_blocking` parks it on the
        // blocking-IO thread pool instead.
        let result = tauri::async_runtime::spawn_blocking(move || {
            apple_fm_bridge::polish_blocking(&system, &transcript_owned, max_tokens, TIMEOUT_SECS)
        })
        .await;

        match result {
            Ok(Ok(out)) => {
                log::info!(
                    "apple-fm: polished {}→{} chars in {} ms",
                    trimmed.len(),
                    out.len(),
                    started.elapsed().as_millis()
                );
                if out.trim().is_empty() && trimmed.len() > 20 {
                    log::warn!(
                        "apple-fm: polish returned EMPTY content for {}-char input; session will fail-open to raw transcript",
                        trimmed.len()
                    );
                }
                Ok(out)
            }
            Ok(Err(e)) => {
                log::warn!(
                    "apple-fm: polish FAILED in {} ms — session will fail-open: {e}",
                    started.elapsed().as_millis()
                );
                Err(anyhow::anyhow!("apple-fm: {e}"))
            }
            Err(e) => Err(anyhow::anyhow!("apple-fm spawn_blocking: {e}")),
        }
    }

    async fn cleanup_stream(
        &self,
        transcript: &str,
        system_prompt: &str,
        dictionary: &[String],
        on_chunk: ChunkCallback,
    ) -> anyhow::Result<String> {
        let trimmed = transcript.trim();
        if trimmed.split_whitespace().count() < 2 {
            return Ok(trimmed.to_string());
        }

        match apple_fm_bridge::availability() {
            apple_fm_bridge::AvailabilityCode::Available => {}
            other => {
                return Err(anyhow::anyhow!(
                    "Apple Intelligence unavailable: {}",
                    availability_message(other)
                ));
            }
        }

        let started = Instant::now();
        let max_tokens = dynamic_max_tokens(trimmed.len());
        let system = build_system_prompt(system_prompt, dictionary);
        let transcript_owned = trimmed.to_string();

        log::info!(
            "apple-fm: streaming-polish {} chars, system_prompt={} chars, max_tokens={}",
            trimmed.len(),
            system.len(),
            max_tokens
        );

        // The Swift bridge fires the cumulative-partial callback on its
        // own internal serial queue; we forward and stash the latest
        // snapshot so we can return it as the final value.
        let last_snapshot: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
        let stash = last_snapshot.clone();

        let bridged_cb: Box<dyn Fn(&str) + Send + Sync + 'static> =
            Box::new(move |s: &str| {
                {
                    let mut g = stash.lock();
                    *g = s.to_string();
                }
                on_chunk(s);
            });

        let result = tauri::async_runtime::spawn_blocking(move || {
            apple_fm_bridge::polish_stream_blocking(
                &system,
                &transcript_owned,
                max_tokens,
                TIMEOUT_SECS,
                bridged_cb,
            )
        })
        .await;

        match result {
            Ok(Ok(())) => {
                let final_text = last_snapshot.lock().clone();
                log::info!(
                    "apple-fm: streamed polish {}→{} chars in {} ms",
                    trimmed.len(),
                    final_text.len(),
                    started.elapsed().as_millis()
                );
                Ok(final_text)
            }
            Ok(Err(e)) => Err(anyhow::anyhow!("apple-fm stream: {e}")),
            Err(e) => Err(anyhow::anyhow!("apple-fm stream spawn_blocking: {e}")),
        }
    }

    fn id(&self) -> &'static str {
        "apple-fm"
    }

    fn label(&self) -> String {
        "Apple Intelligence (Foundation Models)".into()
    }

    fn requires_network(&self) -> bool {
        false
    }
}

/// Maps the bridge's typed availability code to a user-readable hint.
/// Used both in error paths and for the IPC `apple_fm_availability`
/// command surfaced to the Settings / onboarding UI.
pub fn availability_message(code: apple_fm_bridge::AvailabilityCode) -> &'static str {
    use apple_fm_bridge::AvailabilityCode::*;
    match code {
        Available => "available",
        DeviceNotEligible => "this Mac does not support Apple Intelligence",
        IntelligenceNotEnabled => "Apple Intelligence is disabled in System Settings",
        ModelNotReady => "Apple Intelligence model still downloading",
        OsTooOld => "requires macOS 26 (Tahoe) or later",
        InternalError => "unknown internal error",
        FrameworkMissing => "Foundation Models framework not present in this build",
    }
}

fn build_system_prompt(system_override: &str, dictionary: &[String]) -> String {
    let base = if system_override.trim().is_empty() {
        DEFAULT_SYSTEM_PROMPT.to_string()
    } else {
        system_override.to_string()
    };
    let cleaned: Vec<&str> = dictionary
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    let dictionary_block = if cleaned.is_empty() {
        String::new()
    } else {
        format!(
            "\n\nPreserve these proper nouns and acronyms exactly as written: {}.",
            cleaned.join(", ")
        )
    };
    // Reinforcement comes LAST so it sits adjacent to the user message
    // — Apple's small model attends most strongly to the tail of the
    // instruction context.
    format!("{base}{dictionary_block}{APPLE_FM_REINFORCEMENT}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_system_prompt_appends_dictionary() {
        let s = build_system_prompt("base prompt", &["Anthropic".into(), "Vox".into()]);
        assert!(s.contains("base prompt"));
        assert!(s.contains("Anthropic"));
        assert!(s.contains("Vox"));
    }

    #[test]
    fn build_system_prompt_uses_default_for_empty_override() {
        let s = build_system_prompt("   ", &[]);
        assert!(s.starts_with(DEFAULT_SYSTEM_PROMPT));
    }

    #[test]
    fn build_system_prompt_appends_reinforcement_for_apple_small_model() {
        // The reinforcement block is the load-bearing fix for Apple's
        // small model ignoring length / paragraph rules buried mid-prompt.
        // Don't pin the exact text (it'll evolve with prompt tuning) —
        // just assert the load-bearing tokens are present.
        let s = build_system_prompt("base prompt", &[]);
        assert!(
            s.contains("CRITICAL OUTPUT CONSTRAINTS"),
            "reinforcement header missing"
        );
        assert!(
            s.contains("shorter than or equal to the input"),
            "conciseness rule missing"
        );
        assert!(
            s.contains("paragraph break"),
            "paragraph-break rule missing"
        );
    }

    #[test]
    fn build_system_prompt_places_reinforcement_after_dictionary() {
        // Dictionary must sit between the base prompt and the
        // reinforcement so the latter remains adjacent to the user
        // message — that's where Apple's model focuses attention.
        let s = build_system_prompt(
            "base",
            &["Anthropic".into(), "Vox".into()],
        );
        let dict_pos = s.find("Anthropic").expect("dictionary should appear");
        let reinforce_pos = s
            .find("CRITICAL OUTPUT CONSTRAINTS")
            .expect("reinforcement should appear");
        assert!(
            dict_pos < reinforce_pos,
            "reinforcement must come after the dictionary block"
        );
    }

    #[test]
    fn dynamic_max_tokens_floors_short_inputs() {
        // 256-token floor protects tiny dictations (e.g. "hello world")
        // from being capped at zero by the 1.2× formula.
        assert_eq!(dynamic_max_tokens(0), 256);
        assert_eq!(dynamic_max_tokens(50), 256);
    }

    #[test]
    fn dynamic_max_tokens_caps_growth_to_input_plus_overhead() {
        // 4 chars/token × 0.3 (= 1.2× minus 0.9×) gives ~30% slack on
        // top of input → output ≤ ~1.3× input tokens. Confirms the
        // 1.2× formula; if someone bumps the multiplier this test
        // fails and forces a doc update.
        let cap = dynamic_max_tokens(4_000);
        // 1000 input tokens → 1000 * 1.2 + 96 = 1296.
        assert_eq!(cap, 1_296);
    }

    #[test]
    fn dynamic_max_tokens_caps_pathological_inputs() {
        // 30-min dictation safety net.
        assert_eq!(dynamic_max_tokens(200_000), 8_192);
    }

    #[test]
    fn availability_message_covers_all_codes() {
        use apple_fm_bridge::AvailabilityCode::*;
        // Every variant must produce a non-empty hint so the UI never
        // surfaces a blank tooltip.
        for code in [
            Available,
            DeviceNotEligible,
            IntelligenceNotEnabled,
            ModelNotReady,
            OsTooOld,
            InternalError,
            FrameworkMissing,
        ] {
            assert!(!availability_message(code).is_empty());
        }
    }
}
