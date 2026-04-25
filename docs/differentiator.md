# Vox — Differentiator Decision

> Decision doc. One owner: PM. Last updated: 2026-04-24.
> Constraint: local-first. Default STT = local Whisper. Default cleanup = local Gemma. Cloud is opt-in.
> Goal: pick ONE killer feature that makes a Whispr Flow user uninstall Whispr.

---

## 1. Competitive landscape

| Product | Price | Killer feature | Weakness vs. local-first Vox |
| --- | --- | --- | --- |
| Whispr Flow | $15/mo Pro, $12/seat Teams, free 2k words/wk | Context-aware cleanup per app + Command Mode + 100+ langs auto-switch | Cloud-only. SOC2/HIPAA but data leaves device. No local model. Subscription forever. |
| Superwhisper | Free / Pro $8.49/mo / Lifetime $249.99 | True offline + custom modes (Message, Email, Voice) + Cursor/Claude-Code agentic mode | Modes exist but cleanup quality lags Claude. UI-heavy. Lifetime pricing high. |
| MacWhisper | Free / Pro $79.99 lifetime | File-batch transcription with local Whisper | NOT a real-time dictation tool. No system-wide hotkey injection loop. |
| Aqua Voice | Free 1k words / Pro $8/mo / Team $12/mo | "Avalon" custom STT (97.3% claim) + on-screen context awareness | Cloud-only. Custom model is their moat — locks user in. |
| Willow Voice | Free / ~$15/mo | "AI Mode": expand short utterances into full polished messages + style-match | Cloud-only. Zero retention claim but no local fallback. |
| VoiceInk | $39.99 one-time, GPL v3 OSS | Open-source, 100% local Whisper, Power Mode (per-app config) | Indie, slower iteration. Cleanup is template-based, no LLM polish locally. |
| TypeWhisper | OSS / freemium | Bundled local Gemma via MLX on Apple Silicon — same architecture we're targeting | Niche, limited UX polish. No mode system. No voice editing. |
| Voicy | $8.49/mo or $220 lifetime | Cross-platform incl. mobile + 50+ langs | Cloud-only. Not differentiated on capability. |
| Talon Voice | Free + Patreon | Hands-free OS control via voice + noise + eye-tracking; Python scripting | Power-user only. Steep learning curve. Not a dictation product. |
| Apple Dictation | Free | On-device on Apple Silicon, system-built-in | ~1 min cap, no custom vocab on macOS, no LLM cleanup, accuracy degrades in noise. |
| Otter.ai | Free 300 min / Pro / Biz / Ent | Meeting transcription + summaries + OtterPilot bot | Different category — meeting notes, not dictation. Cloud-only, EN/FR/ES. |
| Glaido | unknown | Reads on-screen text + cursor context to format | Early stage, cloud. Same idea Whispr already ships. |
| VoiceOS | unknown | Cross-app context-aware tone | Cloud, less mature than Whispr. |

**The market reads as three buckets:**
1. **Cloud-cleanup leaders** (Whispr, Aqua, Willow) — winning on cleanup quality, losing on price + privacy.
2. **Local-first niche** (VoiceInk, Superwhisper, TypeWhisper) — winning on privacy, losing on cleanup smartness and modal/personality features.
3. **Adjacent** (MacWhisper, Otter, Talon, Apple) — different jobs.

**Open lane for Vox:** be the *only* product that combines the polish of a Whispr cleanup with the privacy/economics of local. The differentiator must be something a cloud product cannot easily clone *and* something the local-only products lack the AI smarts to ship.

---

## 2. Idea brainstorm

Scoring legend (1-5): UV = User Value, DIFF = Differentiation (truly novel), LF = Local-First Feasibility (5 = trivially local, 1 = needs cloud), EFF = Effort inverse (5 = small, 1 = huge). VALUE_INDEX = (UV × DIFF × LF × EFF) / 25 — products of the four scaled to a 1-25 range.

