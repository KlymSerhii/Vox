# Vox — Product Requirements (Whispr Flow clone, v1)

> Target release: v1 MVP. Stack: Tauri (Rust core + web UI for settings). macOS first, Windows secondary, Linux deferred.
> Last updated: 2026-04-24.

---

## 1. Product vision

Vox is a desktop voice-dictation utility that lets a user hold one global hotkey, speak naturally in any focused text field (Slack, VS Code, Gmail, a browser form), and have a cleaned, punctuated, context-appropriate transcription injected at the cursor within roughly one second of releasing the key. It replaces typing for drafting, messaging, and prompting — keeping the user in whatever app they already have open. v1 is a single-user, BYO-API-key desktop app focused on the fundamental loop (hotkey → mic → Whisper → LLM cleanup → paste). It is not a meeting recorder, not a cloud workspace, and not a transcription archive.

---

## 2. User personas & core use case

**Primary persona — "The keyboard power user."** Software engineer, PM, founder, or writer on macOS. Already lives in keyboard shortcuts. Types 60-90 wpm but finds long-form messages (Slack threads, PR descriptions, Cursor prompts, emails) slow. Has an OpenAI / Anthropic / Groq API key. Cares about: latency, staying in the focused app, privacy, not another Electron memory hog.

**Secondary persona — "The accessibility-motivated user."** Wrist pain, RSI, or a preference for speaking. Needs reliable text injection across every app, not a dedicated "Vox window."

**Golden-path scenario (must work flawlessly on day one):**

1. User is composing a Slack message. Cursor is blinking in the Slack input.
2. User presses and holds the right `Option` key (default hotkey).
3. A small pill appears at the bottom-center of the primary screen showing a live waveform and "Listening…"
4. User speaks: *"hey team um let's push the launch to next thursday actually no next friday the 8th because the staging env is still flaky"*
5. User releases `Option`. The pill changes to "Transcribing…" then "Polishing…"
6. Within ~1 s of release, the following text appears, typed into the Slack input at the cursor: *"Hey team, let's push the launch to next Friday the 8th because the staging environment is still flaky."*
7. The pill disappears. No extra window stole focus. User hits Enter to send.

Everything else in the product is a refinement of that loop.

---

## 3. Core features (MUST have for v1)

### 3.1 Global push-to-talk hotkey

**Description.** A system-wide hotkey that, when held, starts microphone capture; when released, ends capture and triggers the transcription pipeline. Works regardless of which app is focused. Does not steal focus from the current app.

**User flow.**
1. App is running (in menu bar / tray).
2. User focuses any text field in any app.
3. User presses and holds the configured hotkey (default: right `Option` on macOS, `Ctrl+Win` on Windows).
4. Recording indicator appears (see 3.6). Audio capture starts within 80 ms of keydown.
5. User speaks.
6. User releases the hotkey. Capture stops immediately. Pipeline begins (see 3.3).
7. If the user re-presses the hotkey while the pipeline is still running on a previous utterance, the previous utterance is cancelled (no paste) and a new recording starts.

**Acceptance criteria.**
- Default macOS hotkey is right `Option` (`kVK_RightOption`). Default Windows hotkey is `Ctrl+Win`.
- Hotkey is rebindable from Settings (see 3.7). Any single modifier, any combo up to 3 keys, and `Fn` on Macs that expose it are supported.
- Keydown-to-mic-open latency ≤ 80 ms p95.
- Releasing the hotkey stops recording within 50 ms p95.
- Holding for <300 ms is treated as a misfire: recording is discarded, no pipeline runs, no paste.
- Holding for >60 s is capped at 60 s in v1 (longer is hands-free territory; see 4.1).
- Hotkey does not "leak" into the focused app (if the user binds `Right Option`, the app must not receive `Right Option` keystrokes while Vox is capturing). Use a low-level tap (CGEventTap on macOS; raw input hook on Windows).
- Hotkey survives sleep/wake and display changes.

### 3.2 Microphone capture

