# Vox

Fully on-device macOS dictation. Press a hotkey, speak, and the transcript lands on your clipboard ready for `⌘V`.

This file is the canonical project reference. The README that used to live at the project root was removed when the app pivoted to a clipboard-only, local-only build; the surviving `docs/liquid-glass-*.md` files are design notes for the existing light/dark UI tokens, not the project overview.

---

## What it is today

- **macOS menu-bar app**, no Dock icon (`ActivationPolicy::Accessory`).
- **Tauri 2** Rust core + **SolidJS** + **Tailwind 4** frontend.
- **Whisper** for STT (in-process via `whisper-rs`).
- **Two polishing engines, user-pickable in onboarding and Settings → Models:**
  - **Apple Intelligence** via Apple's `FoundationModels` framework. Implemented as an in-process Swift FFI bridge (`swift/AppleFmBridge.swift` compiled to a static library by `build.rs::build_apple_fm_bridge`, called from `engines::apple_fm` through `engines::apple_fm_bridge`). Recommended default on Macs with macOS 26+ Apple Silicon and Apple Intelligence enabled in System Settings. No download. Apple's safety guardrails can refuse certain dictation; the engine returns a clear error and `session.rs` fail-opens to the raw Whisper transcript.
  - **Gemma 4** out-of-process via a downloaded `llama-server` binary we talk to over loopback HTTP (`engines::gemma_http`). Default fallback when Apple Intelligence is unavailable, and the only option on macOS < 26.
- All inference is on-device: Metal / Core ML on Apple Silicon, plus the Apple Neural Engine and `modelinferenced` system daemon for Foundation Models. Models live on disk; nothing leaves the Mac.
- **Cloud is gone.** No Groq, no Anthropic, no API keys, no telemetry, no analytics. The only network calls remaining are model + `llama-server` downloads from Hugging Face / GitHub and the auto-updater pulling release manifests from the `KlymSerhii/vox-releases` GitHub release (minisign-pinned, see `tauri.conf.json::plugins.updater`).
- **Output is the clipboard by default.** The transcript replaces the clipboard contents and the user pastes manually — no synthetic `Cmd+V`, no Accessibility required for the default pipeline. Users who want hands-off insertion can opt into **auto-paste** in Settings → General → Text insertion; that path synthesises `⌘V` at the HID event tap (`mac_paste::post_cmd_v`) from inside `session::SessionManager::deliver_transcript` after polish completes, and restores the user's prior clipboard text ~250 ms later. A rapid second dictation that lands inside the restore window cancels the previous restore and inherits its captured prior text, so a chain of pipelines still ends with the user's true pre-chain clipboard. Auto-paste requires Accessibility; when the toggle is on but Accessibility isn't granted, `deliver_transcript` fail-opens to the clipboard-only path and the pill toast reads "Copied — grant Accessibility for auto-paste".

## Defaults

