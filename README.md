# Vox

A macOS menu-bar dictation app. Hold a hotkey, speak, and your words are
transcribed and pasted into the currently focused text field.

**Vox runs offline by default** — local Whisper for speech-to-text and
Gemma 4 for transcript cleanup, both via Metal/Core ML on Apple Silicon.
Cloud (Groq Whisper Turbo + Claude Haiku 4.5) is an opt-in fallback for
users who prefer the extra 200–400 ms of speed.

This is the **local-first** build: core dictation loop, on-device
inference, model catalog + downloader, a floating listening pill, a
tabbed settings window with a Models tab, a tray popover, a first-run
onboarding wizard, a soft migration dialog for existing cloud users,
and macOS Keychain-backed API keys for cloud-mode.

## Requirements

- macOS 13 (Ventura) or later, Apple Silicon
- Rust 1.77+ (`rustup show`)
- Node.js 22, npm 11
- Xcode Command Line Tools (`xcode-select --install`)

## First run

```bash
# from the repo root
npm install
npm run tauri dev   # launches the app in dev mode
```

On first launch the onboarding wizard walks you through:

1. **Microphone** permission — triggered by an in-wizard button.
2. **Accessibility** permission — required for the global hotkey and the
   simulated Cmd+V paste. macOS won't prompt for this; the wizard opens the
   right settings pane and polls for the grant every 2s.
3. **Choose your stack** — Run offline (recommended; ~1.9 GB download of
   Whisper + Gemma 4) or Use cloud (BYO API keys).
4. **API keys** (cloud branch only) — Groq + Anthropic.
   **Download models** (offline branch only) — single combined progress
   bar; on completion the next step works fully offline.
5. **Try it** — a live text field; hold the hotkey and speak.

Cloud API keys (when used) live in the macOS Keychain under service
`app.vox.vox`. Local model files live under
`~/Library/Application Support/com.vox.app/models/<stt|llm>/<id>/` and
can be managed from Settings → Models.

## Using Vox

Focus any text field, hold **Right Option** (the default), speak, and
release. A pill appears at the bottom-center of your primary display
showing the pipeline state:

- **Listening** — pulsing mic icon + live waveform from real mic RMS.
- **Transcribing** — spinner + label while Groq returns the raw text.
- **Polishing** — violet spinner while Claude Haiku cleans the transcript.
  Only shown when `cleanup_enabled` and an Anthropic key is set.
- **Pasted** — green check for 700 ms, then the pill fades out.
- **Error** — red alert for ~3 s with a short message; the pill is
  clickable in this state so you can dismiss it.

Press **Esc** (the default cancel hotkey) while recording to abort the
session without pasting.

The tray icon opens a **popover** with the current status, a dictation
on/off toggle, the last 5 transcripts (in-memory only, cleared on quit),
the active hotkey, and Settings / Quit buttons. Right-click still opens a
small native fallback menu.

`Cmd+,` from the Settings window focuses it.

## Settings

Open from the tray popover or the system menu-bar. Tabs:

- **General** — cleanup toggle, launch-at-login (stub), sound cues (stub),
  default microphone (stub).
- **Hotkeys** — push-to-talk (`AltRight`, `F13–F15`, `Cmd+Shift+Space`),
  cancel hotkey.
- **API Keys** — Groq and Anthropic fields, show/hide, Test buttons.
- **Dictionary** — proper nouns and technical terms appended to the LLM
  cleanup prompt so spellings are preserved.
- **Privacy** — cloud-dictation on/off, a summary of what leaves the
  device and what doesn't.
- **About** — version, Replay onboarding.

All changes save immediately. Hotkey rebinding requires a restart.

## Build a release `.app`

```bash
# Cloud-only build (fast, ~7 MB binary, no local STT/LLM):
npm run tauri build -- --bundles app

# Local-first build (links whisper.cpp + llama.cpp via Metal —
# requires Xcode CLT + cmake; ~2 min cold compile):
npm run tauri build -- --bundles app --features local-engines

# .app at src-tauri/target/release/bundle/macos/Vox.app
```

The built app is **not signed or notarized** yet. If macOS Gatekeeper
blocks it, control-click → Open once to trust it.

For local-first builds the hardened runtime needs
`com.apple.security.cs.allow-jit` (already present in
`src-tauri/entitlements.plist`) so the Metal shader cache can compile.

## Privacy

- Audio is never written to disk. It lives in a RAM buffer during the
  capture + pipeline window, then is freed.
- Transcripts are kept in memory only for the recent-5 tray popover list.
  They are cleared on app quit. No persistence to disk.
- No telemetry, no analytics, no crash uploads.
- API keys live in the macOS Keychain under `app.vox.vox`.

Data that does leave the device when you dictate:

- **Local STT (default):** nothing — audio stays on device.
- **Cloud STT:** audio is sent to Groq for Whisper transcription.
- **Local cleanup (default):** nothing — transcript stays on device.
- **Cloud cleanup:** transcript is sent to Anthropic for Claude Haiku.
- **Off:** transcript stays on device, no cleanup applied.

The Privacy tab shows a derived chip (LOCAL/CLOUD) for each direction
so you always know what's leaving your machine.

## Project layout

```
/Vox
├── src/                         — SolidJS frontend
│   ├── entries/
│   │   ├── settings.tsx         — Settings window entry
│   │   ├── pill.tsx             — Listening pill entry
│   │   ├── onboarding.tsx       — Onboarding wizard entry
│   │   └── tray-popover.tsx     — Tray popover entry
│   ├── views/
│   │   ├── Settings.tsx         — Tabbed settings
│   │   ├── Pill.tsx             — Pill UI + waveform
│   │   ├── Onboarding.tsx       — 5-step wizard
│   │   └── TrayPopover.tsx      — Tray popover UI
│   ├── lib/ipc.ts
│   └── styles.css
├── index.html, pill.html, onboarding.html, tray-popover.html
├── src-tauri/                   — Rust core
│   ├── src/
│   │   ├── main.rs, lib.rs
│   │   ├── audio.rs             — cpal capture + live RMS
│   │   ├── stt.rs               — Groq whisper-large-v3-turbo
│   │   ├── llm.rs               — Claude Haiku 4.5 cleanup
│   │   ├── inject.rs            — clipboard + Cmd+V paste
│   │   ├── hotkey.rs            — push-to-talk + cancel
│   │   ├── session.rs           — pipeline orchestrator
│   │   ├── pill.rs              — pill window + events + macOS panel style
│   │   ├── onboarding.rs        — onboarding window lifecycle
│   │   ├── transcripts.rs       — in-memory ring buffer
│   │   ├── settings.rs          — store + Keychain (keyring)
│   │   ├── commands.rs          — #[tauri::command] IPC handlers
│   │   └── tray.rs              — menu-bar icon + popover
│   ├── tauri.conf.json
│   ├── capabilities/default.json
│   └── Info.plist, entitlements.plist
├── package.json
├── vite.config.ts
└── tsconfig.json
```

## Phase 3 TODOs

- Real tray icon state animation (idle / recording / processing / error /
  permission-needed).
- Launch-at-login and default-microphone picker (currently stubs).
- Start/stop sound cues (currently stub).
- VAD-based silence trimming with `silero-vad-rs`.
- Offline fallback to `whisper.cpp` + Core ML.
- Auto-updater, code signing, notarization.
- Move the pill to the display containing the focused window (currently
  always primary).

## Debug

```bash
VOX_LOG=debug npm run tauri dev
```

Logs go to stdout. Non-secret settings live at
`~/Library/Application Support/com.vox.app/vox-settings.json`.
