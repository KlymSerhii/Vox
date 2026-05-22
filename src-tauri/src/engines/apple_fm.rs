//! Apple Intelligence (Foundation Models) polishing engine.
//!
//! Engine ID: `"apple-fm"`. Calls into the Swift bridge in
//! `apple_fm_bridge`, which uses `LanguageModelSession` from
//! Apple's `FoundationModels` framework.
//!
//! Session lifecycle: `LanguageModelSession` instances are reused
//! across calls inside the Swift `SessionCache` actor (see
//! `swift/AppleFmBridge.swift`), keyed on system-prompt string and
//! capped at 4 entries with LRU eviction. A session whose call errored
//! or was cancelled by timeout is evicted before the next caller wakes
//! so we never inherit a corrupted transcript history. The Rust side
//! is intentionally stateless w.r.t. session reuse — every `cleanup`
//! / `cleanup_stream` invocation goes through the bridge, and the
//! actor handles the cache key match.
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
        // Apple's Foundation Model has no chain-of-thought toggle; the
        // value is accepted to satisfy the trait contract and ignored.
        _enable_thinking: bool,
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
        // Apple's Foundation Model has no chain-of-thought toggle.
        _enable_thinking: bool,
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
        // One-shot TTFT log: the Swift bridge runs on a dispatch queue,
        // so the closure fires across threads. Wrap the Option in an
        // Arc<Mutex> so the first non-empty snapshot logs once and
        // every later snapshot skips the branch.
        let first_chunk_at: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
        let first_slot = first_chunk_at.clone();

        let bridged_cb: Box<dyn Fn(&str) + Send + Sync + 'static> =
            Box::new(move |s: &str| {
                {
                    let mut g = stash.lock();
                    *g = s.to_string();
                }
                if !s.is_empty() {
                    let mut slot = first_slot.lock();
                    if slot.is_none() {
                        *slot = Some(Instant::now());
                        log::debug!(
                            "apple-fm: first polish chunk at {} ms (acc={} chars)",
                            started.elapsed().as_millis(),
                            s.len(),
                        );
                    }
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

fn build_system_prompt(system_prompt: &str, dictionary: &[String]) -> String {
    let cleaned: Vec<&str> = dictionary
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if cleaned.is_empty() {
        return system_prompt.to_string();
    }
    format!(
        "{system_prompt}\n\nPreserve these proper nouns and acronyms exactly as written: {}.",
        cleaned.join(", ")
    )
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
    fn build_system_prompt_adds_no_reinforcement() {
        // The mode prompt (from Settings) is the single source of truth —
        // the engine must not append its own constraints block or fall
        // back to a hardcoded default. With no dictionary the output is
        // exactly the passed prompt; with one, only the dictionary line
        // is appended.
        let bare = build_system_prompt("base prompt", &[]);
        assert_eq!(bare, "base prompt");
        assert!(!bare.contains("CRITICAL OUTPUT CONSTRAINTS"));

        let with_dict = build_system_prompt("base prompt", &["Anthropic".into()]);
        assert!(with_dict.starts_with("base prompt"));
        assert!(with_dict.contains("Anthropic"));
        assert!(!with_dict.contains("CRITICAL OUTPUT CONSTRAINTS"));
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
