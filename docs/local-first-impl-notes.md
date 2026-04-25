# Vox — Local-First Implementation Notes

> Status: landed. Sr Eng B handoff for Voice Modes (next agent) and QA.
> Date: 2026-04-24.

This file is the reality-vs-spec diff for the local-first pivot. It
catalogs every file touched, every settings key added or removed, the
exact engine trait surface I shipped, the model catalog with URLs (and
the sha256 status), the download flow, and the open TODOs.

## 1. Files touched

### Created
- `src-tauri/src/engines/mod.rs` — `SttEngine` and `LlmEngine` traits +
  `Registry` (lazy, process-wide).
- `src-tauri/src/engines/whisper_local.rs` — local Whisper via
  `whisper-rs` 0.16 (`coreml + metal` features). Lazy-loaded
  `WhisperContext` cached in a `OnceCell<Arc<Mutex<…>>>`. Decodes the
  16 kHz PCM-16 WAV `audio.rs` produces back into `f32` for inference.
- `src-tauri/src/engines/groq.rs` — cloud STT trait wrapper. Same wire
  shape as the original `stt.rs`.
- `src-tauri/src/engines/anthropic.rs` — cloud LLM cleanup trait
  wrapper. Same wire shape as the original `llm.rs`.
- `src-tauri/src/engines/gemma_local.rs` — local LLM cleanup via
  `llama-cpp-2` 0.1 (`metal`). Lazy backend + model load with greedy
  decoding and a hard 3 s wall-clock timeout (fail-open to raw
  transcript).
- `src-tauri/src/engines/none.rs` — no-op cleanup engine. Replaces the
  old `cleanup_enabled = false` boolean.
- `src-tauri/src/models.rs` — compile-time catalog, on-disk cache
  (`~/Library/Application Support/com.vox.app/models/<kind>/<id>/`),
  resumable HTTP `Range` downloader, streaming sha256 verification,
  cancel via `tokio_util::sync::CancellationToken`, progress events
  emitted at most every 200 ms.
- `src-tauri/src/migration.rs` — opens the soft-migration window.
- `src/views/Migration.tsx`, `src/entries/migration.tsx`,
  `migration.html` — one-time migration dialog window.
- `docs/local-first-impl-notes.md` — this file.

### Modified
- `src-tauri/Cargo.toml` — added `whisper-rs`, `llama-cpp-2`,
  `async-trait`, `tokio-util`, `sysinfo`, `dirs`, `sha2`, `num_cpus`,
  `futures-util`, `encoding_rs`. Added the `local-engines` feature so
  `cargo check` / clippy stays fast on CI; production builds opt in.
- `src-tauri/src/lib.rs` — registered the new `engines` and `models`
  modules, the migration window module, the engine `Registry` in app
  state, and the new IPC commands. Soft-migration trigger is the
  startup branch right after onboarding.
- `src-tauri/src/settings.rs` — full rewrite of the keys/store with
  the local-first additions and migration of the legacy
  `cleanup_enabled` / `send_audio_to_cloud` booleans (see §2). Dropped
  `cleanup_enabled` and `send_audio_to_cloud` from `Settings`.
- `src-tauri/src/session.rs` — engine-driven pipeline. Reads
  `prefs.stt_engine` / `prefs.llm_engine` and resolves through the
  registry. Passes `prefs.language` and `prefs.dictionary` through to
  both STT and LLM engines.
- `src-tauri/src/commands.rs` — added `list_models`, `download_model`,
  `cancel_download`, `delete_model`, `load_model_warm`,
  `recommend_tier`, `accept_gemma_tos`, `dismiss_migration`. Migration
  window joins `SECRET_WINDOWS`. Cloud test commands now delegate to
  the engine modules.
- `src-tauri/entitlements.plist` — added
  `com.apple.security.cs.allow-jit` so llama.cpp/whisper.cpp Metal
  shader compilation succeeds under hardened runtime.
- `src-tauri/capabilities/default.json` — `migration` joins the
  allowed window list.
