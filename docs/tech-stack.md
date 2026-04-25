# Vox — Technical Stack Decisions

> Status: **v0.1 — CTO decision, engineering to implement against.**
> Target: macOS 13+ on Apple Silicon (M1–M5). Windows/Linux deferred.
> Date: 2026-04-24.

## Summary (the stack in one table)

| Concern | Choice | Rationale (1 line) |
| --- | --- | --- |
| App shell | Tauri 2.3+ (Rust core + WebView) | Non-negotiable; smallest footprint for native-feeling desktop app |
| Frontend framework | **SolidJS 1.9 + Vite + TypeScript** | Tiny bundle, no VDOM, reactive — perfect for a micro settings/tray UI |
| CSS | Tailwind CSS 4 | Trivial styling for a <10-screen settings surface |
| STT (primary) | **Groq `whisper-large-v3-turbo`** (cloud) | ~200x realtime, ~$0.04/hr, sub-second round trip on short utterances |
| STT (fallback) | `whisper.cpp` + CoreML `small.en` | Offline mode; ANE gives 3–4x speedup, keeps app usable without network |
| LLM cleanup | **Anthropic Claude Haiku 4.5** | ~0.74s TTFT, 80–120 tok/s, $1/$5 per MTok — fastest quality-reliable model |
| LLM fallback | Groq `llama-3.3-70b-versatile` | Even faster tok/s if Anthropic is down; quality drop acceptable for cleanup |
| Microphone | `cpal` 0.15 (CoreAudio backend) | De-facto standard Rust audio I/O crate |
| VAD | `silero-vad-rs` 0.2 (ONNX via `ort` 2.0) | Tiny model, way better than amplitude thresholds for noisy rooms |
| Global hotkey | `tauri-plugin-global-shortcut` 2.3+ | Official plugin; push-to-talk pattern avoids Fn-key limitation |
| Text injection | **Clipboard-save + CGEvent Cmd+V + clipboard-restore** | Only approach that works reliably across Electron/Chrome/native/web-views |
| Input simulation crate | `enigo` 0.5 (for keystrokes) + `arboard` 3 (clipboard) | Both pure Rust, no Objective-C bridging needed in app code |
| Accessibility check | `tauri-plugin-macos-permissions` 2.0 | Clean API to prompt/check Accessibility + Microphone TCC |
| Settings store | `tauri-plugin-store` 2.4 (JSON on disk) | Dead simple, typed, survives app updates |
| Secrets | `keyring` 3 crate → macOS Keychain | API keys never touch disk in plaintext |
| Tray / menu bar | Built-in Tauri 2 `TrayIconBuilder` + `ActivationPolicy::Accessory` | No Dock icon, menu-bar-only — matches Whispr Flow UX |
| Packaging | Tauri bundler → `.dmg`, `codesign` + `notarytool` | Standard Apple distribution chain |
| CI | GitHub Actions `macos-14` (arm64) runners | Native Apple Silicon builds & notarization |

---

## Architecture overview

```
                        +-----------------------------------------------+
                        |                    Vox (Rust core)            |
                        |                                               |
 user holds             |  +------------------+   +------------------+  |
 Right-Option  ------>  |  | global-shortcut  |-->| session manager  |  |
 (push-to-talk)         |  | plugin           |   | (tokio task)     |  |
                        |  +------------------+   +---+----------+---+  |
                        |                             |          |      |
                        |              release key    |          |      |
                        |  +------------------+       v          v      |
 macOS microphone  -->  |  | cpal capture     |   +---------+  +------+ |
 (16 kHz mono f32)      |  | ring buffer      |-->| silero  |  |pre-  | |
                        |  +------------------+   | VAD     |  |roll  | |
                        |                         +----+----+  |buf   | |
                        |                              |       +------+ |
                        |                              v                |
                        |   +-----------------------------------+       |
                        |   | STT: Groq whisper-large-v3-turbo  |       |
                        |   |  (HTTPS, multipart wav upload)    |       |
                        |   +-----------------+-----------------+       |
                        |                     |                         |
                        |                     v                         |
                        |   +-----------------------------------+       |
                        |   | LLM cleanup: Claude Haiku 4.5     |       |
                        |   | (streaming, ~400 ms)              |       |
                        |   +-----------------+-----------------+       |
                        |                     |                         |
                        |                     v                         |
                        |   +-----------------------------------+       |
                        |   | Clipboard swap + CGEvent Cmd+V    |       |
                        |   | (enigo + arboard)                 |       |
                        |   +-----------------+-----------------+       |
                        +-----------------------|---------------------+
                                                v
                                    focused text field
                                    in ANY macOS app
                        +-----------------------------------------------+
                        |  WebView (SolidJS)                            |
                        |  - Settings window (⌘,)                       |
                        |  - Tray menu bar icon + popover               |
                        |  - Permission onboarding flow                 |
                        |  talks to core via Tauri IPC `invoke`         |
                        +-----------------------------------------------+
```