**Description.** Low-latency capture of the system's default input device at a quality sufficient for Whisper (16 kHz mono, PCM 16-bit).

**User flow.**
1. On first launch, the app requests microphone permission via the OS dialog.
2. If denied, the tray icon shows a red dot and clicking it opens a "Grant microphone access" screen with a deep-link button to System Settings.
3. Once granted, audio is captured from the OS-selected default input device.
4. If the user changes default input mid-session (e.g., plugs in AirPods), Vox picks up the new device on the next hotkey press without a restart.

**Acceptance criteria.**
- Captures 16 kHz mono PCM16. Internally records at device native rate, downsamples before sending.
- VAD (voice activity detection) trims leading silence so dead air at the start of a clip isn't sent to Whisper. WebRTC VAD or Silero VAD is acceptable.
- Audio buffer never holds more than the current utterance + 500 ms pre-roll. No continuous recording when hotkey is not pressed.
- Peak/RMS level is exposed to the waveform UI at 30 fps minimum.
- Handles 0-input (muted mic, hardware off) by showing a "No audio detected" toast instead of sending silence to the API.

### 3.3 Transcription pipeline (Whisper)

**Description.** Sends captured audio to a speech-to-text service and returns a raw transcript.

**User flow (invisible to user, happens between hotkey release and text injection).**
1. On hotkey release, audio buffer is closed.
2. VAD-trimmed audio is sent to the configured STT provider.
3. Raw transcript returned.
4. Transcript passed to cleanup stage (3.4).

**Acceptance criteria.**
- v1 ships with **OpenAI Whisper API** (`whisper-1` or `gpt-4o-transcribe` — whichever is cheaper/faster at ship time) as the default provider.
- v1 supports a second provider: **Groq Whisper** (`whisper-large-v3-turbo`) as a "fast mode" option, since Groq typically returns transcripts in <300 ms for <30 s audio.
- Provider is selectable in Settings. API key is stored in the OS keychain (macOS Keychain, Windows Credential Manager), never in plaintext config.
- Whisper is called with `language=auto` by default; user can pin a language in settings (see 4.3).
- On network failure, show a tray-icon error state and a toast: "Transcription failed — check connection." Keep the audio buffer in memory for 60 s so the user can retry via a "Retry last" menu item.
- Timeout for the STT call is 15 s. Beyond that, abort and show the same error.
- Audio is sent over TLS. Audio is never written to disk in v1 (see 6.3).

### 3.4 AI cleanup / polish stage

**Description.** Takes the raw Whisper transcript and rewrites it into clean, punctuated, context-appropriate text. Removes filler words ("um," "uh," "like"), applies mid-utterance self-corrections ("let's meet at 2, actually 3" → "Let's meet at 3."), adds punctuation, capitalizes proper nouns, and preserves the speaker's voice.

**User flow (invisible).**
1. Raw transcript from 3.3 plus a short system prompt is sent to the configured LLM.
2. Cleaned transcript returned.
3. Cleaned transcript is passed to the paste stage (3.5).

**Acceptance criteria.**
- v1 default provider is **Anthropic Claude Haiku 4.5** or **OpenAI gpt-4.1-mini** — whichever benches faster at <500 ms for a 200-token response at ship time. Engineering picks one; the setting is user-changeable.
- API key stored in OS keychain. Reuses the same key field if provider is OpenAI for both STT and LLM.
- Cleanup prompt is a single fixed system prompt shipped with the app, editable only via a hidden dev flag in v1 (not exposed in settings).
- Cleanup prompt MUST:
  - remove filler words ("um", "uh", "like" as filler, "you know", "sort of")
  - apply mid-sentence self-corrections
  - add sentence-final punctuation and commas
  - preserve the speaker's register (don't formalize casual speech)
  - never add content the user didn't say (no hallucinated facts, names, or links)
  - never refuse or editorialize — it returns cleaned text only, no preambles like "Here is your cleaned text:"