| Setting | Default | Notes |
| --- | --- | --- |
| `hotkey` | `Cmd+Alt+Period` (⌘⌥.) | Hardcoded — `settings::load` always writes the default into the persisted store on every launch. Bound via `tauri-plugin-global-shortcut` (Carbon `RegisterEventHotKey`). No Accessibility required. |
| `cancel_hotkey` | `Escape` | Aborts the in-flight session. The OS grab is acquired only while a session is active (`hotkey::arm_cancel` at `session::begin_recording`, `hotkey::disarm_cancel` from `PipelineGuard::drop`) so Escape works normally in every other app when Vox isn't dictating. |
| `mode_cycle_hotkey` | `Alt+M` | Advances through Voice Modes. |
| `hands_free_enabled` | `true` | Press once to start, again to stop. |
| `dictation_enabled` | `true` | Master kill-switch from the tray popover. |
| `stt_engine` / `stt_model_id` | `whisper-local` / tier-recommended | Auto-tier via `recommend_tier`. |
| `llm_engine` / `llm_model_id` | `gemma-local` / tier-recommended on disk; onboarding flips to `apple-fm` when `apple_fm_availability` reports `available` | `apple-fm` ignores the `llm_model_id` field but we still keep a tier-matched Gemma id there so a later switch in Settings doesn't have to re-pick one. |
| `polishing_enabled` | `true` | UI toggle. Maps to `llm_engine ∈ {gemma-local, apple-fm, none}`. Toggling off only sets `polishing_enabled = false` — `llm_engine` is preserved so a user with Apple Intelligence selected gets it back when they toggle on (lockstep behaviour lives in `session.rs` short-circuiting to the no-op engine when polishing is off). |
| `enable_thinking` | `false` | Gemma 4 only. When on, Gemma reasons silently before producing the polished output (slower, can improve quality on complex inputs). Sent as `chat_template_kwargs: {"enable_thinking": true}` per request. Apple Intelligence has no equivalent knob and ignores the value. |
| `language` | `auto` | Whisper language hint; `auto` lets Whisper detect. |
| `active_mode_id` | `general` | One of `general`, `email`, `chat`, `code-comment`, `notes`, `summary`. |
| `auto_suggest_modes` | `true` | Pill suggests a mode based on the focused app/window. Suggestion only — never auto-switches. |
| `appearance` | `dark` | `light` or `dark`; coerced to `dark` if unrecognised. |
| `auto_paste_enabled` | `false` | Opt-in. When on, the pipeline synthesises `⌘V` after the clipboard write so the polished transcript lands in the focused text field; the user's prior clipboard text is restored 250 ms later. Requires Accessibility — when the toggle is on but `AXIsProcessTrusted()` is false, the paste step fails and the pipeline fail-opens to clipboard-only with a warning. Backed by `mac_paste.rs` (`CGEventCreateKeyboardEvent` + `CGEventPost(kCGHIDEventTap, …)` for both key-down and key-up of V with the Command flag set). |
| `inference_api_key` | `""` | Random token generated lazily on first `llama-server` spawn (see `inference_proc::persistent_api_key`); reused so the startup-time orphan reaper can identify our child processes. Loopback-only — not a security secret. |

## Pipeline