- `src/lib/ipc.ts` — added the model catalog/event/command types and
  wrapper functions, plus `acceptGemmaTos`/`dismissMigration`/
  `recommendTier`. Removed `cleanup_enabled` and `send_audio_to_cloud`
  from the `Settings` interface.
- `src/views/Settings.tsx` — new "Models" tab with engine + model
  pickers, per-model status (Downloaded / Downloading X% / Use /
  Delete), Gemma TOS notice. Privacy tab is now derived (read-only)
  from `stt_engine` / `llm_engine`.
- `src/views/Onboarding.tsx` — added a "Choose your stack" step
  between Accessibility and the API-keys step. If the user picks
  "Run offline (recommended)", the wizard skips API keys and runs a
  combined Whisper+Gemma download with a single progress bar; an
  inline Gemma-TOS click-through gates the download. The post-
  download "Try it now" step works fully offline.
- `src/views/TrayPopover.tsx` — status dot honours the new
  engine state (Local engines need no API key).
- `vite.config.ts` — registered `migration.html` as a fourth entry.

### Deleted
- `src-tauri/src/stt.rs`
- `src-tauri/src/llm.rs`

## 2. Settings keys added / removed

### Added (persisted in the tauri-plugin-store JSON, not the Keychain)
| Key | Type | Default | Notes |
| --- | --- | --- | --- |
| `stt_engine` | `String` | `"whisper-local"` for new, `"groq"` for upgraders | Used at every `begin_recording` to pick the STT engine |
| `stt_model_id` | `String` | tier-recommended (Balanced → `whisper-large-v3-turbo-q5_0`) | One of the catalog ids |
| `llm_engine` | `String` | `"gemma-local"` for new; `"anthropic"` if upgrader has Anthropic key, else `"none"` | One of `gemma-local`/`anthropic`/`none` |
| `llm_model_id` | `String` | `"gemma-4-e2b-q4_k_m"` | One of the catalog ids |
| `migration_seen` | `bool` | `false` | Flipped to `true` once the migration dialog is dismissed |
| `gemma_tos_accepted` | `bool` | `false` | Flipped on first Gemma download click-through |

### Removed
- `cleanup_enabled: bool` — superseded by `llm_engine = "none"`. The
  load path migrates `cleanup_enabled = false` → `llm_engine = "none"`.
- `send_audio_to_cloud: bool` — replaced by `stt_engine`. The Privacy
  toggle is gone; the Privacy tab now shows a derived line for each
  data-flow direction (audio + transcript) based on the engine
  choice. Existing `send_audio_to_cloud == true` users on upgrade map
  to `stt_engine = "groq"`.

Both legacy keys are still *read* by the load path so existing users
don't lose their preferences; on the first save they're deleted from
the store.

## 3. Engine trait surface (final, for Voice Modes)

```rust
#[async_trait::async_trait]
pub trait SttEngine: Send + Sync {
    async fn transcribe(
        &self,
        wav: &[u8],
        language: Option<&str>,
        dictionary: &[String],
    ) -> anyhow::Result<String>;

    fn id(&self) -> &'static str;
    fn label(&self) -> String;
    fn requires_network(&self) -> bool;
}

#[async_trait::async_trait]
pub trait LlmEngine: Send + Sync {
    /// `system_prompt` is reserved for Voice Modes — pass `""` to use
    /// the engine's built-in default cleanup prompt.
    async fn cleanup(
        &self,
        transcript: &str,
        system_prompt: &str,
        dictionary: &[String],
    ) -> anyhow::Result<String>;

    fn id(&self) -> &'static str;
    fn label(&self) -> String;
    fn requires_network(&self) -> bool;
}
```

The engine `Registry` lazy-builds an `Arc<dyn SttEngine>` /
`Arc<dyn LlmEngine>` per `(engine_id, model_id)` pair and caches it.

**Voice Modes integration point:** the `cleanup()` `system_prompt`
parameter is the only seam Voice Modes needs. Pass an empty string
for the built-in transcript-cleaner prompt; pass a mode-specific
prompt (e.g. "rewrite this in Slack tone") to override.