| # | Idea | Pitch | What's new | Feasibility (Tauri 2 / Rust / Apple Silicon / local) | UV | DIFF | LF | EFF | INDEX |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | **Voice modes / personas** | Switch among "Slack / Email / Code / Notes / Casual" — each with own cleanup prompt, dictionary, capitalization, length bias. | Whispr "tone adapts" auto; Superwhisper has 3 fixed modes. Vox makes modes a first-class persona system you can build, share, tag to apps + hotkeys. | Local Gemma takes per-mode system prompt; mode = JSON. Tauri store. ~1 wk. | 4 | 3 | 5 | 5 | 12.0 |
| 2 | **Context-aware cleanup (focused-app sniffing)** | Read frontmost app + window title; pick mode automatically. | Whispr already does it; the novelty is doing it locally + transparent (you can see why a mode was chosen + override it). | Read NSWorkspace frontmostApplication; AX trees not needed. ~3 days. | 4 | 2 | 5 | 5 | 8.0 |
| 3 | **Voice editing inline** ("change meeting to sync", "delete last sentence", "make it more formal") | After paste, hold key + speak an edit; Vox finds-and-replaces in the just-pasted text without a second paste. | MS Word + Windows have "delete that" *during* dictation. Nobody does post-hoc edits across arbitrary apps. Combined with local Gemma rewrite this is genuinely new. | Need to track last paste range (clipboard provenance + selection); Gemma rewrite is local. AX selection/replace is brittle but doable. ~2 wks. | 5 | 4 | 4 | 3 | 9.6 |
| 4 | **Whisper diff on retry** | If user re-records (Esc + hotkey within 5s), show a diff and let them pick the better take. | Nobody ships this; usability win for accents + rare words. | Compute Levenshtein diff between two transcripts; render in pill. Pure local. ~1 wk. | 3 | 4 | 5 | 4 | 9.6 |
| 5 | **Vocabulary auto-learning** | When the LLM cleanup edits a token (e.g., "Tory" → "Tauri"), Vox suggests adding it to the dictionary; one keystroke confirms. | Whispr learns silently; Vox makes it visible + reversible + local. | Diff raw STT vs cleaned text, surface mismatched tokens, queue confirmations in tray. Local. ~1 wk. | 4 | 3 | 5 | 4 | 9.6 |
| 6 | **Multi-language seamless mid-utterance** | Detect language change inside one utterance ("OK так слухай sorry I was saying"). | Whispr claims 100+ langs but switches between sessions, not within. | Local Whisper can do it; need segmentation + per-segment lang ID. Hard accuracy ceiling. ~2 wks but risky. | 3 | 4 | 4 | 2 | 4.8 |
| 7 | **"Speak as my boss" rewriter** | Second pass: rewrite my last transcript in a target voice / register. | Cute. Willow's "AI Mode" is adjacent. Risk of feeling gimmicky. | Local Gemma. Trivial. ~3 days. | 3 | 2 | 5 | 5 | 6.0 |
| 8 | **Real-time streaming pre-paste preview** | Stream Whisper tokens into a translucent overlay; Space confirms paste, Esc edits. | Whispr commits on key release. Vox could expose the "see before you ship" pattern. | Whisper streaming is engineering-heavy on local; whisper.cpp does have streaming via overlapping windows. ~3 wks. | 4 | 3 | 3 | 2 | 7.2 |
| 9 | **Snippets / voice command palette** | "vox sig" → email signature; "vox eta next sprint" → date. | Whispr Pro has Snippet Library. Common feature. Not a differentiator. | Trivial: prefix matcher in Rust before paste. ~3 days. | 3 | 1 | 5 | 5 | 3.0 |
| 10 | **Air-gap / privacy-proof badge** | Visible "0 bytes left this Mac" indicator; one-click network kill switch on dictation path. | A *story* differentiator more than a feature. Underpins everything else. Strong with local-first pivot. | macOS pf rules per-process is hard; per-call accounting is easy. ~1 wk for badge + accounting; ~3 wks for true kill switch. | 4 | 3 | 5 | 4 | 9.6 |
| 11 | **Voice-driven app actions** ("vox search Slack for X") | Routes a command to the focused app via AppleScript / shortcuts. | Talon territory; complex; 99% of dictation users won't use it. | Per-app brittle. Months of work. Out of scope. | 3 | 3 | 3 | 1 | 1.8 |
| 12 | **Smart paragraph breaks via prosody** | Use audio pauses + tone, not just text patterns. | Slight quality polish; users won't notice. | Doable with VAD timing already captured. ~3 days. | 2 | 2 | 5 | 5 | 4.0 |
| 13 | **Replay with edits (audio ring buffer)** | Last 30s of audio kept in RAM; "vox fix that" re-runs cleanup with a different model / mode. | New + practical. Pairs with #1, #3, and the local-first story. | Already have RAM buffer. Just keep last clip 60s + rerun pipeline. ~1 wk. | 4 | 4 | 5 | 4 | 12.8 |
| 14 | **Whisper-mode mic** (low-volume input) | Boost gain + fine-tune model for whisper-volume speech in libraries / open offices. | Willow advertises whispering support. Useful but narrow. | Audio gain + small dataset fine-tune. ~2 wks. | 3 | 3 | 4 | 3 | 4.32 |
| 15 | **Inline citations / speaker timestamps** | Capture timestamps + audio bookmarks in research notes mode. | Otter territory but inline at point-of-paste. | whisper.cpp returns segments; trivial. ~3 days. | 3 | 3 | 5 | 5 | 9.0 |
| 16 | **Punctuation by gesture** (clicks, breath) | Tap the spacebar twice for a paragraph break. | Cute but Apple/Whispr don't do it. Risk of accidental triggers. | Detect via low-level audio events. ~2 wks. | 2 | 4 | 5 | 3 | 4.8 |
| 17 | **Personal style profile** | Local SLM fine-tunes (LoRA) over time on YOUR style — phrasing tics, common openings. | Willow "style-match" cloud-only. We can do it locally with LoRA on Gemma. Strong privacy story. | LoRA training on-device is feasible on M-series via MLX but still hours-long; complex UX to gate. ~6 wks. | 4 | 5 | 4 | 1 | 3.2 |
| 18 | **Multi-app routing tags** | Speak once with "to slack: ... to email: ..." tags; Vox routes to multiple windows. | Genuinely novel, low utility, niche. | Window-stack management + multi-paste. ~3 wks. | 2 | 4 | 5 | 2 | 3.2 |

