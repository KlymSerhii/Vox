# Windows Parakeet (ONNX + DirectML) — Implementation Plan

Author: Staff SWE pass · Target branch: `windows-support` · Implementer: Senior SWE phase

This plan adds NVIDIA Parakeet TDT 0.6B v3 to Windows via the `parakeet-rs`
v0.3 Rust crate (which wraps the `ort` 2.0-rc.12 ONNX Runtime binding).
The plan does **not** add any code itself. macOS keeps the existing
FluidAudio (Core ML / ANE) path bit-for-bit; this is a Windows-only
addition wired through the existing `parakeet-local` engine id.

---

## 1. Executive summary

- Add a Windows-only Parakeet STT backend that runs in-process via
  `parakeet-rs` + `ort` 2.0-rc.12 with the **DirectML** execution
  provider (GPU on any DX12 box) and a CPU EP fallback when DirectML
  fails to initialise.
- Register the same engine id `"parakeet-local"` on both platforms;
  the concrete implementation is chosen by `cfg(target_os = …)`. The
  Settings doc, the onboarding flow, and the engine recommender stay
  OS-agnostic.
- Same Hugging Face model id (`parakeet-tdt-0.6b-v3`) on both
  platforms, but a **different on-disk file manifest per OS**:
  - macOS: existing FluidAudio Core ML bundle from
    `FluidInference/parakeet-tdt-0.6b-v3-coreml` (unchanged).
  - Windows: ONNX bundle from `istupakov/parakeet-tdt-0.6b-v3-onnx`
    (encoder + external weights + decoder/joint + vocab).
- The `models.rs` descriptor branches per host on `url`/manifest. The
  existing multi-file downloader (`download_parakeet_v3`) is extended
  with an OS-keyed file manifest plus a host-specific install probe.
  The single-file `stream_to_file` is unchanged.
- `parakeet_availability` on Windows now reports `available: true`
  whenever the `parakeet` feature is compiled in. There is no
  per-host gating beyond "feature on" because DirectML installs by
  default on Windows 10 1903+ (every Vox-supported target). When the
  GPU initialisation fails at session-create the engine falls back to
  the CPU EP — still `available: true`.
- `recommendedSttEngineFor` already prefers Parakeet over Whisper
  when its language list matches. With the Windows engine wired, the
  onboarding LanguageStep will default Windows users to Parakeet on
  any of the 25 supported languages and on `auto`, exactly mirroring
  Apple Silicon today.
- Tauri's MSI bundler now needs to ship `onnxruntime.dll` and (when
  the user picks the DX12 GPU path) `DirectML.dll` next to `vox.exe`.
  `ort`'s default `copy-dylibs` feature copies both into the Cargo
  target dir — Tauri's WiX template picks the binaries up
  automatically from there.
- Pre-warm parity: `engines::prewarm_stt_in_background` already hands
  off to `SttEngine::prewarm` on whatever engine is selected. We
  implement `prewarm` on the Windows engine to do a load-only round
  trip (`ParakeetTDT::from_pretrained` → cache the `Arc<Mutex<…>>`).
- Fail-open semantics preserved: ONNX init / DLL missing / model
  file missing / DirectML init failure all surface as
  `SttEngine::transcribe` errors. STT errors abort the session with a
  friendly pill exactly as Whisper does today (Whisper is still the
  fallback engine that the user can pick).
- After this lands, a Windows user with a 25-language pick (or
  `auto`) sees: Parakeet downloaded during onboarding (~3 GB),
  hardware-accelerated dictation on first session, the same
  "Listening → Transcribing → Polishing → Done" pill we have today.

---

## 2. Architecture diagram

```
                    ┌───────────────────────────────────────────┐
                    │            SttEngine trait                │
                    │  src-tauri/src/engines/mod.rs             │
                    └───────────────┬───────────────────────────┘
                                    │
        ┌───────────────────────────┼────────────────────────────┐
        │                           │                            │
        ▼                           ▼                            ▼
 whisper_local            parakeet_local (router)            none, etc.
 (cross-platform)         engines/parakeet_local.rs
                                    │
              ┌─────────────────────┴─────────────────────┐
              │                                           │
              ▼                                           ▼
   #[cfg(macos+aarch64+parakeet)]            #[cfg(windows+parakeet)]
   imp_fluidaudio (existing)                 imp_onnx  (NEW)
              │                                           │
              ▼                                           ▼
   parakeet_bridge.rs (Swift FFI)            parakeet_rs::ParakeetTDT
              │                                           │
              ▼                                           ▼
   swift-parakeet/ SwiftPM (FluidAudio)       ort 2.0-rc.12 → onnxruntime.dll
              │                                           │
              ▼                                           ▼
   Core ML .mlmodelc + ANE                  DirectML EP (DX12) → GPU
   (NSPasteboard / aned daemon)              ↳ falls back to CPU EP

  Disk layout on Windows:
    %APPDATA%\com.vox.app\models\stt\parakeet-tdt-0.6b-v3\
      encoder-model.onnx
      encoder-model.onnx.data
      decoder_joint-model.onnx
      vocab.txt
      (lives directly in <model_dir>, NO `parakeet-tdt-0.6b-v3-coreml/`
       sub-directory — that one is FluidAudio-specific and stays
       macOS-only.)

  Cargo features:                  Per-target deps:
    parakeet (default-on)            [target.macos]
      ↓                                whisper-rs (coreml+metal)
    apple-fm (mac only at link)        objc2 / objc2-app-kit / …
    local-engines                    [target.windows]
                                       whisper-rs (CPU)
                                       parakeet-rs 0.3 ┐
                                         features=["directml"]
                                       windows = "0.58"
                                       ort (transitively)

  build.rs:
    build_parakeet_bridge()   — macOS aarch64 only (existing)
    build_apple_fm_bridge()   — macOS only (existing)
    [no new build.rs step needed on Windows; ort 2.0-rc.12 with
     default `download-binaries` + `copy-dylibs` features handles
     onnxruntime.dll + DirectML.dll copy automatically into the
     Cargo target dir, which Tauri's WiX bundler picks up.]
```

---

## 3. Decisions made during research

### 3.1 Crate choice — `parakeet-rs` 0.3.5

Confirmed on crates.io and on the GitHub README:
- TDT (multilingual, 25 langs) is the relevant variant — exposed via
  `ParakeetTDT::from_pretrained(path, Option<ExecutionConfig>)` →
  trait method `Transcriber::transcribe_samples(&mut self, audio:
  Vec<f32>, sample_rate: u32, channels: u16, mode: Option<TimestampMode>)
  -> Result<TranscriptionResult>`. Synchronous, mutable receiver. We
  feed it our 16 kHz mono `Vec<f32>` (the same buffer Whisper
  consumes) with `sample_rate=16000, channels=1`.
