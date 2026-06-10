# Vox

Fully on-device dictation for **macOS and Windows**. Press a hotkey, speak, and the transcript lands on your clipboard ready to paste (`⌘V` on macOS, `Ctrl+V` on Windows).

This file is the canonical project reference. The README that used to live at the project root was removed when the app pivoted to a clipboard-only, local-only build; the surviving `docs/liquid-glass-*.md` files are design notes for the existing light/dark UI tokens, not the project overview.

---

## What it is today

- **macOS menu-bar app** (no Dock icon, `ActivationPolicy::Accessory`) and **Windows system-tray app**. Same Rust core + frontend on both; OS-specific behaviour is `cfg`-gated in Rust and branched via `src/lib/platform.ts` (`isMac` / `isWindows` / `deviceWord`) in the UI. Windows is **x86_64 only** in v1 (runs on Windows-on-ARM under x64 emulation; native ARM64 is wired but not shipped — see Platforms).
- **Tauri 2** Rust core + **SolidJS** + **Tailwind 4** frontend.
- **Two STT engines, user-pickable in onboarding and Settings → Models:**
  - **Whisper** in-process via `whisper-rs` (cross-platform: CoreML+Metal on macOS, CPU on Windows). Always the safe fallback.
  - **Parakeet TDT 0.6B v3** (NVIDIA, multilingual / 25 European languages) — recommended over Whisper wherever it's available. It runs in-process two different ways, selected by `cfg(target_os)` behind the **same** engine id `parakeet-local` and model id `parakeet-tdt-0.6b-v3`:
    - **macOS (Apple Silicon):** the **FluidAudio** Swift package (Core ML + Apple Neural Engine), compiled by `build.rs::build_parakeet_bridge` from `src-tauri/swift-parakeet/` and called through `engines::parakeet_bridge` → `engines::parakeet_local`. On-disk artifact: the FluidAudio Core ML bundle.
    - **Windows:** **ONNX Runtime via the `parakeet-rs` crate** (`ort` 2.0-rc, pinned) with the **DirectML** execution provider (GPU on any DX12 box) and an automatic **CPU-EP fallback**. On-disk artifact: the ONNX bundle from `istupakov/parakeet-tdt-0.6b-v3-onnx` (~2.5 GB). `commands::parakeet_availability` reports `available` on Windows whenever the `parakeet` feature is compiled in.
- **Two polishing engines, user-pickable in onboarding and Settings → Models:**
  - **Apple Intelligence** via Apple's `FoundationModels` framework. Implemented as an in-process Swift FFI bridge (`swift/AppleFmBridge.swift` compiled to a static library by `build.rs::build_apple_fm_bridge`, called from `engines::apple_fm` through `engines::apple_fm_bridge`). Recommended default on Macs with macOS 26+ Apple Silicon and Apple Intelligence enabled in System Settings. No download. Apple's safety guardrails can refuse certain dictation; the engine returns a clear error and `session.rs` fail-opens to the raw Whisper transcript.
  - **Gemma 4** out-of-process via a downloaded `llama-server` binary we talk to over loopback HTTP (`engines::gemma_http`). Default fallback when Apple Intelligence is unavailable, and the only option on macOS < 26.
- All inference is on-device. macOS: Metal / Core ML on Apple Silicon, plus the Apple Neural Engine and `modelinferenced` system daemon for Foundation Models. Windows: DirectML (any DX12 GPU) or CPU for Parakeet ONNX, CPU for Whisper, and the `llama-server` subprocess for Gemma. Models live on disk; nothing leaves the device.
- **Cloud is gone.** No Groq, no Anthropic, no API keys, no telemetry, no analytics. The only network calls remaining are model + `llama-server` downloads from Hugging Face / GitHub and the auto-updater pulling release manifests from the `KlymSerhii/vox-releases` GitHub release (minisign-pinned, see `tauri.conf.json::plugins.updater`).
- **Output is the clipboard by default.** The transcript replaces the clipboard contents and the user pastes manually — no synthetic paste, no extra permission required for the default pipeline. Users who want hands-off insertion can opt into **auto-paste** in Settings → General → Text insertion; that path synthesises a paste keystroke from inside `session::SessionManager::deliver_transcript` after polish completes, then restores the user's prior clipboard text ~250 ms later. A rapid second dictation that lands inside the restore window cancels the previous restore and inherits its captured prior text, so a chain of pipelines still ends with the user's true pre-chain clipboard. The synth path is OS-split: **macOS** uses `mac_paste::post_cmd_v` (`⌘V` at the HID event tap via `CGEventPost`), which needs Accessibility — when the toggle is on but the grant is missing, `deliver_transcript` fail-opens to clipboard-only and the toast reads "Copied — grant Accessibility for auto-paste"; **Windows** uses `win_paste::post_ctrl_v` (`Ctrl+V` via `SendInput`), which needs **no** Accessibility-style grant (a normal-integrity process can inject into another normal-integrity foreground window; the one failure mode is a higher-integrity/elevated foreground window silently dropping the keystroke, which still fail-opens to clipboard-only).

## Platforms

The app targets **macOS (Apple Silicon, aarch64)** and **Windows (x86_64)** from one codebase. OS-specific Rust lives behind `#[cfg(target_os = "…")]`; OS-specific UI copy branches on `src/lib/platform.ts` (`isMac` / `isWindows`, and `deviceWord` = `"Mac"` / `"PC"` for sentences like "runs entirely on this PC"). Do not hard-code "Mac" / "Apple Silicon" in user-facing strings — use `deviceWord` / `isMac`.

What differs by OS:

- **Polishing.** macOS gets Apple Intelligence (default on macOS 26+) *or* Gemma; **Windows is Gemma-only** (no Apple Intelligence analogue — see Known limitations). The Gemma `llama-server` pipeline is identical on both.
- **Parakeet STT backend.** FluidAudio (Core ML / ANE) on macOS vs ONNX Runtime + DirectML on Windows — same engine id, different `cfg`-selected impl and on-disk bundle (see "What it is today").
- **Auto-paste.** `mac_paste` (`⌘V`, needs Accessibility) vs `win_paste` (`Ctrl+V` via `SendInput`, no grant). The `session.rs` finishing step has a three-arm `cfg` (`macos` / `windows` / other → "unsupported").
- **Pill window chrome.** The floating pill is a transparent, decoration-less window on both, but the OS drop shadow is **disabled on Windows** (`pill.rs` passes `.shadow(!cfg!(target_os = "windows"))`) because Windows DWM draws the shadow around the rectangular HWND, ignoring the content's rounded alpha — it would paint a visible rectangle around the pill. Depth comes from the inner CSS `box-shadow` there. With live-transcript on, `session.rs` calls `pill::set_wide(true)` at recording start on Windows so the wide listening layout doesn't clip while the slower WebView2 → DWM resize round-trip catches up.
- **Tray popover placement.** `tray.rs` opens the popover **above** the tray icon when the icon sits in the bottom half of the display (Windows default taskbar) and **below** when it's in the top half (macOS menu bar). The decision is dynamic on the icon's Y, so a relocated Windows taskbar flips it automatically. The macOS menu-bar height (~22 px) is only subtracted from the height clamp on macOS.
- **Hotkey path.** macOS binds the default `⌘⌥.` via Carbon `RegisterEventHotKey` (no Accessibility); Windows binds through the same `tauri-plugin-global-shortcut` surface.