---

## Detailed decisions

### 1. Frontend framework — **SolidJS**

- **Choice:** SolidJS 1.9 + Vite 6 + TypeScript 5.6 + Tailwind CSS 4.
- **Alternatives considered:** React (familiar, heavy), Svelte 5 (great but runes churn), vanilla (fine but loses reactivity ergonomics), Preact (smaller React but still VDOM).
- **Rationale:** The web surface is tiny — a settings panel, onboarding wizard, tray popover. SolidJS compiles to direct DOM updates, no VDOM, ~7 kB runtime, JSX syntax that any React-capable engineer can read on day one. Startup of the WebView is measurably faster than React, which matters when the user opens the tray popover and expects instant render. Ecosystem (solid-router, @kobalte/core for a11y primitives) is sufficient for our needs.
- **Bundle expectation:** <80 kB gzipped for the full settings UI.

### 2. Speech-to-text — **Groq `whisper-large-v3-turbo` (primary), whisper.cpp (offline fallback)**

- **Primary: Groq** `whisper-large-v3-turbo` over HTTPS.
  - ~200x realtime, so a 10-second utterance transcribes in ~50–100 ms server-side plus ~150 ms RTT. Effectively imperceptible.
  - Pricing: ~$0.04/hr of audio — an order of magnitude cheaper than OpenAI Whisper ($0.36/hr).
  - Same model family as Whisper large-v3, so WER is effectively identical to OpenAI.
- **Alternatives considered:**
  - *OpenAI Whisper API:* ~2 s median latency on short clips. Disqualifying for the critical path.
  - *Deepgram Nova-3 streaming:* sub-300 ms TTFT, but priced at ~$0.46/hr streaming (~11x Groq). Reserve for a future "live dictation" mode where we want token-by-token preview.
  - *whisper.cpp local:* great for offline, but even `small.en` on M3 is ~1.5x realtime cold — too slow vs Groq on warm network.