- `ParakeetTDT::from_pretrained` reads:
  - `vocab.txt` (required, name fixed)
  - one of `encoder-model.onnx | encoder.onnx | encoder-model.int8.onnx`
    (we ship `encoder-model.onnx`)
  - one of `decoder_joint-model.onnx | decoder_joint-model.int8.onnx
    | decoder_joint.onnx | decoder-model.onnx` (we ship
    `decoder_joint-model.onnx`)
  - The external weights file `encoder-model.onnx.data` is **not
    referenced by name in `parakeet-rs`** — it's resolved by ONNX
    Runtime itself from the `location` field in the
    `encoder-model.onnx` protobuf and must sit next to the .onnx
    file with the exact filename the protobuf records (which is
    `encoder-model.onnx.data` for this repo). Rename = silent load
    failure.
- License: MIT OR Apache-2.0. Clean.
- Pinned ort version: `ort = "2.0.0-rc.12"`. Default `ort` features
  are `["std","ndarray","tracing","download-binaries","tls-native",
  "copy-dylibs","api-26"]` — i.e. it will fetch a prebuilt
  `onnxruntime.dll` at build time from `cdn.pyke.io` and copy it
  next to the binary.

### 3.2 Execution provider — DirectML (selected) with CPU EP fallback

Decision: enable the `parakeet-rs/directml` Cargo feature on the
Windows target and at runtime call
`ExecutionConfig::new().with_execution_provider(ExecutionProvider::DirectML)`.
`parakeet-rs::execution::ModelConfig::apply_to_session_builder`
already pushes a CPU EP as the second EP in the list with
`.error_on_failure()`, so DirectML failure falls back to CPU
inside the same session — no extra Rust code needed for the
fallback.

DirectML quirks called out in the task brief (parallel_execution
false, memory_pattern false, no Auto-mode): these are recommended
for some workloads but `parakeet-rs` does not apply them by
default. The crate exposes `ExecutionConfig::with_custom_configure`
(takes `Fn(SessionBuilder) -> ort::Result<SessionBuilder>`) so we
can layer those in if perf or stability proves to need them. Plan:
**add them defensively from day one** — the cost is one extra
closure call per session-create and the upside is bypassing a
known DirectML failure mode (mismatched memory pattern with
dynamic shapes). Code excerpt in §4.

### 3.3 ONNX Runtime DLL distribution — keep `ort` defaults, let Tauri bundle

Decision: do **not** enable `load-dynamic` or `--no-default-features`
on `ort`. The default `ort` features (`download-binaries` +
`copy-dylibs`) cause the ort build script to download the
matching prebuilt ONNX Runtime archive from `cdn.pyke.io` at
build time and copy `onnxruntime.dll` + `DirectML.dll` to the
Cargo target dir as build-script output. Tauri 2's WiX bundler
walks the target dir and includes any `*.dll` siblings of the
final `.exe` in the MSI by default (the same way `tauri-plugin-*`
DLL sidecars get bundled).

Risks of this approach: the `cdn.pyke.io` archive needs HTTPS
egress on the **build** machine. CI (which we already plumb for
the `windows-support` work) has that. Devs on TLS-intercepting
corporate networks: `ort` honours the `ORT_LIB_LOCATION`
environment variable; document it. No runtime cost — the bytes
are baked into the .msi at bundle time.

Verified by reading the upstream:
- `pykeio/ort` `Cargo.toml` default features include
  `"download-binaries"` and `"copy-dylibs"`.
- `pykeio/ort` `ort-sys/build/download/dist.txt` lists
  `none → x86_64-pc-windows-msvc → cdn.pyke.io/.../1.26.0/
  x86_64-pc-windows-msvc.tar.lzma2` (sha
  `aabf0e2f9903f36253ba03685057268c9c4f777e8c009e4a72ffe0481d6bc0bb`).
- DirectML EP support is compiled into Microsoft's official ORT
  Windows builds (which is what pyke redistributes), so flipping
  the `directml` cargo feature does not require a different
  prebuilt blob — it just toggles the Rust-side feature gate that
  uncovers `ort::ep::DirectML`.

### 3.4 Same engine id, OS-specific descriptor

Decision: keep engine id `"parakeet-local"` on both platforms. The
on-disk model id is also identical (`"parakeet-tdt-0.6b-v3"`). The
descriptor differs per OS only in the `url` / file list and the
installed-probe path. This avoids splitting:
- `Settings::stt_engine` (the user picks "Parakeet" once and it
  works on both their Mac and PC if they sync).
- `recommendedSttEngineFor` (TypeScript + Rust mirror).
- `parakeetSupportsLanguage` (the 25-language table is OS-agnostic
  — Parakeet is the same model, just a different runtime wrapper).
- The Settings UI's `engineId: "parakeet-local" | "whisper-local"`
  union (it stays a two-element union).

Alternative considered and rejected: introducing
`"parakeet-onnx"` as a new engine id on Windows. Would have forced
us to teach every cross-window listener (Tray popover,
Onboarding, Settings) two ids meaning the same thing.

### 3.5 Descriptor branching style — `#[cfg]` inside the constant

Decision: keep the single `ModelDescriptor` for
`"parakeet-tdt-0.6b-v3"` in `CATALOG` but change the descriptor's
`url`, `size_bytes`, and `name` fields with `#[cfg(target_os)]`
branches inline. The multi-file download path
(`download_parakeet_v3`) gets an OS-branched manifest function
(`parakeet_v3_files_macos()` vs `parakeet_v3_files_windows()`).
The install-probe in `list()` already special-cases
`"parakeet-tdt-0.6b-v3"`; we extend it to probe a different file
per host.

Alternative considered and rejected: a second descriptor with a
different id per OS. Would break the Settings round-trip the user
expects: a Mac user who picked Parakeet should still have Parakeet
selected if they open Vox on Windows and vice versa.

### 3.6 `try_parakeet_dismissed` and the "Try Parakeet" banner

The existing banner only fires when (a) user is on Whisper,
(b) Parakeet is available on this host, (c) language is supported.
Once Windows reports `available: true`, the banner fires for
Windows users in the same way it fires for Apple-Silicon-Mac
users. No code change needed beyond the availability flip.

`try_parakeet_dismissed` is sticky per Settings doc so a user who
dismisses it on macOS will see it suppressed on Windows too if
they share a settings file. That is the right behaviour.

### 3.7 Language pinning forwarded to engine — drop, like macOS

`parakeet-rs` auto-detects language across the 25-language set and
does not accept a per-call hint. The macOS engine takes `_language`
and ignores it; the Windows engine will do the same. The session
recommender (`recommendedSttEngineFor`) already routes
non-Parakeet-supported languages to Whisper at the engine layer,
so the user's pinned language still matters — it picks the engine,
not a hint inside it.

### 3.8 Audio shape — already matches

`audio::encode_recorded` produces a 16 kHz mono WAV; our existing
`whisper_local::decode_wav_to_f32_mono_16k` decodes it to
`Vec<f32>`. Pass the same buffer with `sample_rate=16000, channels=1`
to `transcribe_samples`. No resampling, no remixing.

### 3.9 Threading model

`ParakeetTDT::transcribe_samples(&mut self, …)` is synchronous and
CPU-bound. Match `WhisperLocalEngine` exactly: wrap the call in
`tauri::async_runtime::spawn_blocking` so the Tokio runtime isn't
pinned. `Arc<Mutex<ParakeetTDT>>` because `transcribe_samples`
needs `&mut self`.

