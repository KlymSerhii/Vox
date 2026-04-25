# Vox

A macOS menu-bar dictation app. Hold a hotkey, speak, and your words are
transcribed via Groq Whisper and pasted into the currently focused text field.

This is the **Phase 1** scaffold — the end-to-end core loop. LLM cleanup, the
listening pill, the tray popover, onboarding, and Keychain-backed secrets are
all Phase 2.

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

On first launch, macOS will ask you to grant:

1. **Microphone** — required for dictation. Granted once on the first recording.
2. **Accessibility** — required to register a global hotkey and to simulate
   the Cmd+V paste. macOS will NOT prompt for this automatically; open
   **System Settings → Privacy & Security → Accessibility** and add Vox
   (or the built app bundle), then restart the app.

Once granted, click the tray icon → **Open Settings**, paste your Groq API
key, and hit **Save**. Use **Test connection** to verify the key.

Now focus any text field (Slack, Notes, VS Code, a browser form) and hold
**Right Option** (or your chosen hotkey). Speak. Release. Your transcript is
pasted.

## Build a release `.app`

```bash
npm run tauri build
# .app at src-tauri/target/release/bundle/macos/Vox.app
# .dmg  at src-tauri/target/release/bundle/dmg/
```

The built app is **not signed or notarized** in Phase 1. If macOS Gatekeeper
blocks it, control-click → Open once to trust it.

## Hotkey notes

The default is **Right Option** (`AltRight`). This works via
`tauri-plugin-global-shortcut`. If you find your keyboard layout doesn't
surface `AltRight` as a raw code (some external keyboards map it differently),
pick one of these alternatives in Settings:

- `Cmd+Shift+Space` (chord, most portable)
- `F13`, `F14`, `F15` (unassigned function keys, great on full-size keyboards)

The app logs which hotkey actually registered at `VOX_LOG=info` level.

## Project layout

```
/Vox
├── src/                  — SolidJS settings UI (TypeScript + Tailwind 4)
├── src-tauri/            — Rust core
│   ├── src/
│   │   ├── main.rs
│   │   ├── lib.rs        — Tauri setup, ActivationPolicy::Accessory
│   │   ├── audio.rs      — cpal capture on a dedicated thread + WAV encode
│   │   ├── stt.rs        — Groq whisper-large-v3-turbo HTTP client
│   │   ├── inject.rs     — clipboard + enigo Cmd+V paste
│   │   ├── hotkey.rs     — global-shortcut wiring
│   │   ├── session.rs    — pipeline orchestrator
│   │   ├── settings.rs   — tauri-plugin-store wrapper
│   │   ├── commands.rs   — #[tauri::command] IPC handlers
│   │   └── tray.rs       — menu-bar icon + menu
│   ├── tauri.conf.json
│   ├── capabilities/default.json
│   └── Info.plist, entitlements.plist
├── package.json
├── vite.config.ts
├── tsconfig.json
└── index.html
```

## Phase 2 TODOs

- Secrets → macOS Keychain (`keyring` crate), not JSON on disk.
- LLM cleanup step (Claude Haiku 4.5).
- Listening pill (bottom-center floating window).
- Tray popover with recent transcripts.
- Full tabbed settings window (General / Hotkeys / API Keys / Dictionary /
  Privacy / About).
- Onboarding wizard with live permission polling.
- VAD (silero-vad-rs) for silence trimming and auto-stop.
- Offline fallback to `whisper.cpp` + Core ML.
- Auto-updater, code signing, notarization.

## Debug

```bash
VOX_LOG=debug npm run tauri dev
```

Logs go to stdout. The app writes settings to
`~/Library/Application Support/com.vox.app/vox-settings.json`.