- If the raw transcript is empty or <2 words, cleanup is skipped and the raw text is used (or nothing is pasted if empty).
- Cleanup timeout: 3 s. On timeout, fall back to raw Whisper transcript and show a subtle "Polish skipped" hint in the tray.
- End-to-end p95 latency (hotkey release → text visible in target app) ≤ 1500 ms for utterances ≤ 15 s. Target p50 ≤ 900 ms.

### 3.5 Text injection into focused field

**Description.** Inserts the cleaned transcript into whatever text input has OS focus, without stealing focus and without opening a Vox window.

**User flow (invisible).**
1. Cleaned transcript arrives from 3.4.
2. Vox writes the text to the system clipboard (preserving the user's prior clipboard contents in a temp buffer).
3. Vox simulates `Cmd+V` (macOS) / `Ctrl+V` (Windows) to the currently focused app.
4. After a 100 ms delay, Vox restores the user's original clipboard contents.
5. If paste fails (no text-input focus, app blocks synthetic paste), Vox falls back to synthesizing keystrokes character-by-character via `CGEventKeyboardSetUnicodeString` (macOS) or `SendInput` with unicode (Windows).
6. If that also fails, Vox keeps the cleaned text on the clipboard, shows a tray notification "Dictation ready — press Cmd+V to paste," and exposes a "Paste last transcript" hotkey (default `Cmd+Shift+V` on macOS).

**Acceptance criteria.**
- Default injection method is clipboard + simulated paste. Fallback to keystroke synthesis is automatic and logged.
- Original clipboard contents are always restored within 500 ms of paste.
- Requires macOS Accessibility permission. On first paste attempt without permission, show a blocking modal explaining why and deep-linking to the permission pane.
- Text injection works in: Chrome, Safari, Slack, VS Code, Cursor, Notion, Messages, Mail, Terminal, iTerm2, Discord, Obsidian, Linear, Figma comment fields. Test matrix must pass all of these before v1 ship.
- Never injects into a Vox-owned window.
- If no app has keyboard focus (e.g., Finder with nothing selected), paste is suppressed and the transcript is placed on the clipboard with a toast.

### 3.6 Visual listening indicator

**Description.** A small, always-on-top, click-through "pill" overlay that shows recording state. Replaces the need to focus a main app window.

**User flow.**
1. Pill is hidden by default.
2. On hotkey press, pill fades in at bottom-center of the active screen (configurable position in Settings) within 100 ms.
3. Pill shows states: `Listening` (pulsing mic + waveform), `Transcribing` (spinner), `Polishing` (different spinner), `Error` (red, 2 s).
4. On successful paste, pill fades out within 200 ms.

**Acceptance criteria.**
- Pill window is borderless, non-activating (does not take focus), click-through by default.
- Pill never covers more than a 280×56 px area at 1× scale.
- Pill is drawn on all displays in mirrored mode; in extended-display mode it appears on the display containing the focused window.
- Pill respects the macOS menu bar / notch and never overlaps them.
- Waveform is rendered from real mic RMS at ≥ 30 fps; no fake animation when the mic is silent.
- Full-screen apps (video, games, presentations): pill still renders over them on macOS via `.screenSaver` window level.

### 3.7 Settings window

**Description.** A standard app window (Tauri webview) with the minimum set of controls a user needs to configure Vox.

**User flow.**
1. User clicks the tray icon and selects "Settings…" (or presses `Cmd+,` when Vox is focused).
2. Settings window opens with tabs: General, Hotkeys, API Keys, Privacy, About.
3. User changes a setting; changes apply immediately (no "Save" button) and persist to config.

**Acceptance criteria.**
- **General:** launch at login toggle, start hidden toggle, indicator position (bottom-center / bottom-right / top-center), language preset (Auto / English / pinned list).
- **Hotkeys:** rebind push-to-talk, rebind paste-last-transcript, rebind cancel. Conflict detection warns if the chosen combo is already used by macOS.
- **API Keys:** one field per provider (OpenAI, Groq, Anthropic). Masked by default, reveal on click. "Test connection" button calls a cheap endpoint to validate. Keys stored in OS keychain.
- **Privacy:** "Do not send audio if offline" toggle (defaults on, since v1 is online-only — see 6.2). "Delete all local data" button wipes config and keychain entries.
- **About:** version, build hash, "Check for updates," link to docs, link to open logs folder.

### 3.8 Tray / menu-bar icon

**Description.** The primary entry point. Vox has no dock icon by default.

**User flow.**
1. App launches → tray icon appears in the macOS menu bar / Windows system tray.
2. User clicks icon → menu opens: status line, "Start listening" (same as holding hotkey, but as a click for discoverability), "Paste last transcript," "Settings…", "Check for updates," "Quit."

**Acceptance criteria.**
- Tray icon is a template image on macOS (monochrome, adapts to light/dark menu bar).
- Icon animates subtly while recording (dot pulse).
- Icon changes color/shape on error state (red) or missing-permission state (yellow).
- Left-click opens the menu; right-click on Windows opens the same menu.
- Right-click on macOS opens the same menu (standard `NSStatusItem` behavior).

### 3.9 Onboarding

**Description.** First-run flow that gets the user from install to first successful dictation in under 90 seconds.

**User flow.**
1. User launches app for the first time.
2. Welcome screen: "Vox — talk to any app." [Get started].
3. Step 1: request Microphone permission (OS dialog triggered by a button).
4. Step 2: request Accessibility permission (macOS only) with a clear "why we need this" explanation and a deep-link button.
5. Step 3: paste API key(s). Validation runs live; green check when valid.
6. Step 4: "Try it now." An in-app text field with a prompt: "Hold [Right Option] and say something." The first successful transcription is inserted into this field, and a confetti/checkmark marks completion.
7. Step 5: "You're done. Vox lives in your menu bar." [Finish].

**Acceptance criteria.**
- Each step is skippable except mic permission (which is a hard block).
- If the user quits mid-onboarding, relaunching resumes at the last unfinished step.
- Completing step 6 is the trigger for marking onboarding done; clicking "skip" on step 6 also marks it done.

---

## 4. Secondary features (NICE to have for v1)

### 4.1 Hands-free mode (double-tap toggle)

**Description.** Double-tapping the hotkey starts recording without needing to hold. Pressing it once more (or `Esc`) stops.

**User flow.** Double-tap right `Option` → pill shows "Listening (hands-free)." Speak as long as needed. Press `Option` once or hit `Esc` to stop → pipeline runs as normal.

**Acceptance criteria.**
- Double-tap threshold is 300 ms.
- Maximum hands-free duration 3 minutes in v1 (safety cap; audio buffer size).
- Auto-stops after 30 s of silence (VAD).

### 4.2 Cancel in-flight dictation

**Description.** Pressing `Esc` while a recording or pipeline is in progress cancels the entire operation — no paste.

**Acceptance criteria.** `Esc` during listening, transcribing, or polishing aborts the pipeline within 100 ms and hides the pill. Any pending paste is suppressed.

### 4.3 Language pinning

**Description.** User can pin Whisper to a specific language for accuracy when they don't speak English.

**Acceptance criteria.** Setting persists, is passed to Whisper as `language=<iso>`, and the pill shows a small country flag / code when a pinned language is active.

### 4.4 "Paste last transcript" hotkey

**Description.** Re-pastes the most recent cleaned transcript at the current cursor. Useful for sharing the same message into a second chat.

**Acceptance criteria.** Default hotkey `Cmd+Shift+V` on macOS. Works for up to 5 minutes after the original dictation; after that the buffer is cleared.

### 4.5 Personal dictionary (basic)

**Description.** A user-editable list of custom words/names/acronyms (e.g., "Tauri", "Klymenko", "Vox") that gets injected into the LLM cleanup prompt so the model spells them correctly.

**User flow.** Settings → Dictionary tab → add/remove terms.

**Acceptance criteria.**
- Up to 100 terms in v1.
- Terms are appended to the cleanup system prompt as `Known terms: [...]. Prefer these spellings when phonetically close.`
- No effect on Whisper's raw output (we only influence the LLM pass in v1).

### 4.6 Auto-update

**Description.** Tauri updater checks for new versions on launch and on a 24 h interval.

**Acceptance criteria.** Updates are signed (Tauri updater manifest). User sees a "Update available" toast and can click to install-and-relaunch. No silent updates.

---

## 5. Out of scope for v1

- **Cloud sync, team accounts, shared dictionaries, shared snippets.** v1 is single-user, single-device. No account system at all.
- **Transcript history / searchable archive.** We never store transcripts on disk in v1.
- **Mobile apps (iOS / Android).** Desktop only.
- **Meeting recording / multi-speaker diarization.** Vox is a single-speaker dictation tool.
- **Voice "Command Mode" to edit selected text** (Whispr's feature). Deferred to v2.
- **Custom "tone profiles" per app** (casual for Slack, formal for Gmail). Deferred to v2.
- **Offline / on-device models.** v1 is online-only. v2 may add whisper.cpp + llama.cpp.
- **Snippets / voice shortcuts / text expansion.** Deferred to v2.
- **Usage dashboards, analytics, SSO, HIPAA/SOC2.**
- **Screen-capture / active-window context.** v1 does NOT take screenshots and does NOT read accessibility text from other windows. (Marketing point: privacy advantage over Whispr.)
- **Linux packaging.** Rust core is portable; packaging for Linux is a v2 effort.
- **Streaming partial results** (showing text as you speak). v1 is end-of-utterance only.

---

## 6. Non-functional requirements

### 6.1 Latency

- **End-to-end p50 ≤ 900 ms**, **p95 ≤ 1500 ms**, for utterances of 2-15 s at 100 Mbps+ connection. Measured from hotkey release to first character visible in target app.
- **Hotkey-to-mic-open ≤ 80 ms p95.**
- **Pill render on hotkey press ≤ 100 ms p95.**
- If p95 is not met, the Settings → General tab exposes a "Use fast mode (Groq + mini LLM)" toggle that relaxes accuracy for speed.

### 6.2 Connectivity

- v1 is **online-only**. App detects offline state and shows a clear tray badge + toast on hotkey press: "Vox needs internet." No attempt to transcribe.
- No offline queue in v1 — if you speak offline, it's lost.

### 6.3 Privacy

- Audio is **never written to disk**. It lives in a RAM buffer during the capture + pipeline window and is zeroed after paste or error.
- Raw transcripts and cleaned transcripts are held in memory only long enough to support "Paste last transcript" (5 min TTL), then zeroed.
- API keys stored in OS keychain, never in plaintext config.
- No telemetry in v1 (no Sentry, no PostHog). A single opt-in "share crash reports" checkbox can be added post-1.0 if needed.
- Vox does not capture screenshots, does not read other apps' accessibility trees, does not access the clipboard except to write the final transcript.
- Privacy policy shipped in-app (About tab) and on website.

### 6.4 Supported platforms

- **Primary: macOS 13 (Ventura) and later, both Intel and Apple Silicon.** Universal binary, notarized and stapled.
- **Secondary: Windows 11 (and Windows 10 22H2 best-effort).** Code-signed .msi.
- **Deferred: Linux** (v2).

### 6.5 Resource budget

- Idle RAM ≤ 120 MB (hard goal — explicitly better than Whispr's reported 800 MB Electron footprint).
- Idle CPU ≤ 0.5 % on M-series silicon.
- Binary size ≤ 25 MB (macOS universal).
- No background mic capture when hotkey is not pressed.

### 6.6 Reliability

- Crash-free sessions ≥ 99.5 % in v1.
- On API outage, the app remains usable for settings and shows a clear error state — it does not hang or crash.
- Automated test matrix covers the 13 target apps listed in 3.5 before every release.

### 6.7 Accessibility

- Settings UI passes macOS VoiceOver navigation.
- Indicator pill is visual-only; for deaf/HoH users, no audio feedback is required (none is emitted anyway).
- All hotkeys rebindable; the app does not assume the user can reach `Right Option`.

---

## 7. Key UX decisions

### 7.1 Default hotkey

- **macOS: Right `Option` (held).** Rationale: it's on every Mac keyboard, it's otherwise unused by macOS, it's a single key so push-to-talk feels natural, and Whispr itself uses `Fn` (which is unreliable on external keyboards). Right `Option` is more portable.
- **Windows: `Ctrl+Win` (held).** Matches Whispr's default and is already muscle-memory for users migrating.
- **Paste last transcript: `Cmd+Shift+V` / `Ctrl+Shift+V`.**
- **Cancel: `Esc`** (hardcoded; not rebindable in v1).

### 7.2 Listening indicator

- Small pill, bottom-center of active display by default. 280×56 px.
- Pulsing mic icon + live waveform while listening.
- Two discrete spinners for "transcribing" vs "polishing" so the user knows whether a hang is network-side or model-side.
- Fade in 150 ms, fade out 200 ms. No sound.
- Never steals focus. Click-through.

### 7.3 Settings panel

- Tabbed layout: General, Hotkeys, API Keys, Dictionary, Privacy, About.
- Changes apply instantly; no Save button.
- Accessible via tray menu and via `Cmd+,` when Vox window is focused.
- Dark mode follows system.

### 7.4 Onboarding

- 5-step flow, ~90 seconds to "first successful dictation."
- Mic permission is the only hard gate.
- Step 6 ("try it now") has a built-in text field so the user never has to leave the app to feel the magic moment.

### 7.5 No dock icon by default

- Vox is a menu-bar resident, not a dock app. Setting to "show in dock" exists for users who want a visible window presence, off by default.

### 7.6 Error surfaces

- Errors appear in two places: the pill (red, 2 s flash + message) and the tray icon (persistent red dot for permission/API-key issues).
- No OS notifications for routine errors — only for "Dictation ready but couldn't paste" clipboard-fallback cases.

---

## 8. Open questions & assumptions

**Assumptions (proceeding unless engineering flags otherwise):**
1. Tauri 2.x is mature enough on macOS for a global hotkey tap + always-on-top non-activating overlay window. Fallback: drop to pure Rust + `cacao` or `objc2` for the indicator window if Tauri's overlay story is weak.
2. OpenAI Whisper API and Anthropic/OpenAI LLM APIs are allowed as default providers (user supplies key). No legal/licensing blocker from shipping the default system prompt.
3. The "clipboard + simulated paste" method works in 100 % of our 13-app test matrix. If any app blocks synthetic paste (e.g., 1Password, banking sites), we silently fall back to keystroke synthesis.
4. Users in v1 are comfortable with BYO API key. We will not ship a hosted/subscription backend in v1.
5. Right `Option` is not claimed by any major accessibility tool we care about. (Karabiner users can rebind.)

**Open questions for eng / design:**
1. **Should push-to-talk require a minimum hold (>300 ms) or fire on any press?** Proposal: 300 ms min to avoid accidental triggers; confirm with a dogfood round.
2. **If Groq is faster, should it be the default STT?** Tradeoff: Groq is cheaper and faster but less accurate on accents. Suggest: default to OpenAI Whisper for v1, surface Groq as "Fast mode" toggle.
3. **Do we need a "pause" during long dictation?** E.g., user holds the key, coughs, keeps going. VAD handles trimming silence, but should we show a visual "paused" cue? Defer to post-launch dogfooding.
4. **Whisper streaming vs batch?** OpenAI's batch endpoint is simpler; streaming shaves latency but complicates the pipeline. Recommendation: batch for v1, revisit for v1.1.
5. **Do we keep any in-memory transcript history beyond "last 1"?** Proposal: no — simpler, more private. "Paste last transcript" (singular) only.
6. **Windows parity in v1 or v1.1?** Current plan ships macOS first, Windows as v1.1 two weeks later. Confirm scope with eng.
7. **Notarization + Developer ID Apple account — who owns it?** Needs to be in place before first external build.
8. **Analytics — truly none, or anonymous counters?** Product position is "zero telemetry." Revisit only if we can't diagnose issues from the field.
