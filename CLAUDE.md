# Vox

Fully on-device macOS dictation. Press a hotkey, speak, and the transcript lands on your clipboard ready for `⌘V`.

This file is the canonical project reference. The README that used to live at the project root was removed when the app pivoted to a clipboard-only, local-only build; the surviving `docs/liquid-glass-*.md` files are design notes for the existing light/dark UI tokens, not the project overview.

---

## What it is today

- **macOS menu-bar app**, no Dock icon (`ActivationPolicy::Accessory`).
- **Tauri 2** Rust core + **SolidJS** + **Tailwind 4** frontend.
- **Whisper** for STT (in-process via `whisper-rs`) and **Gemma 4** for transcript polishing (out-of-process via a downloaded `llama-server` binary we talk to over loopback HTTP) — both Metal/Core ML on Apple Silicon. Models live on disk; nothing leaves the Mac. (See `Known limitations` and the `models.rs` doc comment for which Gemma 4 GGUFs run cleanly under the pinned `llama.cpp` release.)
- **Cloud is gone.** No Groq, no Anthropic, no API keys, no telemetry, no analytics. The only network calls remaining are model + `llama-server` downloads from Hugging Face / GitHub and the auto-updater (currently stubbed off until signing is wired up).
- **Output is the clipboard.** The transcript replaces the clipboard contents and the user pastes manually — no synthetic `Cmd+V`, no Accessibility required for the default pipeline.

## Defaults

| Setting | Default | Notes |
| --- | --- | --- |
| `hotkey` | `Cmd+Ctrl+Space` | Hardcoded — `settings::load` rewrites any other persisted value back to this on every launch. Bound via `tauri-plugin-global-shortcut` (Carbon `RegisterEventHotKey`). No Accessibility required. |
| `cancel_hotkey` | `Escape` | Aborts the in-flight session. |
| `mode_cycle_hotkey` | `Alt+M` | Advances through Voice Modes. |
| `hands_free_enabled` | `true` | Press once to start, again to stop. |
| `dictation_enabled` | `true` | Master kill-switch from the tray popover. |
| `stt_engine` / `stt_model_id` | `whisper-local` / tier-recommended | Auto-tier via `recommend_tier`. |
| `llm_engine` / `llm_model_id` | `gemma-local` / tier-recommended | |
| `polishing_enabled` | `true` | UI toggle that maps to `llm_engine ∈ {gemma-local, none}`. Persisted independently so the user's intent survives engine outages. |
| `language` | `auto` | Whisper language hint; `auto` lets Whisper detect. |
| `active_mode_id` | `general` | One of `general`, `email`, `chat`, `code-comment`, `notes`. |
| `auto_suggest_modes` | `true` | Pill suggests a mode based on the focused app/window. Suggestion only — never auto-switches. |
| `appearance` | `dark` | `light` or `dark`; coerced to `dark` if unrecognised. |
| `inference_api_key` | `""` | Random token generated lazily on first `llama-server` spawn (see `inference_proc::persistent_api_key`); reused so the startup-time orphan reaper can identify our child processes. Loopback-only — not a security secret. |

## Pipeline

