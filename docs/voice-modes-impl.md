# Vox — Voice Modes Implementation Notes

> Status: landed. Sr Eng B handoff for QA.
> Date: 2026-04-24.
> Owner: Voice Modes feature track.

This file is the reality-vs-spec diff for the Voice Modes pivot
(differentiator §3-§4). It catalogues files touched, the mode JSON
schema, settings keys added, the IPC command surface (for QA), and a
quick-start guide for users who want to author their own modes.

## 1. Scope

Per the CEO decisions in the engineering brief:

1. **No auto-switch.** The frontmost-app heuristic is implemented and
   surfaced as a *suggestion* on the listening pill, never as a silent
   mode flip.
2. **No URL deep-link** in v1. Sharing is via `.voxmode` JSON files
   (drag-drop + file picker).
3. **Zero telemetry.** No mode-usage counters anywhere. The
   "success metrics" from the differentiator doc are user-visible logs
   only — Vox keeps no per-mode statistics, anonymized or otherwise.
4. **Always-on-by-default.** Every install gets the 5 default modes
   pre-installed; the General mode is selected by default.

## 2. Files touched

### Created
- `src-tauri/src/modes.rs` — `VoiceMode` data model, on-disk store
  (`<app_data_dir>/modes/<id>.json`), built-in catalog, post-processing
  primitives (case, terminal-punct strip, length truncate),
  auto-trigger evaluation. 12 unit tests live here.
- `src-tauri/src/frontmost.rs` — `NSWorkspace.frontmostApplication`
  bundle id + AX-API focused-window-title probe. Always fails open
  (returns `None`) on permission denial or non-macOS.
- `docs/voice-modes-impl.md` — this file.

### Modified
- `src-tauri/src/lib.rs` — registered the `modes` and `frontmost`
  modules, the `ModeStore` app state (seeded on startup if the modes
  directory is empty), and the 10 new IPC commands.
- `src-tauri/src/settings.rs` — added the three new keys (see §4),
  default `active_mode_id = "general"`, default
  `mode_cycle_hotkey = "Alt+M"`, default `auto_suggest_modes = true`,
  plus a `set_active_mode_id` helper used by the cycle command and the
  hotkey handler.
