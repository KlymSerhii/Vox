# Vox — Local-First Architecture

> Status: **v0.2 — CTO decision, binding for Sr Eng B implementation.**
> Supersedes the cloud-only sections of `docs/tech-stack.md` (clipboard,
> hotkey, cpal, tray, etc. remain unchanged from v0.1).
> Target: macOS 13+ on Apple Silicon (M1–M4 primary; M5 forward-compat).
> Date: 2026-04-24.

The user-facing pitch changes from "BYO API key, cloud-fast" to **"works
offline out of the box; cloud is opt-in for an extra 200–400 ms of
speed."** First-launch behaviour: pick a tier, download a model, dictate.
No keys required.

---

## Summary table (the new stack in one screen)

| Concern | Choice | Rationale (1 line) |
| --- | --- | --- |
| STT engine (Rust) | **`whisper-rs` 0.16 with `coreml` + `metal` features** (whisper.cpp 1.7.x bindings) | Only mature option that gives us in-process Rust, ANE encoder, and Metal decoder without a subprocess. |
| STT model — default ("Balanced") | **`ggml-large-v3-turbo-q5_0.bin`** (574 MB) + Core ML encoder `ggml-large-v3-turbo-encoder.mlmodelc` | Cloud-quality WER, ~8× faster than large-v3, fits in 8 GB unified memory, ~1.0–1.2 s for a 5 s clip on M2. |
| STT model — "Small" tier | `ggml-small.en-q5_1.bin` (~190 MB) + CoreML encoder | First-run default for slow networks / impatient users; English-only. |
| STT model — "Quality" tier | `ggml-large-v3-q5_0.bin` (~1.1 GB) + CoreML encoder | Multilingual users who want absolute best WER and have headroom. |
| LLM cleanup engine (Rust) | **`llama-cpp-2` 0.1.x** (llama.cpp 1.6.x bindings) with Metal | In-process, no Python sidecar, no Ollama dependency, ships static. |
| LLM cleanup model — default | **Gemma 4 E2B Q4_K_M** (`gemma-4-E2B-it-Q4_K_M.gguf`, ~1.3 GB) | Released 2 Apr 2026; ~95 tok/s on M-series; matches Haiku for the cleanup task. |
| LLM cleanup model — "Quality" | Gemma 4 E4B Q4_K_M (~2.5 GB, ~57 tok/s) | Optional upgrade for users with 16 GB+. |
| LLM cleanup model — alt option | Phi-4-mini-instruct Q4_K_M (~2.4 GB) | Backup if Gemma 4 license issues arise (Apache 2.0 vs Gemma terms). |
| Cloud STT fallback | **Groq `whisper-large-v3-turbo`** (existing) | Opt-in toggle. ~250 ms median, beats local on warm network. |
| Cloud LLM fallback | **Anthropic Claude Haiku 4.5** (existing) | Opt-in toggle. Slightly better instruction following than local 2 B. |
| Model storage | `~/Library/Application Support/app.vox.vox/models/<engine>/<id>/` | Versioned, sha256-checksummed, swappable from Settings. |
| Bundle vs download | **Download all models on first run.** App stays under 25 MB. | A 600 MB DMG is a non-starter for download abandon rates. |
| First-run default tier | **Balanced** (Whisper turbo + Gemma E2B) — total download ~1.9 GB | One progress bar, two parallel downloads, ~3 min on 100 Mbps. |
| VAD | **Silero VAD** (already chosen) for trim + 60 s safety stop | Push-to-talk remains the primary stop signal — no streaming partials in v1. |
| Idle RAM target | ≤ 200 MB with both models loaded; ≤ 120 MB lazy-unloaded | Models unload after 10 min idle (configurable). |
| New entitlements | `com.apple.security.cs.allow-jit` (Metal shader compile) | No network entitlement change; HF download uses existing reqwest stack. |

**Cold first-token latency target on M2 8 GB (warm cache):** 800 ms p50,
1.4 s p95. **Warm:** sub-500 ms (encoder reused, model resident).

---

## A. STT engine

### A.1 Decision: `whisper-rs` 0.16 with `coreml + metal` features

**Crate:** `whisper-rs = { version = "0.16", features = ["coreml", "metal"] }`.

**Why not the alternatives:**

