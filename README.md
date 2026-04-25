# Vox

A macOS menu-bar dictation app. Hold a hotkey, speak, and your words are
transcribed via Groq Whisper, optionally polished by Claude Haiku 4.5, and
pasted into the currently focused text field.

This is the **Phase 2** build: core dictation loop plus LLM cleanup, a
floating listening pill, a tabbed settings window, a tray popover, a
first-run onboarding wizard, and macOS Keychain-backed API keys.

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
3. **API keys** — Groq (required) and Anthropic (optional, enables cleanup).
4. **Try it** — a live text field; hold the hotkey and speak.

Keys are written to the macOS Keychain under service `app.vox.vox`
(accounts `groq` and `anthropic`). If Keychain access is denied we fall
back to the plaintext settings store and log a warning.

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
npm run tauri build -- --bundles app
# .app at src-tauri/target/release/bundle/macos/Vox.app
```

The built app is **not signed or notarized** yet. If macOS Gatekeeper
blocks it, control-click → Open once to trust it.

## Privacy

- Audio is never written to disk. It lives in a RAM buffer during the
  capture + pipeline window, then is freed.
- Transcripts are kept in memory only for the recent-5 tray popover list.
  They are cleared on app quit. No persistence to disk.
- No telemetry, no analytics, no crash uploads.
- API keys live in the macOS Keychain under `app.vox.vox`.

Data that does leave the device when you dictate:

- Audio is sent to Groq for Whisper transcription.
- The resulting transcript is sent to Anthropic for Claude Haiku cleanup
  **only if** cleanup is enabled and an Anthropic key is set.

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
