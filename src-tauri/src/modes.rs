//! Voice Modes — Vox's differentiator feature.
//!
//! A `VoiceMode` bundles a system prompt, mode-specific dictionary, and
//! formatting rules (punctuation, case, length cap). The 5 built-in modes
//! ship with every install; users can fork them, create new ones, and
//! share via `.voxmode` JSON files.
//!
//! Persistence: each mode is one JSON file under
//! `<app_data_dir>/modes/<id>.json`. Built-in modes are written to the
//! directory on first launch only when the directory is empty — once a
//! built-in is on disk the user can fork-and-edit it freely; we never
//! overwrite an existing file. "Reset to defaults" re-imports any
//! built-in that has been deleted.
//!
//! No telemetry. No mode-usage counters. Per the CEO decision, success
//! metrics are user-visible logs only.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime};

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

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
    /// Matches frontmost app's bundle id (e.g. "com.tinyspeck.slackmacgap").
    /// Supports a trailing `*` wildcard (e.g. "com.jetbrains.*").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_id_glob: Option<String>,
    /// Matches the focused window's title (case-insensitive substring).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_title_substring: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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
    #[serde(default)]
    pub auto_triggers: Vec<AutoTrigger>,
    #[serde(default)]
    pub built_in: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

impl VoiceMode {
    /// Apply this mode's post-processing rules on the cleaned text.
    pub fn apply_post_processing(&self, text: &str) -> String {
        let mut out = text.to_string();

        // Case rule. Sentence and AsSpoken are no-ops — the LLM already
        // handles them. Title and Lower are post-processed deterministically.
        match self.case {
            CaseStyle::Title => out = title_case(&out),
            CaseStyle::Lower => out = out.to_lowercase(),
            CaseStyle::Sentence | CaseStyle::AsSpoken => {}
        }

        // Strip trailing punctuation per-line if mode says None.
        if matches!(self.punctuation, PunctuationStyle::None) {
            out = strip_terminal_punctuation(&out);
        }

        // Length cap (last so all other rules already applied).
        if let Some(cap) = self.max_length_chars {
            out = truncate_to_cap(&out, cap);
        }

        out
    }
}

// ---------------------------------------------------------------------------
// Built-in catalog (5 defaults)
// ---------------------------------------------------------------------------

/// Returns the 5 built-in voice modes. They are cloned from this
/// function on first launch to populate the modes directory; they are
/// also used by `reset_defaults` to re-seed any deleted built-in.
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
            auto_triggers: vec![],
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
    ]
}

const GENERAL_PROMPT: &str = "You clean up voice-dictation transcripts. Return plain text only — no preface, no quotes, no reasoning, no explanations.