- **Fallback plan:** Ship `whisper.cpp` + Core ML `ggml-small.en-encoder.mlmodelc` bundled in the app resources. When Groq call fails or user toggles "Offline mode," we run locally via `whisper-rs` 0.13 bindings. 3–4x CPU speedup via Metal, 8–12x with CoreML+Metal combined on Apple Silicon. Accuracy drop from large-v3-turbo → small.en is real but tolerable for a fallback.
- **Audio format sent to Groq:** 16 kHz mono WAV, PCM16. Under the 25 MB limit we will effectively never hit (that's ~4 hours of audio at 16 kHz).
- **Key libraries:** `reqwest` 0.12 with `multipart` feature, `hound` 3.5 for WAV encoding.

### 3. LLM cleanup — **Anthropic Claude Haiku 4.5**

- **Choice:** `claude-haiku-4-5` via Anthropic Messages API with streaming enabled.
- **Alternatives considered:**
  - *GPT-4o-mini:* solid but Anthropic Haiku has better TTFT and our prompt engineering is stronger with Claude.
  - *Groq llama-3.3-70b:* blistering tok/s, but cleanup of dictation benefits from strong instruction following — Haiku wins on "don't paraphrase, just clean."
  - *Local SLM (Gemma 2B / Phi-3.5-mini via llama.cpp):* adds gigabytes of model weights to the installer, and cold-start on first dictation is bad UX.
- **Rationale:** Claude Haiku 4.5 TTFT ~740 ms, output speed 80–120 tok/s. A typical cleanup output is 30–80 tokens → end-to-end ~1.0–1.3 s *non-streamed*, but we stream and paste as tokens arrive, so the user sees first characters in ~0.8 s. Pricing ($1 in / $5 out per MTok) is negligible for our volume. We will also cache a system prompt (prompt caching cuts repeat-input to $0.10/MTok).
- **Fallback:** Groq `llama-3.3-70b-versatile` on Anthropic 5xx/timeout. Same prompt, quality degradation is acceptable for a fallback.
- **Critical path budget:**
  - Mic → STT: ~300 ms (network + Groq inference)
  - STT → LLM first token: ~800 ms (Haiku TTFT)
  - LLM first token → paste start: ~20 ms (IPC + clipboard)
  - **User-perceived "stop talking → text appearing": ~1.1 s target, 1.5 s ceiling.**

### 4. Microphone capture — **`cpal` 0.15**

- **Confirmed.** `cpal` is the right choice. CoreAudio backend is stable; April 2026 commit activity confirms active maintenance. Requires Rust 1.80+.
- Capture config: 16 kHz mono `f32` → convert to `i16` PCM before upload. Use a `ringbuf` 0.4 ring buffer sized for ~60 s of audio (enough for any reasonable dictation plus 5 s pre-roll).
- Permission: macOS microphone TCC is prompted on first `build_input_stream` call; the Info.plist `NSMicrophoneUsageDescription` must be set in `tauri.conf.json` → `bundle.macOS`.

### 5. Global hotkey — **`tauri-plugin-global-shortcut` 2.3+**

- **Status:** Official, stable in Tauri 2.x. Works reliably for standard keys and modifier combos on macOS. Known community issues around first-run panics are tied to missing Accessibility permission — we handle that explicitly.
- **Fn key caveat:** The Fn/Globe key on modern Apple keyboards is *not* reliably capturable through CGEventTap-based APIs (it is consumed by the system before reaching hotkey handlers). **We will not try to bind Fn.** Default binding: **hold Right-Option** (push-to-talk) with a user-configurable alternative (e.g. `Cmd+Shift+Space`).
- **Implementation pattern:** push-to-talk, not toggle. On `ShortcutEvent::Pressed`, start capture; on `ShortcutEvent::Released`, stop and run the pipeline. This is clearer UX than a toggle and avoids "stuck recording" bugs.
- **Permission story:** Accessibility (`kTCCServiceAccessibility`) is required for a hotkey plugin on macOS. We detect missing permission with `tauri-plugin-macos-permissions` and show a first-run onboarding screen with a "Grant Accessibility Access" button that opens System Settings → Privacy → Accessibility.

### 6. Text injection into other apps — **Clipboard-swap + synthetic Cmd+V**

- **Choice:** The Whispr Flow / Superwhisper / Wispr Flow approach:
  1. Save current clipboard contents (`arboard::Clipboard::get_text`).
  2. Write cleaned transcript to clipboard.
  3. Synthesize ⌘V via `CGEventPost` (through `enigo::Keyboard::key(Key::Unicode('v'), Direction::Click)` with Cmd held, or raw `core-graphics` calls).
  4. Sleep ~80 ms, restore original clipboard.
- **Why not AXUIElement direct insert:** `AXUIElementSetAttributeValue(kAXValueAttribute, ...)` works in AppKit `NSTextField` but silently fails or misbehaves in Electron apps (VS Code, Slack, Discord, Notion), Chrome content editables, and most web `<textarea>`s. Whispr Flow needs to work *everywhere*, so we accept the clipboard approach.
- **Why not AppleScript:** works but requires `osascript` subprocess (slow), cannot target arbitrary frontmost app reliably, and triggers a second permission prompt (Automation).
- **Why not pure `enigo`:** `enigo.text("...")` on macOS types character-by-character via synthetic keystrokes. Unicode, emoji, and dead-key languages break. `text()` is also *slow* for long strings (~30–50 ms for a paragraph). Cmd+V is instant.
- **Permissions required:** Accessibility (for `CGEventPost`). Same permission as the hotkey, so one grant covers both.
- **UX tradeoffs we accept:**
  - Brief clipboard hijack window (~80 ms). Some clipboard managers (Paste, Maccy) may catch and log the transcript — we document this.
  - Will not work in sandboxed Mac App Store distribution. **We ship via Developer ID + notarization, not MAS.** Decided.
- **Crates:** `enigo` 0.5 for the keystroke, `arboard` 3 for clipboard. Both pure Rust.

### 7. State & settings storage — **`tauri-plugin-store` + `keyring` for secrets**

- Non-secret settings (hotkey binding, enabled microphone, LLM cleanup style preset, telemetry opt-in) → `tauri-plugin-store` 2.4. JSON file at `~/Library/Application Support/com.vox.app/settings.json`.
- Secrets (Groq API key, Anthropic API key) → `keyring` 3 crate, which writes to the login Keychain under service `com.vox.app`, account `groq` / `anthropic`. Never serialized to disk, never logged, never crossing the IPC boundary to JS unless the user is actively editing on the settings screen (then masked behind `type="password"` + blur on paste).

### 8. Tray icon / menu bar — **Built-in Tauri 2 `TrayIconBuilder`**

- **Confirmed.** Tauri 2 ships first-class tray support.
- Set `tauri::ActivationPolicy::Accessory` under `#[cfg(target_os = "macos")]` in `setup()` so the app has **no Dock icon** — menu-bar-only, matching Whispr Flow.
- Tray icon states: idle (mic outline), recording (mic filled, red), processing (spinner), error (mic with exclamation).
- Left-click: open a small popover (a WebView window configured as `decorations: false, skip_taskbar: true, always_on_top: true, transparent: true`). Right-click: native context menu with Settings / Pause / Quit.
- Reference implementation: `ahkohd/tauri-macos-menubar-app-example`.

### 9. Audio VAD / silence detection — **Silero VAD via `silero-vad-rs`**

- **Choice:** `silero-vad-rs` 0.2 (ONNX model loaded through `ort` 2.0).
- **Alternatives considered:**
  - *Amplitude threshold:* fails in open offices, coffee shops, HVAC noise. Disqualifying.
  - *webrtc-vad (GMM-based):* old, noisy, false-positive-prone. Silero is strictly better and similar CPU cost on Apple Silicon.
- **Usage pattern:** We actually do *not* need VAD for the primary push-to-talk path (the hotkey release is the stop signal). VAD is used for:
  1. **Auto-stop** if the user locks down push-to-talk and walks away (safety net at 60 s).
  2. **Trim leading/trailing silence** before sending to Groq (saves tokens on the LLM step and sharpens Whisper's first-word accuracy).
  3. Future "hands-free" mode with wake-word/voice-start detection.
- ONNX Runtime 1.22.x required — pin it in `Cargo.toml` to avoid breakage.

### 10. Build + packaging — **Tauri bundler + Developer ID + notarytool**

- `tauri build` produces a universal2 `.app` → wrap in a DMG.
- Hardened runtime entitlements required:
  - `com.apple.security.device.audio-input` (microphone)
  - `com.apple.security.automation.apple-events` (only if we fall back to AppleScript; probably omit)
  - *No* `com.apple.security.app-sandbox` (we need CGEventPost to foreign apps).
- Code sign with Developer ID Application cert → notarize with `xcrun notarytool submit --wait` → staple with `xcrun stapler staple`.
- Auto-updates via `tauri-plugin-updater` 2.3 with Ed25519-signed release manifests hosted on GitHub Releases or a simple S3 bucket.
- Build arch: `--target aarch64-apple-darwin` first; `--target universal-apple-darwin` for public release.

---

## Critical risks

### Risk 1 — Accessibility permission friction kills activation

Users must grant Accessibility *and* Microphone on first run. Every dropped grant is a churned user. **Mitigation:** a dedicated onboarding screen that (a) explains *why* each permission is needed in one sentence, (b) shows a live "✓ Granted / ✗ Not yet" indicator per permission polled every 500 ms via `tauri-plugin-macos-permissions`, (c) deep-links directly to the right System Settings pane (`x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility`). Do **not** let the user proceed until both are granted — silent failures are much worse than a blocked wizard.

### Risk 2 — Clipboard-swap races & conflicts with clipboard managers

If the user has Paste.app / Maccy / Raycast clipboard history, our transcript will be captured. Worse: if a clipboard manager writes back during our 80 ms restore window, we corrupt the user's clipboard. **Mitigation:** (a) document the behavior in settings ("Vox briefly uses your clipboard to paste — install X if you want transcripts excluded"); (b) bump restore delay to 150 ms and verify the clipboard still holds *our* text before overwriting; (c) expose a "Never touch clipboard, type instead" fallback mode that uses `enigo.text()` — slower but clipboard-safe. Consider this the escape hatch, not the default.

### Risk 3 — LLM/STT cloud tail latency blows the UX budget

Groq and Anthropic are fast at median but tail latency (p99) can hit 3–5 s on bad days. Above ~2 s, the user starts wondering if it crashed. **Mitigation:** (a) show a live processing indicator in the tray icon and optionally a small HUD overlay; (b) hard-timeout each cloud call at 4 s and fall back (Groq→whisper.cpp local; Anthropic→Groq Llama); (c) emit metrics (opt-in) for `stt_ms`, `llm_ms`, `paste_ms` so we can tune; (d) prompt-cache the LLM system message to stabilize TTFT; (e) begin pasting the *raw Whisper output* if the LLM hasn't returned first-token by 1.5 s — user sees *something* happen.

---

## Dev prerequisites

- **macOS 14+ on Apple Silicon** (M1 minimum; CI runs on `macos-14`).
- **Xcode 16+** with Command Line Tools (`xcode-select --install`). Needed for signing, linking against system frameworks, and Core ML model compilation.
- **Rust 1.82+** via `rustup`. Toolchain components: `rustfmt`, `clippy`, targets `aarch64-apple-darwin` and `x86_64-apple-darwin` (for universal builds).
- **Node.js 22 LTS** + **pnpm 9** (lockfile committed, monorepo-friendly).
- **Tauri CLI:** `cargo install tauri-cli --version "^2.3"` and/or `pnpm add -D @tauri-apps/cli@^2`.
- **ONNX Runtime** (for Silero VAD): bundled via `ort` crate's `download-binaries` feature; no system install required.
- **Apple Developer account** with a Developer ID Application certificate in the login Keychain. `notarytool` credentials stored via `xcrun notarytool store-credentials`.
- **API keys** exported into dev-only `.env.local` (gitignored): `GROQ_API_KEY`, `ANTHROPIC_API_KEY`. In production they live in Keychain.
- **One-liner bootstrap:** see `scripts/bootstrap.sh` (to be written) — installs rustup toolchain, pnpm, tauri-cli, and verifies Xcode.

---

## Project layout (recommended)

```
/Vox
├── src-tauri/                         # Rust core
│   ├── Cargo.toml
│   ├── tauri.conf.json                # bundle ID, Info.plist, capabilities
│   ├── build.rs
│   ├── capabilities/
│   │   └── default.json               # plugin permission manifest
│   ├── icons/                         # menu-bar icon states
│   └── src/
│       ├── main.rs                    # Tauri bootstrap, ActivationPolicy::Accessory
│       ├── commands.rs                # #[tauri::command] IPC handlers
│       ├── hotkey.rs                  # global-shortcut plugin wiring
│       ├── audio/
│       │   ├── mod.rs
│       │   ├── capture.rs             # cpal input stream
│       │   ├── ring.rs                # lock-free ring buffer
│       │   └── vad.rs                 # silero-vad-rs wrapper
│       ├── stt/
│       │   ├── mod.rs                 # trait SttProvider
│       │   ├── groq.rs                # primary
│       │   └── whisper_local.rs       # whisper-rs fallback
│       ├── llm/
│       │   ├── mod.rs                 # trait LlmProvider
│       │   ├── anthropic.rs
│       │   └── groq_llama.rs
│       ├── inject/
│       │   ├── mod.rs
│       │   ├── clipboard_paste.rs     # default strategy
│       │   └── type_fallback.rs       # enigo.text() fallback
│       ├── permissions.rs             # macOS TCC checks
│       ├── secrets.rs                 # keyring wrapper
│       ├── settings.rs                # tauri-plugin-store wrapper
│       ├── tray.rs                    # TrayIconBuilder + state
│       └── session.rs                 # end-to-end pipeline orchestrator
│
├── src/                               # SolidJS frontend
│   ├── index.html
│   ├── main.tsx
│   ├── App.tsx
│   ├── routes/
│   │   ├── onboarding.tsx
│   │   ├── settings.tsx
│   │   └── tray-popover.tsx
│   ├── components/
│   ├── lib/
│   │   ├── ipc.ts                     # typed invoke<T>() wrappers
│   │   └── stores.ts                  # Solid stores
│   └── styles.css                     # Tailwind entry
│
├── resources/
│   ├── whisper-small-en-encoder.mlmodelc/
│   ├── whisper-small-en.bin
│   └── silero_vad.onnx
│
├── scripts/
│   ├── bootstrap.sh
│   ├── sign-and-notarize.sh
│   └── release.sh
│
├── .github/workflows/
│   ├── ci.yml                         # macos-14, lint + test
│   └── release.yml                    # build, sign, notarize, publish DMG
│
├── docs/
│   ├── tech-stack.md                  # ← this file
│   ├── permissions.md
│   └── pipeline-budget.md
│
├── package.json                       # pnpm workspace root
├── pnpm-lock.yaml
├── vite.config.ts
├── tailwind.config.ts
├── tsconfig.json
└── README.md
```

---

## Open questions for the team (call these out in the first sprint plan)

1. **Streaming LLM output → incremental paste?** Viable but complicates clipboard-swap (we'd need to type chars instead). Recommended: v1 waits for full LLM response (simpler, ~1.2 s); v2 streams via `enigo.text()` to reduce perceived latency.
2. **Telemetry / error reporting:** opt-in Sentry vs custom PostHog vs nothing? Decide before public beta.
3. **Licensing:** Is the app free / paid / freemium? Affects whether we need license-key validation (probably `keygen.sh` if paid).
4. **Model distribution for offline fallback:** ship `~250 MB` of whisper weights in the DMG vs on-demand download on first offline trigger? Leaning toward on-demand with a clear prompt.