- `src-tauri/src/session.rs` — resolves the active mode at the start of
  every dictation, passes its `system_prompt` and merged dictionary
  (global ∪ mode-specific, dedup'd) into `LlmEngine::cleanup`, and
  applies post-processing rules to the cleaned text. Streams an
  `pill:active_mode` event so the listening pill renders the chip
  mid-recording. When `auto_suggest_modes` is on, fires a non-blocking
  background probe via `frontmost::detect()` and emits a
  `pill:mode_suggestion` event if the result differs from the active
  mode. **Never auto-switches.** Cleanup failure path still falls back
  to the raw transcript exactly as before.
- `src-tauri/src/pill.rs` — added `emit_active_mode` and
  `emit_mode_suggestion` helpers + payload types.
- `src-tauri/src/hotkey.rs` — added `Alt+M` / `Alt+Shift+M` /
  `Cmd+Shift+M` shortcut codes, accepted a `mode_cycle_preferred`
  argument, and registered a press-to-cycle handler that updates
  `active_mode_id` and shows an 800 ms HUD on the pill.
- `src-tauri/src/commands.rs` — added the 10 commands listed in §5.
- `src/lib/ipc.ts` — added `VoiceMode`, `AutoTrigger`,
  `PunctuationStyle`, `CaseStyle` types, the new Settings fields, and
  the typed wrappers around every new command/event.
- `src/views/Pill.tsx` — added the read-only mode chip
  (`bg-zinc-800/70 ring-1 ring-white/10`, ⌥M-styled spec) and the
  suggestion HUD on the right edge of the pill.
- `src/views/TrayPopover.tsx` — added the active-mode dropdown row
  below the dictation toggle. Click → switches mode for next utterance.
- `src/views/Settings.tsx` — new **Modes** tab between API Keys and
  Dictionary, plus the **Cycle voice mode** hotkey row in the Hotkeys
  tab.

### Deleted
- (none)

## 3. Mode JSON schema (`.voxmode`)

```jsonc
{
  "id": "email",
  "name": "Email",
  "icon": "📧",
  "description": "Formal, well-punctuated email body.",
  "system_prompt": "You are formatting a dictated message as the BODY of a professional email...",
  "dictionary": ["Tauri", "Klymenko"],
  "punctuation": "Aggressive",   // "Aggressive" | "Standard" | "Minimal" | "None"
  "case": "Sentence",            // "Sentence" | "Title" | "Lower" | "AsSpoken"
  "max_length_chars": null,      // optional integer, null = no cap
  "auto_triggers": [
    { "bundle_id_glob": "com.apple.mail" },
    { "bundle_id_glob": "com.google.Chrome", "window_title_substring": "Gmail" }
  ],
  "built_in": false,             // imports always set this false
  "created_at": 1714000000,
  "updated_at": 1714000000
}
```

Validation on import:
- `name` and `system_prompt` must be non-empty.
- `id` collisions auto-resolve to `<id>-2`, `<id>-3`, … or are
  generated from the slugified `name` if the imported file's id is
  missing/invalid.
- `built_in` is forced to `false` on import — built-in status is a
  property of the shipped catalog, not the file.

## 4. Settings keys added

| Key | Type | Default | Purpose |
| --- | --- | --- | --- |
| `active_mode_id` | `String` | `"general"` | Active mode used by the next utterance. Updated by tray dropdown, Settings → Modes → Active mode select, and the cycle hotkey. |
| `mode_cycle_hotkey` | `String` | `"Alt+M"` | Global shortcut that advances to the next mode in display order. Set to `""` to disable. |
| `auto_suggest_modes` | `bool` | `true` | When on, a background frontmost-app probe at the start of each dictation emits a non-blocking "Switch to X?" hint on the pill. Never auto-switches. |

These persist alongside the existing local-first keys in
`vox-settings.json`. Removing `active_mode_id` from the store falls
back to `"general"`.

## 5. IPC commands (Tauri)

All commands added to `src-tauri/src/commands.rs` and wired into
`invoke_handler!` in `lib.rs`:

```ts
list_modes(): VoiceMode[]
get_mode(id: string): VoiceMode | null
save_mode(mode: VoiceMode): void          // settings/onboarding/migration only
delete_mode(id: string): void             // settings/onboarding/migration only
duplicate_mode(id: string): VoiceMode     // settings/onboarding/migration only
set_active_mode(id: string): VoiceMode
export_mode(id: string): string           // returns JSON string
import_mode_json(json: string): VoiceMode // settings/onboarding/migration only
reset_default_modes(): number             // re-seeds any deleted built-in
cycle_active_mode(): VoiceMode            // used by the hotkey
suggest_active_mode(): string | null      // used by the suggestion HUD
```

The mutation commands (`save_mode`, `delete_mode`, `duplicate_mode`,
`import_mode_json`, `reset_default_modes`) are gated to the
secret-bearing windows (`settings`, `onboarding`, `migration`) via
`assert_secret_window`. Read-only commands and the active-mode
selector are callable from any window.

Two new pill events:
- `pill:active_mode` — `{ id, name, icon }`. Emitted when a
  recording starts and when the cycle hotkey is pressed.
- `pill:mode_suggestion` — `{ id, name, icon } | null`. Emitted in
  parallel with the recording start; cleared on `pill:show`.

## 6. Storage layout

```
~/Library/Application Support/com.vox.app/
└── modes/
    ├── general.json
    ├── email.json
    ├── chat.json
    ├── code-comment.json
    ├── notes.json
    └── <user-mode-id>.json   # any custom or imported modes
```

First launch seeds the 5 built-ins **only** when the directory is
empty. Subsequent launches never overwrite existing files. "Reset to
defaults" re-imports any built-in whose file is missing.

## 7. Quick-start: authoring your own mode

1. Open **Settings → Modes**.
2. Click **+ New mode**, or click **Duplicate to edit** on a built-in.
3. Fill in:
   - **Name** — display name.
   - **Icon** — emoji or short text glyph (rendered in its own
     `<span>` so users without an emoji font see plain text).
   - **System prompt** — your cleanup instructions. Aim for
     150–250 tokens. End with an explicit "Output only the cleaned
     text." line so the LLM doesn't volunteer commentary.
   - **Punctuation** — choose Aggressive (full punctuation, paragraph
     breaks), Standard (current behavior), Minimal (chat-style), or
     None (post-process strips terminal `.!?,;:` from each line).
   - **Case** — Sentence (LLM decides), Title (post-processed,
     small-word-aware), Lower (post-process to all lowercase), or
     AsSpoken (no case change).
   - **Max length** — optional. Truncates at the last sentence
     boundary that fits; falls back to a hard cut with a `…` glyph.
   - **Mode dictionary** — terms appended to the global dictionary at
     runtime.
   - **Auto-triggers** — bundle id (with optional trailing `*`
     wildcard) plus optional case-insensitive window-title substring.
     These feed the suggestion HUD only — Vox never auto-switches.
4. Click **Save**.

## 8. Sharing a mode

- **Export.** Click **Export** on a custom mode in Settings → Modes.
  Vox saves a `<id>.voxmode` file via the browser download path
  (typically `~/Downloads`).
- **Import.** Drag a `.voxmode` file (or a `.json` with the same
  schema) onto the Modes tab, or click **Import .voxmode** and pick a
  file. Imported modes always come in as `built_in: false` so the
  user can edit them freely.

A built-in mode can never be edited or deleted directly — duplicate it
first. The clone is a regular custom mode.

## 9. Privacy & telemetry posture

- **No telemetry.** There are no event counters, no anonymized
  hashes, no opt-in usage reporting. The differentiator doc's §5
  metrics are interpreted as "user-visible logs": users can see what
  mode is active on the pill and in the tray, and they can read their
  recent transcripts list — that's the only "metric" Vox keeps.
- **Frontmost-app probing** uses `NSWorkspace.frontmostApplication`
  for the bundle id and the macOS Accessibility API for the focused
  window title. Both calls return `None` when permission is denied;
  the suggestion just doesn't appear. Nothing leaves the device.
- **Auto-suggest can be turned off** in Settings → Modes via the
  "Show suggestion HUD" toggle.

## 10. Build status

| Check | Result |
| --- | --- |
| `cargo check --manifest-path src-tauri/Cargo.toml` | clean |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` | clean |
| `cargo check --manifest-path src-tauri/Cargo.toml --features local-engines` | clean |
| `cargo test --manifest-path src-tauri/Cargo.toml --lib` | 12 passed (modes module) |
| `npm run build` | clean |
| `npm run tauri build -- --bundles app` | OK — bundles `Vox.app` |

## 11. Manual smoke checklist (for QA)

1. Fresh install — open Settings → Modes. Five built-ins are present
   in this order: General, Email, Chat, Code Comment, Notes.
2. Tray popover shows "Voice mode" row with the active mode chip.
3. Click a different mode in the tray dropdown; close popover; dictate
   in any app. Pill shows the new mode chip during listening.
4. Press `Alt+M` (no recording in progress) — pill flashes the next
   mode chip for ~800 ms then hides.
5. Open Slack and start a recording (`AltRight`-hold). If the active
   mode isn't `chat`, a "Switch to Chat? ⌥M" hint appears on the right
   edge of the pill. Pressing `Alt+M` switches; ignoring it lets the
   current mode handle the dictation. Never auto-switches.
6. Settings → Modes → click **Duplicate to edit** on Email. Edit the
   name + system prompt. Save. Use it. Dictate a long Slack message
   in the Email mode — output should be in formal sentences without a
   greeting line.
7. Click **Export** on the custom mode. Check `~/Downloads`. Drag the
   `.voxmode` back onto the Modes tab — Vox imports it with a `-2`
   suffixed id (collision dedup).
8. Delete the custom mode → confirm dialog → mode disappears.
9. Manually delete `<app data dir>/modes/email.json` from Finder.
   Click **Reset to defaults**. Email reappears in the list.
10. Settings → Modes → toggle "Show suggestion HUD" off. Repeat
    step 5 — no hint should appear.

## 12. Out of scope (not implemented)

- `vox://import-mode` URL handler (deferred to v1.3 per CEO).
- KV-cache pre-warming per mode (CTO punted to v2).
- Auto-switch on frontmost app (we have the heuristic; we deliberately
  never call `set_active_mode` from the probe).
- Mode marketplace / registry / cloud sync.
