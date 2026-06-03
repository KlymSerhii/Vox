//! Voice Modes: per-mode JSON files at `<app_data_dir>/modes/<id>.json`.
//! Any missing built-in is seeded on every launch — so new built-ins
//! shipped in an update appear automatically. Existing files are
//! never overwritten, so user edits (and manually deleted built-ins
//! that the user later re-creates) are preserved.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum PunctuationStyle {
    Aggressive,
    Standard,
    Minimal,
    None,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum CaseStyle {
    Sentence,
    Title,
    Lower,
    AsSpoken,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutoTrigger {
    /// Frontmost app bundle id; supports a trailing `*` wildcard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_id_glob: Option<String>,
    /// Case-insensitive substring of the focused window title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_title_substring: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VoiceMode {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub description: String,
    pub system_prompt: String,
    #[serde(default)]
    pub dictionary: Vec<String>,
    pub punctuation: PunctuationStyle,
    pub case: CaseStyle,
    #[serde(default)]
    pub max_length_chars: Option<usize>,
    /// Per-mode override of the global `Settings::enable_thinking`. None
    /// (the JSON field is omitted) means inherit the global; Some(b)
    /// pins the value for this mode regardless of the global toggle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_thinking: Option<bool>,
    #[serde(default)]
    pub auto_triggers: Vec<AutoTrigger>,
    #[serde(default)]
    pub built_in: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

impl VoiceMode {
    /// Convenience wrapper used by tests and any future caller that
    /// holds a VoiceMode directly. The runtime pipeline doesn't reach
    /// this — it post-processes through `session::ModeSnapshot` so a
    /// mid-session edit to the underlying mode can't change the
    /// in-flight transcript. Both paths converge on
    /// [`apply_post_processing`].
    #[allow(dead_code)]
    pub fn apply_post_processing(&self, text: &str) -> String {
        apply_post_processing(
            &self.case,
            &self.punctuation,
            self.max_length_chars,
            text,
        )
    }
}

/// Free-function form of post-processing. Both `VoiceMode` and
/// `ModeSnapshot` (in `session.rs`) call into here so the rules are
/// defined exactly once. The case / punctuation / max-length args are
/// taken by ref so callers can pass borrowed copies of the snapshot
/// fields without cloning.
pub fn apply_post_processing(
    case: &CaseStyle,
    punctuation: &PunctuationStyle,
    max_length_chars: Option<usize>,
    text: &str,
) -> String {
    let mut out = text.to_string();

    match case {
        CaseStyle::Title => out = title_case(&out),
        CaseStyle::Lower => out = out.to_lowercase(),
        CaseStyle::Sentence | CaseStyle::AsSpoken => {}
    }

    if matches!(punctuation, PunctuationStyle::None) {
        out = strip_terminal_punctuation(&out);
    }

    if let Some(cap) = max_length_chars {
        out = truncate_to_cap(&out, cap);
    }

    out
}

pub fn builtins() -> Vec<VoiceMode> {
    let now = Utc::now().timestamp();
    vec![
        VoiceMode {
            id: "general".into(),
            name: "General".into(),
            icon: "💬".into(),
            description: "Balanced cleanup. The default when nothing else applies.".into(),
            system_prompt: GENERAL_PROMPT.into(),
            dictionary: vec![],
            punctuation: PunctuationStyle::Standard,
            case: CaseStyle::Sentence,
            max_length_chars: None,
            enable_thinking: None,
            auto_triggers: vec![],
            built_in: true,
            created_at: now,
            updated_at: now,
        },
        VoiceMode {
            id: "minimal".into(),
            name: "Minimal".into(),
            icon: "✍️".into(),
            description: "Minimal cleanup. Keeps every word; only removes fillers and adds punctuation.".into(),
            system_prompt: MINIMAL_PROMPT.into(),
            dictionary: vec![],
            punctuation: PunctuationStyle::Standard,
            case: CaseStyle::Sentence,
            max_length_chars: None,
            enable_thinking: None,
            auto_triggers: vec![
                AutoTrigger {
                    bundle_id_glob: Some("com.literatureandlatte.scrivener3".into()),
                    window_title_substring: None,
                },
                AutoTrigger {
                    bundle_id_glob: Some("com.literatureandlatte.scrivener".into()),
                    window_title_substring: None,
                },
                AutoTrigger {
                    bundle_id_glob: Some("com.ulyssesapp.mac".into()),
                    window_title_substring: None,
                },
                AutoTrigger {
                    bundle_id_glob: Some("pro.writer.mac".into()),
                    window_title_substring: None,
                },
                AutoTrigger {
                    bundle_id_glob: Some("com.bloombuilt.dayone-mac".into()),
                    window_title_substring: None,
                },
            ],
            built_in: true,
            created_at: now,
            updated_at: now,
        },
        VoiceMode {
            id: "email".into(),
            name: "Email".into(),
            icon: "📧".into(),
            description: "Formal, well-punctuated email body.".into(),
            system_prompt: EMAIL_PROMPT.into(),
            dictionary: vec![],
            punctuation: PunctuationStyle::Aggressive,
            case: CaseStyle::Sentence,
            max_length_chars: None,
            enable_thinking: None,
            auto_triggers: vec![
                AutoTrigger {
                    bundle_id_glob: Some("com.apple.mail".into()),
                    window_title_substring: None,
                },
                AutoTrigger {
                    bundle_id_glob: Some("com.google.Chrome".into()),
                    window_title_substring: Some("Gmail".into()),
                },
                AutoTrigger {
                    bundle_id_glob: Some("com.google.Chrome".into()),
                    window_title_substring: Some("mail.google".into()),
                },
                AutoTrigger {
                    bundle_id_glob: Some("com.tinyspeck.slackmacgap".into()),
                    window_title_substring: Some("thread".into()),
                },
            ],
            built_in: true,
            created_at: now,
            updated_at: now,
        },
        VoiceMode {
            id: "chat".into(),
            name: "Chat".into(),
            icon: "💭".into(),
            description: "Short, casual messages for Slack/Discord/iMessage.".into(),
            system_prompt: CHAT_PROMPT.into(),
            dictionary: vec![],
            punctuation: PunctuationStyle::Minimal,
            case: CaseStyle::Sentence,
            max_length_chars: Some(280),
            enable_thinking: None,
            auto_triggers: vec![
                AutoTrigger {
                    bundle_id_glob: Some("com.tinyspeck.slackmacgap".into()),
                    window_title_substring: None,
                },
                AutoTrigger {
                    bundle_id_glob: Some("com.hnc.Discord".into()),
                    window_title_substring: None,
                },
                AutoTrigger {
                    bundle_id_glob: Some("com.apple.MobileSMS".into()),
                    window_title_substring: None,
                },
                AutoTrigger {
                    bundle_id_glob: Some("org.whispersystems.signal-desktop".into()),
                    window_title_substring: None,
                },
            ],
            built_in: true,
            created_at: now,
            updated_at: now,
        },
        VoiceMode {
            id: "code-comment".into(),
            name: "Code Comment".into(),
            icon: "//".into(),
            description: "Concise technical comments for code editors and PRs.".into(),
            system_prompt: CODE_COMMENT_PROMPT.into(),
            dictionary: vec![],
            punctuation: PunctuationStyle::Standard,
            case: CaseStyle::Sentence,
            max_length_chars: None,
            enable_thinking: None,
            auto_triggers: vec![
                AutoTrigger {
                    bundle_id_glob: Some("com.microsoft.VSCode".into()),
                    window_title_substring: None,
                },
                AutoTrigger {
                    bundle_id_glob: Some("com.todesktop.230313mzl4w4u92".into()),
                    window_title_substring: None,
                },
                AutoTrigger {
                    bundle_id_glob: Some("com.jetbrains.*".into()),
                    window_title_substring: None,
                },
                AutoTrigger {
                    bundle_id_glob: Some("com.apple.dt.Xcode".into()),
                    window_title_substring: None,
                },
            ],
            built_in: true,
            created_at: now,
            updated_at: now,
        },
        VoiceMode {
            id: "notes".into(),
            name: "Notes".into(),
            icon: "📝".into(),
            description: "Structured notes with optional bullets.".into(),
            system_prompt: NOTES_PROMPT.into(),
            dictionary: vec![],
            punctuation: PunctuationStyle::Aggressive,
            case: CaseStyle::Sentence,
            max_length_chars: None,
            enable_thinking: None,
            auto_triggers: vec![
                AutoTrigger {
                    bundle_id_glob: Some("com.apple.Notes".into()),
                    window_title_substring: None,
                },
                AutoTrigger {
                    bundle_id_glob: Some("notion.id".into()),
                    window_title_substring: None,
                },
                AutoTrigger {
                    bundle_id_glob: Some("md.obsidian".into()),
                    window_title_substring: None,
                },
                AutoTrigger {
                    bundle_id_glob: Some("com.bear-app.bear".into()),
                    window_title_substring: None,
                },
            ],
            built_in: true,
            created_at: now,
            updated_at: now,
        },
        VoiceMode {
            id: "summary".into(),
            name: "Summary".into(),
            icon: "📋".into(),
            description: "Condense the dictation into a short summary of key points.".into(),
            system_prompt: SUMMARY_PROMPT.into(),
            dictionary: vec![],
            punctuation: PunctuationStyle::Standard,
            case: CaseStyle::Sentence,
            max_length_chars: None,
            enable_thinking: None,
            auto_triggers: vec![],
            built_in: true,
            created_at: now,
            updated_at: now,
        },
    ]
}

const GENERAL_PROMPT: &str = "You clean up and structure voice-dictation transcripts. Return plain text only — no preface, no quotes, no reasoning, no explanations.

Write the output in the same language as the transcript: detect the language the speaker used and respond in that exact language. Never translate. The rules and examples below are written in English only for illustration — apply the equivalent cleanup in the speaker's language (e.g. German fillers \"äh\"/\"ähm\", French \"euh\").

Rules:
1. Remove filler (\"um\", \"uh\", \"like\" as filler, \"you know\", \"sort of\", \"kind of\", \"basically\", \"I mean\" as filler — and their equivalents in the speaker's language). Keep them when meaningful.
2. Apply standard punctuation and capitalization for that language. For longer passages, group related ideas into paragraphs separated by blank lines.
3. Fix mid-sentence self-corrections.
   - \"go to the — I mean, the store\" → \"go to the store\"
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
6. Preserve the speaker's register and word choice. Do not formalize casual speech.
7. Never add or substitute facts, names, links, or content. Every proper noun, name, identifier, and number from the transcript must appear verbatim in the output — do not \"correct\" a name to a different but similar-sounding one (e.g. Claude ↛ Claudia, Anthropic ↛ Anthropik), even if the other word feels more familiar.

Examples:
- \"uh so like the meeting is um at three pm\" → \"The meeting is at 3 PM.\"
- \"I'm not using the model, no, disregard that, I am using the model\" → \"I am using the model.\"
- \"first we ship the build then we run tests and finally we deploy\" →
   \"1. Ship the build.\n2. Run tests.\n3. Deploy.\"
- \"I need you to first of all add tests and then also verify the build passes\" →
   \"I need you to:\n1. Add tests.\n2. Verify the build passes.\"
- \"so everything looks fine but now I need you to first of all add tests and then also to verify that the build passes appropriately\" →
   \"Everything looks fine. I need you to:\n1. Add tests.\n2. Verify that the build passes appropriately.\"";

const MINIMAL_PROMPT: &str = "You clean up voice-dictation transcripts for users who want their exact words preserved — novelists drafting prose, journalers, interviewers transcribing speech, anyone who needs the speech captured as-said. Return plain text only — no preface, no quotes, no reasoning, no explanations.

Write the output in the same language as the transcript: detect the language the speaker used and respond in that exact language. Never translate. The rules and examples below are written in English only for illustration — apply the equivalent cleanup in the speaker's language.

The hard rule: preserve every meaningful word from the transcript, in the order spoken. Do not paraphrase. Do not \"polish\" word choice. Do not reorder clauses. Do not merge sentences. Do not split sentences except where end-of-sentence punctuation naturally requires it.

What you DO change:
1. Remove obvious disfluencies only:
   - Standalone \"um\", \"uh\", \"er\", \"ah\", \"mm\", \"hmm\" used as hesitations.
   - Stutter repeats caused by speech disfluency: \"the the dog\" → \"the dog\", \"I— I think\" → \"I think\", \"we we we should\" → \"we should\".
   - Trailing \"...\" or \"—\" that came from a mid-word cutoff with nothing after it.
2. Add standard punctuation and capitalization for the transcript's language where the spoken cadence implies it, following that language's conventions (e.g. German capitalizes all nouns). Sentence-case the first word of each sentence. Capitalize proper nouns the speaker pronounced as proper nouns.
3. Apply paragraph breaks at long pauses or clear topic shifts. When in doubt, fewer breaks.

What you DO NOT change — this list is exhaustive and overrides any instinct to \"improve\":
- Word choice and register. If the speaker said \"gonna\", leave \"gonna\". If they said \"like\" as a discourse marker between clauses (\"it was, like, really good\"), leave it — it carries tone. Only remove \"like\" when it is a pure standalone hesitation (\"uh, like, um\").
- Sentence fragments, run-ons, comma splices the speaker produced deliberately or stylistically. Punctuate them as said; do not rewrite into \"correct\" grammar.
- Self-corrections, false starts, and changes of mind. \"I went to the store, no, the market\" stays as \"I went to the store, no, the market.\" — the speaker is showing their thinking and that belongs in the draft.
- Retracted content. \"Send it to John. Actually, send it to Mary.\" stays as both sentences. If the user wanted the retraction removed, they would use General.
- Enumerations. \"First we ship, then we test, then we deploy\" stays as one sentence. Do not bullet or number anything.
- Nonsense words, placeholders, and repeated tokens the speaker said deliberately. \"blah blah blah\", \"yada yada\", \"lorem ipsum\", \"the whatever, you know what I mean\" — these are intentional and stay verbatim. The test: if the same word appears more than once with normal cadence between repetitions, it is deliberate; only back-to-back disfluent repeats get collapsed.
- Profanity, slang, dialect, made-up words. Leave them exactly as said.
- Every proper noun, name, identifier, number, and quoted phrase from the transcript must appear verbatim in the output. Do not \"correct\" a name to a similar-sounding one.

When in doubt, keep the word. The user picked this mode specifically because they want their voice preserved.

Examples:
- \"um so I was thinking like maybe we could uh meet on Tuesday\" → \"So I was thinking, like, maybe we could meet on Tuesday.\"
- \"the the dog ran ran across the the yard\" → \"The dog ran across the yard.\"
- \"I went to the store, no, the market, to buy some bread\" → \"I went to the store, no, the market, to buy some bread.\"
- \"she said blah blah blah and then walked out, you know\" → \"She said blah blah blah and then walked out, you know.\"
- \"gonna grab coffee real quick brb\" → \"Gonna grab coffee real quick, brb.\"
- \"first we ship the build then we run tests and finally we deploy\" → \"First we ship the build, then we run tests, and finally we deploy.\"";

const EMAIL_PROMPT: &str = "You are formatting a dictated message as the BODY of a professional email. Return plain text only.

Rules:
- Write the output in the same language the speaker used. Detect the input language and respond in that language; never translate.
- Use complete, well-punctuated sentences. Apply paragraph breaks where the speaker pauses or shifts topic.
- Maintain a polite, professional tone — but do NOT add a salutation (\"Hi\", \"Dear\") or sign-off (\"Best\", \"Thanks\") that the user did not say.
- Remove fillers (um, uh, like, you know).
- Spell out abbreviations only if the speaker said them spelled out.
- Never invent recipients, dates, or specifics.
- Output only the email body text.";

const CHAT_PROMPT: &str = "You are formatting a dictated message for a casual chat (Slack, Discord, iMessage). Return plain text only.

Rules:
- Write the output in the same language the speaker used. Detect the input language and respond in that language; never translate.
- Keep it short and conversational. Trim filler ruthlessly.
- Preserve the speaker's casual register — contractions are fine, sentence fragments are fine.
- Use minimal punctuation. One terminal mark per sentence; no Oxford-comma fussiness.
- Do not capitalize words for emphasis the speaker did not pronounce as proper nouns.
- Preserve emoji the speaker said by name (e.g. \"smiley face\" → 🙂) only when unambiguous.
- Never add greetings, sign-offs, or closing remarks.
- Output only the chat text.";

const CODE_COMMENT_PROMPT: &str = "You are rewriting dictation as a technical code comment or PR comment. Return plain text only.

Rules:
- Write the comment in the same language the speaker used. Detect the input language and respond in that language; never translate.
- Be concise, present-tense, third-person (\"adjusts the cache\", not \"I adjusted the cache\").
- Preserve identifier names verbatim — CamelCase, snake_case, kebab-case, dotted paths, file names.
- Preserve technical terms exactly as said (HTTP, API, GPU, gRPC, KV-cache).
- Do not add a leading // or # — output only the comment text.
- Do not add markdown unless the speaker dictated bullet structure.
- Never invent function names, variables, or behavior the user did not state.
- Output only the comment text.";

const NOTES_PROMPT: &str = "You are formatting dictation as structured notes (Apple Notes, Notion, Obsidian). Return plain text only.

Rules:
- Write the notes in the same language the speaker used. Detect the input language and respond in that language; never translate.
- Use full sentences with complete punctuation.
- If the speaker signals a list (\"first…, second…, third…\" or \"one…, two…, three…\"), format as a bullet list (one item per line, prefixed with \"- \").
- Otherwise, use newline-separated paragraphs at natural topic breaks.
- Do not add headers, titles, or section markers the speaker did not request.
- Never invent items, dates, or specifics.
- Output only the note body.";

const SUMMARY_PROMPT: &str = "You are summarizing a voice-dictation transcript. Produce a concise summary that captures the speaker's key points and intent. Return plain text only — no preface, no quotes, no reasoning, no explanations.

Write the summary in the same language as the transcript: detect the language the speaker used and respond in that exact language. Never translate. The rules and examples below are written in English only for illustration.

Rules:
1. Identify the core ideas, decisions, action items, and conclusions. Drop filler, false starts, and tangents that don't add information.
2. Keep it brief — aim for one short paragraph for most inputs, or a short bulleted list (one item per line, prefixed with \"- \") when the speaker enumerated multiple distinct items.
3. Preserve the speaker's intent and point of view. If the input was instructions, the summary reads as instructions; if reflection, it reads as reflection.
4. Drop content the speaker explicitly retracts.
5. Never invent facts, names, dates, numbers, or commitments. Every proper noun, name, identifier, and number from the transcript must appear verbatim in the output — do not \"correct\" a name to a different but similar-sounding one.
6. Use complete, well-punctuated sentences. Do not add headings like \"Summary:\" or \"Key points:\" — output only the summary body.

Examples:
- \"um so I was thinking we should ship the new auth flow next week, maybe Tuesday or Wednesday, and uh we also need to make sure the migration script is ready, and oh also we should tell the support team in advance\" → \"Ship the new auth flow Tuesday or Wednesday next week. Prepare the migration script and notify the support team in advance.\"
- \"today I learned about caching and I think the main thing is that we should always set a max age and also we should think about cache invalidation more carefully\" → \"Two takeaways on caching: always set a max age, and think more carefully about cache invalidation.\"
- \"the meeting went well, we discussed the roadmap, John said he can take on the API work, and Sarah will own the design refresh, we agreed to sync again on Friday\" → \"Meeting went well. Roadmap discussed; John will take the API work and Sarah will own the design refresh. Next sync on Friday.\"";

/// Process-wide cache rebuilt lazily; invalidated on every mutation.
#[derive(Default)]
pub struct ModeStore {
    inner: RwLock<Option<HashMap<String, VoiceMode>>>,
    root: RwLock<Option<PathBuf>>,
}

impl ModeStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn init_with_root(self: &Arc<Self>, root: PathBuf) -> Result<()> {
        fs::create_dir_all(&root).context("create modes dir")?;
        *self.root.write() = Some(root.clone());
        for mode in builtins() {
            let path = mode_path(&root, &mode.id);
            if !path.exists() {
                write_mode_file(&path, &mode)?;
            }
        }
        *self.inner.write() = None;
        Ok(())
    }

    fn root_or_err(&self) -> Result<PathBuf> {
        self.root
            .read()
            .clone()
            .ok_or_else(|| anyhow!("ModeStore not initialised"))
    }

    fn load_all(&self) -> Result<HashMap<String, VoiceMode>> {
        let root = self.root_or_err()?;
        let mut map = HashMap::new();
        if !root.exists() {
            return Ok(map);
        }
        for entry in fs::read_dir(&root)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                match read_mode_file(&path) {
                    Ok(m) => {
                        map.insert(m.id.clone(), m);
                    }
                    Err(e) => {
                        log::warn!("skipping malformed mode {:?}: {e}", path);
                    }
                }
            }
        }
        Ok(map)
    }

    fn cached(&self) -> Result<HashMap<String, VoiceMode>> {
        if let Some(map) = self.inner.read().clone() {
            return Ok(map);
        }
        let loaded = self.load_all()?;
        *self.inner.write() = Some(loaded.clone());
        Ok(loaded)
    }

    fn invalidate(&self) {
        *self.inner.write() = None;
    }

    /// Built-ins first (canonical order), then user modes alphabetically.
    pub fn list(&self) -> Vec<VoiceMode> {
        let map = match self.cached() {
            Ok(m) => m,
            Err(e) => {
                log::warn!("ModeStore::list failed: {e}");
                return Vec::new();
            }
        };
        let mut out: Vec<VoiceMode> = builtins()
            .iter()
            .filter_map(|b| map.get(&b.id).cloned())
            .collect();
        let mut customs: Vec<VoiceMode> =
            map.values().filter(|m| !m.built_in).cloned().collect();
        customs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        out.extend(customs);
        out
    }

    pub fn get(&self, id: &str) -> Option<VoiceMode> {
        self.cached().ok().and_then(|m| m.get(id).cloned())
    }

    /// Save a user mode; rejects built-ins and unsafe ids (path traversal).
    pub fn save(&self, mode: &VoiceMode) -> Result<()> {
        if !is_safe_mode_id(&mode.id) {
            return Err(anyhow!("invalid mode id"));
        }
        if let Some(existing) = self.get(&mode.id) {
            if existing.built_in {
                return Err(anyhow!("is_builtin"));
            }
        }
        let mut to_save = mode.clone();
        to_save.built_in = false;
        to_save.updated_at = Utc::now().timestamp();
        let root = self.root_or_err()?;
        write_mode_file(&mode_path(&root, &to_save.id), &to_save)?;
        self.invalidate();
        Ok(())
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        if !is_safe_mode_id(id) {
            return Err(anyhow!("invalid mode id"));
        }
        let mode = self.get(id).ok_or_else(|| anyhow!("not_found"))?;
        if mode.built_in {
            return Err(anyhow!("is_builtin"));
        }
        let root = self.root_or_err()?;
        let path = mode_path(&root, id);
        if path.exists() {
            fs::remove_file(&path).context("delete mode file")?;
        }
        self.invalidate();
        Ok(())
    }

    /// Duplicate any mode (built-in or user) into a new editable mode.
    pub fn duplicate(&self, id: &str) -> Result<VoiceMode> {
        if !is_safe_mode_id(id) {
            return Err(anyhow!("invalid mode id"));
        }
        let src = self.get(id).ok_or_else(|| anyhow!("not_found"))?;
        let mut copy = src.clone();
        copy.built_in = false;
        copy.id = self.unique_id_from(&format!("{}-copy", src.id));
        copy.name = format!("{} (Copy)", src.name);
        let now = Utc::now().timestamp();
        copy.created_at = now;
        copy.updated_at = now;
        // Assert again so a future slugifier change can't widen the fs surface.
        if !is_safe_mode_id(&copy.id) {
            return Err(anyhow!("invalid mode id"));
        }
        let root = self.root_or_err()?;
        write_mode_file(&mode_path(&root, &copy.id), &copy)?;
        self.invalidate();
        Ok(copy)
    }

    pub fn export_json(&self, id: &str) -> Result<String> {
        let mode = self.get(id).ok_or_else(|| anyhow!("not_found"))?;
        Ok(serde_json::to_string_pretty(&mode)?)
    }

    /// Import a mode from JSON. Generates a new id on collision or
    /// invalid id. Always saved as `built_in: false`.
    pub fn import_json(&self, json: &str) -> Result<VoiceMode> {
        let mut mode: VoiceMode =
            serde_json::from_str(json).context("invalid .voxmode JSON")?;
        validate(&mode)?;
        mode.built_in = false;
        if !is_safe_mode_id(&mode.id) || self.get(&mode.id).is_some() {
            mode.id = self.unique_id_from(&slugify(&mode.name));
        }
        if !is_safe_mode_id(&mode.id) {
            return Err(anyhow!("could not derive a valid mode id"));
        }
        let now = Utc::now().timestamp();
        if mode.created_at == 0 {
            mode.created_at = now;
        }
        mode.updated_at = now;
        let root = self.root_or_err()?;
        write_mode_file(&mode_path(&root, &mode.id), &mode)?;
        self.invalidate();
        Ok(mode)
    }

    /// Restore the shipped built-ins to their factory definitions:
    /// re-seeds any deleted built-in *and* force-overwrites one whose
    /// on-disk copy has drifted from the shipped version (e.g. an older
    /// app shipped a different prompt). Built-ins aren't editable in the
    /// UI, so the only drift is a stale factory version or a rare
    /// power-user hand-edit, both of which an explicit "Reset built-ins"
    /// is expected to discard. Files that already match are left
    /// untouched so the returned count — and the UI's "already present"
    /// message — stays meaningful on a repeat click. Unlike
    /// [`Self::init_with_root`] (launch-time seeding, which never
    /// overwrites), this is the one path that refreshes existing files.
    pub fn reset_defaults(&self) -> Result<usize> {
        let root = self.root_or_err()?;
        let mut count = 0;
        for mode in builtins() {
            let path = mode_path(&root, &mode.id);
            let needs_write = match read_mode_file(&path) {
                Ok(existing) => !builtin_matches(&existing, &mode),
                Err(_) => true, // missing or unreadable → (re)write
            };
            if needs_write {
                write_mode_file(&path, &mode)?;
                count += 1;
            }
        }
        self.invalidate();
        Ok(count)
    }

    /// Returns the id of the first matching mode in display order;
    /// suggestion only — never auto-switches.
    pub fn suggest_for(
        &self,
        bundle_id: Option<&str>,
        window_title: Option<&str>,
    ) -> Option<String> {
        for mode in self.list() {
            for trigger in &mode.auto_triggers {
                if trigger_matches(trigger, bundle_id, window_title) {
                    return Some(mode.id.clone());
                }
            }
        }
        None
    }

    fn unique_id_from(&self, base: &str) -> String {
        let base = slugify(base);
        if base.is_empty() {
            return format!("mode-{}", Utc::now().timestamp_millis());
        }
        let mut candidate = base.clone();
        let mut suffix = 2;
        while self.get(&candidate).is_some() {
            candidate = format!("{base}-{suffix}");
            suffix += 1;
        }
        candidate
    }
}

pub fn list_modes<R: Runtime>(app: &AppHandle<R>) -> Vec<VoiceMode> {
    store(app).list()
}

pub fn get_mode<R: Runtime>(app: &AppHandle<R>, id: &str) -> Option<VoiceMode> {
    store(app).get(id)
}

pub fn save_mode<R: Runtime>(app: &AppHandle<R>, mode: &VoiceMode) -> Result<()> {
    store(app).save(mode)
}

pub fn delete_mode<R: Runtime>(app: &AppHandle<R>, id: &str) -> Result<()> {
    store(app).delete(id)
}

pub fn duplicate_mode<R: Runtime>(app: &AppHandle<R>, id: &str) -> Result<VoiceMode> {
    store(app).duplicate(id)
}

pub fn export_mode<R: Runtime>(app: &AppHandle<R>, id: &str) -> Result<String> {
    store(app).export_json(id)
}

pub fn import_mode_json<R: Runtime>(app: &AppHandle<R>, json: &str) -> Result<VoiceMode> {
    store(app).import_json(json)
}

pub fn reset_defaults<R: Runtime>(app: &AppHandle<R>) -> Result<usize> {
    store(app).reset_defaults()
}

pub fn suggest_mode_for<R: Runtime>(
    app: &AppHandle<R>,
    bundle_id: Option<&str>,
    window_title: Option<&str>,
) -> Option<String> {
    store(app).suggest_for(bundle_id, window_title)
}

/// Cycle to the next mode in display order.
pub fn cycle_active<R: Runtime>(
    app: &AppHandle<R>,
    current_id: &str,
) -> Option<VoiceMode> {
    let list = list_modes(app);
    if list.is_empty() {
        return None;
    }
    let idx = list.iter().position(|m| m.id == current_id).unwrap_or(0);
    Some(list[(idx + 1) % list.len()].clone())
}

fn store<R: Runtime>(app: &AppHandle<R>) -> Arc<ModeStore> {
    app.state::<Arc<ModeStore>>().inner().clone()
}

pub fn modes_dir<R: Runtime>(app: &AppHandle<R>) -> PathBuf {
    if let Ok(p) = app.path().app_data_dir() {
        return p.join("modes");
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("com.vox.app/modes")
}

fn mode_path(root: &Path, id: &str) -> PathBuf {
    root.join(format!("{id}.json"))
}

fn write_mode_file(path: &Path, mode: &VoiceMode) -> Result<()> {
    let json = serde_json::to_string_pretty(mode)?;
    fs::write(path, json).with_context(|| format!("write mode file {:?}", path))?;
    Ok(())
}

/// True if an on-disk built-in already matches the shipped definition,
/// ignoring the seed timestamps (`created_at` / `updated_at` differ on
/// every run since `builtins()` stamps them with `now`). Used by
/// [`ModeStore::reset_defaults`] to skip rewriting files that are
/// already at the factory version.
fn builtin_matches(existing: &VoiceMode, shipped: &VoiceMode) -> bool {
    let mut normalized = existing.clone();
    normalized.created_at = shipped.created_at;
    normalized.updated_at = shipped.updated_at;
    &normalized == shipped
}

fn read_mode_file(path: &Path) -> Result<VoiceMode> {
    let json = fs::read_to_string(path)
        .with_context(|| format!("read mode file {:?}", path))?;
    let mode: VoiceMode =
        serde_json::from_str(&json).with_context(|| format!("parse mode file {:?}", path))?;
    Ok(mode)
}

fn validate(mode: &VoiceMode) -> Result<()> {
    if mode.name.trim().is_empty() {
        return Err(anyhow!("mode name is empty"));
    }
    if mode.system_prompt.trim().is_empty() {
        return Err(anyhow!("mode system_prompt is empty"));
    }
    Ok(())
}

/// True if `id` is safe as a mode-JSON filename component:
/// `[a-z0-9-]{1,64}`, no edge dashes. Intentionally narrower than what
/// `slugify` produces so a slugifier change can't widen the fs-write
/// surface area (path-traversal protection).
pub fn is_safe_mode_id(id: &str) -> bool {
    if id.is_empty() || id.len() > 64 {
        return false;
    }
    if id.starts_with('-') || id.ends_with('-') {
        return false;
    }
    id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

pub fn slugify(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut prev_dash = false;
    for c in input.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

fn trigger_matches(
    trigger: &AutoTrigger,
    bundle_id: Option<&str>,
    window_title: Option<&str>,
) -> bool {
    let bundle_ok = match (&trigger.bundle_id_glob, bundle_id) {
        (Some(glob), Some(actual)) => glob_match(glob, actual),
        (Some(_), None) => false,
        (None, _) => true,
    };
    if !bundle_ok {
        return false;
    }
    match (&trigger.window_title_substring, window_title) {
        (Some(needle), Some(title)) => {
            title.to_lowercase().contains(&needle.to_lowercase())
        }
        (Some(_), None) => false,
        (None, _) => true,
    }
}

/// Tiny glob: trailing `*` only.
fn glob_match(pattern: &str, value: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix('*') {
        value.starts_with(prefix)
    } else {
        pattern == value
    }
}

fn title_case(input: &str) -> String {
    const SMALL: &[&str] = &[
        "a", "an", "and", "as", "at", "but", "by", "for", "if", "in", "nor", "of",
        "on", "or", "per", "so", "the", "to", "up", "via", "vs", "with", "yet",
    ];
    let mut out = String::with_capacity(input.len());
    for (line_idx, line) in input.split('\n').enumerate() {
        if line_idx > 0 {
            out.push('\n');
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        for (i, w) in words.iter().enumerate() {
            if i > 0 {
                out.push(' ');
            }
            let lower = w.to_lowercase();
            let is_small = SMALL.contains(&lower.as_str());
            let is_edge = i == 0 || i + 1 == words.len();
            if is_small && !is_edge {
                out.push_str(&lower);
            } else {
                out.push_str(&capitalize_first(&lower));
            }
        }
    }
    out
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn strip_terminal_punctuation(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for (i, line) in input.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let trimmed = line.trim_end();
        let stripped = trimmed.trim_end_matches(|c: char| ".!?,;:".contains(c));
        out.push_str(stripped);
    }
    out
}

/// Truncate at the last sentence boundary that fits, else hard-truncate
/// with a single `…` character.
pub fn truncate_to_cap(input: &str, cap: usize) -> String {
    if input.chars().count() <= cap {
        return input.to_string();
    }
    let chars: Vec<char> = input.chars().collect();
    let head: String = chars.iter().take(cap).collect();
    if let Some(idx) = head.rfind(['.', '!', '?']) {
        let cut = &head.as_bytes()[..=idx];
        if let Ok(s) = std::str::from_utf8(cut) {
            let sliced = s.trim().to_string();
            if !sliced.is_empty() {
                return sliced;
            }
        }
    }
    let safe_cap = cap.saturating_sub(1).max(1);
    let head: String = chars.iter().take(safe_cap).collect();
    format!("{}\u{2026}", head.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    /// RAII temp dir under `std::env::temp_dir()`, removed on drop.
    struct ScratchDir {
        path: PathBuf,
    }

    impl ScratchDir {
        fn new() -> Self {
            static N: AtomicU64 = AtomicU64::new(0);
            let n = N.fetch_add(1, Ordering::SeqCst);
            let pid = std::process::id();
            let path = std::env::temp_dir()
                .join(format!("vox-modes-test-{pid}-{n}"));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }
        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for ScratchDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn fresh_store() -> (Arc<ModeStore>, ScratchDir) {
        let dir = ScratchDir::new();
        let store = Arc::new(ModeStore::new());
        store.init_with_root(dir.path().to_path_buf()).unwrap();
        (store, dir)
    }

    #[test]
    fn first_launch_seeds_seven_builtins() {
        let (store, _tmp) = fresh_store();
        let modes = store.list();
        assert_eq!(modes.len(), 7);
        let ids: Vec<&str> = modes.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "general",
                "minimal",
                "email",
                "chat",
                "code-comment",
                "notes",
                "summary"
            ]
        );
        assert!(modes.iter().all(|m| m.built_in));
    }

    #[test]
    fn every_builtin_prompt_preserves_input_language() {
        // Regression guard for the German→English bug: every built-in
        // must instruct the model to write in the speaker's language
        // and not translate. Without this, the all-English prompts
        // pushed Gemma / Apple FM to "clean up" non-English dictation
        // into English.
        for mode in builtins() {
            let p = mode.system_prompt.to_lowercase();
            assert!(
                p.contains("same language"),
                "mode '{}' prompt lacks a same-language instruction",
                mode.id
            );
            assert!(
                p.contains("never translate") || p.contains("not translate"),
                "mode '{}' prompt lacks a no-translate instruction",
                mode.id
            );
            // The old hard-coded directive must be gone everywhere.
            assert!(
                !p.contains("english punctuation"),
                "mode '{}' still hard-codes English punctuation",
                mode.id
            );
        }
    }

    #[test]
    fn init_seeds_new_builtin_into_existing_dir() {
        // Simulates an upgrade: user already has the older built-ins
        // on disk from a prior launch, and we ship a new built-in.
        // The new one must appear without the user having to click
        // "Reset built-ins".
        let dir = ScratchDir::new();
        // Pre-populate the dir with everything *except* the new mode.
        for mode in builtins().into_iter().filter(|m| m.id != "minimal") {
            write_mode_file(&mode_path(dir.path(), &mode.id), &mode).unwrap();
        }
        // Sanity: the new built-in is missing before init runs.
        assert!(!mode_path(dir.path(), "minimal").exists());

        let store = Arc::new(ModeStore::new());
        store.init_with_root(dir.path().to_path_buf()).unwrap();

        // After init, the missing built-in is on disk and listed.
        assert!(mode_path(dir.path(), "minimal").exists());
        let ids: Vec<String> = store.list().into_iter().map(|m| m.id).collect();
        assert!(ids.contains(&"minimal".to_string()));
        assert_eq!(ids.len(), 7);
    }

    #[test]
    fn init_does_not_overwrite_edited_builtin_file() {
        // A built-in JSON edited on disk (e.g. by a power-user) must
        // survive re-init. Only *missing* files get written.
        let dir = ScratchDir::new();
        let mut email = builtins().into_iter().find(|m| m.id == "email").unwrap();
        email.system_prompt = "custom on-disk prompt".into();
        write_mode_file(&mode_path(dir.path(), "email"), &email).unwrap();

        let store = Arc::new(ModeStore::new());
        store.init_with_root(dir.path().to_path_buf()).unwrap();

        let reloaded = store.get("email").unwrap();
        assert_eq!(reloaded.system_prompt, "custom on-disk prompt");
    }

    #[test]
    fn test_export_import_roundtrip() {
        let (store, _tmp) = fresh_store();
        let dup = store.duplicate("email").unwrap();
        // Edit the duplicate via save.
        let mut edited = dup.clone();
        edited.name = "My Email".into();
        edited.system_prompt = "Custom email prompt.".into();
        store.save(&edited).unwrap();

        let json = store.export_json(&edited.id).unwrap();
        assert!(json.contains("My Email"));

        // Import into a fresh store should produce a valid mode with an
        // id collision generating a new id, name preserved.
        let imported = store.import_json(&json).unwrap();
        assert_eq!(imported.name, "My Email");
        assert_ne!(imported.id, edited.id);
        assert!(!imported.built_in);
    }

    #[test]
    fn test_suggest_for_slack_returns_chat() {
        let (store, _tmp) = fresh_store();
        let id = store.suggest_for(Some("com.tinyspeck.slackmacgap"), Some("general"));
        assert_eq!(id.as_deref(), Some("chat"));

        // But Slack thread → email auto-trigger comes first because email
        // is listed before chat in the builtins order.
        let id2 = store.suggest_for(Some("com.tinyspeck.slackmacgap"), Some("Slack thread"));
        assert_eq!(id2.as_deref(), Some("email"));

        // VS Code → code-comment.
        let id3 = store.suggest_for(Some("com.microsoft.VSCode"), None);
        assert_eq!(id3.as_deref(), Some("code-comment"));

        // JetBrains glob.
        let id4 = store.suggest_for(Some("com.jetbrains.intellij"), None);
        assert_eq!(id4.as_deref(), Some("code-comment"));

        // Random app → no suggestion.
        let id5 = store.suggest_for(Some("com.acme.unknown"), None);
        assert_eq!(id5, None);
    }

    #[test]
    fn test_truncate_at_sentence_boundary() {
        // Sentence boundary fits: cut at the period.
        let out = truncate_to_cap("Hello world. This is a longer second sentence.", 20);
        assert_eq!(out, "Hello world.");

        // No sentence boundary fits: hard truncate with ellipsis.
        let out2 = truncate_to_cap("a very long unbroken word stream", 12);
        assert_eq!(out2, "a very long\u{2026}");
        // Length is at most 12 chars (including the ellipsis).
        assert!(out2.chars().count() <= 12);

        // Already short enough: no-op.
        let out3 = truncate_to_cap("short", 100);
        assert_eq!(out3, "short");
    }

    #[test]
    fn save_rejects_built_in() {
        let (store, _tmp) = fresh_store();
        let mut general = store.get("general").unwrap();
        general.system_prompt = "hacked".into();
        let err = store.save(&general).unwrap_err();
        assert!(err.to_string().contains("is_builtin"));
    }

    #[test]
    fn delete_rejects_built_in() {
        let (store, _tmp) = fresh_store();
        let err = store.delete("general").unwrap_err();
        assert!(err.to_string().contains("is_builtin"));
    }

    #[test]
    fn reset_defaults_reseeds_deleted_builtin() {
        let (store, tmp) = fresh_store();
        // Manually delete a built-in's file (bypassing the delete API).
        let path = mode_path(tmp.path(), "email");
        std::fs::remove_file(&path).unwrap();
        store.invalidate();
        assert!(store.get("email").is_none());

        let n = store.reset_defaults().unwrap();
        assert_eq!(n, 1);
        assert!(store.get("email").is_some());
    }

    #[test]
    fn reset_defaults_skips_unchanged_builtins() {
        // A fresh store already has every built-in at the factory
        // version, so reset is a no-op and returns 0 — which drives the
        // UI's "All defaults are already present." message.
        let (store, _tmp) = fresh_store();
        assert_eq!(store.reset_defaults().unwrap(), 0);
    }

    #[test]
    fn reset_defaults_overwrites_stale_builtin_prompt() {
        // The German→English fix ships new built-in prompts. Existing
        // installs cache the old prompt on disk; a built-in is not
        // UI-editable, so "Reset built-ins" must force-overwrite the
        // stale file rather than leave it (the old re-seed-only
        // behaviour would never deliver the new prompt).
        let (store, tmp) = fresh_store();
        let mut stale = store.get("general").unwrap();
        stale.system_prompt = "OLD English-only prompt".into();
        write_mode_file(&mode_path(tmp.path(), "general"), &stale).unwrap();
        store.invalidate();
        assert_eq!(store.get("general").unwrap().system_prompt, "OLD English-only prompt");

        // Only the drifted file is rewritten; the rest already match.
        let n = store.reset_defaults().unwrap();
        assert_eq!(n, 1);
        let shipped = builtins().into_iter().find(|m| m.id == "general").unwrap();
        assert_eq!(store.get("general").unwrap().system_prompt, shipped.system_prompt);
    }

    #[test]
    fn duplicate_built_in_makes_editable_copy() {
        let (store, _tmp) = fresh_store();
        let copy = store.duplicate("email").unwrap();
        assert!(!copy.built_in);
        assert!(copy.id.starts_with("email-copy"));
        assert_eq!(copy.name, "Email (Copy)");
        // Now we can save edits to the copy.
        let mut edited = copy.clone();
        edited.system_prompt = "edited prompt".into();
        store.save(&edited).unwrap();
        let reloaded = store.get(&copy.id).unwrap();
        assert_eq!(reloaded.system_prompt, "edited prompt");
    }

    #[test]
    fn post_processing_lowercase_strips_punct() {
        let mode = VoiceMode {
            id: "x".into(),
            name: "X".into(),
            icon: "".into(),
            description: "".into(),
            system_prompt: "x".into(),
            dictionary: vec![],
            punctuation: PunctuationStyle::None,
            case: CaseStyle::Lower,
            max_length_chars: None,
            enable_thinking: None,
            auto_triggers: vec![],
            built_in: false,
            created_at: 0,
            updated_at: 0,
        };
        let out = mode.apply_post_processing("Hello World.\nSecond Line!");
        assert_eq!(out, "hello world\nsecond line");
    }

    #[test]
    fn post_processing_title_case_skips_articles() {
        let mode = VoiceMode {
            id: "x".into(),
            name: "X".into(),
            icon: "".into(),
            description: "".into(),
            system_prompt: "x".into(),
            dictionary: vec![],
            punctuation: PunctuationStyle::Standard,
            case: CaseStyle::Title,
            max_length_chars: None,
            enable_thinking: None,
            auto_triggers: vec![],
            built_in: false,
            created_at: 0,
            updated_at: 0,
        };
        let out = mode.apply_post_processing("the lord of the rings");
        assert_eq!(out, "The Lord of the Rings");
    }

    #[test]
    fn post_processing_truncates_chat_at_cap() {
        let chat = builtins().into_iter().find(|m| m.id == "chat").unwrap();
        let long = "Hello there. ".repeat(40);
        let out = chat.apply_post_processing(&long);
        assert!(out.chars().count() <= 280);
        assert!(out.ends_with('.') || out.ends_with('\u{2026}'));
    }

    #[test]
    fn slugify_smoke() {
        assert_eq!(slugify("Hello World"), "hello-world");
        assert_eq!(slugify("  Foo!! Bar  "), "foo-bar");
        assert_eq!(slugify(""), "");
    }

    #[test]
    fn is_safe_mode_id_accepts_slug_shape() {
        assert!(is_safe_mode_id("general"));
        assert!(is_safe_mode_id("code-comment"));
        assert!(is_safe_mode_id("mode-2"));
        assert!(is_safe_mode_id("a"));
        assert!(is_safe_mode_id("123"));
    }

    #[test]
    fn is_safe_mode_id_rejects_unsafe_shapes() {
        // empty / whitespace
        assert!(!is_safe_mode_id(""));
        assert!(!is_safe_mode_id(" "));
        // path traversal
        assert!(!is_safe_mode_id("../foo"));
        assert!(!is_safe_mode_id("foo/bar"));
        assert!(!is_safe_mode_id("/etc/passwd"));
        assert!(!is_safe_mode_id(".."));
        assert!(!is_safe_mode_id("."));
        // dotfiles / hidden
        assert!(!is_safe_mode_id(".hidden"));
        // upper case
        assert!(!is_safe_mode_id("General"));
        // unicode
        assert!(!is_safe_mode_id("café"));
        // dashes at edges
        assert!(!is_safe_mode_id("-foo"));
        assert!(!is_safe_mode_id("foo-"));
        // overly long
        let long = "a".repeat(65);
        assert!(!is_safe_mode_id(&long));
    }

    #[test]
    fn import_rejects_traversal_id() {
        let (store, tmp) = fresh_store();
        // Crafted .voxmode that tries to write outside the modes dir.
        let evil = serde_json::json!({
            "id": "../../../../tmp/vox-pwned",
            "name": "Pwned",
            "icon": "",
            "description": "",
            "system_prompt": "x",
            "punctuation": "Standard",
            "case": "Sentence",
            "created_at": 0,
            "updated_at": 0
        })
        .to_string();
        // Import succeeds, but the id is rewritten to a slug of the name.
        let imported = store.import_json(&evil).unwrap();
        assert!(is_safe_mode_id(&imported.id));
        // The expected file lives under the modes dir only.
        let expected = mode_path(tmp.path(), &imported.id);
        assert!(expected.exists());
        // The traversal target must NOT exist.
        let traversal = std::env::temp_dir().join("vox-pwned.json");
        assert!(!traversal.exists());
    }

    #[test]
    fn save_rejects_traversal_id() {
        let (store, _tmp) = fresh_store();
        let mut bad = store.duplicate("email").unwrap();
        bad.id = "../../etc/owned".into();
        let err = store.save(&bad).unwrap_err();
        assert!(err.to_string().contains("invalid mode id"));
    }

    #[test]
    fn delete_rejects_traversal_id() {
        let (store, _tmp) = fresh_store();
        let err = store.delete("../../etc/passwd").unwrap_err();
        assert!(err.to_string().contains("invalid mode id"));
    }

}