### 3.10 Settings persistence flag

No new Settings field needed. `try_parakeet_dismissed` already
exists, `stt_engine` / `stt_model_id` already exist, and the
recommender flips on `parakeet_availability().available`. The
engine choice is the only piece of state that has to be set
correctly during onboarding's DownloadStep, and that is already
driven by the recommender.

---

## 4. File-by-file change list

### 4.1 `src-tauri/Cargo.toml` (modified)

Add `parakeet-rs` to the Windows-only deps table with the
`directml` feature flag. Update the Cargo `parakeet` feature
comment to note that it now produces a real Windows engine.

```toml
[features]
# … existing comment block …
# Parakeet STT.
#   - macOS aarch64: FluidAudio (Swift/CoreML/ANE) via swift-parakeet.
#     build.rs::build_parakeet_bridge gates on `target_arch == aarch64`
#     and a working `swift` toolchain.
#   - Windows x86_64: parakeet-rs + ort (ONNX Runtime) with DirectML EP
#     and CPU EP fallback. ort's default `download-binaries` +
#     `copy-dylibs` features baked into the Cargo target dir; Tauri's
#     WiX bundler ships them in the .msi.
#   The Rust engine module (`engines::parakeet_local`) is the same name
#   on both hosts; the concrete impl is selected by `cfg(target_os)`.
parakeet = []

# Single-source-of-truth dep is per target table so the Windows build
# doesn't pull `swift-parakeet`'s transitive deps and the macOS build
# doesn't pull ort/parakeet-rs/onnxruntime.
[target.'cfg(target_os = "windows")'.dependencies]
# … existing whisper-rs (CPU), windows = "0.58" …
# `directml` enables the ort::ep::DirectML variant.
# Default features of parakeet-rs (`cpu`, `ort-defaults`) are kept so
# the CPU EP fallback path is always available.
parakeet-rs = { version = "0.3", default-features = true, features = ["directml"], optional = true }
```

And in the `[features]` block:

```toml
parakeet = ["dep:parakeet-rs"]
```

This makes `parakeet-rs` a real optional dep tied to the `parakeet`
feature, with `cfg`-gated module imports keeping the macOS build
free of it. (Cargo handles "this optional dep is only declared in
one target table; the feature flag still resolves" fine — same
pattern we already use for `whisper-rs` and `optional = true`.)

Verification: on macOS aarch64, `cargo metadata` must show no
`parakeet-rs` in the dep tree; on `x86_64-pc-windows-msvc` with
`--features parakeet`, it must show `parakeet-rs 0.3.x` and
`ort 2.0.0-rc.12`.

### 4.2 `src-tauri/src/engines/parakeet_local.rs` (modified)

Replace the current single-imp module with the same `cfg`-gated
shape as `parakeet_bridge.rs`: a router struct
`ParakeetLocalEngine` that on macOS delegates to the existing
FluidAudio path (via `parakeet_bridge`) and on Windows delegates
to the new `parakeet_onnx::imp` module. The public surface
(`SttEngine` impl, `new`, `id`, `label`, `prewarm`) stays
identical so `engines::Registry` doesn't change.

Code shape (Windows branch — replaces the macOS-only body):

```rust
// engines/parakeet_local.rs

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use tauri::{AppHandle, Manager, Wry};

use crate::engines::{SegmentCallback, SttEngine};
use crate::models::{self, ModelKind};

#[cfg(feature = "local-engines")]
use crate::engines::whisper_local::decode_wav_to_f32_mono_16k;

use once_cell::sync::OnceCell;

#[cfg(all(target_os = "macos", target_arch = "aarch64", feature = "parakeet"))]
use crate::engines::parakeet_bridge;

#[cfg(all(target_os = "windows", feature = "parakeet"))]
mod onnx_imp {
    //! Windows ONNX backend — parakeet-rs + ort + DirectML EP.
    use anyhow::Result;
    use parakeet_rs::{
        ExecutionConfig, ExecutionProvider, ParakeetTDT, TimestampMode, Transcriber,
    };
    use std::path::Path;
    use std::sync::Arc;
    use parking_lot::Mutex;

    pub type Loaded = Arc<Mutex<ParakeetTDT>>;

    pub fn load_blocking(model_dir: &Path) -> Result<Loaded> {
        // DirectML quirks (per `parakeet-rs` README + ORT docs): disable
        // memory pattern so DirectML's allocator doesn't trip on the
        // model's dynamic shapes. parallel_execution is off in ort by
        // default; we set it explicitly for clarity.
        let cfg = ExecutionConfig::new()
            .with_execution_provider(ExecutionProvider::DirectML)
            .with_custom_configure(|b| {
                b.with_memory_pattern(false)?
                 .with_parallel_execution(false)
            });
        let model = ParakeetTDT::from_pretrained(model_dir, Some(cfg))
            .map_err(|e| anyhow::anyhow!("parakeet onnx load: {e}"))?;
        Ok(Arc::new(Mutex::new(model)))
    }

    pub fn transcribe_blocking(loaded: &Loaded, pcm: Vec<f32>) -> Result<String> {
        let mut guard = loaded.lock();
        let result = guard
            .transcribe_samples(pcm, 16_000, 1, Some(TimestampMode::Sentences))
            .map_err(|e| anyhow::anyhow!("parakeet onnx transcribe: {e}"))?;
        Ok(result.text)
    }
}

/// Shared OnceCell holding the load result. Same shape as macOS.
type LoadOnce<T> = OnceCell<Arc<Mutex<Result<T, String>>>>;

pub struct ParakeetLocalEngine {
    app: AppHandle<Wry>,
    model_id: String,
    // Windows: the actual ParakeetTDT lives here (one per model_id).
    #[cfg(all(target_os = "windows", feature = "parakeet"))]
    onnx_loaded: OnceCell<onnx_imp::Loaded>,
    // macOS: the FluidAudio model is owned by the Swift side; this
    // cell just records load success.
    #[cfg(all(target_os = "macos", target_arch = "aarch64", feature = "parakeet"))]
    macos_loaded: OnceCell<Arc<Mutex<Result<(), String>>>>,
}
```

The `transcribe()` method on Windows decodes the WAV with the
existing helper and dispatches through `spawn_blocking`. The
`prewarm()` method on Windows runs `onnx_imp::load_blocking` in a
blocking task and stuffs the `Arc<Mutex<ParakeetTDT>>` into the
OnceCell. The macOS branches stay exactly as they are today.

The model-dir resolution is **different from macOS**: on Windows
the ONNX files live directly under
`<app_data>/models/stt/parakeet-tdt-0.6b-v3/` — no
`parakeet-tdt-0.6b-v3-coreml/` sub-dir (that one mirrors
FluidAudio's `Repo.parakeetV3.folderName` convention and is
macOS-specific). So `model_dir()` returns the same root on both
hosts, and the macOS branch keeps its `.join(FLUIDAUDIO_REPO_FOLDER)`
helper while the Windows branch hands the root directly to
`from_pretrained`.

### 4.3 `src-tauri/src/engines/parakeet_bridge.rs` (unchanged-but-listed)