### Rankings by VALUE_INDEX (top 5)

1. **#13 Replay with edits (audio ring buffer + re-run pipeline)** — 12.8
2. **#1 Voice modes / personas** — 12.0
3. **#3 Voice editing inline** — 9.6 (tied)
3. **#4 Whisper diff on retry** — 9.6 (tied)
3. **#5 Vocabulary auto-learning** — 9.6 (tied)
3. **#10 Privacy-proof / air-gap badge** — 9.6 (tied)

---

## 3. The winner: **Voice Modes (Personas)**

I'm picking **#1 Voice Modes** — *not* the bare-numbers winner #13 — because the value index undervalues platform/ecosystem effects. Modes are the *substrate* that makes #2, #3, #5, #10, #13 better. Pick modes first; everything else slots in around it.

### Pitch (one line)

> A mode is a named cleanup preset (system prompt + dictionary + length + casing + auto-trigger rule). Vox ships with five great defaults, lets users build their own, share them as a single JSON file, and (optional) auto-picks the right mode based on the focused app.

### Why this one

**1. It is the cleanest answer to "why Vox over Whispr."** Whispr's "tone adapts" is a single hidden cloud heuristic — you cannot inspect, edit, or share it. Superwhisper has three fixed presets. VoiceInk has Power Mode but no AI-driven cleanup. **Nobody ships authorable, shareable, AI-powered modes that run locally.** That's the gap.

**2. Local-first is the unlock, not the constraint.** Modes only make sense if the cleanup model is fast and free. With cloud Claude every mode switch costs latency and money — so cloud products converge to one tone. With local Gemma, swapping system prompts is free, and we can ship aggressive prompts (longer, more opinionated, length-targeted) that would be cost-prohibitive in cloud. Local turns "more modes" from a liability into a feature.

**3. It is the platform every other idea sits on.**
- Mode contains the system prompt → enables #5 (per-mode dictionary learning).
- Mode contains an auto-trigger rule → enables #2 (context-aware app sniffing) without a separate feature.
- Mode contains a tone target → enables #3 (voice editing "more formal" = switch mode + rewrite).
- Mode is the unit of community sharing → drives organic distribution (a "Cursor Coding Mode" goes viral on X).
- Mode persistence means #13 (replay) can re-run with a different mode.

**4. Effort is bounded.** Modes are roughly: a JSON schema, a settings tab, a hotkey-to-mode binding, a UI widget on the pill that shows current mode, optional auto-pick rules. ~2 weeks of work; can ship before Phase 3 ends.

