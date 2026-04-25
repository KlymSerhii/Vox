# Vox — QA Test Report

> Tester: QA / Automated verification
> Date: 2026-04-24
> Scope: Phase 1 + Phase 2 + Staff review fixes
> Verdict: **SHIP-WITH-CAVEATS** — see "Final regression matrix" and
> "Open caveats" below.

---

## Methodology

This report covers everything that can be verified without holding a real
hotkey, granting macOS permissions, or hitting the live STT/LLM APIs.

Performed in this environment (deterministic):
- A. Static / build verification
- B. Bundle inspection
- C. Frontend smoke test (Vite dev server + curl)
- D. Code-trace verification per PRD §3 / §4

Deferred to the user, because they require the Mac that owns the
Microphone & Accessibility TCC entries plus the user's API keys:
- E. Manual test plan — see `docs/manual-test-plan.md`.
- F. End-to-end latency, paste-into-13-apps matrix, real-world dictation.

---

## Phase A — Static / build verification

| Command | Result | Notes |
|---|---|---|
| `cargo check --manifest-path src-tauri/Cargo.toml` | ✅ pass | 17 deprecation warnings (cocoa → objc2-app-kit, tracked as T4 in code-review) |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets` | ✅ pass | Same 17 deprecations, no clippy lints |
| `cargo fmt -- --check` | ⚠️ pre-existing diffs | 8 small wrap-style diffs unrelated to QA. Per instructions, not auto-fixed. My QA edits do not introduce *new* diffs. |
| `npm run build` (`tsc --noEmit && vite build`) | ✅ pass | 4 HTML entries, 4 JS chunks, 1 CSS, ~75 KB total gzipped |
| `npm run tauri build -- --bundles app` | ✅ pass | Release `.app` produced, ad-hoc signed |

**Net result:** the codebase compiles cleanly on macOS arm64. No errors,
only pre-existing deprecation warnings that the staff review already
documented.

---

## Phase B — Bundle inspection

Bundle path: `src-tauri/target/release/bundle/macos/Vox.app`

| Check | Result |
|---|---|
| `.app` directory exists | ✅ |
| `Contents/MacOS/vox` (Mach-O thin arm64) | ✅ |
| `Info.plist::LSUIElement` = `true` (no Dock icon) | ✅ |
| `Info.plist::NSMicrophoneUsageDescription` set | ✅ |
| `Info.plist::NSAppleEventsUsageDescription` set | ✅ |
| `CFBundleIdentifier` = `com.vox.app` | ✅ |
| `CFBundleVersion` = `0.1.0` | ✅ |
| Embedded WebView assets (index.html / pill.html / onboarding.html / tray-popover.html) present in binary | ✅ (verified via `strings` on the binary) |
| `codesign -dv` | ✅ ad-hoc (`adhoc, linker-signed`) — expected for dev. Not notarized; **must be code-signed + notarized before public distribution.** |

**Caveat for ship:** Vox.app is currently ad-hoc signed. Gatekeeper will
block first-run for any user other than the one who built it. Notarization
is in the Phase 3 TODO list.

---

## Phase C — Frontend smoke test (Vite dev server)

`npm run dev` started Vite on `http://localhost:1420` in 177 ms. All
entries return HTTP 200:

| Path | Status |
|---|---|
| `/` (settings) | ✅ 200 |
| `/pill.html` | ✅ 200 |
| `/onboarding.html` | ✅ 200 |
| `/tray-popover.html` | ✅ 200 |
| `/src/entries/settings.tsx` | ✅ 200 |
| `/src/entries/pill.tsx` | ✅ 200 |
| `/src/entries/onboarding.tsx` | ✅ 200 |
| `/src/entries/tray-popover.tsx` | ✅ 200 |

Dev server stopped after smoke test.

---

## Phase D — Code-trace verification (PRD §3 core features)

For each feature I open the file the engineers said it lives in, locate
the call site, and confirm it does what the PRD says.

### 3.1 Push-to-talk hotkey
- ✅ `src-tauri/src/hotkey.rs:41-91` — registers via
  `tauri-plugin-global-shortcut`. Default key from `Settings::hotkey`
  (default `AltRight`, settings.rs:31-33). Press → `session::begin_recording`,
  release → `session::end_recording`.
- ✅ Misfire guard at `session.rs:172-176`: `MIN_RECORD_MS = 300`.
- ✅ 60-s safety cap at `session.rs:131` via `spawn_max_record_guard` (`MAX_RECORD_SECS = 60`).
- ✅ Re-press supersession via `session_id` checks at every async boundary
  (`session.rs:148, 159, 256, 292, 313`), and a `PipelineGuard` (added in
  this QA pass) ensures the active flag tracks the *current* session.