No changes. The `cfg(not(macos aarch64 parakeet))` stub branch is
already correct for Windows. The Windows engine doesn't touch the
Swift bridge.

### 4.4 `src-tauri/src/engines/mod.rs` (modified — minimal)

The `Registry::stt` "parakeet-local" branch stays as-is; the
constructor signature didn't change.

`drop_for_model`'s `parakeet_bridge::evict()` call is no-op on
Windows (the stub branch in `parakeet_bridge::imp` already does
nothing for non-macOS). On Windows the on-disk-bytes-released
behaviour falls out of dropping the cached `Arc<ParakeetLocalEngine>`
from the Registry's `stt` map, which drops the
`Arc<Mutex<ParakeetTDT>>` inside it, which closes the ort
sessions, which frees the mmap'd ONNX weights. The
`parakeet_bridge::evict()` cross-platform call still happens
unconditionally — it's just a no-op on Windows. Fine.

### 4.5 `src-tauri/src/models.rs` (modified — significant)

Three changes:

1. The `parakeet-tdt-0.6b-v3` descriptor's `name`, `url`, and
   `size_bytes` get OS-branched. `url` stays empty on **both**
   hosts (the entry is a marker; the multi-file downloader still
   reads from `parakeet_v3_files_*()`).

   ```rust
   ModelDescriptor {
       id: "parakeet-tdt-0.6b-v3",
       kind: ModelKind::Stt,
       #[cfg(target_os = "macos")]
       name: "Parakeet TDT 0.6B v3 (multilingual, 25 languages, ANE-accelerated)",
       #[cfg(not(target_os = "macos"))]
       name: "Parakeet TDT 0.6B v3 (multilingual, 25 languages, DirectML-accelerated)",
       #[cfg(target_os = "macos")]
       size_bytes: 500 * 1024 * 1024,    // existing macOS value
       #[cfg(not(target_os = "macos"))]
       size_bytes: 2_549_855_823,        // sum of Windows manifest, see §4.5.2
       sha256: None,
       url: "",
       engine: "parakeet-local",
       ram_peak_mb: 800,                 // safe upper bound on both hosts
       tier: "balanced",
   },
   ```

2. `parakeet_v3_files()` becomes `parakeet_v3_files_macos()` and a
   new `parakeet_v3_files_windows()`. The descriptor struct is
   unchanged. The downloader path dispatches by `cfg!(target_os =
   "windows")`:

   ```rust
   fn parakeet_v3_files() -> Vec<ParakeetFile> {
       #[cfg(target_os = "macos")] return parakeet_v3_files_macos();
       #[cfg(target_os = "windows")] return parakeet_v3_files_windows();
       #[cfg(not(any(target_os = "macos", target_os = "windows")))]
       return Vec::new(); // unsupported host — downloader returns "no
                          // files" so installed-probe stays false.
   }
   ```

3. The Windows manifest. Sourced from the HF API
   (`huggingface.co/api/models/istupakov/parakeet-tdt-0.6b-v3-onnx/tree/main`)
   on 2026-06-01. Each file's `lfs.oid` is its sha256 — we pin
   them as soft canaries in `parakeet_v3_files_windows()` via the
   existing `lock_file` mechanism (sha256 is set to `None` so the
   first successful download appends to `models.lock`). Exact
   sizes and the file list:

   | file | size (bytes) | sha256 (lfs oid) |
   |---|---|---|
   | `encoder-model.onnx` | 41,770,866 | `98a74b21b4cc0017c1e7030319a4a96f4a9506e50f0708f3a516d02a77c96bb1` |
   | `encoder-model.onnx.data` | 2,435,420,160 | `9a22d372c51455c34f13405da2520baefb7125bd16981397561423ed32d24f36` |
   | `decoder_joint-model.onnx` | 72,520,893 | `e978ddf6688527182c10fde2eb4b83068421648985ef23f7a86be732be8706c1` |
   | `vocab.txt` | 93,939 | (small file, no LFS — sha not set) |

   Total: 2,549,805,858 bytes (~2.37 GiB). We deliberately do
   **not** ship `nemo128.onnx` (it's an alt entrypoint not used
   by `parakeet-rs`), nor the `*.int8.onnx` quantised variants
   (the FP32 path is what we tested and what the README
   recommends for accuracy).

   Manifest shape (no sub-directory on Windows — files live at
   the bundle dir root):

   ```rust
   fn parakeet_v3_files_windows() -> Vec<ParakeetFile> {
       const BASE: &str = "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main/";
       vec![
           ParakeetFile { relpath: "encoder-model.onnx",
                          url: concat!("https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main/", "encoder-model.onnx"),
                          size_bytes: 41_770_866 },
           ParakeetFile { relpath: "encoder-model.onnx.data",
                          url:  …, size_bytes: 2_435_420_160 },
           ParakeetFile { relpath: "decoder_joint-model.onnx",
                          url:  …, size_bytes: 72_520_893 },
           ParakeetFile { relpath: "vocab.txt",
                          url:  …, size_bytes: 93_939 },
       ]
   }
   ```

   The macOS manifest function is the **current** `parakeet_v3_files()`
   body, lifted verbatim.

4. **`list()`'s install probe gets an OS branch**.

   - macOS: existing probe — looks for
     `parakeet-tdt-0.6b-v3-coreml/parakeet_vocab.json` under the
     bundle dir. Unchanged.
   - Windows: probe for `vocab.txt` directly under the bundle dir.
     ```rust
     let probe = bundle_dir.join("vocab.txt");
     let bytes_on_disk = if probe.metadata().map(|m| m.len() > 0).unwrap_or(false) {
         parakeet_v3_files_windows().iter().map(|f| {
             bundle_dir.join(f.relpath).metadata().map(|m| m.len()).unwrap_or(0)
         }).sum()
     } else { 0 };
     let installed = bytes_on_disk > 0
         && bundle_dir.join("encoder-model.onnx.data")
              .metadata().map(|m| m.len() >= 2_400_000_000).unwrap_or(false);
     ```
     Why probe `encoder-model.onnx.data`'s size too: it's the
     huge file that always finishes last; if it's missing or
     short, the bundle isn't usable even if `vocab.txt` landed.

5. The macOS-specific `FLUIDAUDIO_REPO_FOLDER` join in the
   downloader stays inside the `parakeet_v3_files_macos()`
   manifest (the `relpath` strings already include the sub-dir
   prefix). On Windows the `relpath` strings are bare filenames
   so the downloader's `bundle_dir.join(f.relpath)` lands them at
   the root.

6. The `download_parakeet_v3` and `do_parakeet_download` flow is
   **byte-for-byte unchanged**. The skip-when-complete short-circuit,
   the per-file `.partial` resume, the 250 ms ticker, the cancel
   cleanup — all of it operates on the manifest, so it just
   works against the Windows list.

### 4.6 `src-tauri/src/commands.rs::parakeet_availability` (modified)

The Windows branch flips from "macOS only" to "compiled in →
available". The `reason` strings get a clean per-host taxonomy:

```rust
#[tauri::command]
pub async fn parakeet_availability() -> ParakeetAvailability {
    let available = crate::engines::parakeet_bridge::is_available()
        || cfg!(all(target_os = "windows", feature = "parakeet"));
    if available {
        return ParakeetAvailability { available: true, reason: None };
    }
    let reason = if !cfg!(any(target_os = "macos", target_os = "windows")) {
        "macOS and Windows only"
    } else if cfg!(target_os = "macos") && !cfg!(target_arch = "aarch64") {
        "requires Apple Silicon"
    } else if !cfg!(feature = "parakeet") {
        "Parakeet not compiled into this build"
    } else {
        "Parakeet not available on this host"
    };
    ParakeetAvailability { available: false, reason: Some(reason.into()) }
}
```

Why don't we probe DirectML *availability* here: DirectML is
shipped with Windows 10 1903+ as part of the OS (and is always
present on every Vox-supported target). The CPU EP fallback covers
the (extremely rare) DX12-less box. Probing here would mean
spinning up an ORT session at command time just to ask — wasted
RAM and IPC. We surface the actual failure at session-create time
through the standard `SttEngine::transcribe` error path instead.

The `is_available()` helper from `parakeet_bridge` already returns
false on Windows (its stub branch), which is correct — the bridge
is the **macOS Swift FFI** specifically. The Windows engine has
no bridge; it's pure Rust against `parakeet-rs`. So
`parakeet_availability` on Windows must look at
`cfg!(all(target_os = "windows", feature = "parakeet"))` directly
as shown above.

### 4.7 `src-tauri/src/engines/mod.rs::Registry::drop_for_model` (modified — minor)

The `parakeet_bridge::evict()` call is already cfg-conditioned via
the stub module. On Windows it's a no-op which is correct (cache
drop happens through `Registry`'s `HashMap::retain`). No change.

### 4.8 `src-tauri/src/lib.rs::run` setup hook (unchanged-but-listed)

`engines::prewarm_stt_in_background` already routes through
`Registry::stt(app, engine_id, model_id) → engine.prewarm()`. On
Windows with `stt_engine == "parakeet-local"` and the bundle on
disk, this will cold-load the ONNX sessions + DirectML EP in a
spawn_blocking task. The 1–2 s build doesn't change. The hook is
idempotent.

### 4.9 `src-tauri/src/settings.rs` (unchanged)

No new field. `try_parakeet_dismissed`, `stt_engine`, `stt_model_id`
already exist. The recommender already produces the right defaults
post-availability flip.

### 4.10 `src/lib/ipc.ts` (unchanged)

The TypeScript mirror of `parakeet_availability`, `recommended-
SttEngineFor`, `parakeetSupportsLanguage`, and
`PARAKEET_SUPPORTED_LANGUAGES` is **all OS-agnostic** — none of
these need any change. The frontend learns Parakeet is available
on Windows entirely via `parakeetAvailability()` flipping to
`available: true`.

### 4.11 `src/views/onboarding/steps/LanguageStep.tsx` (modified — copy)

Two text strings reference "Mac". Replace with `deviceWord`
(`isMac ? "Mac" : "PC"`) so the LanguageStep prints
"Parakeet isn't available on this PC" on Windows instead of "this
Mac".

Specific change at lines 183–188:

```tsx
<Show when={!parakeetOk() && props.parakeet}>
  <p class="text-[11px] text-[var(--color-ink-faint)]">
    Parakeet isn't available on this {deviceWord}
    {props.parakeet!.reason ? ` (${props.parakeet!.reason})` : ""} —
    all languages will use Whisper.
  </p>
</Show>
```

With the `import { deviceWord } from "@/lib/platform"` added at
the top. (When `parakeet().available === true` this entire block
is hidden, so the copy only matters in the disabled-host case.)

### 4.12 `src/views/onboarding/steps/DownloadStep.tsx` (unchanged)

Lines 452-470 (the Parakeet branching) already work — they read
`s.stt_engine === "parakeet-local"` agnostically. The Windows
ram-warning copy (line 459) already uses `isMac ? "Your Mac" :
"Your PC"`. Good.

### 4.13 `src/views/settings/tabs/ModelsTab.tsx` (modified — copy only)

Line 334 — the "Try Parakeet" banner — says "Runs on the Apple
Neural Engine". On Windows that's wrong. Replace with a
host-conditional string:

```tsx
<div class="flex-1 text-[12px] text-[var(--color-ink)]">
  <span class="font-medium">Try Parakeet</span> — typically
  5-10× faster on your language.
  {isMac
    ? " Runs on the Apple Neural Engine."
    : " GPU-accelerated via DirectML."}
</div>
```

Plus `import { isMac } from "@/lib/platform"` at the top (the
`ModelsTab` doesn't currently import from platform.ts — needed
here).

### 4.14 `src/views/settings/tabs/SttEngineCard.tsx` (unchanged)

The `engineId: "parakeet-local" | "whisper-local"` prop union is
shared. No change.

### 4.15 `build.rs` (unchanged)

`build_parakeet_bridge()` already early-exits with a
`cargo:warning` when the arch isn't aarch64 — Windows builds skip
it cleanly. No new build-script step needed: `ort` and
`parakeet-rs` are pure Rust deps that participate in normal Cargo
resolution; their build scripts do the ONNX Runtime fetch.

The build.rs `main()` `#[cfg(target_os = "macos")]` block already
gates `build_parakeet_bridge()` correctly — no change there.

### 4.16 `tauri.conf.json` (potentially modified — WiX bundler)

If the default Tauri WiX template does not pick up `*.dll`
siblings of the .exe in the Cargo target dir, we add an explicit
`bundle.windows.wix.fragmentPaths` or `bundle.resources` entry
pointing at `target/release/onnxruntime.dll` and
`target/release/DirectML.dll`.

**Verification step in §14**: build a release MSI on the
`windows-support` branch and inspect the resulting `.msi` payload
with `dark` or `7-Zip`. If the DLLs are in there, no
`tauri.conf.json` change. If not, add a `bundle.resources` glob:

```jsonc
{
  "bundle": {
    "resources": [
      "../target/release/onnxruntime.dll",
      "../target/release/DirectML.dll"
    ],
    "windows": { /* … */ }
  }
}
```

Note: Tauri 2's path is resolved from the conf file's directory
(src-tauri/), so the `../target/release/...` form is right. We
**do not** rely on `externalBin` — that's for separate sidecar
binaries, not for sibling DLLs.

### 4.17 `scripts/dev-clean-state.sh` (unchanged)

Path is in app-data-dir; the script already wipes the
`models/stt/parakeet-tdt-0.6b-v3/` sub-tree by removing the whole
models root. Works the same on Windows after the existing port.

---

## 5. Cargo / build wiring

### 5.1 Feature graph

- `default = ["local-engines", "apple-fm", "parakeet"]` — unchanged.
- `parakeet = ["dep:parakeet-rs"]` — was `[]`, now declares a
  Cargo dep edge so the optional dep resolves only when the
  feature is on.
- `local-engines = ["dep:whisper-rs"]` — unchanged.
- `apple-fm = []` — unchanged.

### 5.2 Per-target deps