Rules:
1. Remove filler (\"um\", \"uh\", \"like\" as filler, \"you know\", \"sort of\", \"kind of\", \"basically\", \"I mean\" as filler). Keep them when meaningful.
2. Apply standard English punctuation and capitalization. For longer passages, group related ideas into paragraphs separated by blank lines.
3. Fix mid-sentence self-corrections.
   - \"go to the — I mean, the store\" → \"go to the store\"
   - \"we ship Friday, no Thursday\" → \"we ship Thursday\"
4. Drop content the speaker explicitly retracts — including everything before the retraction marker.
   - \"X. Actually no, disregard that. Y.\" → \"Y.\"
   - \"Send it to John. Wait, scratch that, send it to Mary.\" → \"Send it to Mary.\"
   - \"Forget what I just said. Z.\" → \"Z.\"
5. When the speaker enumerates items or steps, format them as a numbered list. Only do this when the enumeration is explicit (\"first… second… third…\", \"the three things are…\", \"step one… step two…\"). Leave flowing narrative as prose.
6. Preserve the speaker's register and word choice. Do not formalize casual speech.
7. Never add facts, names, links, or content the speaker did not say.

Examples:
- \"uh so like the meeting is um at three pm\" → \"The meeting is at 3 PM.\"
- \"I'm not using the model, no, disregard that, I am using the model\" → \"I am using the model.\"
- \"first we ship the build then we run tests and finally we deploy\" →
   \"1. Ship the build.\n2. Run tests.\n3. Deploy.\"";

const EMAIL_PROMPT: &str = "You are formatting a dictated message as the BODY of a professional email. Return plain text only.

Rules:
- Use complete, well-punctuated sentences. Apply paragraph breaks where the speaker pauses or shifts topic.
- Maintain a polite, professional tone — but do NOT add a salutation (\"Hi\", \"Dear\") or sign-off (\"Best\", \"Thanks\") that the user did not say.
- Remove fillers (um, uh, like, you know).
- Spell out abbreviations only if the speaker said them spelled out.
- Never invent recipients, dates, or specifics.
- Output only the email body text.";

const CHAT_PROMPT: &str = "You are formatting a dictated message for a casual chat (Slack, Discord, iMessage). Return plain text only.

Rules:
- Keep it short and conversational. Trim filler ruthlessly.
- Preserve the speaker's casual register — contractions are fine, sentence fragments are fine.
- Use minimal punctuation. One terminal mark per sentence; no Oxford-comma fussiness.
- Do not capitalize words for emphasis the speaker did not pronounce as proper nouns.
- Preserve emoji the speaker said by name (e.g. \"smiley face\" → 🙂) only when unambiguous.
- Never add greetings, sign-offs, or closing remarks.
- Output only the chat text.";

const CODE_COMMENT_PROMPT: &str = "You are rewriting dictation as a technical code comment or PR comment. Return plain text only.

Rules:
- Be concise, present-tense, third-person (\"adjusts the cache\", not \"I adjusted the cache\").
- Preserve identifier names verbatim — CamelCase, snake_case, kebab-case, dotted paths, file names.
- Preserve technical terms exactly as said (HTTP, API, GPU, gRPC, KV-cache).
- Do not add a leading // or # — output only the comment text.
- Do not add markdown unless the speaker dictated bullet structure.
- Never invent function names, variables, or behavior the user did not state.
- Output only the comment text.";

const NOTES_PROMPT: &str = "You are formatting dictation as structured notes (Apple Notes, Notion, Obsidian). Return plain text only.

Rules:
- Use full sentences with complete punctuation.
- If the speaker signals a list (\"first…, second…, third…\" or \"one…, two…, three…\"), format as a bullet list (one item per line, prefixed with \"- \").
- Otherwise, use newline-separated paragraphs at natural topic breaks.
- Do not add headers, titles, or section markers the speaker did not request.
- Never invent items, dates, or specifics.
- Output only the note body.";

// ---------------------------------------------------------------------------
// Persistence: directory layout, in-memory cache
// ---------------------------------------------------------------------------

/// Process-wide cache of modes loaded from disk. Rebuilt lazily on
/// first access; refreshed by every mutation.
#[derive(Default)]
pub struct ModeStore {
    inner: RwLock<Option<HashMap<String, VoiceMode>>>,
    root: RwLock<Option<PathBuf>>,
}

impl ModeStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind the storage root for this app. Called once at startup.
    pub fn init_with_root(self: &Arc<Self>, root: PathBuf) -> Result<()> {
        fs::create_dir_all(&root).context("create modes dir")?;
        *self.root.write() = Some(root.clone());
        // Seed built-ins if the directory is empty (first launch).
        if dir_is_empty(&root)? {
            for mode in builtins() {
                let path = mode_path(&root, &mode.id);
                if !path.exists() {
                    write_mode_file(&path, &mode)?;
                }
            }
        }
        // Refresh stale built-in prompts. Existing installs already
        // wrote per-mode JSON files at first launch with whatever
        // the prompt constants were at the time. When we improve a
        // built-in prompt (e.g. teaching General to handle
        // retractions and structuring), users on an upgraded build
        // would otherwise stay on the old prompt forever because
        // `reset_defaults` only fills in MISSING files.
        //
        // The migration is conservative: it only overwrites when
        // the on-disk `system_prompt` field exactly matches a known
        // legacy version. If the user has customised the prompt at
        // all, the on-disk prompt won't match the legacy fingerprint
        // and we leave it alone. Other fields (dictionary,
        // auto_triggers, punctuation/case overrides) are always
        // preserved — only the prompt text changes.
        let _ = migrate_legacy_builtin_prompts(&root);
        // Force a refresh on next read.
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

    /// Sorted list — built-ins first (in canonical order), then user
    /// modes alphabetically by name.
    pub fn list(&self) -> Vec<VoiceMode> {
        let map = match self.cached() {
            Ok(m) => m,
            Err(e) => {
                log::warn!("ModeStore::list failed: {e}");
                return Vec::new();
            }
        };
        let mut builtins_in_order = Vec::new();
        for b in builtins() {
            if let Some(m) = map.get(&b.id) {
                builtins_in_order.push(m.clone());
            }
        }
        let mut customs: Vec<VoiceMode> = map
            .values()
            .filter(|m| !m.built_in)
            .cloned()
            .collect();
        customs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        builtins_in_order.extend(customs);
        builtins_in_order
    }

    pub fn get(&self, id: &str) -> Option<VoiceMode> {
        self.cached().ok().and_then(|m| m.get(id).cloned())
    }

    /// Save a mode. Built-ins reject (the UI surfaces "Duplicate to edit"
    /// for built-ins, and a fork is a non-built-in clone).
    pub fn save(&self, mode: &VoiceMode) -> Result<()> {
        // Defense in depth: even though the webview-only commands gate
        // this path, refuse any id that doesn't match our slug shape so
        // a future plugin / IPC caller cannot path-traverse through the
        // modes directory.
        if !is_safe_mode_id(&mode.id) {
            return Err(anyhow!("invalid mode id"));
        }
        if let Some(existing) = self.get(&mode.id) {
            if existing.built_in {
                return Err(anyhow!("is_builtin"));
            }
        }
        let mut to_save = mode.clone();
        to_save.built_in = false; // Force false for any user-saved mode.
        to_save.updated_at = Utc::now().timestamp();
        let root = self.root_or_err()?;
        let path = mode_path(&root, &to_save.id);
        write_mode_file(&path, &to_save)?;
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
        // unique_id_from is built from slugify, which already produces a
        // safe id, but assert one more time so a future change to the
        // slugifier can't silently regress this.
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

    /// Import a mode from JSON. Generates a new id if there's a collision
    /// or if the id is missing/invalid. Always inserted as `built_in: false`.
    pub fn import_json(&self, json: &str) -> Result<VoiceMode> {
        let mut mode: VoiceMode =
            serde_json::from_str(json).context("invalid .voxmode JSON")?;
        validate(&mode)?;
        mode.built_in = false;
        // Re-slug any id that doesn't fit our safe-id shape, then dedup.
        // This both prevents path traversal (`"../foo"` -> rejected and
        // regenerated from the name) and mirrors the prior behavior of
        // generating a fresh id when the source file's id is malformed.
        if !is_safe_mode_id(&mode.id) || self.get(&mode.id).is_some() {
            mode.id = self.unique_id_from(&slugify(&mode.name));
        }
        // unique_id_from always produces a safe id; assert again so a
        // future regression is caught early rather than at the fs::write.
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

    /// Re-seed any built-in that has been deleted; never overwrite an
    /// existing file.
    pub fn reset_defaults(&self) -> Result<usize> {
        let root = self.root_or_err()?;
        let mut count = 0;
        for mode in builtins() {
            let path = mode_path(&root, &mode.id);
            if !path.exists() {
                write_mode_file(&path, &mode)?;
                count += 1;
            }
        }
        self.invalidate();
        Ok(count)
    }

    /// Heuristic auto-trigger evaluation. Returns the id of the FIRST
    /// matching mode (in display order), or `None`.
    ///
    /// Per the CEO decision this never auto-switches — it's only used
    /// by the UI to render a "Suggested mode" affordance.
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

// ---------------------------------------------------------------------------
// Public façade — Tauri-aware helpers used by commands and session.
// ---------------------------------------------------------------------------

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

/// Cycle to the next mode in display order. Returns the now-active mode.
pub fn cycle_active<R: Runtime>(
    app: &AppHandle<R>,
    current_id: &str,
) -> Option<VoiceMode> {
    let list = list_modes(app);
    if list.is_empty() {
        return None;
    }
    let idx = list.iter().position(|m| m.id == current_id).unwrap_or(0);
    let next = list[(idx + 1) % list.len()].clone();
    Some(next)
}

fn store<R: Runtime>(app: &AppHandle<R>) -> Arc<ModeStore> {
    app.state::<Arc<ModeStore>>().inner().clone()
}

/// Compute the directory where mode JSONs live.
pub fn modes_dir<R: Runtime>(app: &AppHandle<R>) -> PathBuf {
    if let Ok(p) = app.path().app_data_dir() {
        return p.join("modes");
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("app.vox.vox/modes")
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn dir_is_empty(dir: &Path) -> Result<bool> {
    if !dir.exists() {
        return Ok(true);
    }
    Ok(fs::read_dir(dir)?.next().is_none())
}

/// Past `GENERAL_PROMPT` revisions. When an existing install has any
/// of these *byte-for-byte* on disk, it means the user never edited
/// the General mode and is just running an older default — so we
/// safely refresh it to the current `GENERAL_PROMPT`.
///
/// Append a new entry every time we revise `GENERAL_PROMPT`.
const LEGACY_GENERAL_PROMPTS: &[&str] = &[
    // Pre-2026-04-28 default: filler removal + mid-sentence
    // self-corrections only. Did not handle retractions
    // (\"X. Disregard that. Y.\") or list-style structuring.
    "You clean up voice-dictation transcripts. Return plain text only — no preface, no quotes, no explanations.\n\nRules:\n- Remove filler words (um, uh, like as filler, you know, sort of, kind of as filler).\n- Apply standard punctuation and capitalization (commas, periods, question marks).\n- Fix obvious mid-sentence self-corrections (\"go to the — I mean, the store\" → \"go to the store\").\n- Preserve the speaker's register and word choice. Do not formalize casual speech.\n- Never add facts, names, links, or content the user did not say.\n- Output only the cleaned text.",
];

/// Walk every built-in's stored JSON; when its `system_prompt`
/// matches a known legacy fingerprint, replace just the prompt
/// field with the current default. Other fields (dictionary,
/// auto-triggers, punctuation/case overrides) are preserved.
fn migrate_legacy_builtin_prompts(root: &Path) -> Result<()> {
    for current in builtins() {
        let path = mode_path(root, &current.id);
        if !path.exists() {
            continue;
        }
        let mut on_disk = match read_mode_file(&path) {
            Ok(m) => m,
            Err(e) => {
                log::warn!(
                    "modes: skipping legacy-prompt migration for {} — read failed: {e}",
                    current.id
                );
                continue;
            }
        };

        let is_legacy = match current.id.as_str() {
            "general" => LEGACY_GENERAL_PROMPTS
                .iter()
                .any(|p| *p == on_disk.system_prompt),
            // Other built-ins have no past revisions worth
            // migrating yet. Add similar arms as they evolve.
            _ => false,
        };

        if !is_legacy {
            continue;
        }

        log::info!(
            "modes: refreshing built-in '{}' system_prompt from a legacy revision to current default",
            current.id
        );
        on_disk.system_prompt = current.system_prompt.clone();
        on_disk.updated_at = current.updated_at;
        if let Err(e) = write_mode_file(&path, &on_disk) {
            log::warn!(
                "modes: legacy-prompt migration failed to write {}: {e}",
                current.id
            );
        }
    }
    Ok(())
}

fn mode_path(root: &Path, id: &str) -> PathBuf {
    root.join(format!("{id}.json"))
}

fn write_mode_file(path: &Path, mode: &VoiceMode) -> Result<()> {
    let json = serde_json::to_string_pretty(mode)?;
    fs::write(path, json).with_context(|| format!("write mode file {:?}", path))?;
    Ok(())
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

/// True if the given id is safe to use as the *filename component* of a
/// mode JSON file. Allowed shape: `[a-z0-9-]{1,64}`, no leading or
/// trailing dash. This is intentionally narrower than what `slugify`
/// produces so a slugifier change can't widen the filesystem-write
/// surface area.
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

/// Tiny glob: `*` only at end (e.g. `com.jetbrains.*`). Bundle ids
/// don't have spaces or fancy chars; this is enough for v1.
fn glob_match(pattern: &str, value: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix('*') {
        value.starts_with(prefix)
    } else {
        pattern == value
    }
}

// ---------------------------------------------------------------------------
// Post-processing primitives
// ---------------------------------------------------------------------------

fn title_case(input: &str) -> String {
    // Lower-case "small words" except when they're the first or last
    // word of the line.
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
            let is_first = i == 0;
            let is_last = i + 1 == words.len();
            if is_small && !is_first && !is_last {
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

/// Truncate at the last sentence boundary that fits, else hard truncate
/// with an ellipsis (one "…" character — never three dots).
pub fn truncate_to_cap(input: &str, cap: usize) -> String {
    if input.chars().count() <= cap {
        return input.to_string();
    }
    let chars: Vec<char> = input.chars().collect();
    let head: String = chars.iter().take(cap).collect();
    // Look for the last `.`, `!`, `?` in the head — cap at that boundary.
    let last_term = head.rfind(['.', '!', '?']);
    if let Some(idx) = last_term {
        // idx is a byte index into `head` (which is ASCII-clean for these
        // sentinel chars). Include the terminator.
        let cut = &head.as_bytes()[..=idx];
        if let Ok(s) = std::str::from_utf8(cut) {
            let sliced = s.trim().to_string();
            if !sliced.is_empty() {
                return sliced;
            }
        }
    }
    // Otherwise, hard truncate keeping room for the ellipsis.
    let safe_cap = cap.saturating_sub(1).max(1);
    let head: String = chars.iter().take(safe_cap).collect();
    format!("{}\u{2026}", head.trim_end())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    /// RAII temp dir with no extra dep — created under
    /// `std::env::temp_dir()` and removed on drop.
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
    fn first_launch_seeds_five_builtins() {
        let (store, _tmp) = fresh_store();
        let modes = store.list();
        assert_eq!(modes.len(), 5);
        let ids: Vec<&str> = modes.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["general", "email", "chat", "code-comment", "notes"]
        );
        assert!(modes.iter().all(|m| m.built_in));
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