**Windows build toolchain.** `npm run tauri build`/`dev` on Windows needs: an MSVC dev shell (`cl.exe`/`link.exe` on PATH — the x64 Native Tools / `arm64_x64` Developer prompt on Windows-on-ARM), `LIBCLANG_PATH` pointing at a 64-bit LLVM (`whisper-rs-sys` bindgen), and network egress (the `ort` crate's `download-binaries` + `copy-dylibs` features fetch a prebuilt `onnxruntime.dll` + `DirectML.dll` from `cdn.pyke.io` into the target dir at build time — those DLLs must ship next to `vox.exe`). On Windows-on-ARM the Rust toolchain is x86_64 via `rustup default stable-x86_64-pc-windows-msvc --force-non-host` so build scripts and the final binary are all x64; `CARGO_BUILD_TARGET` must stay unset.

## Defaults

| Setting | Default | Notes |
| --- | --- | --- |
| `hotkey` | `Cmd+Alt+Period` (⌘⌥.) | User-pickable in Settings → Hotkeys; defaults apply on first launch via `default_hotkey()` and `#[serde(default)]`. Bound via `tauri-plugin-global-shortcut` (Carbon `RegisterEventHotKey`). No Accessibility required. |
| `cancel_hotkey` | `Escape` | Aborts the in-flight session. The OS grab is acquired only while a session is active (`hotkey::arm_cancel` at `session::begin_recording`, `hotkey::disarm_cancel` from `PipelineGuard::drop`) so Escape works normally in every other app when Vox isn't dictating. |
| `mode_cycle_hotkey` | `Alt+M` | Advances through Voice Modes. |
| `hands_free_enabled` | `true` | Press once to start, again to stop. |
| `wake_word_enabled` | `false` | Opt-in "Hey Vox" voice trigger (Settings → General → "Start dictation by voice"). `wake.rs` keeps a low-cost mic capture + energy VAD running on a dedicated thread while Vox is idle, transcribes each short utterance with the user's selected STT engine (the same cached `Registry` instance the pipeline uses — no extra model RAM), and fuzzy-matches the phrase. On a match it releases its stream, starts a normal session via `SessionManager::begin`, and auto-stops the recording after ~1.5 s of trailing silence so the whole flow is hands-free. The listener suspends (mic fully released) whenever a session is recording or a pipeline is in flight, when dictation/the toggle is off, or when mic permission is missing — it never triggers the TCC prompt itself. Capture buffer is RAM-only, capped at a few seconds, never persisted. |
| `wake_phrase` | `"hey vox"` | The spoken trigger phrase. Matched fuzzily (per-token Levenshtein + a joined fallback) so STT near-homophones like "hey fox" / "heyvox" still trigger; blank falls back to the default. `wake::refresh` re-snapshots the listener's config after `set_settings`, `set_dictation_enabled`, `set_audio_input_device`, and `finish_onboarding`. |
| `dictation_enabled` | `true` | Master kill-switch from the tray popover. |
| `stt_engine` / `stt_model_id` | `whisper-local` / tier-recommended on disk; onboarding's LanguageStep flips to `parakeet-local` / `parakeet-tdt-0.6b-v3` when `parakeet_availability` reports `available` (Apple Silicon, or Windows with the `parakeet` feature) and the chosen language is one of Parakeet's 25 | Auto-tier via `recommend_tier`. `recommendedSttEngineFor(language, parakeetAvailable)` in `ipc.ts` routes unsupported languages (and any host where Parakeet is unavailable) to `whisper-local`. Engine id is OS-agnostic — the Settings doc round-trips between a Mac and a PC. |
| `llm_engine` / `llm_model_id` | `gemma-local` / tier-recommended on disk; onboarding flips to `apple-fm` when `apple_fm_availability` reports `available` | `apple-fm` ignores the `llm_model_id` field but we still keep a tier-matched Gemma id there so a later switch in Settings doesn't have to re-pick one. |
| `polishing_enabled` | `true` | UI toggle. Maps to `llm_engine ∈ {gemma-local, apple-fm, none}`. Toggling off only sets `polishing_enabled = false` — `llm_engine` is preserved so a user with Apple Intelligence selected gets it back when they toggle on (lockstep behaviour lives in `session.rs` short-circuiting to the no-op engine when polishing is off). |
| `enable_thinking` | `false` | Gemma 4 only. When on, Gemma reasons silently before producing the polished output (slower, can improve quality on complex inputs). Sent as `chat_template_kwargs: {"enable_thinking": true}` per request. Apple Intelligence has no equivalent knob and ignores the value. |
| `language` | `auto` | Whisper language hint; `auto` lets Whisper detect. Also forwarded to the cleanup LLM: when pinned (not `auto`), `session::with_language_directive` appends an explicit "write the output in <language>" instruction to the system prompt so polish stays in the spoken language. Built-in prompts are language-agnostic regardless — they instruct the model to detect the transcript's language and never translate (the prior English-hardcoded prompts caused non-English dictation to come back in English). |
| `active_mode_id` | `general` | One of `general`, `minimal`, `email`, `chat`, `code-comment`, `notes`, `summary`, `braindump`. |
| `auto_suggest_modes` | `true` | Pill suggests a mode based on the focused app/window. Suggestion only — never auto-switches. |
| `appearance` | `dark` | `light` or `dark`; coerced to `dark` if unrecognised. |
| `auto_paste_enabled` | `false` | Opt-in. When on, the pipeline synthesises `⌘V` after the clipboard write so the polished transcript lands in the focused text field; the user's prior clipboard text is restored 250 ms later. Requires Accessibility — when the toggle is on but `AXIsProcessTrusted()` is false, the paste step fails and the pipeline fail-opens to clipboard-only with a warning. Backed by `mac_paste.rs` (`CGEventCreateKeyboardEvent` + `CGEventPost(kCGHIDEventTap, …)` for both key-down and key-up of V with the Command flag set). |
| `inference_api_key` | `""` | Random token generated lazily on first `llama-server` spawn (see `inference_proc::persistent_api_key`); reused so the startup-time orphan reaper can identify our child processes. Loopback-only — not a security secret. |

## Pipeline

Hotkey down → **Listening** pill + live waveform → hotkey up → **Transcribing** (Whisper or Parakeet, whichever is selected) → optional **Polishing** (Apple Intelligence or Gemma local, with the active mode's system prompt + dictionary) → **Done** with `Copied — press ⌘V to paste.` (`Ctrl+V` on Windows; or `Inserted` when `auto_paste_enabled` is on and — on macOS — Accessibility is granted).

Errors surface in the pill: missing model, mic denied, audio device error, silence, polishing timeout. The pipeline is fail-open: any LLM error — including Apple Intelligence guardrail rejections and `LanguageModelSession.GenerationError` — logs `WARN` and the raw Whisper output is copied instead. The auto-paste step has the same fail-open semantics: if `⌘V` synthesis fails (typically missing Accessibility), the transcript is still on the clipboard, the pill toast nudges the user to grant Accessibility, and the user can paste manually.

## Voice Modes

Eight built-ins seeded from `src-tauri/src/modes.rs::builtins()` on first launch:

- `general` — Balanced cleanup. Default fallback.
- `minimal` — Preserve every word in order; only strips obvious disfluencies (um/uh, stutter repeats) and adds punctuation/capitalization. For novel drafting, journaling, and interview transcription.
- `email` — Formal tone, full punctuation.
- `chat` — Casual, drops trailing periods, capped length.
- `code-comment` — Lowercase, slash-prefixed.
- `notes` — Sentence-case bullet style.
- `summary` — Condenses the dictation into a short summary of key points.
- `braindump` — Understands a stream-of-thought dump and rewrites it as clear, structured text covering everything said (reworded and reordered, nothing condensed away). For explaining a feature request to an AI, fleshing out ideas, etc.

Modes are JSON files under `~/Library/Application Support/com.vox.app/modes/`. Built-ins re-seed if deleted; `reset_default_modes` also force-overwrites a built-in whose on-disk copy has drifted from the shipped definition (so a prompt change in an app update reaches existing installs), skipping files that already match so its returned count stays meaningful. Launch-time seeding (`init_with_root`) still never overwrites — only the explicit reset does. Users can clone via `duplicate_mode`, edit via `save_mode`, export/import as JSON, or auto-trigger by bundle id glob / window-title substring.

Each mode contributes:
- `system_prompt` — given to the cleanup engine.
- `dictionary` — merged with the global `dictionary` and pushed into both the STT bias prompt and the cleanup prompt.
- `punctuation` / `case` / `max_length_chars` — applied as post-processing after the LLM step.
- `enable_thinking: bool | null` — overrides the global `enable_thinking` setting for this mode. `null` (the default for every built-in) inherits the global value; `true`/`false` pins it. Only the Gemma engine reads this; Apple Intelligence and the no-op engine ignore it.

## Permissions

- **Microphone** — required, prompted lazily inside the onboarding wizard via `AVCaptureDevice.requestAccessForMediaType`. The non-prompting `authorizationStatusForMediaType:` is used everywhere else (so the tray icon doesn't trigger a TCC prompt at launch).
- **Accessibility** — **optional**, used only by the opt-in auto-paste path. The default `Cmd+Alt+Period` (⌘⌥.) push-to-talk goes through Carbon `RegisterEventHotKey`, which doesn't need a TCC grant, and the default pipeline ends at clipboard write. Flipping Settings → General → Text insertion → "Paste into focused field" on engages `mac_paste::post_cmd_v`, which calls `CGEventPost(kCGHIDEventTap, …)` — that path needs Accessibility, so the Settings UI surfaces an inline "Open System Settings / Re-check" row when the toggle is on but `AXIsProcessTrusted()` is false. The pipeline fail-opens to clipboard-only if the grant is missing at session time. The `mac_chord.rs` / `CGEventTap` modifier-only chord path is dormant — no UI picker exposes it in v1, and the cancel/mode-cycle hotkey pickers only offer non-chord options (Escape, F13–F15, ⌥M, ⌥⇧M, ⌘⇧M). The chord plumbing is retained for a future "chord PTT" Settings option. About → Diagnostics → "Binary path for Accessibility" is a dev-build-only escape hatch (signature rotation makes the dev TCC entry stale).

## Repository layout

```
/
├── CLAUDE.md                          ← this file
├── package.json, vite.config.ts, tsconfig.json
├── index.html, pill.html, onboarding.html, tray-popover.html
├── src/                               ← SolidJS frontend
│   ├── entries/                       ← one entry per Tauri webview
│   ├── views/
│   │   ├── Settings.tsx               ← tabbed settings window
│   │   ├── Pill.tsx                   ← floating listening pill — slim orchestrator (base signals, `pill:*` event listeners, derived memos, JSX)
│   │   ├── pill/                      ← Pill.tsx internals (same `Foo.tsx` + `foo/` split Settings/Onboarding/TrayPopover use)
│   │   │   ├── types.ts               ← PillState + `pill:*` event payload interfaces
│   │   │   ├── constants.ts           ← reveal/hold/HUD timings + width-floor / window-padding layout constants
│   │   │   ├── helpers.ts             ← clamp, edgeEnvelope
│   │   │   ├── useReveal.ts           ← rate-limited streaming-text reveal ticker; owns transcript/polish stream buffers
│   │   │   ├── useTranscriptHold.ts   ← post-final "keep showing transcribing for ≥2 s" timer
│   │   │   ├── useModeHud.ts          ← voice-mode HUD (idle) / companion chip (mid-session) presenter
│   │   │   ├── usePillResize.ts       ← ResizeObserver + dynamic width-floor lerp; owns the wrapper/pill DOM refs
│   │   │   └── components/            ← Waveform, StreamingText, StageLabels, TypingDots, ModeDrawer, ModeHudView, TerminalLine, QueueDot, icons
│   │   ├── Onboarding.tsx             ← 6-step first-run wizard
│   │   └── TrayPopover.tsx            ← menu-bar popover
│   ├── components/
│   │   ├── VoxLogo.tsx                ← shared logo glyph
│   │   ├── HotkeyKeys.tsx             ← renders a Tauri shortcut spec as a row of keycap badges (`⌘ Cmd + ⌥ Opt + .`)
│   │   └── SparkleGlyph.tsx           ← 4-point sparkle SVG; shared by the tray popover's re-polish trigger and the onboarding tips step
│   ├── lib/
│   │   ├── ipc.ts                     ← typed bindings to Tauri commands + events (incl. `parakeetAvailability`, `recommendedSttEngineFor`, `parakeetSupportsLanguage`)
│   │   ├── hotkey.ts                  ← shared `formatHotkeyTokens` / `formatHotkey` for chord display
│   │   ├── platform.ts                ← UA-sniffed `isMac` / `isWindows` / `deviceWord` for render-time OS branching
│   │   └── theme.ts                   ← appearance bootstrap
│   └── styles.css
├── src-tauri/
│   ├── Cargo.toml                     ← `default = ["local-engines"]`
│   ├── tauri.conf.json                ← bundle id `com.vox.app`
│   ├── capabilities/default.json
│   ├── entitlements.plist             ← includes cs.allow-jit for Metal
│   └── src/
│       ├── lib.rs, main.rs            ← entry + invoke_handler
│       ├── audio.rs                   ← cpal capture + RMS levels
│       ├── session.rs                 ← pipeline orchestrator + cancel/timeout
│       ├── inject.rs                  ← clipboard write + snapshot/restore primitives
│       ├── hotkey.rs                  ← global shortcuts + chord fallback chain
│       ├── wake.rs                    ← opt-in "Hey Vox" wake-phrase listener (VAD + STT fuzzy match)
│       ├── mac_chord.rs               ← CGEventTap modifier-only chord listener (macOS)
│       ├── mac_paste.rs               ← synthetic ⌘V via CGEventPost (opt-in auto-paste, macOS)
│       ├── mac_perm.rs                ← AVFoundation auth FFI (non-prompting, macOS)
│       ├── win_paste.rs               ← synthetic Ctrl+V via SendInput (opt-in auto-paste, Windows)
│       ├── pill.rs                    ← pill window + state events
│       ├── onboarding.rs              ← wizard window lifecycle
│       ├── tray.rs, tray_icon.rs      ← menu-bar icon + popover anchor
│       ├── tray_glyphs.rs             ← state glyphs
│       ├── frontmost.rs               ← focused-app probe for mode suggestion
│       ├── transcripts.rs             ← in-memory ring buffer (cleared on quit)
│       ├── settings.rs                ← tauri-plugin-store backed Settings doc
│       ├── modes.rs                   ← Voice Modes store + JSON I/O + post-processing
│       ├── models.rs                  ← model catalog + resumable downloader (`stream_to_file` is reused for the llama-server archive)
│       ├── inference_server.rs        ← llama-server release-binary catalog + downloader/extractor
│       ├── inference_proc.rs          ← spawn / health-poll / supervise / orphan-reap the llama-server child
│       ├── commands.rs                ← #[tauri::command] handlers
│       └── engines/
│           ├── mod.rs                 ← Registry + SttEngine / LlmEngine traits + STT pre-warm helper
│           ├── whisper_local.rs       ← whisper-rs 0.16 (CoreML+Metal on macOS, CPU on Windows), in-process
│           ├── parakeet_local.rs      ← Parakeet SttEngine; cfg-split router → FluidAudio (macOS) or parakeet-rs/ONNX/DirectML (Windows)
│           ├── parakeet_bridge.rs     ← unsafe FFI to the Swift ParakeetBridge static lib (macOS aarch64 only; stubbed elsewhere)
│           ├── gemma_http.rs          ← OpenAI chat-completions client against the loopback llama-server
│           ├── apple_fm.rs            ← Apple Intelligence (Foundation Models) LlmEngine, macOS 26+
│           ├── apple_fm_bridge.rs     ← unsafe FFI wrappers for the Swift static library
│           └── none.rs                ← no-op LLM (raw transcript)
├── src-tauri/swift/
│   └── AppleFmBridge.swift            ← Swift `@_cdecl` exports calling LanguageModelSession
├── src-tauri/swift-parakeet/         ← SwiftPM package wrapping FluidAudio; built by build.rs::build_parakeet_bridge (macOS aarch64)
└── scripts/
    ├── dev-bundle.sh                  ← build + open a debug .app for TCC sanity
    ├── dev-clean-state.sh             ← wipe ALL Vox state for a clean first run
    ├── dev-reset-permissions.sh       ← only resets TCC, keeps settings/models
    ├── release.sh                     ← macOS release: build/sign/notarize, publish darwin-aarch64 to vox-releases
    └── release-windows.ps1            ← Windows release (PowerShell): build/sign, additively merge windows-x86_64 into the existing latest.json
```

Files that no longer exist are documented inline: `inject.rs` (Cmd+V removal) and `engines/gemma_http.rs` (replaced the old in-process `gemma_local.rs` + `llama-cpp-2` to keep C++ asserts from killing the menu-bar app). `build.rs` builds two Swift bridges on macOS — `build_apple_fm_bridge` (Apple Intelligence) and `build_parakeet_bridge` (FluidAudio); both early-out cleanly on non-macOS / wrong-arch / missing `swift`, leaving the engine to report unavailable at runtime.

## Development

### First run

```bash
npm install
npm run tauri dev
```

The first compile is slow because `local-engines` pulls in `whisper-rs` with its Metal backend, and the `apple-fm` + `parakeet` features drive `build.rs` to invoke `swiftc` on both `swift/AppleFmBridge.swift` and the `swift-parakeet/` package. On Windows the first compile instead builds `whisper-rs-sys` (CMake + MSVC) and resolves `parakeet-rs` + `ort` (whose build script downloads the prebuilt ONNX Runtime), and needs `LIBCLANG_PATH` set (see Platforms). Gemma polishing lives in a separate `llama-server` subprocess we download at runtime, so it doesn't participate in the Cargo build. Subsequent compiles are incremental.

CI / dev machines without Xcode-bundled `swiftc` can opt out of the Swift bridge:

```bash
cargo build --manifest-path src-tauri/Cargo.toml --no-default-features --features local-engines
```

In that mode `commands::apple_fm_availability` returns `framework_missing`, the onboarding wizard's Apple Intelligence card is disabled, and only the Gemma 4 path is reachable.

Onboarding fires automatically when `onboarded == false`: Welcome → Language → Microphone → Download local models → Try a few tricks (Voice Modes + Repolish showcase) → How to use. (Six steps. The Tips step is index 4 (`TIPS_STEP`) and lands *after* the model downloads finish so users feel rewarded rather than gated; it's optional (Skip link). No Accessibility step — the default ⌘⌥. hotkey doesn't need it, and chord PTT isn't user-pickable in v1.)

### Reset for a clean first-launch experience

```bash
./scripts/dev-clean-state.sh   # wipes settings, modes, models, TCC
npm run tauri dev
```

### Clean state directory locations

| What | Where |
| --- | --- |
| Settings JSON | `~/Library/Application Support/com.vox.app/vox-settings.json` |
| Voice Modes | `~/Library/Application Support/com.vox.app/modes/*.json` |
| Whisper / Gemma weights | `~/Library/Application Support/com.vox.app/models/{stt,llm}/<id>/model.{bin,gguf}` |
| `llama-server` binary tree | `~/Library/Application Support/com.vox.app/inference-server/<llama.cpp tag>/` (sentinel: `.complete`) |
| TCC entries | macOS-managed; reset with `tccutil reset {Microphone,Accessibility} com.vox.app` |

On Windows the same tree lives under `%APPDATA%\com.vox.app\` (settings, `modes\`, `models\{stt,llm}\<id>\`, `inference-server\<tag>\`). The Windows Parakeet ONNX bundle lands flat in `models\stt\parakeet-tdt-0.6b-v3\` (no `…-coreml\` sub-dir — that one is FluidAudio/macOS-only). The `dev-clean-state.sh` / `dev-reset-permissions.sh` helpers are macOS bash; on Windows, clear the app-data tree by hand. No TCC on Windows — auto-paste needs no grant there.

### Building a release

The releases repo is `KlymSerhii/vox-releases`; the auto-updater fetches `latest.json` from its latest release (minisign-pinned in `tauri.conf.json::plugins.updater`). The `latest.json` `platforms` object carries one entry per OS — `darwin-aarch64` and/or `windows-x86_64`.

**macOS** (`scripts/release.sh`, the only supported macOS path) builds/signs/notarizes the `.app` + `.dmg`, then on `--publish` **delete-and-recreates** the GitHub release with a `darwin-aarch64`-only `latest.json`. Output: `src-tauri/target/release/bundle/macos/Vox.app`. Run it from the signing Mac with the updater key at `~/.tauri/vox-updater.key` + `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.

**Windows** (`scripts/release-windows.ps1`, PowerShell) builds the NSIS `-setup.exe` and its minisign `.sig`. In this Tauri version the Windows updater payload **is the `-setup.exe` itself** (passive install) — there is no `.nsis.zip`; one `.exe` is both the user download and the updater artifact. On `-Publish` it is **additive**: it `gh release upload --clobber`s the `.exe` + `.exe.sig` + a `latest.json` that it **merges** `windows-x86_64` into (preserving any `darwin-aarch64` entry), and it **never** deletes the release. Run order is **macOS first, Windows second** (the Mac script's delete-recreate would otherwise wipe the Windows assets). Needs `gh` authenticated, the same `~/.tauri/vox-updater.key` (whose pubkey is embedded in `tauri.conf.json`) + password, plus the Windows build toolchain (see Platforms).

For UI-only fast iteration (no whisper / llama in the binary), opt out of the heavy engines:

```bash
cargo build --manifest-path src-tauri/Cargo.toml --no-default-features
```

The runtime falls back to a clear "Local engines not enabled in this build" error in that mode.

### Verification commands

These four together are the project smoke test:

```bash
cargo build  --manifest-path src-tauri/Cargo.toml
cargo test   --manifest-path src-tauri/Cargo.toml --lib
npx tsc --noEmit
npx vite build
```

### Logs

```bash
VOX_LOG=debug npm run tauri dev
```

`VOX_LOG` is read by `env_logger` in `lib.rs::run`. Defaults to `info`. Logs go to stdout.

### Dev-mode permission quirk

`tauri dev` produces an unbundled binary at `src-tauri/target/debug/vox` whose code signature changes on every rebuild. macOS TCC keys grants by signature, so any feature that touches Accessibility (Microphone in particular, since the v1 pipeline only needs that) may show stale grants across rebuilds. `./scripts/dev-bundle.sh` builds a proper `.app` whose identity is stable; use it for permission-sensitive work.

The default `Cmd+Alt+Period` shortcut sidesteps Accessibility entirely — it goes through the Carbon `RegisterEventHotKey` path. The dormant chord-PTT code in `mac_chord.rs` would need Accessibility if ever exposed in v1.1+; until then, the only TCC surface that affects v1 end users is Microphone.

## Settings tabs

- **General** — language, hotkey behaviour (toggle vs hold), the "Start dictation by voice" wake-phrase toggle + trigger-phrase field (renders only while the toggle is on), launch at login, dictation toggle, streaming pill toggle, **Text insertion** ("Paste into focused field" auto-paste toggle, with an inline Accessibility nudge that only renders when the toggle is on and the grant is missing).
- **Hotkeys** — push-to-talk, cancel, mode-cycle. Each row shows the configured value, the runtime-bound value (which can differ if a fallback kicked in), and an inline "Test" button that reads the live binding.
- **Models** — STT engine card (Whisper + the Parakeet card, shown when `parakeet_availability` reports available; copy is host-conditional via `isMac` — "ANE-accelerated" on macOS, "DirectML-accelerated" on Windows) + STT model rows; a "Try Parakeet" prompt for Whisper users on a supported language; the inference-server installer card; Polishing on/off toggle + Gemma model rows. Rows show installed-on-disk size, RAM peak estimate, tier, and Use/Download/Cancel/Delete actions.
- **Modes** — list, edit, duplicate, export/import, reset built-ins.
- **Dictionary** — global proper-noun + acronym list merged into every mode's dictionary.
- **Privacy** — informational; everything LOCAL.
- **About** — version, "Replay onboarding", diagnostics (live hotkey snapshot + dev binary path reveal for TCC troubleshooting).

## Tray popover

Click the menu-bar icon for: dictation on/off, recent transcripts (last 5, in-memory only), the active mode picker, current hotkey label, light/dark appearance toggle, Settings, Quit.

## Important conventions

- **Settings are the source of truth**. The `commands::set_settings` handler diffs hotkey-relevant fields, calls `hotkey::register` to live-rebind, and emits `settings:changed` so the Settings window's diagnostic chips and the tray popover both refresh without a restart. Listen for `settings:changed` from any window via `onSettingsChanged` in `src/lib/ipc.ts`.
- **Voice Mode list changes broadcast a `modes:changed` event.** Every mode-mutating command (`save_mode`, `delete_mode`, `duplicate_mode`, `import_mode_json`, `reset_default_modes` when `n > 0`) calls `commands::emit_modes_changed`, which re-reads the full list and fires `modes:changed` with `Vec<VoiceMode>` as payload. The Tray popover webview is built once and reused on every tray click, so it can't poll on open — subscribing to `onModesChanged` in `src/lib/ipc.ts` is the *only* way a custom mode created in Settings shows up in the popover's picker without restarting Vox. The Settings ModesTab subscribes too (instead of refetching after each mutation), so the event is the single source of truth for the in-memory mode list across windows.
- **No audio touches disk.** Capture buffer lives in RAM, gets WAV-encoded in-place, then is freed.
- **No transcript persists.** The "recent 5" tray list is `transcripts::snapshot()` from a process-lifetime ring buffer; it's gone on quit.
- **Engine errors are fail-open.** STT errors abort the session; LLM errors log a warning and the raw transcript is copied. Both states surface a friendly pill.
- **Mode prompts come from the active mode at session start.** Switching modes mid-pipeline does not affect the current utterance.
- **Voice Mode IDs are validated** against `[a-z0-9-]{1,64}` with no edge dashes (`is_safe_mode_id`) to prevent path traversal in the JSON store.
- **Engines pre-warm in the background at launch.** `lib.rs::run`'s setup hook spawns `inference_proc::prewarm_in_background` (boots `llama-server` only when `llm_engine == "gemma-local"` and the binary + Gemma model are installed — it's a no-op for `apple-fm`, which has no spawn cost to amortise; the OS's `modelinferenced` daemon handles its own warmup), `engines::prewarm_stt_in_background` (cold-loads the selected STT engine — Whisper GGML weights, or the Parakeet model: FluidAudio on macOS / the ONNX sessions + DirectML EP on Windows), and `inject::prewarm_in_background` (a read-and-restore NSPasteboard round-trip that forces macOS's first-use source-app signature check to run during onboarding rather than on the user's first dictation, where it otherwise stalls the post-polish clipboard write for 30-60 s with a "Verifying com.vox…" system dialog; verification is cached by Gatekeeper so subsequent launches are sub-millisecond no-ops). All three are idempotent and no-op when their preconditions aren't met.
- **`llama-server` orphans get reaped at startup.** macOS has no `prctl(PR_SET_PDEATHSIG)`, so a force-killed previous Vox parent can leave a `llama-server` child running. `inference_proc::reap_orphans_at_startup` walks the process table and SIGKILLs any whose `--api-key` argument matches the persisted `inference_api_key`. That's why the api key is persisted across launches even though it's not security-relevant.
- **Hotkey display uses one shared formatter.** `src/lib/hotkey.ts::formatHotkeyTokens` parses a Tauri shortcut spec (`"CommandOrControl+Alt+Period"`, `"Alt+Shift+M"`, `"AltRight"`, `"F13"`, …) into an array of `{kind, symbol, label}` tokens. `<HotkeyKeys spec={…} />` in `src/components/HotkeyKeys.tsx` renders the tokens as keycap badges — each modifier shows **both** the macOS glyph and a short word (`⌘ Cmd`, `⌥ Opt`, `⇧ Shift`, `⌃ Ctrl`) so the chord stays legible to users who don't recognise the symbols. The shifted-glyph collapse for legibility (`Cmd+Shift+/` → `⌘?`, not `⌘⇧/`) lives in the same tokeniser. For plain-text contexts that can't host rich markup (the onboarding "couldn't be bound" warning, `<option>` labels in the Hotkeys picker dropdown), `formatHotkey(spec)` returns the word form joined with ` + ` (`"Cmd + Opt + ."`); the cycle-hotkey `<option>` labels also keep the symbol form in parens (`"Opt+M  (⌥M)"`). Do not roll a new symbol map at a new callsite — extend the shared tables instead.

## Known limitations

- **Apple Intelligence content guardrails.** The Foundation Models framework hard-blocks high-severity content categories (self-harm, violence, adult sexual content) on both input *and* output, and there is no developer opt-out — guardrail rejections surface as `LanguageModelSession.GenerationError`. We translate that into an `anyhow::Error` from `engines::apple_fm`, which the existing `session.rs` fail-open path turns into a "use the raw Whisper transcript" outcome. Users dictating medical / legal / profanity-heavy content who'd rather not hit this should switch to Gemma 4 in Settings → Models.
- **Apple Intelligence requires user opt-in.** Even on a supported Mac, `SystemLanguageModel.default.availability` returns `.unavailable(.appleIntelligenceNotEnabled)` until the user toggles Apple Intelligence on in System Settings → Apple Intelligence & Siri. The onboarding card detects this and shows the reason; the Settings → Models tab has a "Re-check" button that re-probes via `commands::apple_fm_availability` so the user can flip the System Settings toggle without restarting Vox.
- **Windows polishing is Gemma-only.** There is no Apple Intelligence analogue on Windows. Phi Silica was evaluated and rejected for v1: Microsoft archived `windows-app-rs` (no Rust binding path), Phi Silica is still a Limited Access Feature behind a token gate on the experimental Windows App SDK 2.0-preview1, and only ~3% of Windows boxes meet the Copilot+ PC hardware requirement. Windows reuses the existing `llama-server` + Gemma pipeline (llama.cpp publishes Windows binaries on the same `LLAMA_CPP_TAG`); `commands::apple_fm_availability` returns `framework_missing` there and the onboarding/Settings Apple Intelligence cards stay hidden.
- **Windows is x86_64-only in v1.** Native ARM64 Windows isn't shipped: the `ort`/`parakeet-rs` prebuilt ONNX Runtime is sourced for x64, and the toolchain story is "x64 Rust under emulation." The x64 build runs fine on Windows-on-ARM via emulation. `release-windows.ps1` and `commands::parakeet_availability` have the `aarch64-pc-windows-msvc` / `windows-aarch64` paths wired for when native ARM64 is enabled, but no ARM64 ONNX bundle or CI matrix exists yet.
- **Parakeet on Windows: ONNX/DirectML specifics.** The Windows engine uses `parakeet-rs` (pinned `=0.3.x`) over `ort` (pinned `=2.0.0-rc`), DirectML EP with automatic CPU-EP fallback — a DX12-less box silently runs on CPU (slower, no error surfaced). The `parakeet-tdt-0.6b-v3` model descriptor in `models.rs` is `cfg`-branched (`name` / `size_bytes` per OS); the Windows bundle is the four-file ONNX set from `istupakov/parakeet-tdt-0.6b-v3-onnx` (`encoder-model.onnx` + the ~2.27 GB `encoder-model.onnx.data` external weights + `decoder_joint-model.onnx` + `vocab.txt`, ~2.5 GB total), downloaded by the existing multi-file `parakeet_v3_files_windows()` path and probed for install via `vocab.txt` + the size of the big `.data` file. Like the macOS FluidAudio path, it does **not** forward dictionary biasing — dictionary-heavy users stay on Whisper. Long-form (>~4–5 min single dictation) can truncate on TDT models; normal hotkey-bounded dictation is well under that.
- **Gemma polish runs out-of-process.** `inference_server.rs` pins `LLAMA_CPP_TAG` (currently `b8953`) and downloads the matching official `llama-server` archive from the llama.cpp GitHub release. `inference_proc.rs` spawns it as a process-group / Job-Object child so the parent's death tears the child down too, even on SIGKILL. Bumping llama.cpp = bump `LLAMA_CPP_TAG` and (when set) `expected_sha256`; users on the old tag re-download into a different version directory and `cleanup_stale_versions` reaps the previous tree on the launch after.
- **MatFormer Gemma 4 GGUFs from Unsloth crash llama.cpp.** Their conversion produces `GGML_OP_GATED_DELTA_NET` tensors whose names don't carry the `LLAMA_TENSOR_NAME_FGDN_AR` prefix that the FGDN auto-detect path in `llama-context.cpp::sched_reserve` expects, so a `GGML_ASSERT` fires. With the engine out-of-process the assert now exits the child cleanly and `session.rs` fail-opens to the raw transcript (instead of crashing the menu-bar app, as it did under the old in-process `llama-cpp-2`). **bartowski's MatFormer Gemma 4 GGUFs avoid this** — their conversion uses tensor names that do not trip FGDN auto-detect, so we source E2B and E4B from bartowski.
- **LLM catalog**: `models.rs` ships three polishing options — **Gemma 4 E2B** (`gemma-4-e2b-q5_k_m`, ~3.7 GB, small — default for <12 GB RAM; bartowski Q5_K_M), **Gemma 4 E4B** (`gemma-4-e4b-q5_k_m`, ~5.4 GB, balanced — default for ≥12 GB RAM; bartowski), and **Gemma 4 26B-A4B MoE** (`gemma-4-26b-a4b-q4_k_m`, ~17 GB, quality; Unsloth UD-Q4_K_M for its imatrix calibration edge — safe under any converter because the MoE graph has zero GDN ops, dispatching through `llm_build_gemma4_iswa`'s standard `build_attn` path with `shared_kv_layers=0` and no per-layer embedding). All Gemma releases share the same `<start_of_turn>` chat template. `recommended_llm_model_id()` returns `gemma-4-e2b-q5_k_m` for the `small` tier (<12 GB RAM), `gemma-4-e4b-q5_k_m` for `balanced`, and `gemma-4-26b-a4b-q4_k_m` only when the user explicitly opts in via Settings.
- **Pill follows the cursor's display.** It's placed on the cursor's screen at `pill::show` time (`active_screen_logical_frame`), and `pill::spawn_display_follower` (a poll every `FOLLOW_POLL_MS`, keyed on `PILL_GENERATION`, no-op on single-display Macs) re-centres it onto a new screen if the user moves there while a session is in flight. Following the *focused window's* display rather than the cursor's is a future enhancement.

## Contact points for changes

- New IPC command: add to `src-tauri/src/commands.rs`, register in `src-tauri/src/lib.rs::invoke_handler!`, expose a typed binding in `src/lib/ipc.ts`.
- New Settings field: extend the `Settings` struct + `Default` impl + `load`/`save` in `settings.rs`, and mirror the field in `src/lib/ipc.ts::Settings`. New fields rely on per-field `#[serde(default)]` to fall back when absent from an older on-disk doc.
- New Voice Mode built-in: add a clone in `modes::builtins()` plus a brand new `*_PROMPT` constant, update tests in `modes::tests`.
- New engine: implement `SttEngine` or `LlmEngine` in `engines/` and wire it into `engines::Registry`. The engine id is used as-is from the stored Settings doc — `Registry::stt`/`Registry::llm` return `None` for an unknown id, which `session.rs` surfaces as a clear error (STT) or fail-opens to the raw transcript (LLM).
- OS-split behaviour for the **same** engine id (the Parakeet pattern): keep one id + one model id; select the concrete backend with `#[cfg(target_os = "…")]` inside the engine module (`parakeet_local.rs` routes to FluidAudio on macOS, `parakeet-rs`/ONNX on Windows), and `cfg`-branch the `models.rs` descriptor + a `*_files_<os>()` manifest so download URLs / sizes / on-disk layout differ per host while the Settings doc stays OS-agnostic. Mirror this rather than minting a per-OS engine id (which would force every cross-window listener to learn two ids meaning the same thing).
- New user-facing OS-conditional copy: branch on `isMac` / `deviceWord` from `src/lib/platform.ts` — never hard-code "Mac" / "Apple Silicon" / "Apple Neural Engine". For OS-specific Rust window/HID behaviour, gate with `cfg!(target_os = "…")` (see `pill.rs` shadow, `tray.rs` popover placement, the `session.rs` paste three-arm).
- Release plumbing: `latest.json` carries one `platforms` entry per OS. `scripts/release.sh` (macOS) is authoritative and delete-recreates the release; `scripts/release-windows.ps1` (Windows) is additive and merges `windows-x86_64` in without deleting. Adding a third platform = a new additive publisher that merges its own `platforms` key, run after the Mac. Run order is always macOS first.
- New native FFI bridge (Swift / Objective-C / C++): mirror `swift/AppleFmBridge.swift` + `engines/apple_fm_bridge.rs` + `build.rs::build_apple_fm_bridge`. The pattern: hand-rolled `@_cdecl` exports with `strdup`'d return strings, a thread-local error slot, and a Cargo feature flag so non-macOS builds skip both the build.rs step and the engine module via `#[cfg(all(target_os = "macos", feature = "..."))]`.
- New hotkey display surface: import `HotkeyKeys` from `src/components/HotkeyKeys.tsx` and pass the raw Tauri shortcut spec — do *not* hand-roll a `Keycap` chain or hard-code glyphs at the callsite. New modifier / key tokens (e.g. a future fn-row binding) go into the `MODIFIER_SYMBOLS` / `KEY_UNSHIFTED` / `KEY_SHIFTED` tables in `src/lib/hotkey.ts`, plus the matching `MODIFIER_LABELS` entry for the word form.
- New cross-window broadcast: emit from `commands.rs` via `app.emit("namespace:changed", &payload)` (see `emit_modes_changed` / `settings:changed` for the pattern: re-read authoritative state, serialise the full snapshot, log on failure). Add a typed `onXxxChanged(cb)` listener in `src/lib/ipc.ts` and subscribe in every webview that displays the affected state — the Tray popover in particular is built once and reused, so it *requires* live events to stay current.
- Pill UI / dictation overlay changes: `src/views/Pill.tsx` is a thin orchestrator — it owns only the base reactive state (`state`, `message`, `levels`, `sessionActive`, …), the `pill:*` Tauri event listeners in `onMount`, the derived memos (`effectiveState`, `meterColor`, `isStreaming`, …), and the JSX. All stateful behaviour lives in single-purpose hooks under `src/views/pill/`: `useReveal` (the rate-limited streaming-text ticker + `applyStreamPayload` / `kickSlowRevealForTranscribing`), `useTranscriptHold` (the post-final ≥2 s hold), `useModeHud` (HUD vs companion-chip), and `usePillResize` (ResizeObserver + width-floor; owns the wrapper/pill DOM refs and hands back `setWrapperRef` / `setPillRef` for `ref={…}`). Cross-hook needs are injected as **getter references** (`useReveal({ state, reducedMotion, … })`) so reactivity survives; never snapshot a signal at call time. Each hook owns its own signals, timers, and `onCleanup` — add new pill state to the relevant hook, not inline in the component. Pure presentational pieces (no signals) go in `src/views/pill/components/`; shared timing/layout numbers in `src/views/pill/constants.ts`.