## 4. Model catalog shipped

| id | kind | URL | size (MB) | sha256 | tier |
| --- | --- | --- | --- | --- | --- |
| `whisper-tiny-en-q5_1` | stt | `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en-q5_1.bin` | ~78 | TBD-pin-on-first-download | small |
| `whisper-small-en-q5_1` | stt | `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.en-q5_1.bin` | ~264 | TBD | small |
| `whisper-large-v3-turbo-q5_0` | stt | `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin` | ~574 | TBD | balanced (DEFAULT) |
| `whisper-large-v3-q5_0` | stt | `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-q5_0.bin` | ~1100 | TBD | quality |
| `gemma-4-e2b-q4_k_m` | llm | `https://huggingface.co/unsloth/gemma-4-E2B-it-GGUF/resolve/main/gemma-4-E2B-it-Q4_K_M.gguf` | ~1300 | TBD | balanced (DEFAULT) |
| `gemma-4-e4b-q4_k_m` | llm | `https://huggingface.co/unsloth/gemma-4-E4B-it-GGUF/resolve/main/gemma-4-E4B-it-Q4_K_M.gguf` | ~2500 | TBD | quality |
| `phi-4-mini-q4_k_m` | llm | `https://huggingface.co/bartowski/microsoft_Phi-4-mini-instruct-GGUF/resolve/main/microsoft_Phi-4-mini-instruct-Q4_K_M.gguf` | ~2400 | TBD | quality (Gemma backup) |

sha256s are **not** pinned in code at this commit — every catalog
entry has `sha256: None`. On first successful download we compute the
hash and append `<id> <hex>` to
`~/Library/Application Support/com.vox.app/models/models.lock`. Once
QA has run a download from a clean machine, copy the lock contents
into the catalog and flip the `Option`s to `Some(…)`. Treat the
in-code hash as authoritative going forward; the lock file then acts
as a debug receipt.

## 5. Download flow

1. Frontend calls `download_model(id)` IPC.
2. Rust resolves the catalog descriptor, ensures
   `~/.../models/<kind>/<id>/` exists, opens (or appends to) a
   `.partial` file in the cache dir.
3. `reqwest` `GET` with `Range: bytes=<resume_from>-` if the partial
   exists; otherwise plain GET.
4. Stream the body, hashing each chunk with SHA-256 and writing to
   disk. Emit `model:progress` events to the frontend at most every
   200 ms with `bytes_downloaded` / `total_bytes` / `eta_secs`.
5. On completion: if the catalog has a pinned sha256 we verify; if it
   doesn't, we append the computed hash to `models.lock` for QA.
6. Atomic rename of `.partial → model.bin` (or `model.gguf`).
7. Emit `model:done`.

Cancellation goes through a `CancellationToken` stashed in a
`once_cell::Lazy<Arc<Mutex<HashMap<id, token>>>>`. A canceled download
emits `model:canceled`; partial bytes are kept on disk so a later
resume can pick up where it stopped.

## 6. Tier auto-pick

`commands::recommend_tier` is exposed for both Onboarding and the
Migration dialog. It detects physical RAM via the `sysinfo` crate; if
RAM `< 12 GB` it sets `tier = "small"` and `low_ram_warning = true`.
Per the CEO decision the downgrade is **soft** — we recommend the
Small tier with a dismissable banner; we never force the user.

## 7. Build status

| Check | Result |
| --- | --- |
| `cargo check --manifest-path src-tauri/Cargo.toml` | clean |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` | clean |
| `cargo check --manifest-path src-tauri/Cargo.toml --features local-engines` | clean (heavy first-time C++ build) |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --features local-engines --all-targets -- -D warnings` | clean |
| `npm run build` | clean |
| `npm run tauri build -- --bundles app` | OK — bundles `Vox.app` (cloud-only, ~7 MB) |
| `cargo build --release --features local-engines` | OK (~1m 42s incremental, ggml linked statically) |

`local-engines` is **opt-in** at build time via `--features
local-engines`. Default is off so:
- CI `cargo check` / `clippy` doesn't need cmake or Xcode CLT for
  ggml.