Hotkey down → **Listening** pill + live waveform → hotkey up → **Transcribing** (Whisper local) → optional **Polishing** (Apple Intelligence or Gemma local, with the active mode's system prompt + dictionary) → **Done** with `Copied — press ⌘V to paste.` (or `Inserted` when `auto_paste_enabled` is on and Accessibility is granted).

Errors surface in the pill: missing model, mic denied, audio device error, silence, polishing timeout. The pipeline is fail-open: any LLM error — including Apple Intelligence guardrail rejections and `LanguageModelSession.GenerationError` — logs `WARN` and the raw Whisper output is copied instead. The auto-paste step has the same fail-open semantics: if `⌘V` synthesis fails (typically missing Accessibility), the transcript is still on the clipboard, the pill toast nudges the user to grant Accessibility, and the user can paste manually.

## Voice Modes

Six built-ins seeded from `src-tauri/src/modes.rs::builtins()` on first launch:

- `general` — Balanced cleanup. Default fallback.
- `email` — Formal tone, full punctuation.
- `chat` — Casual, drops trailing periods, capped length.
- `code-comment` — Lowercase, slash-prefixed.
- `notes` — Sentence-case bullet style.
- `summary` — Condenses the dictation into a short summary of key points.

Modes are JSON files under `~/Library/Application Support/com.vox.app/modes/`. Built-ins re-seed if deleted (`reset_default_modes`). Users can clone via `duplicate_mode`, edit via `save_mode`, export/import as JSON, or auto-trigger by bundle id glob / window-title substring.

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
│   │   ├── Pill.tsx                   ← floating listening pill
│   │   ├── Onboarding.tsx             ← 5-step first-run wizard
│   │   └── TrayPopover.tsx            ← menu-bar popover
│   ├── lib/ipc.ts                     ← typed bindings to Tauri commands + events
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
│       ├── mac_chord.rs               ← CGEventTap modifier-only chord listener
│       ├── mac_paste.rs               ← synthetic ⌘V via CGEventPost (opt-in auto-paste)
│       ├── mac_perm.rs                ← AVFoundation auth FFI (non-prompting)
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
│           ├── whisper_local.rs       ← whisper-rs 0.16 (CoreML+Metal), in-process
│           ├── gemma_http.rs          ← OpenAI chat-completions client against the loopback llama-server
│           ├── apple_fm.rs            ← Apple Intelligence (Foundation Models) LlmEngine, macOS 26+
│           ├── apple_fm_bridge.rs     ← unsafe FFI wrappers for the Swift static library
│           └── none.rs                ← no-op LLM (raw transcript)
├── src-tauri/swift/
│   └── AppleFmBridge.swift            ← Swift `@_cdecl` exports calling LanguageModelSession
└── scripts/
    ├── dev-bundle.sh                  ← build + open a debug .app for TCC sanity
    ├── dev-clean-state.sh             ← wipe ALL Vox state for a clean first run
    └── dev-reset-permissions.sh       ← only resets TCC, keeps settings/models
```

Files that no longer exist are documented inline: `inject.rs` (Cmd+V removal) and `engines/gemma_http.rs` (replaced the old in-process `gemma_local.rs` + `llama-cpp-2` to keep C++ asserts from killing the menu-bar app).

## Development

### First run

```bash
npm install
npm run tauri dev
```

The first compile is slow because `local-engines` pulls in `whisper-rs` with its Metal backend, and the `apple-fm` feature drives `build.rs::build_apple_fm_bridge` to invoke `swiftc` on `src-tauri/swift/AppleFmBridge.swift`. Gemma polishing lives in a separate `llama-server` subprocess we download at runtime, so it doesn't participate in the Cargo build. Subsequent compiles are incremental.

CI / dev machines without Xcode-bundled `swiftc` can opt out of the Swift bridge:

```bash
cargo build --manifest-path src-tauri/Cargo.toml --no-default-features --features local-engines
```

In that mode `commands::apple_fm_availability` returns `framework_missing`, the onboarding wizard's Apple Intelligence card is disabled, and only the Gemma 4 path is reachable.

Onboarding fires automatically when `onboarded == false`: Welcome → Language → Microphone → Download local models → How to use. (Five steps. No Accessibility step — the default ⌘⌥. hotkey doesn't need it, and chord PTT isn't user-pickable in v1.)

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

### Building a release `.app`

```bash
# Local-first (the only supported build):
npm run tauri build -- --bundles app

# Output:
src-tauri/target/release/bundle/macos/Vox.app
```

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

- **General** — language, hotkey behaviour (toggle vs hold), launch at login, dictation toggle, streaming pill toggle, **Text insertion** ("Paste into focused field" auto-paste toggle, with an inline Accessibility nudge that only renders when the toggle is on and the grant is missing).
- **Hotkeys** — push-to-talk, cancel, mode-cycle. Each row shows the configured value, the runtime-bound value (which can differ if a fallback kicked in), and an inline "Test" button that reads the live binding.
- **Models** — STT engine info + Whisper model rows; the inference-server installer card; Polishing on/off toggle + Gemma model rows. Rows show installed-on-disk size, RAM peak estimate, tier, and Use/Download/Cancel/Delete actions.
- **Modes** — list, edit, duplicate, export/import, reset built-ins.
- **Dictionary** — global proper-noun + acronym list merged into every mode's dictionary.
- **Privacy** — informational; everything LOCAL.
- **About** — version, "Replay onboarding", diagnostics (live hotkey snapshot + dev binary path reveal for TCC troubleshooting).

## Tray popover

Click the menu-bar icon for: dictation on/off, recent transcripts (last 5, in-memory only), the active mode picker, current hotkey label, light/dark appearance toggle, Settings, Quit.

## Important conventions

- **Settings are the source of truth**. The `commands::set_settings` handler diffs hotkey-relevant fields, calls `hotkey::register` to live-rebind, and emits `settings:changed` so the Settings window's diagnostic chips and the tray popover both refresh without a restart. Listen for `settings:changed` from any window via `onSettingsChanged` in `src/lib/ipc.ts`.
- **No audio touches disk.** Capture buffer lives in RAM, gets WAV-encoded in-place, then is freed.
- **No transcript persists.** The "recent 5" tray list is `transcripts::snapshot()` from a process-lifetime ring buffer; it's gone on quit.
- **Engine errors are fail-open.** STT errors abort the session; LLM errors log a warning and the raw transcript is copied. Both states surface a friendly pill.
- **Mode prompts come from the active mode at session start.** Switching modes mid-pipeline does not affect the current utterance.
- **Voice Mode IDs are validated** against `[a-z0-9-]{1,64}` with no edge dashes (`is_safe_mode_id`) to prevent path traversal in the JSON store.
- **Engines pre-warm in the background at launch.** `lib.rs::run`'s setup hook spawns `inference_proc::prewarm_in_background` (boots `llama-server` only when `llm_engine == "gemma-local"` and the binary + Gemma model are installed — it's a no-op for `apple-fm`, which has no spawn cost to amortise; the OS's `modelinferenced` daemon handles its own warmup), `engines::prewarm_stt_in_background` (cold-loads the Whisper GGML weights), and `inject::prewarm_in_background` (a read-and-restore NSPasteboard round-trip that forces macOS's first-use source-app signature check to run during onboarding rather than on the user's first dictation, where it otherwise stalls the post-polish clipboard write for 30-60 s with a "Verifying com.vox…" system dialog; verification is cached by Gatekeeper so subsequent launches are sub-millisecond no-ops). All three are idempotent and no-op when their preconditions aren't met.
- **`llama-server` orphans get reaped at startup.** macOS has no `prctl(PR_SET_PDEATHSIG)`, so a force-killed previous Vox parent can leave a `llama-server` child running. `inference_proc::reap_orphans_at_startup` walks the process table and SIGKILLs any whose `--api-key` argument matches the persisted `inference_api_key`. That's why the api key is persisted across launches even though it's not security-relevant.

## Known limitations

- **Apple Intelligence content guardrails.** The Foundation Models framework hard-blocks high-severity content categories (self-harm, violence, adult sexual content) on both input *and* output, and there is no developer opt-out — guardrail rejections surface as `LanguageModelSession.GenerationError`. We translate that into an `anyhow::Error` from `engines::apple_fm`, which the existing `session.rs` fail-open path turns into a "use the raw Whisper transcript" outcome. Users dictating medical / legal / profanity-heavy content who'd rather not hit this should switch to Gemma 4 in Settings → Models.
- **Apple Intelligence requires user opt-in.** Even on a supported Mac, `SystemLanguageModel.default.availability` returns `.unavailable(.appleIntelligenceNotEnabled)` until the user toggles Apple Intelligence on in System Settings → Apple Intelligence & Siri. The onboarding card detects this and shows the reason; the Settings → Models tab has a "Re-check" button that re-probes via `commands::apple_fm_availability` so the user can flip the System Settings toggle without restarting Vox.
- **Windows polishing is Gemma-only.** Phi Silica was evaluated as a Windows analogue of Apple Intelligence and rejected for v1: Microsoft archived `windows-app-rs` (no Rust binding path), Phi Silica is still a Limited Access Feature behind a token gate on the experimental Windows App SDK 2.0-preview1, and only ~3% of Windows boxes meet the Copilot+ PC hardware requirement. When Vox eventually ships on Windows we'll reuse the existing `llama-server` + Gemma pipeline (llama.cpp publishes Windows binaries on the same release tag).
- **Gemma polish runs out-of-process.** `inference_server.rs` pins `LLAMA_CPP_TAG` (currently `b8953`) and downloads the matching official `llama-server` archive from the llama.cpp GitHub release. `inference_proc.rs` spawns it as a process-group / Job-Object child so the parent's death tears the child down too, even on SIGKILL. Bumping llama.cpp = bump `LLAMA_CPP_TAG` and (when set) `expected_sha256`; users on the old tag re-download into a different version directory and `cleanup_stale_versions` reaps the previous tree on the launch after.
- **MatFormer Gemma 4 GGUFs from Unsloth crash llama.cpp.** Their conversion produces `GGML_OP_GATED_DELTA_NET` tensors whose names don't carry the `LLAMA_TENSOR_NAME_FGDN_AR` prefix that the FGDN auto-detect path in `llama-context.cpp::sched_reserve` expects, so a `GGML_ASSERT` fires. With the engine out-of-process the assert now exits the child cleanly and `session.rs` fail-opens to the raw transcript (instead of crashing the menu-bar app, as it did under the old in-process `llama-cpp-2`). **bartowski's MatFormer Gemma 4 GGUFs avoid this** — their conversion uses tensor names that do not trip FGDN auto-detect, so we source E2B and E4B from bartowski.
- **LLM catalog**: `models.rs` ships three polishing options — **Gemma 4 E2B** (`gemma-4-e2b-q5_k_m`, ~3.7 GB, small — default for <12 GB RAM; bartowski Q5_K_M), **Gemma 4 E4B** (`gemma-4-e4b-q5_k_m`, ~5.4 GB, balanced — default for ≥12 GB RAM; bartowski), and **Gemma 4 26B-A4B MoE** (`gemma-4-26b-a4b-q4_k_m`, ~17 GB, quality; Unsloth UD-Q4_K_M for its imatrix calibration edge — safe under any converter because the MoE graph has zero GDN ops, dispatching through `llm_build_gemma4_iswa`'s standard `build_attn` path with `shared_kv_layers=0` and no per-layer embedding). All Gemma releases share the same `<start_of_turn>` chat template. `recommended_llm_model_id()` returns `gemma-4-e2b-q5_k_m` for the `small` tier (<12 GB RAM), `gemma-4-e4b-q5_k_m` for `balanced`, and `gemma-4-26b-a4b-q4_k_m` only when the user explicitly opts in via Settings.
- **Pill always renders on the primary display.** Multi-display follow-the-focus is a future enhancement.

## Contact points for changes

- New IPC command: add to `src-tauri/src/commands.rs`, register in `src-tauri/src/lib.rs::invoke_handler!`, expose a typed binding in `src/lib/ipc.ts`.
- New Settings field: extend the `Settings` struct + `Default` impl + `load`/`save` in `settings.rs`, and mirror the field in `src/lib/ipc.ts::Settings`. New fields rely on per-field `#[serde(default)]` to fall back when absent from an older on-disk doc.
- New Voice Mode built-in: add a clone in `modes::builtins()` plus a brand new `*_PROMPT` constant, update tests in `modes::tests`.
- New engine: implement `SttEngine` or `LlmEngine` in `engines/` and wire it into `engines::Registry`. The engine id is used as-is from the stored Settings doc — `Registry::stt`/`Registry::llm` return `None` for an unknown id, which `session.rs` surfaces as a clear error (STT) or fail-opens to the raw transcript (LLM).
- New native FFI bridge (Swift / Objective-C / C++): mirror `swift/AppleFmBridge.swift` + `engines/apple_fm_bridge.rs` + `build.rs::build_apple_fm_bridge`. The pattern: hand-rolled `@_cdecl` exports with `strdup`'d return strings, a thread-local error slot, and a Cargo feature flag so non-macOS builds skip both the build.rs step and the engine module via `#[cfg(all(target_os = "macos", feature = "..."))]`.