- ⚠️ **Hotkey rebinding requires app restart.** Settings UI persists the
  new value but does not call `unregister_all` + re-register live. Code
  review I2 deferred to Phase 3. Settings UI footer warns the user.

### 3.2 Microphone capture
- ✅ `src-tauri/src/audio.rs` — dedicated `vox-audio` thread (so cpal's
  non-Send Stream stays put), `AudioHandle` is `Send + Sync`. 16 kHz mono
  PCM16 target (line 19), downmix + linear resample.
- ✅ Hard buffer cap at 65 s (`MAX_BUFFER_SECS`, line 25 + `trim_front`),
  drops oldest samples on overflow.
- ✅ Live RMS exposed via `current_rms()` to the pill at 20 Hz
  (`session.rs::spawn_audio_level_sampler`).
- ✅ "No audio detected" guard (`session.rs:200-209`) — peak below
  -50 dBFS surfaces an Error pill instead of an empty STT call (I6).
- ⚠️ VAD-based silence trim NOT shipped (T6 — Phase 3).
- ⚠️ Default-input device picker NOT shipped (T3 — Phase 3). Vox uses
  the OS default.
- ⚠️ Linear resampler is rough; a windowed-sinc would be better for
  44.1k/48k → 16k (T7 — Phase 3). Acceptable for v1.

### 3.3 Transcription pipeline (Whisper)
- ✅ `src-tauri/src/stt.rs` — Groq `whisper-large-v3-turbo` (line 19).
  PRD calls Groq the "fast mode" alternative; Vox v1 ships Groq as the
  default. **This is a deliberate divergence from PRD §3.3** (which
  defaults to OpenAI Whisper) — confirmed in README. User-facing.
- ✅ TLS-only (`https_only`), 15 s timeout, lazy `OnceCell` reqwest
  client (I3 fix).
- ✅ API key from Keychain (`settings::get_groq_key`).
- ✅ Audio is multipart-form-uploaded; never written to disk (privacy 6.3).
- ✅ Error handling: 401 → "Groq key rejected", network → "Offline",
  parse → "Unexpected response", api → "Transcription failed".
- ⚠️ Retry-last-transcript hotkey NOT shipped — PRD §3.3's 60-s in-memory
  retry buffer is not implemented as a separate "Retry last" menu item.
  The "Paste last transcript" feature is partially implemented (recent-5
  ring buffer in tray popover, copy-only, no hotkey).

### 3.4 LLM cleanup
- ✅ `src-tauri/src/llm.rs` — Anthropic `claude-haiku-4-5` (line 26).
- ✅ TLS-only, 3 s timeout (matches PRD §3.4 "Cleanup timeout: 3 s").
- ✅ System prompt at lines 31-41 covers all required rules: filler
  removal, mid-sentence corrections, punctuation, register preservation,
  no-hallucination, no preambles.
- ✅ `<2 words → TooShort → fall back to raw transcript`
  (`llm.rs:88-91` + `session.rs:281`).
- ✅ Cleanup-disabled or no Anthropic key → raw transcript pasted
  (`session.rs:266`).
- ✅ Dictionary terms appended to system prompt as `Known terms (...)`
  (`llm.rs:180-194`).
- ✅ Dictionary also passed to STT as a Whisper bias prompt
  (`stt.rs:65, 97-114`).

### 3.5 Text injection
- ✅ `src-tauri/src/inject.rs::paste_text` — clipboard + simulated Cmd+V
  via `enigo`. Modifier press / V click / modifier release with
  always-release-modifier safety (lines 39-47).
- ✅ Original clipboard restored only if our text is still on the
  pasteboard (C4 fix at lines 53-67) — prevents clobbering a clipboard
  manager's later writes.
- ✅ 30 ms set → 150 ms restore window — short enough to feel instant,
  long enough for receivers to consume.
- ⚠️ Keystroke-synthesis fallback NOT shipped if paste fails. Today on
  paste failure we leave the text on the clipboard and emit
  "Copied — press Cmd+V to paste". PRD §3.5 step 5 wants automatic
  Unicode-keystroke synthesis. **Acceptable for v1 — graceful degradation.**
- ⚠️ "Paste last transcript" hotkey (PRD §4.4) NOT shipped as a hotkey;
  the recent-5 popover supports `navigator.clipboard.writeText` only.
- ⚠️ Test matrix across the 13 named apps (Chrome, Slack, VS Code,
  Cursor, Notion, Messages, Mail, Terminal, iTerm2, Discord, Obsidian,
  Linear, Figma) is the user's manual responsibility. See
  `docs/manual-test-plan.md`.