Hotkey down → **Listening** pill + live waveform → hotkey up → **Transcribing** (Whisper local) → optional **Polishing** (Gemma local with the active mode's system prompt + dictionary) → **Done** with `Copied — press ⌘V to paste.`

Errors surface in the pill: missing model, mic denied, audio device error, silence, polishing timeout. The pipeline is fail-open: any LLM error logs `WARN` and the raw Whisper output is copied instead.

## Voice Modes

Five built-ins seeded from `src-tauri/src/modes.rs::builtins()` on first launch:

- `general` — Balanced cleanup. Default fallback.
- `email` — Formal tone, full punctuation.
- `chat` — Casual, drops trailing periods, capped length.
- `code-comment` — Lowercase, slash-prefixed.
- `notes` — Sentence-case bullet style.

Modes are JSON files under `~/Library/Application Support/com.vox.app/modes/`. Built-ins re-seed if deleted (`reset_default_modes`). Users can clone via `duplicate_mode`, edit via `save_mode`, export/import as JSON, or auto-trigger by bundle id glob / window-title substring.

Each mode contributes:
- `system_prompt` — given to the cleanup engine.
- `dictionary` — merged with the global `dictionary` and pushed into both the STT bias prompt and the cleanup prompt.
- `punctuation` / `case` / `max_length_chars` — applied as post-processing after the LLM step.

## Permissions

- **Microphone** — required, prompted lazily inside the onboarding wizard via `AVCaptureDevice.requestAccessForMediaType`. The non-prompting `authorizationStatusForMediaType:` is used everywhere else (so the tray icon doesn't trigger a TCC prompt at launch).
- **Accessibility** — **no longer required** for the dictation pipeline. It is still optionally used by the modifier-only chord hotkey path (`mac_chord.rs`, `CGEventTap`), but the default `Cmd+Ctrl+Space` shortcut does not need it.

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
│       ├── inject.rs                  ← clipboard write (no keystroke synth)
│       ├── hotkey.rs                  ← global shortcuts + chord fallback chain
│       ├── mac_chord.rs               ← CGEventTap modifier-only chord listener
│       ├── mac_perm.rs                ← AVFoundation auth FFI (non-prompting)
│       ├── pill.rs                    ← pill window + state events
│       ├── onboarding.rs              ← wizard window lifecycle
│       ├── tray.rs, tray_icon.rs      ← menu-bar icon + popover anchor
│       ├── tray_glyphs.rs             ← state glyphs
│       ├── frontmost.rs               ← focused-app probe for mode suggestion
│       ├── transcripts.rs             ← in-memory ring buffer (cleared on quit)
│       ├── settings.rs                ← tauri-plugin-store + legacy scrubbing
│       ├── modes.rs                   ← Voice Modes store + JSON I/O + post-processing
│       ├── models.rs                  ← model catalog + resumable downloader (`stream_to_file` is reused for the llama-server archive)
│       ├── inference_server.rs        ← llama-server release-binary catalog + downloader/extractor
│       ├── inference_proc.rs          ← spawn / health-poll / supervise / orphan-reap the llama-server child
│       ├── commands.rs                ← #[tauri::command] handlers
│       └── engines/
│           ├── mod.rs                 ← Registry + SttEngine / LlmEngine traits + STT pre-warm helper
│           ├── whisper_local.rs       ← whisper-rs 0.16 (CoreML+Metal), in-process
│           ├── gemma_http.rs          ← OpenAI chat-completions client against the loopback llama-server
│           └── none.rs                ← no-op LLM (raw transcript)
└── scripts/
    ├── dev-bundle.sh                  ← build + open a debug .app for TCC sanity
    ├── dev-clean-state.sh             ← wipe ALL Vox state for a clean first run
    └── dev-reset-permissions.sh       ← only resets TCC, keeps settings/models
```

Files that no longer exist (and the migrations that removed them) are documented inline: `settings.rs` (cloud scrub), `inject.rs` (Cmd+V removal), and `engines/gemma_http.rs` (replaced the old in-process `gemma_local.rs` + `llama-cpp-2` to keep C++ asserts from killing the menu-bar app).

## Development

### First run

```bash
npm install
npm run tauri dev
```

The first compile is slow because `local-engines` pulls in `whisper-rs` with its Metal backend. Gemma polishing now lives in a separate `llama-server` subprocess we download at runtime, so it no longer participates in the Cargo build. Subsequent compiles are incremental.

Onboarding fires automatically when `onboarded == false`: Welcome → Mic → Accessibility (optional, only matters for chord hotkeys) → Download local models → How to use.

### Reset for a clean first-launch experience

```bash
./scripts/dev-clean-state.sh   # wipes settings, modes, models, TCC, legacy keychain entries
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
| Keychain (legacy only) | service `app.vox.vox`, accounts `groq` / `anthropic` — auto-scrubbed on launch |

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

`tauri dev` produces an unbundled binary at `src-tauri/target/debug/vox` whose code signature changes on every rebuild. macOS TCC keys grants by signature, so chord-hotkey users may see "Vox is in the Accessibility list but `AXIsProcessTrusted()=false`". `./scripts/dev-bundle.sh` builds a proper `.app` whose identity is stable; use that for any work that touches Accessibility.

The default `Cmd+Ctrl+Space` shortcut sidesteps this entirely — it goes through the Carbon `RegisterEventHotKey` path which doesn't require Accessibility.

## Settings tabs

- **General** — language, hotkey behaviour (toggle vs hold), launch at login, dictation toggle.
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
- **Engines pre-warm in the background at launch.** `lib.rs::run`'s setup hook spawns `inference_proc::prewarm_in_background` (boots `llama-server` if the binary + Gemma model are installed) and `engines::prewarm_stt_in_background` (cold-loads the Whisper GGML weights). Both are idempotent and no-op when their preconditions aren't met, so the first dictation doesn't pay the spawn / load cost.
- **`llama-server` orphans get reaped at startup.** macOS has no `prctl(PR_SET_PDEATHSIG)`, so a force-killed previous Vox parent can leave a `llama-server` child running. `inference_proc::reap_orphans_at_startup` walks the process table and SIGKILLs any whose `--api-key` argument matches the persisted `inference_api_key`. That's why the api key is persisted across launches even though it's not security-relevant.

## Known limitations

- **Auto-update is stubbed.** `commands::check_for_updates` short-circuits to `unconfigured` until the placeholder endpoint and pubkey are replaced and the bundle is signed + notarized.
- **No code signing yet.** Built `.app` is ad-hoc signed; macOS Gatekeeper will block it on first launch (control-click → Open once to trust).
- **Gemma polish runs out-of-process.** `inference_server.rs` pins `LLAMA_CPP_TAG` (currently `b8953`) and downloads the matching official `llama-server` archive from the llama.cpp GitHub release. `inference_proc.rs` spawns it as a process-group / Job-Object child so the parent's death tears the child down too, even on SIGKILL. Bumping llama.cpp = bump `LLAMA_CPP_TAG` and (when set) `expected_sha256`; users on the old tag re-download into a different version directory and `cleanup_stale_versions` reaps the previous tree on the launch after.
- **MatFormer Gemma 4 GGUFs from Unsloth crash llama.cpp.** Their conversion produces `GGML_OP_GATED_DELTA_NET` tensors whose names don't carry the `LLAMA_TENSOR_NAME_FGDN_AR` prefix that the FGDN auto-detect path in `llama-context.cpp::sched_reserve` expects, so a `GGML_ASSERT` fires. With the engine out-of-process the assert now exits the child cleanly and `session.rs` fail-opens to the raw transcript (instead of crashing the menu-bar app, as it did under the old in-process `llama-cpp-2`). **bartowski's MatFormer Gemma 4 GGUFs avoid this** — their conversion uses tensor names that do not trip FGDN auto-detect, so we source E2B and E4B from bartowski. If a future Unsloth-converted MatFormer variant is ever added, gate it via `is_known_broken_llm_id`.
- **LLM catalog**: `models.rs` ships three polishing options — **Gemma 4 E2B** (`gemma-4-e2b-q4_k_m`, ~3.5 GB, small — default for <12 GB RAM; bartowski), **Gemma 4 E4B** (`gemma-4-e4b-q4_k_m`, ~5.4 GB, balanced — default for ≥12 GB RAM; bartowski), and **Gemma 4 26B-A4B MoE** (`gemma-4-26b-a4b-q4_k_m`, ~17 GB, quality; Unsloth UD-Q4_K_M for its imatrix calibration edge — safe under any converter because the MoE graph has zero GDN ops, dispatching through `llm_build_gemma4_iswa`'s standard `build_attn` path with `shared_kv_layers=0` and no per-layer embedding). All Gemma releases share the same `<start_of_turn>` chat template. `settings.rs::load` migrates retired ids (currently just `phi-4-mini-q4_k_m`) to the current default via `models::is_known_broken_llm_id`.
- **Pill always renders on the primary display.** Multi-display follow-the-focus is a future enhancement.

## Contact points for changes

- New IPC command: add to `src-tauri/src/commands.rs`, register in `src-tauri/src/lib.rs::invoke_handler!`, expose a typed binding in `src/lib/ipc.ts`.
- New Settings field: extend the `Settings` struct + `Default` impl + `load`/`save` in `settings.rs`, mirror the field in `src/lib/ipc.ts::Settings`, and add a one-shot migration if old installs need backfill.
- New Voice Mode built-in: add a clone in `modes::builtins()` plus a brand new `*_PROMPT` constant, update tests in `modes::tests`.
- New engine: implement `SttEngine` or `LlmEngine` in `engines/`, wire it into `engines::Registry`, add a settings-id case for it in `set_settings` migration if reachable.