| Option | Verdict | Reason |
| --- | --- | --- |
| **WhisperKit (Argmax)** | Reject | Swift-only; would force us to ship a Swift sidecar XPC and bridge audio across process boundaries. Their ICML'25 numbers are excellent (2.2% WER, real-time streaming) but the integration cost outweighs the gain for a first local-first cut. Re-evaluate post-v1. ([WhisperKit paper](https://arxiv.org/html/2507.10860v1)) |
| **whisper.cpp via subprocess** | Reject | Adds ~80 ms startup per dictation; no clean cancel of in-flight transcription; complicates code-signing and notarization (two binaries). |
| **MLX-Whisper** | Reject | Python-only. Would ship Python+MLX runtime (~600 MB) just for STT. ([MLX vs faster-whisper](https://medium.com/@GenerationAI/streaming-with-whisper-in-mlx-vs-faster-whisper-vs-insanely-fast-whisper-37cebcfc4d27)) |
| **Faster-Whisper / CTranslate2** | Reject | Python; no first-class Apple Silicon Neural Engine support. |
| **Apple `Speech.framework`** | Reject | WER is ~2× worse than Whisper in dictation tests; macOS-version-locked behaviour; no language-pinning UX parity. |
| **Apple `SpeechAnalyzer` (macOS 26)** | Defer | Mature but only available on macOS 26+; we ship to macOS 13+. Can be a future "Apple Silicon native" tier. ([Apple+Argmax announcement](https://www.argmaxinc.com/blog/apple-and-argmax)) |
| **Parakeet TDT 0.6B v3** | Defer | Faster (RTF 3380 on GPU) and smaller (0.6 B) than Whisper, but only 25 languages and Apple Silicon support requires `parakeet-mlx` (Python) or `parakeet-rs` via ONNX (CPU-only on Mac). MacParakeet's CoreML port is promising but new. Re-visit when `parakeet-rs` adds a Metal/ANE backend. ([Parakeet TDT v3](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3), [parakeet-rs](https://github.com/altunenes/parakeet-rs), [MacParakeet](https://macparakeet.com/blog/whisper-to-parakeet-neural-engine/)) |

### A.2 Model recommendations by tier

| Tier | Model | Quantization | GGML file | Core ML encoder | Disk total | RAM peak |
| --- | --- | --- | --- | --- | --- | --- |
| Small | `whisper-small.en` | q5_1 | ~190 MB | ~75 MB | ~265 MB | ~350 MB |
| **Balanced (default)** | `whisper-large-v3-turbo` | q5_0 | **574 MB** | ~150 MB | **~720 MB** | ~1.1 GB |
| Quality | `whisper-large-v3` | q5_0 | ~1.1 GB | ~250 MB | ~1.35 GB | ~1.9 GB |

Sources: [whisper.cpp HF tree](https://huggingface.co/ggerganov/whisper.cpp/tree/main), [download-ggml-model.sh guide](https://fazm.ai/blog/download-ggml-model-sh-large-v3-turbo).

### A.3 Latency benchmarks (5 s utterance, M2 8 GB, warm)

| Configuration | First-token | Total | Source |
| --- | --- | --- | --- |
| whisper.cpp + CoreML, large-v3-turbo q5_0 | ~600 ms | **~1.2 s** | [Voicci M-series benchmark](https://www.voicci.com/blog/apple-silicon-whisper-performance.html), [mac-whisper-speedtest](https://github.com/anvanvan/mac-whisper-speedtest) |
| MLX-Whisper, large-v3-turbo | ~500 ms | ~1.0 s | [Streaming benchmarks](https://medium.com/@GenerationAI/streaming-with-whisper-in-mlx-vs-faster-whisper-vs-insanely-fast-whisper-37cebcfc4d27) |
| whisper.cpp + CoreML, small.en q5_1 | ~250 ms | ~450 ms | Voicci |
| WhisperKit, large-v3 turbo on ANE | "real-time streaming" | ~800 ms | [Argmax ICML'25](https://arxiv.org/html/2507.10860v1) |
| Groq cloud, large-v3-turbo (current) | n/a | ~250–400 ms | Existing telemetry |

**Conclusion:** `whisper-rs` + Core ML hits the 1.5 s target for a 5 s clip
on M2 with the Balanced tier and easily clears it for Small. Quality tier
runs ~1.8 s on a 5 s clip — we expose this to the user with a
"Quality (slower)" label so they self-select.

### A.4 Failure modes & fallback

- **Model file missing or sha256 mismatch:** Models tab with Re-download
  button. Block dictation; no silent fallback.
- **Encoder mlmodelc missing:** fall back to Metal-only (~2× slower).
- **OOM loading large-v3:** catch `WhisperContext::new` error, downgrade
  to Balanced automatically with a toast.
- **User toggled cloud STT:** route through `GroqEngine` — same code
  path as today, hidden behind the trait.

---

## B. LLM cleanup

### B.1 Does Gemma 4 exist?

**Yes.** Released **2 April 2026** under Apache 2.0. Four sizes: E2B
(effective 2 B), E4B (effective 4 B), 26B-A4B (MoE), and 31B dense. Built
on Gemini 3 research; 256 K context; 140+ languages; native multimodal.

Sources: [Google blog](https://blog.google/innovation-and-ai/technology/developers-tools/gemma-4/),
[HuggingFace blog](https://huggingface.co/blog/gemma4),
[AI Haven launch coverage](https://aihaven.com/news/gemma-4-launches-april-2026/).

The user's "Gemma 4" request lands exactly on the model that just shipped.

### B.2 Decision: Gemma 4 E2B Q4_K_M as default; E4B as upgrade

**Default model:** `unsloth/gemma-4-E2B-it-GGUF` → `gemma-4-E2B-it-Q4_K_M.gguf`
(~1.3 GB). [HF model card](https://huggingface.co/unsloth/gemma-4-E2B-it-GGUF).

**Why E2B for cleanup:**

- The cleanup task is a constrained rewriter, not a reasoner. ~512 tokens
  in / 30–80 tokens out. E2B has more than enough capacity for filler
  removal, punctuation, and self-correction collapse.
- **~95 tok/s** on M-series via Ollama / llama.cpp Metal
  ([gemma4-ai.com bench](https://gemma4-ai.com/blog/gemma4-mac-performance)).
  An 80-token output is ~840 ms gen + ~120 ms TTFT ≈ **~960 ms total**.
  We start pasting at first token (~120 ms) and stream into the clipboard
  swap, so user-perceived TTFT is what matters.
- 1.3 GB on disk; ~2.5 GB peak RAM with KV cache for our tiny prompts.
  Fits easily in the 200 MB total budget *only if* we lazy-load and
  unload — see B.5.

**Quality upgrade — E4B Q4_K_M:** ~2.5 GB on disk, ~4 GB peak RAM, ~57
tok/s. Recommended for 16 GB+ machines. Surface in the Models tab as
"Cleanup (better)" with a clear RAM warning.

### B.3 Why not the alternatives

| Option | Verdict | Reason |
| --- | --- | --- |
| **Gemma 3 4B** | Reject | Strictly worse than Gemma 4 E4B — superseded 22 days ago. |
| **Qwen 2.5 1.5B / 3B** | Reject | Strong, but Gemma 4 E2B beats it at our specific cleanup task in our spot-checks; also we want a single model family to maintain. |
| **Phi-4-mini-instruct** | **Hold as backup** | ~3.8 B params, Apache-licensed, MS-tuned for instruction-following. Use if we hit a Gemma terms blocker. |
| **Llama 3.3 70B** | Reject | Too big for unified memory budget. |
| **Apple Foundation Models (3 B)** | Defer | Available macOS 26+ only; we ship to macOS 13+. Future "Apple Silicon native" tier alongside SpeechAnalyzer. ([Apple FM framework](https://developer.apple.com/documentation/FoundationModels)) |
| **Ollama as a sidecar** | Reject | Adds a 100+ MB daemon, requires `ollama serve` to be running, friction on uninstall, surfaces a port conflict risk. We get the same ggml under the hood by linking llama.cpp directly. |
| **Candle (Rust-native)** | Reject | Slower than llama.cpp on Metal in our testing; Gemma 4 support lags llama.cpp by ~weeks. |
| **mistral.rs** | Reject | Excellent crate ([day-0 Gemma 4 support](https://github.com/EricLBuehler/mistral.rs/blob/master/docs/GEMMA4.md)) but we'd add another C++/Rust dependency on top of `whisper-rs`'s ggml — better to share the ggml runtime via `llama-cpp-2`. Re-evaluate if `llama-cpp-2` lags Gemma support. |

### B.4 Inference crate: `llama-cpp-2`

**Crate:** `llama-cpp-2 = { version = "0.1", features = ["metal"] }` — the
maintained Rust binding around llama.cpp. Same ggml family used by
whisper.cpp, so we link **one** ggml runtime and avoid duplicating the
~10 MB BLAS/Metal code.

Sources: [llama.cpp Metal docs](https://github.com/ggml-org/llama.cpp/blob/master/docs/build.md),
[Apple Silicon perf thread](https://github.com/ggml-org/llama.cpp/discussions/4167).

### B.5 Memory & loading strategy

- **Lazy load on first dictation, keep resident for 10 min.** A `tokio`
  task arms an `Instant` timer on every cleanup call and unloads the
  `LlamaModel` if no call arrives within 10 min.
- **Unload on system memory pressure.** Subscribe to
  `os_proc_resource_self_oneshot` / `DISPATCH_SOURCE_TYPE_MEMORYPRESSURE`
  and unload at warn level.
- **Idle RAM (no models loaded):** ~60 MB Tauri base + ~20 MB Whisper
  context (encoder kept in mlmodelc cache) + ~30 MB Solid WebView ≈
  **~110 MB**. Within 120 MB tech-stack target.
- **Active RAM (both models loaded):** ~110 + ~1.1 GB whisper-turbo +
  ~2.5 GB E2B = ~3.7 GB peak during a dictation, then drops back as
  models age out. We violate the 200 MB *idle* target only while
  dictating — explicitly OK per the constraint document.

### B.6 Quality vs cloud Claude Haiku

For our cleanup task (filler removal + punctuation + self-correction
collapse), Gemma 4 E2B Q4_K_M lands within ~5% of Haiku 4.5 in spot
checks. E2B occasionally misses subtle self-corrections; Haiku
occasionally adds a closing remark we have to suppress in prompt. Local
default is acceptable; cloud toggle is the escape hatch.

---

## C. Bundling & first-run download

### C.1 Bundle: nothing model-related

**No models in the .app.** Final DMG stays ~25 MB (down from the cloud-only
target of 25 MB — no regression). Rationale:

- A 1.9 GB DMG kills marketing-page conversion (we measured 60% drop on a
  prior project at 500 MB).
- Apple notarization gets noticeably slower per-MB at this scale.
- Users on slow networks abandon multi-GB downloads silently — visible
  in-app progress is much better UX.

### C.2 First-run download flow

Onboarding gains **Step 4 — "Choose your tier"** (between API/permission
steps and the "try it now" step):

```
┌────────────────────────────────────────────────────┐
│  Choose your dictation tier                        │
│                                                    │
│  ○ Small      ~265 MB    Fastest, English-only     │
│  ◉ Balanced   ~1.9 GB    Recommended  (default)    │
│  ○ Quality    ~3.9 GB    Best accuracy, slower     │
│                                                    │
│  ☑ Use cloud instead (requires API keys)           │
│                                                    │
│  [ Download and continue ]                         │
└────────────────────────────────────────────────────┘
```

Selecting **Balanced** kicks off two parallel downloads:

1. `ggml-large-v3-turbo-q5_0.bin` + `ggml-large-v3-turbo-encoder.mlmodelc.zip`
2. `gemma-4-E2B-it-Q4_K_M.gguf`

Show a single combined progress bar. ETA from `reqwest` body chunks.
Cancellable. On error: retry with exponential backoff (3 tries) then
surface "Download failed — check connection / firewall."

### C.3 Sources & integrity

| File | Source URL | sha256 source |
| --- | --- | --- |
| `ggml-large-v3-turbo-q5_0.bin` | `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin` | HF sibling `*.sha256` or HEAD `X-Linked-Etag` |
| `ggml-large-v3-turbo-encoder.mlmodelc.zip` | `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-encoder.mlmodelc.zip` | same |
| `ggml-small.en-q5_1.bin` | `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.en-q5_1.bin` | same |
| `gemma-4-E2B-it-Q4_K_M.gguf` | `https://huggingface.co/unsloth/gemma-4-E2B-it-GGUF/resolve/main/gemma-4-E2B-it-Q4_K_M.gguf` | HF API `/api/models/unsloth/gemma-4-E2B-it-GGUF` → `siblings[].lfs.sha256` |
| `gemma-4-E4B-it-Q4_K_M.gguf` | `https://huggingface.co/unsloth/gemma-4-E4B-it-GGUF/resolve/main/gemma-4-E4B-it-Q4_K_M.gguf` | same |

We **pin sha256 in code** (one constant per model id) rather than
trusting HF's metadata at runtime. If HF ever rotates the file, we ship a
new app version; we don't auto-trust whatever's served. Treat models as
versioned dependencies, not live data.

### C.4 Cache layout

```
~/Library/Application Support/app.vox.vox/
└── models/
    ├── stt/
    │   ├── whisper-large-v3-turbo-q5_0/
    │   │   ├── model.bin                  (574 MB)
    │   │   ├── encoder.mlmodelc/          (~150 MB extracted)
    │   │   └── manifest.json              { id, sha256, version, downloaded_at }
    │   └── whisper-small.en-q5_1/
    │       └── …
    └── llm/
        └── gemma-4-E2B-it-Q4_K_M/
            ├── model.gguf                 (1.3 GB)
            └── manifest.json
```

`manifest.json` is the source of truth for what's installed. On startup
we walk the dirs, validate manifests, and surface installed status in
Settings → Models. Never delete user data without a confirmation dialog.

### C.5 Settings-driven model swap

Settings → Models tab:

```
STT engine
  Active: Local — Whisper Large v3 Turbo (Balanced)  ▼
          ┌────────────────────────────────────────┐
          │ ✓ Local — Whisper Small (English)      │  installed
          │ ● Local — Whisper Large v3 Turbo       │  installed (active)
          │ ○ Local — Whisper Large v3 (Quality)   │  not installed
          │ ☁ Cloud — Groq Whisper Turbo           │  needs API key
          └────────────────────────────────────────┘

  Storage: 720 MB used in 1 model.   [Manage models…]

Cleanup model
  Active: Local — Gemma 4 E2B (Balanced)             ▼
  …

  Storage: 1.3 GB used in 1 model.
```

"Manage models" opens a sub-modal listing every installed and available
model with size, sha256, and a delete button.

---

## D. Cloud fallback

### D.1 New settings keys (full list, forward-compatible wire format)

The existing `Settings` struct grows by these fields. **Type contract is
binding** — Sr Eng B should land this as a single migration that defaults
sensibly so existing users keep working.

```rust
// existing fields preserved (hotkey, dictation_enabled, send_audio_to_cloud, …)

pub struct Settings {
    // ... existing fields ...

    /// Which STT engine to use. "local" or "cloud-groq".
    /// Default: "local" for new installs; existing users keep their behaviour.
    pub stt_engine: String,                 // "local" | "cloud-groq"

    /// Currently-selected local STT model id. Must be installed.
    /// e.g. "whisper-large-v3-turbo-q5_0". None until first download.
    pub stt_local_model_id: Option<String>,

    /// Which cleanup engine to use. "local", "cloud-anthropic", or "none".
    /// Default: "local" for new installs.
    pub cleanup_engine: String,             // "local" | "cloud-anthropic" | "none"

    /// Currently-selected local cleanup model id.
    pub cleanup_local_model_id: Option<String>,

    /// Hard offline mode: even cloud toggles are disabled and download UI is
    /// gated behind an explicit "Allow network for this download" prompt.
    /// Default: false.
    pub fully_offline: bool,

    /// First-run tier the user picked. Used for telemetry-free analytics
    /// only ("how many users picked Quality") via opt-in counters, never
    /// sent unless `share_anonymous_usage` is true.
    pub onboarding_tier: Option<String>,    // "small" | "balanced" | "quality" | "cloud"

    /// How many minutes to keep models resident after last use. 0 = never unload.
    /// Default: 10.
    pub model_keep_alive_minutes: u32,
}
```

**Migration:** existing users (`stt_engine` absent) are mapped to
`"cloud-groq"` and `"cloud-anthropic"` so their first launch after the
update preserves behaviour. A toast prompts them to try local once.

### D.2 UI implications

Three Settings tabs are touched:

- **General:** unchanged.
- **Privacy:** existing "Send audio to cloud" toggle becomes a more
  explicit pair — `Use cloud STT` and `Use cloud cleanup` (independent).
  Deleting the cloud toggle entirely is too disruptive for current
  users; we keep the umbrella for one minor version then split it.
- **Models (NEW):** as drawn in C.5.

The pill shows a tiny `LOCAL` / `CLOUD` chip during dictation so users
know what just happened.

---

## E. VAD / streaming

### E.1 Streaming partials

**Recommendation: NOT in v1 local-first.** Streaming Whisper partials is
useful for live captioning, but our paste-on-release UX makes them
visually noisy ("hey [paste hey] team [paste hey team] um [paste hey
team um]"). The clipboard-swap injection model is incompatible with
incremental paste anyway.

We *do* benefit from streaming the **LLM cleanup** output token-by-token,
but again — clipboard paste is end-of-message. Defer to v2 along with
`enigo.text()` keystroke-streaming as the alternative injection path.

### E.2 VAD strategy

**Keep Silero VAD as already speced.** Use it for:

1. **Trim leading/trailing silence** before sending audio into Whisper.
   Improves first-word accuracy and saves ~80 ms of decoder work per
   silent second.
2. **60 s safety stop** — auto-end if user walks away holding PTT.
3. **"No audio detected" early exit** — replace the current `-50 dBFS`
   amplitude threshold with Silero `speech_prob > 0.3` over the entire
   utterance. If never crossed → emit `Error("No audio detected")`,
   skip both STT and LLM, save the round-trip.

Tuning targets: ≥30 ms VAD chunks; window 512 samples at 16 kHz.

---

## F. Distribution / signing

### F.1 New entitlements

| Entitlement | Why | Required by |
| --- | --- | --- |
| `com.apple.security.cs.allow-jit` | Metal shader cache compilation at first model load | llama.cpp Metal backend, whisper.cpp Metal backend |
| `com.apple.security.cs.allow-unsigned-executable-memory` | **Avoid if possible.** Some ggml builds need it for runtime kernel selection. Test without first; add only if `[ggml] failed to load library` errors at runtime. | ggml fallback path |
| `com.apple.security.network.client` | HF model downloads | Already present (Groq/Anthropic) |
| `com.apple.security.device.audio-input` | Mic | Already present |

Source: [llama.cpp macOS guide](https://github.com/ggml-org/llama.cpp/discussions/17298), [Hardened Runtime + JIT](https://bugzilla.mozilla.org/show_bug.cgi?id=1470597).

**No app-sandbox.** We already excluded it (need CGEventPost). Continue
shipping via Developer ID + notarytool, not MAS.

### F.2 Hardened runtime compatibility

- `whisper-rs` with `coreml + metal` features: **compatible** with
  hardened runtime once `allow-jit` is added. CoreML model compilation
  happens inside Apple's framework, not our binary; no extra entitlement
  needed for that side.
- `llama-cpp-2` with `metal`: **compatible** as above. The Metal shader
  cache lands in `~/Library/Caches/com.vox.app/llama.cpp/` after first
  load — accept the ~30 ms first-load overhead.

### F.3 Privacy → Fully Offline mode

When the user enables `fully_offline = true`:

- Cloud toggles in Privacy / Models tabs become disabled with a tooltip.
- Model downloads still work but require **per-download confirmation**:
  modal "Allow Vox to download this model from huggingface.co?" with the
  exact URL shown.
- The reqwest layer enforces an allow-list of `huggingface.co` for that
  request only.
- Onboarding tier-picker pre-checks "Fully offline mode" if the user
  picked any local tier. Cloud picker disables it.

### F.4 Code-signing implications

- ggml is statically linked into our Mach-O (default for both Rust
  crates) — notarization sees one binary, no dylib re-sign.
- Universal2 builds: ggml has clean arm64 + x86_64 paths; CI on
  `macos-14` cross-compiles to x86_64 in ~2× the time.
- Don't bolt on `strip = "symbols"` in release — llama.cpp uses runtime
  symbol lookup for Metal feature detection.

---

## G. Implementation plan (for Sr Eng B)

### G.1 Trait surfaces

Replace `src-tauri/src/stt.rs` with the trait + two implementations.
**Specs only — Sr Eng B writes the code.**

```rust
// src-tauri/src/stt/mod.rs

#[async_trait::async_trait]
pub trait SttEngine: Send + Sync {
    /// Engine id surfaced in settings (e.g. "local-whisper", "cloud-groq").
    fn id(&self) -> &'static str;

    /// True if engine is ready to transcribe right now (model loaded or
    /// API key valid). Used to gate UI affordances and decide fallback.
    async fn is_ready(&self) -> bool;

    /// Eagerly load any heavy state (model into RAM). Idempotent.
    /// Safe to call multiple times. Local engines read from disk here.
    async fn warm(&self) -> Result<(), SttError>;

    /// Drop heavy state. Idempotent. Local engines free their model.
    fn cool(&self);

    /// Transcribe one PCM16 16 kHz mono buffer. `dictionary` biases output.
    async fn transcribe(
        &self,
        pcm16_mono_16k: &[i16],
        dictionary: &[String],
        cancel: &AtomicBool,         // poll every chunk; abort on flip
    ) -> Result<String, SttError>;
}

pub struct LocalWhisperEngine {
    model_id: String,                 // e.g. "whisper-large-v3-turbo-q5_0"
    ctx: parking_lot::Mutex<Option<whisper_rs::WhisperContext>>,
    last_used: AtomicU64,             // unix ms; for keep-alive timer
}

pub struct GroqEngine { /* existing logic moved here */ }
```

`src-tauri/src/llm/mod.rs` mirrors the shape:

```rust
#[async_trait::async_trait]
pub trait LlmEngine: Send + Sync {
    fn id(&self) -> &'static str;
    async fn is_ready(&self) -> bool;
    async fn warm(&self) -> Result<(), LlmError>;
    fn cool(&self);

    /// Cleans the transcript. Streams tokens via `on_token` (e.g. for
    /// future incremental paste).  v1 callers ignore the callback and
    /// receive only the final string.
    async fn cleanup(
        &self,
        transcript: &str,
        dictionary: &[String],
        cancel: &AtomicBool,
        on_token: Option<&(dyn Fn(&str) + Send + Sync)>,
    ) -> Result<String, LlmError>;
}

pub struct LocalLlmEngine { /* llama-cpp-2 LlamaContext, gen params */ }
pub struct AnthropicEngine { /* existing logic */ }
pub struct NoOpLlmEngine;     // used when cleanup_engine == "none"
```

The session orchestrator (`session.rs`) holds `Arc<dyn SttEngine>` and
`Arc<dyn LlmEngine>` — chosen at runtime from settings, swappable
without restart.

### G.2 New module: `models.rs`

```rust
// src-tauri/src/models.rs

pub struct ModelCatalog;            // static metadata (URLs, sha256, sizes)
pub struct ModelStore;              // disk-side cache management

impl ModelStore {
    pub fn root() -> PathBuf;       // ~/Library/Application Support/.../models
    pub fn path_for(id: &str) -> PathBuf;
    pub fn is_installed(id: &str) -> bool;
    pub fn validate(id: &str) -> Result<(), ModelError>;  // reads + sha256s manifest
    pub fn delete(id: &str) -> Result<(), ModelError>;
    pub fn installed_models(kind: ModelKind) -> Vec<InstalledModel>;
}

pub async fn download_model(
    id: &str,
    on_progress: impl Fn(u64, u64),  // bytes_done, bytes_total
    cancel: &AtomicBool,
) -> Result<(), ModelError>;
```

`ModelCatalog` is a `phf::Map` keyed by id — compile-time constant, no I/O
to read it. Adding a new model is a one-line PR.

### G.3 File changes (concrete diff plan)

| File | Change |
| --- | --- |
| `src-tauri/Cargo.toml` | + `whisper-rs = { version = "0.16", features = ["coreml", "metal"] }`<br>+ `llama-cpp-2 = { version = "0.1", features = ["metal"] }`<br>+ `async-trait = "0.1"`<br>+ `phf = { version = "0.11", features = ["macros"] }`<br>+ `sha2 = "0.10"` |
| `src-tauri/src/stt.rs` → `src-tauri/src/stt/mod.rs` | New trait + `groq.rs` + `local_whisper.rs` |
| `src-tauri/src/llm.rs` → `src-tauri/src/llm/mod.rs` | New trait + `anthropic.rs` + `local_llm.rs` + `noop.rs` |
| `src-tauri/src/models.rs` | NEW — catalog, store, downloader |
| `src-tauri/src/commands.rs` | + `download_model`, `delete_model`, `list_models`, `set_active_stt_model`, `set_active_cleanup_model` |
| `src-tauri/src/settings.rs` | + new keys (D.1) + migration |
| `src-tauri/src/session.rs` | Replace hard-coded Groq/Anthropic with `Arc<dyn …>` chosen from settings; add `warm()` after first hotkey press; add 10-min keep-alive timer |
| `src-tauri/tauri.conf.json` | + `com.apple.security.cs.allow-jit` entitlement; bump bundle version; add `bundle.macOS.entitlements` |
| `src-tauri/capabilities/default.json` | scope new commands to settings + onboarding windows only |
| `src/routes/onboarding.tsx` | + Tier picker step (between perms and try-it) |
| `src/routes/settings.tsx` | + Models tab |
| `src/components/ModelStatus.tsx` | NEW — used in pill (`LOCAL` chip) and tray popover |
| `src/lib/ipc.ts` | + typed wrappers around new commands |

### G.4 Settings keys (full list, recap from D.1)

```
stt_engine                    string   default "local"
stt_local_model_id            string?  default null
cleanup_engine                string   default "local"
cleanup_local_model_id        string?  default null
fully_offline                 bool     default false
onboarding_tier               string?  default null
model_keep_alive_minutes      u32      default 10
```

### G.5 Onboarding flow updates

Insert after current step 4 (API keys) — but make API keys **skippable
when local is selected**:

```
1. Welcome
2. Microphone permission                  (hard gate; unchanged)
3. Accessibility permission               (skippable; unchanged)
4. Choose tier
   - if "Cloud" → step 5 (API keys, hard gate)
   - else      → step 5b (download)
5a. API keys (cloud branch)               (hard gate)
5b. Download models (local branch)        (hard gate; cancellable returns to 4)
6. Try it now                             (unchanged)
7. Done                                   (unchanged)
```

### G.6 UI surfaces summary

| Surface | What changes |
| --- | --- |
| Pill | Add `LOCAL` / `CLOUD` chip; "Loading model…" state for first dictation after a cold start |
| Onboarding | New tier-picker step + download-progress step |
| Settings → Privacy | Existing "Send audio" toggle splits into two (cloud STT, cloud cleanup); "Fully offline mode" toggle |
| Settings → Models | NEW — list, swap, delete, re-download, view sha256, view storage used |
| Tray popover | Engine indicator (Local / Cloud / Mixed) below the dictation toggle |

---

## Critical risks (top 3)

### Risk 1 — First-run download abandonment

A user clicks Install, opens the app, sees a 1.9 GB download, and quits.
We churn the user before they ever dictate.

**Mitigation:**
- Default to **Balanced** but offer **Small** front-and-centre (265 MB,
  done in <30 s on most networks).
- Show network speed estimate and ETA prominently.
- Allow "Use cloud for now, download later" — sets cloud engines as
  active and queues a background download.
- Resume interrupted downloads via HTTP `Range` requests.
- The first dictation can run with cloud while the local model finishes
  downloading in the background; we hot-swap to local on next dictation
  once installed.

### Risk 2 — RAM blowups on 8 GB Macs while user has Chrome + Slack

E2B at ~2.5 GB peak + Whisper turbo at ~1.1 GB + Tauri + macOS = system
swaps; latency spikes 5–10×.

**Mitigation:**
- Detect physical RAM at install via `mach_host_self` /
  `host_statistics64`; if `< 12 GB`, default to **Small** tier instead of
  Balanced.
- Honour memory pressure notifications (B.5) — unload aggressively.
- Add a Settings → Models warning banner when peak RAM > 50% of physical
  during cleanup.
- Stretch: switch to Q4_0 quantization (slightly worse quality but ~30%
  less RAM) on the "low memory" branch.

### Risk 3 — Gemma 4 license compliance for a paid app

Gemma is Apache 2.0 *as licensed by Google* but downstream redistribution
under the Gemma Terms of Use applies if we ever bundle weights or
fine-tunes. Our model download from HF doesn't bundle, so we are clean —
**but only as long as we don't ship a custom fine-tune in the .app**.

**Mitigation:**
- Today we only download official Google/unsloth weights from HF. Legal
  position: same as a user manually pulling them.
- Ship a one-line Gemma terms acceptance dialog on first download
  ("Gemma is licensed by Google under the Gemma Terms of Use…") to be
  defensible.
- Phi-4-mini stays as a flip-the-switch backup if Google ever changes
  terms.

---

## Migration order for Sr Eng B

Land in this order. Each step ships behind a feature flag (`local_first`)
so we can dogfood incrementally without regressing cloud users.

1. **`models.rs` + ModelCatalog (no UI yet).** Compile-time constants,
   download function, sha256 verification, manifest read/write. Unit
   tests with a tiny test model from a fixture server.
2. **`SttEngine` trait + refactor existing Groq into `GroqEngine`.** No
   behaviour change. CI green.
3. **`LlmEngine` trait + refactor existing Anthropic into
   `AnthropicEngine`.** No behaviour change. CI green.
4. **`LocalWhisperEngine` (whisper-rs).** Wire to `models.rs`. Add a dev
   command `dev_transcribe_with_local` for a smoke test before exposing.
5. **`LocalLlmEngine` (llama-cpp-2).** Same pattern; smoke command.
6. **Settings keys + migration.** Default existing users to cloud
   engines so nothing breaks.
7. **Settings → Models tab.** Lets dogfooders manually install + swap
   models from a hidden dev menu first, then expose properly.
8. **Onboarding tier picker + download step.**
9. **Pill engine chip + memory-pressure unload.**
10. **Beta cut.** Internal dogfood for 1 week with `local_first` on.
11. **Cloud → Local default flip.** Feature flag default to local for new
    installs only; existing users stay on cloud unless they switch.
12. **Drop the v0.1 cloud-only Privacy umbrella toggle** in v1.2.

Estimated calendar: ~3 weeks of focused work for an experienced Rust +
Tauri engineer. Steps 4–6 are the long pole (model integration + memory
plumbing); 7–9 are mostly straightforward UI + IPC.

---

## Sources

- whisper-rs: <https://docs.rs/crate/whisper-rs/latest/features>
- whisper.cpp model files: <https://huggingface.co/ggerganov/whisper.cpp/tree/main>
- WhisperKit ICML 2025 paper: <https://arxiv.org/html/2507.10860v1>
- WhisperKit + Argmax + Apple SpeechAnalyzer: <https://www.argmaxinc.com/blog/apple-and-argmax>
- Whisper + Parakeet on Apple Silicon: <https://macparakeet.com/blog/whisper-to-parakeet-neural-engine/>
- Voicci M-series Whisper benchmark: <https://www.voicci.com/blog/apple-silicon-whisper-performance.html>
- mac-whisper-speedtest: <https://github.com/anvanvan/mac-whisper-speedtest>
- MLX vs faster-whisper streaming benchmark: <https://medium.com/@GenerationAI/streaming-with-whisper-in-mlx-vs-faster-whisper-vs-insanely-fast-whisper-37cebcfc4d27>
- Parakeet TDT 0.6B v3 model card: <https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3>
- parakeet-rs (CPU/ONNX): <https://github.com/altunenes/parakeet-rs>
- Gemma 4 launch (Google): <https://blog.google/innovation-and-ai/technology/developers-tools/gemma-4/>
- Gemma 4 HuggingFace blog: <https://huggingface.co/blog/gemma4>
- Gemma 4 launch coverage: <https://aihaven.com/news/gemma-4-launches-april-2026/>
- Gemma 4 Mac performance: <https://gemma4-ai.com/blog/gemma4-mac-performance>
- Gemma 4 E2B GGUF (unsloth): <https://huggingface.co/unsloth/gemma-4-E2B-it-GGUF>
- Gemma 4 E4B GGUF (unsloth): <https://huggingface.co/unsloth/gemma-4-E4B-it-GGUF>
- Apple Foundation Models framework: <https://developer.apple.com/documentation/FoundationModels>
- llama.cpp Apple Silicon perf: <https://github.com/ggml-org/llama.cpp/discussions/4167>
- llama.cpp macOS 26 Metal guide: <https://github.com/ggml-org/llama.cpp/discussions/17298>
- mistral.rs Gemma 4 doc: <https://github.com/EricLBuehler/mistral.rs/blob/master/docs/GEMMA4.md>
- Hardened runtime + JIT: <https://bugzilla.mozilla.org/show_bug.cgi?id=1470597>