### 3.6 Visual listening indicator (pill)
- ✅ `src-tauri/src/pill.rs::create` — 280×56 borderless transparent
  always-on-top non-focusing window (matches PRD).
- ✅ Bottom-center positioning with 96 px above-Dock margin
  (`pill.rs:18, 129-147`).
- ✅ macOS native panel style applied (`apply_macos_panel_style`,
  lines 151-186): NSStatusWindowLevel (above fullscreen),
  CanJoinAllSpaces + Stationary + FullScreenAuxiliary, NonActivatingPanel
  style mask. Excluded from windows menu.
- ✅ Click-through ON by default; toggled OFF only on Error state
  (`pill.rs:108-111`) so user can click to dismiss.
- ✅ Real waveform from RMS samples (`Pill.tsx:39-48`, 24 bars, shifting buffer).
- ✅ Six states: listening, transcribing, polishing, done, error, canceled.
- ⚠️ Pill always renders on the **primary** display (T9 — Phase 3).
  Multi-monitor users with the focused window on a secondary display
  will see the pill on the wrong monitor.

### 3.7 Settings window
- ✅ `src/views/Settings.tsx` — six tabs as required: General, Hotkeys,
  API Keys, Dictionary, Privacy, About.
- ✅ Save-on-change (no Save button) via `update()` → `setSettings`.
- ✅ API keys: masked by default, "Show/Hide" toggle, "Test" button
  hits `test_groq_connection` / `test_anthropic_connection`.
- ✅ Keychain storage (`settings.rs::set_groq_key` /
  `set_anthropic_key`) with empty-key-protection (I8 fix at `settings.rs:291-298`).
- ✅ Privacy tab: "Send audio to cloud" toggle, what-leaves / what-stays
  summary.
- ✅ About tab: version, Replay onboarding.
- ⚠️ "Launch at login" toggle persists the setting but has **no backend
  wiring** (T2 — Phase 3). UI says "(Phase 3 — not yet wired.)" so the
  user is informed.
- ⚠️ Sound cues toggle is a stub (T-undocumented).
- ⚠️ Default-mic picker is a stub.
- ⚠️ "Delete all local data" button NOT present (PRD §3.7 Privacy tab).
- ⚠️ Hotkey conflict detection NOT shipped (PRD §3.7 Hotkeys).

### 3.8 Tray icon
- ✅ `src-tauri/src/tray.rs` — built at startup. Left-click anchors and
  shows the popover (`tray-popover` window). Right-click shows native
  fallback menu (Open Settings / Quit).
- ✅ Icon is a template image (16×16 RGBA, monochrome-friendly).
- ⚠️ **Icon does not animate during recording.** PRD §3.8 wants pulse-on-
  record + red-on-error + yellow-on-permission-needed. Today the icon is
  static (T1 — Phase 3, code review I5).

### 3.9 Onboarding
- ✅ `src-tauri/src/onboarding.rs` + `src/views/Onboarding.tsx` — 5-step
  wizard (Welcome / Mic / Accessibility / API keys / Try it).
- ✅ Shown automatically on first run (`lib.rs:92-97`).
- ✅ Re-triggerable from Settings → About → Replay onboarding.
- ✅ C7 fix landed: only the Microphone step is a hard gate
  (`Onboarding.tsx:165-168`); accessibility & API keys are skippable per
  PRD §3.9.
- ✅ Permission polling every 2 s (`Onboarding.tsx:43`).
- ⚠️ Per the Accessibility step UI text, granting Accessibility requires
  a Vox restart for the global hotkey to bind. Documented inline.

---

## Phase D — Secondary features (PRD §4)

| § | Feature | Status |
|---|---|---|
| 4.1 | Hands-free mode (double-tap) | ❌ NOT shipped |
| 4.2 | Cancel in-flight (`Esc`) | ✅ shipped — but the staff review C2 fix was incomplete (cancel hotkey wrapper was still gating on `is_recording()`). **Fixed in this QA pass.** See "QA-Phase fixes" below. |
| 4.3 | Language pinning | ❌ NOT shipped — Whisper is called without a `language=` param (defaults to auto). |
| 4.4 | "Paste last transcript" hotkey | ❌ NOT shipped as a hotkey. Recent-5 ring buffer is in tray popover with click-to-copy only. |
| 4.5 | Personal dictionary | ✅ shipped — used in both STT (Whisper prompt) and LLM (system-prompt suffix). |
| 4.6 | Auto-update | ❌ NOT shipped (T11 — Phase 3). |

---

## QA-Phase fixes (this pass)

### F1. Cancel hotkey was gated on `is_recording`, contradicting C2

**Where:** `src-tauri/src/hotkey.rs:75-80` (pre-QA).