- Cloud-only QA builds skip the heavy C++ compile.

For the production DMG, bundle with `--features local-engines` and
keep no model files in the `.app` bundle (per the CTO doc — DMG stays
~25 MB).

## 8. Known TODOs / risks

| # | Item | Owner | Notes |
| --- | --- | --- | --- |
| T-01 | Pin sha256 for every catalog entry | QA | Run one clean download, copy `models.lock` into the catalog `sha256:` slots, flip `Option<&str>` to `Some(…)`, ship next minor. |
| T-02 | Whisper Core ML encoder bundle | Sr Eng B follow-up | The catalog only downloads the `.bin`. The `.mlmodelc.zip` companion isn't yet auto-fetched; on first Whisper run we fall back to Metal-only (~2× slower). Add a "companion files" array per-descriptor and unzip into the same dir. |
| T-03 | Greedy decoding only for Gemma | follow-up | `gemma_local` does argmax token selection. Quality is fine for cleanup; Voice Modes (free-form rewrites) will want temperature/top-p. The trait already has the `system_prompt` slot so the call-site stays stable. |
| T-04 | Memory pressure unload | follow-up | The CTO doc calls for a 10-min idle timer + DISPATCH_SOURCE_TYPE_MEMORYPRESSURE. Not wired in this cut. Acceptable for v1; consider before shipping wider. |
| T-05 | Streaming token paste | deferred to v2 | CTO punted; trait shape doesn't preclude it. |
| T-06 | Hotkey live re-register | deferred (Phase 3 carryover) | unchanged from phase3-handoff |
| R-01 | Long first-run download abandonment | product | Mitigated by tier auto-pick + Small option in Onboarding. |
| R-02 | RAM blowups on 8 GB Macs | product | Soft downgrade banner is in place; aggressive unload is T-04. |
| R-03 | Gemma TOS click-through coverage | legal | Onboarding + Migration both gate the download behind the TOS click; users who land on the Settings → Models tab and download Gemma directly *also* see a banner pointing at the TOS but the click-through happens in the Models tab via the same `accept_gemma_tos` command. |

## 9. Coordination notes for Voice Modes

- The only trait surface to integrate with is
  `LlmEngine::cleanup(&self, transcript, system_prompt, dictionary)`.
  Pass your mode-specific system prompt into `system_prompt`.
- The session orchestrator currently hardcodes `system_prompt = ""`
  (default). When you wire Voice Modes, plumb the active mode through
  `prefs` (probably a new `voice_mode` settings key with a builtin
  catalog plus a user-defined slot) and pass its prompt down to
  `engine.cleanup(...)`.
- `NoOpEngine` is selected when the user picks "Off" — Voice Modes
  shouldn't try to override that. If a mode requires cleanup it should
  surface a helpful message rather than silently flipping
  `llm_engine`.

## 10. Test plan

Dev smoke-test on a clean Mac:

1. `npm run tauri dev` — onboarding shows.
2. Choose "Run offline" → tier picker shows your detected RAM.
3. Accept Gemma TOS → both downloads start in parallel, progress bar
   advances.
4. Pre-warm completes; "Try it now" lets you dictate offline.
5. Quit + relaunch — no migration dialog (we set `migration_seen` on
   onboarding completion via the patch in step 3).
6. Settings → Models tab: switch to Quality tier, watch the new model
   download, click "Use", dictate, verify the engine label updated.
7. Privacy tab: confirm the chips show LOCAL/LOCAL.
8. Switch STT engine to Groq in Models tab; Privacy chip flips to
   CLOUD; dictation now needs the Groq key (banner appears in API
   Keys tab).

For an "upgrade existing user" smoke-test:

1. Start with a settings store that has `groq_api_key` set,
   `cleanup_enabled = true`, `send_audio_to_cloud = true`,
   `migration_seen` absent.
2. Launch app — migration dialog appears.
3. Click "Switch to local models" → downloads start, settings
   persisted.
4. Quit + relaunch → dialog gone (`migration_seen = true`), local
   engines active.