**5. It produces marketing artifacts.** "Build your own mode" is shareable; community-built modes (Cursor, Linear PR descriptions, kid-emails, sales-followup) are user-generated content. Whispr cannot match this without giving up control.

### Why not the runners-up

- **#13 Replay with edits** — Excellent feature, but a *power-user retry* is rarely the moment that makes someone evangelize the app. Best built *on top of* modes (replay → switch mode → re-run). Defer to Phase 3.5.
- **#3 Voice editing inline** — Genuinely novel and high-value, but technical risk is real (AX selection in arbitrary apps is fragile). Build it as a *mode action* later.
- **#4 Whisper diff on retry** — Smart, but solves a niche edge case; doesn't drive uninstalls of Whispr.
- **#5 Vocabulary auto-learning** — A polish layer on top of modes (learn into the *active mode's* dictionary). Ship after modes exist.
- **#10 Privacy-proof badge** — Important *story* element, not a feature you can demo in 10 seconds. Already implicit in local-first pivot. Ship as a marketing surface, not a roadmap line.

---

## 4. Detailed spec — Voice Modes

### 4.1 Concept

A **Mode** is a named, user-editable bundle:

```jsonc
{
  "id": "ulid",
  "name": "Cursor Coding",
  "icon": "code", // lucide name
  "color": "#7C9CFF",
  "hotkey": null, // optional Cmd+Opt+1..9
  "auto_triggers": [
    { "kind": "frontmost_bundle_id", "match": "com.todesktop.230313mzl4w4u92" }, // Cursor
    { "kind": "frontmost_bundle_id", "match": "com.microsoft.VSCode" }
  ],
  "system_prompt": "You are cleaning dictated text that will be inserted into a code editor or PR description. Keep technical terms exactly as the speaker said them. Prefer markdown bullets for steps. Keep sentences short. NEVER add words that weren't said. Output only the cleaned text.",
  "dictionary": ["Tauri", "Solid", "MLX", "Gemma", "Vox", "Whisper"],
  "length_bias": "concise", // "verbose" | "concise" | "preserve"
  "casing": "smart", // "smart" | "lower" | "title"
  "punctuation": "full", // "full" | "minimal" | "none"
  "transform": null, // future: { "kind": "to_bullet_list" } etc.
  "model_override": null, // future: pin a specific local model
  "created_at": 0,
  "updated_at": 0,
  "source": "user" // "default" | "user" | "imported"
}
```

### 4.2 Defaults shipped with Vox

Five built-in modes, locked-but-clonable:

1. **Casual chat** — short messages, informal, contractions OK, light punctuation. Auto-trigger: Slack, Discord, Messages, Telegram.
2. **Email & docs** — full sentences, formal register, paragraph breaks, no contractions. Auto-trigger: Mail, Gmail web, Notion, Google Docs.
3. **Code & PR** — preserve technical terms, allow markdown bullets, ASCII only. Auto-trigger: Cursor, VS Code, Xcode, Terminal, iTerm2.
4. **Meeting & notes** — bullet-able, timestamps allowed, preserve speaker self-corrections. Auto-trigger: Obsidian, Bear, Apple Notes, Linear (comment field).
5. **Raw transcript** — no LLM cleanup, just punctuation. Auto-trigger: none (manual). For users who don't trust polish.

A built-in mode can be **forked** (clone-to-user). The original cannot be edited or deleted.

### 4.3 Mode resolution at hotkey-press

Pseudocode (lives in `src-tauri/src/session.rs`):

```
1. on_hotkey_press():
2.   active_mode = settings.pinned_mode_id  // user's manual override
3.   if active_mode is None:
4.     for mode in modes_sorted_by_priority():
5.       if any(rule.matches(frontmost_app) for rule in mode.auto_triggers):
6.         active_mode = mode.id
7.         break
8.   if active_mode is None:
9.     active_mode = settings.default_mode_id  // user-set fallback, else "Casual chat"
10.  pill.show(mode=active_mode)  // display mode chip
```

`frontmost_app` is read once at hotkey-press via `NSWorkspace.shared.frontmostApplication.bundleIdentifier`. We do NOT poll continuously.

### 4.4 User flow — using a mode

**A. Default path (auto-pick).**
1. User holds Right-Option in Slack. Pill shows `● Cursor Coding`? No — `● Casual chat` because Slack matched.
2. Speaks. Pill shows `Listening… · Casual chat`.
3. Releases. Pipeline runs. Pasted text is short-form, contraction-heavy, no greeting line.