```
[target.'cfg(target_os = "macos")'.dependencies]
whisper-rs = { version = "0.16", features = ["coreml", "metal"], optional = true }
# parakeet-rs NOT declared here; macOS uses the swift-parakeet bridge.

[target.'cfg(target_os = "windows")'.dependencies]
whisper-rs = { version = "0.16", optional = true }
parakeet-rs = { version = "0.3", default-features = true, features = ["directml"], optional = true }
# Default features include `cpu` and `ort-defaults`, which pulls
# `ort/default = ["std","ndarray","tracing","download-binaries",
# "tls-native","copy-dylibs","api-26"]` — exactly what we want:
# ort downloads prebuilt onnxruntime.dll at build time and copies
# it (plus DirectML.dll, which ships inside the ort prebuild) into
# the Cargo target dir for the WiX bundler.
windows = { version = "0.58", features = […existing…] }
```

### 5.3 Build-script changes

None. `build_parakeet_bridge()` and `build_apple_fm_bridge()` stay
as they are. The early-exit on non-macOS arches is correct.

### 5.4 What gets copied to the build output

After `cargo build --target x86_64-pc-windows-msvc --release`:
- `target/<triple>/release/vox.exe`
- `target/<triple>/release/onnxruntime.dll` (~12-25 MB, from ort's
  `copy-dylibs` post-build step)
- `target/<triple>/release/DirectML.dll` (~7 MB, same source)

Tauri 2's WiX builder packages everything in
`target/<triple>/release/` next to the .exe. Confirm in §14.

---

## 6. Model catalog & download flow

### 6.1 Descriptor extension shape

We deliberately keep the existing `ModelDescriptor` struct
unchanged. The OS-branch lives **inside the `CATALOG` const
literal**, using inline `#[cfg(target_os = ...)]` attributes on
the `name` and `size_bytes` fields. This:
- Keeps the catalog a single static slice (no runtime dispatch).
- Keeps the TypeScript mirror happy (`ModelDescriptor.url` is
  still a string, even though it's `""` for this entry on both
  hosts).
- Avoids a second descriptor and a second id.

### 6.2 Multi-file downloader

The `ParakeetFile` struct and the `download_parakeet_v3` /
`do_parakeet_download` pair are reused unchanged. The only
extension is `parakeet_v3_files()` becoming an OS-branched fan-out
to `parakeet_v3_files_macos()` (the existing list) or
`parakeet_v3_files_windows()` (the four-file list above).

### 6.3 Backward compatibility with already-installed models

- A user who already has the macOS Core ML bundle installed: no
  change. They'll continue to see "installed" because the macOS
  `list()` probe (`vocab.json` inside the sub-dir) still passes.
- A user moving from macOS to Windows: their Settings doc still
  says `stt_engine == "parakeet-local"` and
  `stt_model_id == "parakeet-tdt-0.6b-v3"`. On Windows the install
  probe runs against the bare `vocab.txt` and finds no file. The
  user is shown "not installed" in Settings → Models and prompted
  to download (~2.5 GB ONNX bundle).
- A Windows user with no prior install: clean download path, no
  legacy state to worry about.

### 6.4 On-disk layout — final, OS-specific

```
%APPDATA%\com.vox.app\models\stt\parakeet-tdt-0.6b-v3\
  encoder-model.onnx                 (~42 MB)
  encoder-model.onnx.data            (~2.27 GB external weights)
  decoder_joint-model.onnx           (~73 MB)
  vocab.txt                          (~94 KB)
  models.lock                        (existing soft-canary file)
```

vs

```
~/Library/Application Support/com.vox.app/models/stt/parakeet-tdt-0.6b-v3/
  parakeet-tdt-0.6b-v3-coreml/
    Preprocessor.mlmodelc/...
    Encoder.mlmodelc/...
    Decoder.mlmodelc/...
    JointDecisionv3.mlmodelc/...
    parakeet_vocab.json
  models.lock
```

The macOS sub-dir lives next to but does not overlap with any
Windows file, so a `dev-clean-state` reset works on both.

### 6.5 Download UI parity

The DownloadStep already reads the descriptor's `size_bytes` for
the progress bar's total. On Windows that's ~2.5 GB instead of
the macOS ~480 MB Core ML bundle. The user-visible MB/s and ETA
just work — `do_parakeet_download`'s 250 ms ticker emits a
`bytes_downloaded` that sums all per-file partial sizes.

---

## 7. Availability probe rewiring

### 7.1 New `parakeet_availability` behaviour

| host | feature on | result |
|---|---|---|
| macOS aarch64 (Apple Silicon) | yes | `{ available: true }` |
| macOS x86_64 | yes | `{ available: false, reason: "requires Apple Silicon" }` |
| macOS aarch64 | no | `{ available: false, reason: "Parakeet not compiled into this build" }` |
| Windows x86_64 | yes | **`{ available: true }`** — new |
| Windows aarch64 | yes | `{ available: true }` (ort ships an `aarch64-pc-windows-msvc` prebuild — confirmed in `dist.txt`) |
| Windows any | no | `{ available: false, reason: "Parakeet not compiled into this build" }` |
| linux | any | `{ available: false, reason: "macOS and Windows only" }` |

The `reason` strings are read by `LanguageStep` and the
`SttEngineCard` — they're shown literally to the user so they
must read correctly per host.

### 7.2 No "Try Parakeet" suppression change

`try_parakeet_dismissed` already gates the banner the way we want
— a user who dismissed it on macOS keeps it suppressed on
Windows. The recommender side (LanguageStep auto-selecting
Parakeet on a supported language) is independent of the dismissal
flag.

---

## 8. Onboarding flow changes

### 8.1 LanguageStep (`src/views/onboarding/steps/LanguageStep.tsx`)

- Line 186 — copy fix: replace "this Mac" with `deviceWord` (see
  §4.11).
- Everything else: unchanged. The recommender already routes
  Windows users with supported languages to `parakeet-local` once
  `parakeetOk()` returns true.

### 8.2 DownloadStep (`src/views/onboarding/steps/DownloadStep.tsx`)

- Unchanged. Lines 452-470's `s.stt_engine === "parakeet-local"`
  branching, the `visibleSttModels` filter, the `Recommended`
  label, all OS-agnostic.
- Already uses `isMac ? "Mac" : "PC"` on line 459. Good.

### 8.3 Pre-warm hook on the Tips step

The existing onboarding finishes downloads, then the wizard
transitions to Tips, then to How-to-use. The
`engines::prewarm_stt_in_background` background task runs on
launch and re-runs whenever Settings change. After the user
completes the DownloadStep on Windows, the next `set_settings`
call (commits the engine choice) triggers `prewarm_stt_in_background`
which cold-loads `ParakeetTDT::from_pretrained`. By the time the
user clicks Done, the model is warm.

### 8.4 No new step

The Windows ONNX path needs no permissions and no extra UI —
exactly like the macOS FluidAudio path.

---

## 9. Settings tab changes

### 9.1 ModelsTab (`src/views/settings/tabs/ModelsTab.tsx`)

- Line 334 — copy fix: replace "Runs on the Apple Neural Engine"
  with a host-conditional string (see §4.13).
- The `SttEngineCard` Parakeet section automatically renders
  itself on Windows once `parakeetAvailability()` returns
  `available: true`.
- The model row for `parakeet-tdt-0.6b-v3` shows the OS-specific
  descriptor `name`, `size_bytes`, and installed state.
- The note line ("Custom dictionary biasing isn't supported on
  Parakeet in this version") is correct on Windows too — neither
  the macOS FluidAudio path nor the Windows ONNX path forwards
  the dictionary. Leave as-is.

### 9.2 SttEngineCard (`src/views/settings/tabs/SttEngineCard.tsx`)

- Unchanged. The card renders identically on both hosts; the
  presence of the Parakeet card is driven entirely by the
  `parakeetAvailability` result.

### 9.3 "Re-check" button

The current macOS-side flow doesn't have a Parakeet recheck
button (only Apple Intelligence does). We don't need one on
Windows either — DirectML availability is fixed at OS install
time. If a user reports "Parakeet says unavailable" the fix is
always to update Windows or grant net egress for the model
download, neither of which a recheck button helps with.

---

## 10. Pre-warm + session integration

### 10.1 Pre-warm hook

Existing call site: `lib.rs::run` → `crate::engines::prewarm_stt_in_background(app_handle)`.
That helper reads Settings, looks up the engine in the Registry,
and calls `engine.prewarm()`. The Windows
`ParakeetLocalEngine::prewarm` implementation:

```rust
async fn prewarm(&self) -> anyhow::Result<()> {
    let Some(dir) = self.model_dir() else { return Ok(()) };
    if !dir.join("vocab.txt").exists() { return Ok(()); }
    let cell = self.onnx_loaded.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if cell.get().is_some() { return Ok::<_, anyhow::Error>(()); }
        let started = std::time::Instant::now();
        let loaded = onnx_imp::load_blocking(&dir)?;
        let _ = cell.set(loaded);
        log::info!("parakeet-onnx: prewarm complete in {} ms",
                   started.elapsed().as_millis());
        Ok(())
    }).await
      .map_err(|e| anyhow::anyhow!("parakeet prewarm task panicked: {e}"))??;
    Ok(())
}
```

Reuses the same OnceCell-of-Arc pattern that Whisper uses. Idempotent.

### 10.2 Session integration

`session.rs::run_pipeline` already calls `engine.transcribe(wav, …)`
through the trait object. No change. The Windows engine's
`transcribe` decodes the WAV (`decode_wav_to_f32_mono_16k`), gets
or initialises the `ParakeetTDT`, spawn_blocking-runs
`transcribe_samples`, then fires the optional segment callback
once with the cumulative text at end-of-stream — the same
pattern as the macOS path.

---

## 11. Fail-open behavior

| failure mode | detection | runtime path | user experience |
|---|---|---|---|
| `onnxruntime.dll` missing from MSI payload | first `ParakeetTDT::from_pretrained` call returns ort load error | `transcribe()` returns `Err(anyhow!(…))` | STT error pill ("Couldn't load the speech model"); user can switch to Whisper in Settings |
| `DirectML.dll` missing | first session-create returns a `DXGI_ERROR` from the DirectML EP | inside `parakeet-rs::ModelConfig::apply_to_session_builder`, the CPU EP is registered with `.error_on_failure()` — DirectML failure → CPU EP runs → no user-visible error | dictation works on CPU only (slower; same as macOS Intel fallback) |
| Model file missing on disk | the engine's `ensure_loaded`'s `vocab.txt` probe returns false before any ort call | `transcribe()` returns `"model not downloaded — open Settings → Models"` | STT error pill with that exact message |
| Model file corrupted (sha mismatch on download) | downloader's per-file `expected_sha256` check (when populated) drops the partial and fails the download | the user sees "download failed" before transcribe is ever called | same as today's Whisper download-failure UX |
| Model partially downloaded (cancelled mid-flight) | `list()` install probe returns `installed: false` because `vocab.txt` either isn't there yet or `encoder-model.onnx.data`'s size check fails | engine `transcribe()` returns the "not downloaded" error | same pill as above |
| DirectML init succeeds but inference panics | ort raises an `ort::Error` → captured by `parakeet-rs` as `Err` → propagates as `transcribe()` `Err` | session aborts with friendly pill | user can re-record or switch engines |
| ORT version mismatch (rare — would only happen if user manually replaced the bundled DLL) | first session-create fails with "API version not supported" from `onnxruntime.dll` | `transcribe()` returns `Err` | same as the .dll-missing case; recommend reinstall Vox |
| `parakeet-rs` panic in feature extraction (e.g. STFT plan creation) | `spawn_blocking` returns `Err` (panic captured) | `transcribe()` returns `Err` | STT error pill |
| Audio under 1 second / silence | `audio` module's silence detection already short-circuits before ever calling `transcribe()` — unchanged | same as today | same as today |

In every error path the existing `session::SessionManager` paints a
friendly error pill and abandons the session. Whisper is always
selectable as the fallback engine; no path silently switches
engines under the user.

---

## 12. Risk register

Ranked by likelihood × impact (high → low):

1. **WiX MSI omits `onnxruntime.dll` / `DirectML.dll`** — Tauri's
   default WiX template may not pick up build-script copy-dylibs
   output. *Detection*: build a release MSI locally and inspect
   with `7-Zip`. *Mitigation*: explicit `bundle.resources` entry in
   `tauri.conf.json` pointing to `../target/release/*.dll`.
   *Fallback*: switch `ort` to `load-dynamic` + bundle the DLLs
   with `externalBin`. **Likelihood: medium. Impact: high (binary
   crashes on launch)**.

2. **ort `download-binaries` blocked on the build machine by
   corporate proxy** — `cdn.pyke.io` HTTPS egress. *Detection*:
   first `cargo build` on a fresh dev box fails. *Mitigation*:
   document the `ORT_LIB_LOCATION` env var override pointing at a
   manually-downloaded ORT archive. *Fallback*: bake a copy of
   the ORT prebuild into a release branch + use `ORT_LIB_LOCATION`
   on CI. **Likelihood: low for solo-dev, high for enterprise
   adopters. Impact: medium (blocks build, not runtime)**.

3. **DirectML EP fails to initialise on a tiny set of GPUs** —
   integrated GPUs older than DX12 (pre-2015) genuinely don't
   support DirectML. *Detection*: log line at session-create.
   *Mitigation*: CPU EP fallback inside `parakeet-rs` is
   automatic (`.error_on_failure()` on CPU). *Fallback*: user
   sees CPU performance only; ~5× slower than DirectML but still
   ~2× faster than Whisper on the same box (per `parakeet-rs`
   README claim). **Likelihood: low. Impact: low (no crash,
   slower)**.

4. **`encoder-model.onnx.data` external weights file gets renamed
   during transport** — HF resolve URLs sometimes redirect
   through CDNs that add Content-Disposition. *Detection*: a
   load-time ort error referencing the missing weights location.
   *Mitigation*: the downloader writes to the `relpath` we
   control, not Content-Disposition. Verify with a smoke-test
   download. **Likelihood: low. Impact: high (model unusable)**.

5. **HF model repo updates and changes file sizes** — sha256
   pinning is `None` by default; soft-canary in `models.lock`.
   *Detection*: a download produces a file noticeably larger than
   our pinned `size_bytes`. *Mitigation*: same as Gemma/Whisper —
   the `>= size / 2` install gate accepts small upward drift;
   significant drift will fail the gate and the user re-downloads.
   *Fallback*: pin sha256 in a follow-up PR once we observe the
   first successful download. **Likelihood: low. Impact: medium
   (forced re-download)**.

6. **ort 2.0-rc.12 → 2.0 stable bump changes the API** — the rc
   period is intentionally unstable. *Detection*: a follow-up
   build fails to compile. *Mitigation*: pin `parakeet-rs = "=0.3.5"`
   (or whatever the verified version is) and `ort = "=2.0.0-rc.12"`
   so a transitive bump doesn't auto-update. *Fallback*: maintain
   a `cargo update -p parakeet-rs` watch on the windows-support
   branch CI. **Likelihood: medium. Impact: medium**.

7. **Long-form audio truncation** — `parakeet-rs` README warns of
   a ~4-5 minute audio length limit on TDT models. *Detection*:
   transcribe of a 5+ minute clip returns truncated text.
   *Mitigation*: Vox sessions are bounded by hotkey press
   duration; a typical dictation is <60 s. Document in
   CLAUDE.md's "Known limitations" if we ever surface
   long-form. **Likelihood: very low (single dictation > 4 min
   is unusual). Impact: low**.

8. **Bundle size jump from ~480 MB (macOS) to ~2.5 GB (Windows)**
   pushes Vox's download-to-first-dictation latency higher on
   slow connections. *Detection*: onboarding telemetry (we don't
   have any) or user reports. *Mitigation*: the existing stalled-
   download detector in DownloadStep already surfaces a hint
   when 15 s pass without progress. *Fallback*: in a follow-up,
   ship the int8 variant (~670 MB combined) under a "low
   bandwidth" toggle. **Likelihood: certain. Impact: low (one
   slow download once)**.

---

## 13. Out of scope

- **CUDA execution provider on Windows** — NVIDIA discrete GPUs.
  Would require shipping a different ort prebuild
  (`x86_64-pc-windows-msvc+cu12.tar.lzma2`, ~400 MB extra). v1
  ships DirectML only; DirectML covers every DX12 GPU including
  NVIDIA, AMD, Intel.
- **TensorRT** — same reasoning, even more vendor-specific.
- **Streaming inference** (`ParakeetEOU` / `Nemotron` variants).
  Current pipeline is hotkey-bounded recording; partial streaming
  improvement is a bigger refactor in `session.rs`.
- **INT8 quantised models**. The HF repo ships
  `encoder-model.int8.onnx` (~650 MB) + `decoder_joint-model.int8.onnx`
  (~18 MB) but accuracy regression has not been measured against
  Vox's test set. v1 ships FP32.
- **Re-exporting the model ourselves**. The plan uses `istupakov/
  parakeet-tdt-0.6b-v3-onnx` verbatim. A NeMo-side re-export is
  out of scope; if `istupakov` rotates the repo we cross that
  bridge.
- **Custom dictionary biasing on Parakeet**. The macOS engine
  doesn't have it either; same TODO applies to both hosts. Vox
  users who rely on dictionary biasing keep using Whisper.
- **Language-pinning Parakeet** (forcing a specific language
  rather than auto-detect). The TDT model auto-detects and
  there's no per-call hint in `parakeet-rs`'s public API.
- **macOS arm64 → ONNX as a second option**. The macOS path stays
  FluidAudio-only. No ONNX on Mac in v1.
- **Apple Intelligence Windows analogue / Phi Silica**. Already
  out of scope per CLAUDE.md.

---

## 14. Verification checklist

The implementer should run all of the following before declaring
done. macOS verification still runs first to prove zero regression.

### 14.1 macOS smoke (must pass unchanged)

```bash
# From repo root, on Apple Silicon with the existing toolchain:
cargo build  --manifest-path src-tauri/Cargo.toml
cargo test   --manifest-path src-tauri/Cargo.toml --lib
npx tsc --noEmit
npx vite build
npm run tauri build -- --bundles app
```

All four must produce identical artifacts to the pre-change
baseline:
- `cargo metadata` shows no `parakeet-rs` in the macOS dep tree.
- `Vox.app/Contents/MacOS/vox` binary size within ±2% of the
  pre-change build (no accidental cross-link of ort).
- Onboarding flow still defaults to FluidAudio Parakeet on an
  English pick.
- `parakeet_availability` returns `{ available: true }`.

### 14.2 Windows smoke

```bat
:: From repo root, on x86_64-pc-windows-msvc with VS 2022 + Rust 1.88+
cargo build  --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-msvc
cargo test   --manifest-path src-tauri/Cargo.toml --lib --target x86_64-pc-windows-msvc
npx tsc --noEmit
npx vite build
npm run tauri build -- --bundles msi
```

Then manually verify:
- `target/x86_64-pc-windows-msvc/release/onnxruntime.dll` exists
  (~12-25 MB).
- `target/x86_64-pc-windows-msvc/release/DirectML.dll` exists
  (~7 MB).
- `target/x86_64-pc-windows-msvc/release/bundle/msi/Vox_*.msi`
  contains both DLLs (extract with `7-Zip` and grep).
- Install the MSI, launch Vox.
- Onboarding: pick English. LanguageStep should show the Auto card
  as "Recommended — Parakeet". DownloadStep should show the Windows
  ONNX bundle (~2.4 GB) under the Parakeet section.
- Click Download. Watch the progress bar; it should hit 100%
  in finite time.
- Wait for the "How to use" step. Press the default hotkey
  (`Ctrl+Alt+Period`), speak "hello world", release. The pill
  should paint Listening → Transcribing → (Polishing) → Done,
  and "hello world" lands on the clipboard.
- Settings → Models: the Parakeet card is "Active". The Whisper
  card is selectable as a fallback.
- Open `vox.log` (or stdout): grep for `parakeet-onnx: prewarm
  complete in` to confirm the pre-warm fired.

### 14.3 Negative-path smoke

- Rename `onnxruntime.dll` in the install dir → re-launch Vox →
  first dictation should produce a clean error pill, not a crash.
  Restore the file → dictation works again.
- Delete `encoder-model.onnx.data` → first dictation should
  produce a "couldn't load model" error pill. Re-download from
  Settings → Models → re-dictate → works.
- Disable hardware acceleration (Settings → System → Display →
  Graphics → DirectML off — not actually possible on most
  Windows boxes, but a DX9-only GPU would simulate this) → first
  dictation logs DirectML init failure → CPU EP runs → text still
  lands on clipboard.

### 14.4 Static type / lint checks

```bash
# Frontend
npx tsc --noEmit          # cross-platform, runs on the dev Mac
npx vite build            # cross-platform

# Backend (run on each host)
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo clippy --manifest-path src-tauri/Cargo.toml --target x86_64-pc-windows-msvc --all-targets -- -D warnings
```

---

## End of plan