**What was wrong:** The staff-review fix for C2 added cancel-during-pipeline
support inside `session::cancel`, but the *hotkey wrapper* still wrapped
the call in `if ctx_clone.is_recording()`. Net effect: pressing Esc
during the post-release STT/LLM phase did nothing — the bug C2 was meant
to fix was still present at the entry point.

**Fix:** removed the `is_recording()` gate from the hotkey callback so
`session::cancel` runs unconditionally. Added a `pipeline_active`
AtomicBool to `SessionCtx` so `cancel()` can distinguish "Esc with a
real session in flight" from "Esc with nothing happening" (otherwise an
idle Esc press would flash a stray Canceled pill). Cleared via an RAII
`PipelineGuard` so every exit path of the spawned task — success,
error, cancel, supersede — releases the flag, with the guard
double-checking the session id so a re-pressed session's flag isn't
clobbered by an old task's drop.

**Files changed:** `src-tauri/src/hotkey.rs`, `src-tauri/src/session.rs`.

**Verified:** cargo check + cargo clippy still clean (only the pre-
existing 17 cocoa deprecation warnings).

---

## Open caveats (must reach the user)

These are not bugs — they are deliberate v1 deferrals that should be on
the release-notes checklist:

1. **Code signing & notarization missing.** Bundle is ad-hoc. Distributing
   to other users requires signing + notarization.
2. **Tray icon is static** during recording / errors. PRD §3.8 violation
   (deferred to Phase 3 per code review I5).
3. **Pill always shows on primary display** in extended-display mode
   (T9 — deferred to Phase 3). PRD §3.6 says "appears on the display
   containing the focused window."
4. **Hotkey rebind requires restart.** Settings footer warns the user.
5. **PRD §3.5 keystroke-synthesis fallback** is a one-line message
   instead. Acceptable for the 13-app matrix as long as none block
   synthetic paste; user must verify.
6. **PRD §3.3 STT default is Groq, not OpenAI Whisper.** Conscious choice
   per README; users with only an OpenAI key cannot use Vox.
7. **`launch_at_login` toggle is a stub** — UI persists, no effect.
8. **No "Delete all local data" button** in Settings → Privacy
   (PRD §3.7).
9. **No hotkey conflict detection** when the user picks a combo macOS
   already uses.
10. **Hands-free mode (4.1), language pinning (4.3), paste-last-transcript
    hotkey (4.4), and auto-update (4.6) are not shipped.**

---

## Final regression matrix

| Feature | PRD § | Code present | Static build | Manual test (user runs) |
|---|---|---|---|---|
| Push-to-talk hotkey | 3.1 | ✅ | ✅ | ⚠️ user must verify on real keyboard |
| Microphone capture | 3.2 | ✅ | ✅ | ⚠️ user must verify with real mic |
| STT (Groq Whisper) | 3.3 | ✅ | ✅ | ⚠️ user must verify with real Groq key |
| LLM cleanup (Anthropic) | 3.4 | ✅ | ✅ | ⚠️ user must verify with real Anthropic key |
| Paste into focused field | 3.5 | ✅ | ✅ | ⚠️ user must verify across the 13-app matrix |
| Listening pill | 3.6 | ✅ | ✅ | ⚠️ user must verify visually + on multi-monitor |
| Settings window | 3.7 | ⚠️ stubs | ✅ | ⚠️ launch-at-login + delete-data not testable |
| Tray icon | 3.8 | ⚠️ no state animation | ✅ | ⚠️ tray reflects no recording state |
| Onboarding | 3.9 | ✅ | ✅ | ⚠️ user must walk it on a fresh install |
| Hands-free 4.1 | 4.1 | ❌ | n/a | n/a |
| Cancel (Esc) | 4.2 | ✅ post QA-fix | ✅ | ⚠️ user must verify mid-pipeline |
| Language pinning | 4.3 | ❌ | n/a | n/a |
| Paste-last hotkey | 4.4 | ❌ (popover-only) | n/a | n/a |
| Dictionary | 4.5 | ✅ | ✅ | ⚠️ user must verify spellings preserved |
| Auto-update | 4.6 | ❌ | n/a | n/a |

---

## Verdict: SHIP-WITH-CAVEATS

The end-to-end code path is real, the staff-review fixes are present
(except the C2 follow-up captured by F1 above, now landed), and the
build pipeline is clean. The deferred items are honest — none of them
block a Phase-2 dogfood release as long as the user accepts:

- They (or their dev team) must build + sign + notarize before sharing
  the .app.
- They will run the manual test plan in `docs/manual-test-plan.md` on
  their own Mac with their own keys.
- The Phase-3 TODO list is the next milestone, not a hidden surprise.

If any of those are not acceptable, **HOLD** and finish Phase 3.