**B. Pinning a mode (manual override).**
1. In tray popover, user clicks the active-mode chip.
2. A 5-row dropdown appears with the modes; clicking pins for the next utterance.
3. Pill shows `📌 Email & docs` so the user knows it's pinned.
4. After paste, pin clears (default) or persists across utterances if `Settings → Modes → "Pinned mode persists"` is on.

**C. Per-mode hotkey.**
- A mode can be assigned a numeric hotkey (`Cmd+Opt+1` … `Cmd+Opt+9`). Holding that hotkey instead of Right-Option both starts recording AND pins the mode for that utterance.
- Existing global push-to-talk continues to use the resolution rules above.

### 4.5 User flow — building a mode

1. Settings → **Modes** tab (new tab in sidebar, between Hotkeys and Dictionary).
2. List view: built-ins (locked icon) on top, user modes below. Buttons: `+ New mode`, `Import…`, `Sort`.
3. Click a row → editor pane on the right.
4. Editor pane fields:
   - **Name** (text)
   - **Icon** (lucide picker, 24 options)
   - **Color** (6 swatches matching design tokens)
   - **Hotkey** (capture widget, Cmd+Opt+1..9; conflicts highlighted)
   - **Auto-trigger** — list of rules, `+ Add rule`. Rule kinds:
     - `Frontmost app` (with "Pick app…" picker that lists `/Applications` bundle IDs).
     - `Window title contains` (text).
     - `URL host equals` (only for Safari/Chrome; reads via AppleScript optional).
   - **System prompt** (textarea, 8 rows, monospaced, char counter, max 4000 chars). Below it: `Reset to default for this style` link.
   - **Dictionary** (chip-list, max 200 terms; shared chip pool with global dictionary — global dict is appended to every mode's local dict at runtime).
   - **Length bias** (segmented: Concise / Preserve / Verbose).
   - **Casing** (segmented: Smart / Lower / Title).
   - **Punctuation** (segmented: Full / Minimal / None).
5. Footer of the editor: `Test in sandbox` button → opens an in-window text input + "type text" + "Run" → shows cleanup output side-by-side with raw input. Lets user iterate without dictating.
6. Header of the editor: `…` menu with `Duplicate`, `Export…`, `Delete` (disabled for built-ins; for forks, allowed).

### 4.6 Sharing modes

- **Export.** `Modes → … → Export` writes a `.voxmode` file (JSON, the schema in 4.1). Version field included.
- **Import.** Drag-and-drop a `.voxmode` onto Vox window OR `Modes → Import…`. Validation: schema check, name de-dupe, bundle-id sanity. Imported modes have `source: "imported"` and a small "from {filename}" caption.
- **Deep link.** `vox://import-mode?b64=...` URL handler — for sharing on X or in docs. On click, we show a preview modal with the prompt + dictionary BEFORE installing.
- **NO cloud sync, NO mode marketplace in v1.** That's a v2 story tied to accounts. We're explicitly avoiding that scope.

### 4.7 Pill UI changes

The pill (`280×56`) gets a small **mode chip** on the right side:

```
[ mic ]   ||| || |||| ||  Listening…    ● Casual chat
20×20      waveform 144×24                12px chip
```

Mode chip:
- 22px tall pill, `bg-mode-color/16`, `text-mode-color`, dot left, name right (truncate to 14 chars).
- Hidden in error / paste-fallback states (clutter).
- When pinned (manual override), prepend a `📌` glyph (use lucide `pin` instead of emoji).

When a mode auto-triggers, the chip flashes once for 200ms (`flash-done` style but with the mode's color) so the user notices the pick.

### 4.8 Tray popover changes

Below the on/off toggle, add a one-line **Active mode** row:

```
Active mode    [● Casual chat ▾ ]    Edit ▸
```

- The chip is a dropdown. Click → 5–10-row mode picker. Clicking a mode pins it for next utterance.
- `Edit ▸` opens Settings → Modes → that mode.

The recents list (existing) gets a tiny mode-color dot on the left of each row to show which mode produced that transcript.

### 4.9 Settings window changes

New tab order: General · Hotkeys · **Modes** · API Keys · Dictionary · Privacy · About.

The Dictionary tab stays — it's the *global* dictionary appended to every mode. We add a one-liner: "These terms apply across all modes. For mode-specific terms, edit the mode."

The Hotkeys tab gets a new section "Per-mode hotkeys" listing the 9 numeric slots with mode bindings.

### 4.10 Storage

- **Modes file:** `~/Library/Application Support/com.vox.app/modes.json`. Single JSON array. Atomic write. Versioned (`schema_version: 1`).
- **Built-in modes** are NOT persisted in this file — they live in code. Only their per-user *forks* are persisted. This keeps upgrades safe.
- **Default mode + pinned mode** live in `vox-settings.json` (existing tauri-plugin-store): `{ default_mode_id, pinned_mode_id, pin_persists }`.
- **Audit log** of mode resolution (last 20 utterances, in-memory only): `(timestamp, app_bundle_id, picked_mode_id, was_pinned)`. Surface in `Settings → Modes → "Why this mode?"` debug drawer.

### 4.11 IPC commands (Tauri)

New `#[tauri::command]`s in `src-tauri/src/commands.rs`:

```rust
list_modes() -> Vec<Mode>                          // built-ins + user
get_mode(id: String) -> Option<Mode>
create_mode(draft: ModeDraft) -> Mode              // returns saved mode w/ id
update_mode(id: String, patch: ModePatch) -> Mode
delete_mode(id: String) -> ()
duplicate_mode(id: String) -> Mode
pin_mode(id: Option<String>) -> ()                 // None = unpin
set_default_mode(id: String) -> ()
test_mode(id: String, sample_text: String) -> String   // runs local Gemma w/ that mode's prompt
import_mode(json: String) -> Mode
export_mode(id: String) -> String                  // JSON
resolve_mode_for_active_app() -> Mode              // for tray popover
recent_resolutions() -> Vec<ResolutionEvent>
```

`MainWindow → Pill` event payload now includes `{ mode_id, mode_name, mode_color, was_pinned, was_auto_picked }`.

### 4.12 Local Gemma integration

- Pipeline change in `session.rs`:
  1. Resolve mode (4.3).
  2. Build system prompt = `mode.system_prompt + "\n\nKnown terms: " + (global_dict ∪ mode.dict).join(", ")`.
  3. Apply `length_bias` as a soft instruction line.
  4. Stream Gemma output via MLX-rs or llama.cpp Metal backend (whichever wins the perf bench).
- **Prompt caching note:** Local Gemma KV-cache the *prefix* (system prompt) per mode. First use of a mode warms cache; subsequent uses reuse. Implementation: mode_id → cached KV state on disk in `~/Library/Caches/com.vox.app/gemma-kv/`.
- **Cloud fallback** uses the same system prompt verbatim, sent to Claude Haiku; switching cloud↔local does not change mode behavior.

### 4.13 First-run / migration

- Onboarding step 5 (existing "Try it now") stays unchanged.
- After first successful dictation, show a **one-time toast** at the bottom of the pill: `Tip: Vox just used "Casual chat" mode. Open Settings → Modes to customize.`
- Existing users (Phase 2 builds) on upgrade: Vox seeds `default_mode_id = "casual_chat"` and migrates the existing global dictionary into a *shared* pool (no behavior change).

### 4.14 What's stored, where, when

| Data | Where | When written | When read | TTL |
| --- | --- | --- | --- | --- |
| User-created modes | `modes.json` (Application Support) | On `create_mode`/`update_mode`/`delete_mode`/`import_mode` | App start, Settings open, hotkey press | Until user deletes |
| Default + pinned mode IDs | `vox-settings.json` | On user toggle | Hotkey press, popover render | Until user changes |
| Per-mode KV cache | `~/Library/Caches/com.vox.app/gemma-kv/{mode_id}.kv` | After first cleanup with that mode | Subsequent cleanups | Invalidate on prompt edit (hash compare) |
| Resolution audit log | RAM only | Each utterance | Debug drawer | 20-entry ring, cleared on quit |
| Imported `.voxmode` file | NOT retained — content is parsed and inlined into `modes.json` | On import | Never re-read | n/a |

Audio: unchanged from PRD 6.3 — never on disk.

### 4.15 All UI surfaces touched (final list)

- Settings sidebar: new **Modes** tab.
- Settings: existing Hotkeys tab — new "Per-mode hotkeys" section.
- Settings: existing Dictionary tab — clarification copy.
- Pill: new mode chip; flash-on-auto-pick.
- Tray popover: active-mode row with dropdown.
- Tray popover: recent-row mode-color dot.
- Onboarding: one-time post-first-paste tip toast.
- New: `vox://import-mode?...` URL handler with confirmation modal.
- New: `.voxmode` file association in `tauri.conf.json` (`fileAssociations`).

### 4.16 Out of scope for v1 (cut explicitly)

- Cloud sync of modes between devices.
- Public mode marketplace / browse-and-install gallery.
- Mode A/B testing or analytics on which prompts work better.
- Mode-level model overrides (pick which Gemma quant to use). Future.
- Mode "actions" beyond cleanup (e.g., post-paste rewrite, format-as-bullets transform). Hooks scaffolded; not exposed.
- Voice command to switch mode ("vox, switch to email mode") — power-user, deferred.
- Per-mode latency targets / SLOs.

---

## 5. Success metrics & risks

### 5.1 Success metrics (measure 30 days post-launch)

**North-star: % of dictations that use a non-default mode.**
- Target: **≥ 35%** of utterances run with an auto-picked or user-pinned mode (i.e., not the global default).
- Measured locally; if user opts in, anonymized counters reported as `(mode_id_hash, count)`.

**Supporting metrics:**
1. **Mode customization rate.** ≥ 25% of weekly-active users have created or forked at least one mode within 14 days of install.
2. **Auto-pick acceptance.** Of utterances where a mode was auto-picked, ≤ 8% are followed within 60s by a manual pin to a different mode (a proxy for "the auto-pick was wrong").
3. **Sharing surface usage.** ≥ 5% of WAU import a `.voxmode` from elsewhere, OR ≥ 1% export one. (Either signal proves modes spread.)
4. **Retention proxy.** D30 retention for users who have ≥ 2 modes at end of week 1 is ≥ 1.4× the retention of users with 0 custom modes.

### 5.2 Risks

| Risk | Likelihood | Impact | Mitigation |
| --- | --- | --- | --- |
| Local Gemma is too slow per-utterance to make swapping prompts feel free | Medium | High | Prompt-cache KV per mode; bench Gemma 2B/3B Q4 on M2/M3; if first-token > 600ms p95, ship cloud-first as default with local opt-in |
| Mode auto-pick feels magical-but-wrong, users distrust it | Medium | Medium | Always show the chip; expose "Why this mode?" debug; first-30-days default to *not* auto-picking, just *suggesting* on the chip |
| Built-in defaults are mid; users don't bother customizing | Medium | High | Spend a real week prompt-engineering the 5 defaults with 50-utterance test sets per mode; ship a "Compare modes" sandbox UI |
| Modes balloon in scope (people want triggers on URL, on selected text, on time of day) | High | Medium | v1 ships only the 3 trigger kinds in 4.5; everything else is deferred & gated by user signal |
| Conflict between two auto-trigger rules (Slack + Email) | Low | Low | Sort modes by user-defined priority; highest-priority match wins; ties go to most-recently-edited |
| `.voxmode` files become an attack vector (malicious system_prompt steals data) | Low | High | Modes cannot trigger network calls; system_prompt is text-only; import shows a preview AND warns if prompt contains URLs / suspicious patterns |
| Localization: built-in mode names + auto-trigger heuristics are EN-only | Medium | Medium | Names are user-editable post-fork; ship i18n in a follow-up |

### 5.3 Test plan (3 bullets)

- **Unit / integration.** Mode CRUD + import/export round-trip; auto-trigger resolution returns the right mode for ≥ 30 known bundle-IDs; KV cache is invalidated on prompt edit (hash mismatch); pin overrides auto-pick; per-mode hotkey starts capture AND sets pin atomically.
- **End-to-end.** Manual matrix on the existing 13-app target list (PRD 3.5): for each app, dictate 3 sentences, verify the auto-picked mode produces output that matches the mode's stated style (presence/absence of contractions, length, bullet usage). Pass = ≥ 90% subjective match by 3 dogfooders.
- **Performance.** With local Gemma 2B Q4 on M2: median time-to-first-token after warm-up ≤ 350ms, p95 ≤ 700ms, across 100 utterances cycling through all 5 modes. Prompt-cache hit rate ≥ 85% after warm-up. If we miss, fall back to cloud as default and ship modes anyway — the *abstraction* still wins.

---

End of decision doc.
