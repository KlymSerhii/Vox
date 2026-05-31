# Vox — Windows Port: File-by-File Implementation Plan

Status: design only. No production code written yet. Authored by reading the
actual `src-tauri/src/` + `src/` tree against `CLAUDE.md` on 2026-05-27.

---

## Executive summary

**The good news: someone already did 60–70% of the cross-platform plumbing.**
Vox's Rust core was clearly written with a Windows port in mind. Before
proposing anything, verify these *already-present* facts (all cited below):

- `command-group` (`Cargo.toml:107`) already gives us Job-Object
  `KILL_ON_JOB_CLOSE` child teardown on Windows via `group_spawn()`
  (`inference_proc.rs:317`). This is the single hardest piece of the port and
  it is done.
- `inference_server.rs` already ships a **complete Windows descriptor**
  (`win-vulkan-x64.zip`, pinned SHA256, `llama-server.exe`,
  `inference_server.rs:96–107`) and a zip extractor with the same zip-slip
  hardening as the tar path (`extract_zip`, `inference_server.rs:498`).
- `arboard` (`Cargo.toml:77`) is the clipboard layer in `inject.rs` and is
  cross-platform; clipboard read/write/snapshot already work on Windows
  unchanged.
- Every macOS FFI module (`mac_paste`, `mac_perm`, `mac_chord`, `frontmost`,
  `mac_power`, `mac_wake`) is either `#[cfg(target_os = "macos")]`-gated at the
  module declaration (`lib.rs:11–16`) or carries an internal
  `#[cfg(not(target_os = "macos"))]` stub. **The macOS-only build is not at
  risk** from adding Windows paths as long as we keep mirroring that pattern.
- `apple-fm` is correctly triple-gated: build step
  (`build.rs:main` gates on `target_os = "macos"` *and*
  `CARGO_FEATURE_APPLE_FM`), module decl
  (`engines/mod.rs:11`: `#[cfg(all(target_os = "macos", feature = "apple-fm"))]`),
  and the `Registry::llm` match arm (`engines/mod.rs:173`). On Windows the
  engine is unreachable and `Registry::llm("apple-fm", …)` returns `None`,
  which `session.rs` fail-opens to the raw transcript. **No tightening needed.**
- The permission probes already stub correctly on non-macOS:
  `probe_accessibility() -> true` (`commands.rs:626`),
  `probe_microphone() -> true` (`commands.rs:639`),
  `request_microphone() -> true` (`commands.rs:657`).
- The auto-paste delivery path already has the non-macOS arm wired —
  `session.rs:1147` returns `Err("auto-paste is macOS-only")`, which fail-opens
  to clipboard-only. The Windows job is to *replace that stub* with a real
  `win_paste::post_ctrl_v()`.

**What's actually left to do** is concentrated in five buckets:

1. **Build system** — the one true compile-breaker. `whisper-rs` is declared in
   the platform-agnostic `[dependencies]` with `features = ["coreml","metal"]`
   (`Cargo.toml:97`). Those backends are Apple-only. On Windows this will fail
   to build. Must target-split the dependency.
2. **Synthetic paste** — new `win_paste.rs` using `SendInput` (Ctrl+V); wire
   into `session.rs:1145`'s `cfg` ladder. No permission gate on Windows.
3. **Focused-app probe** — new Windows arm in `frontmost.rs` using
   `GetForegroundWindow` → `GetWindowThreadProcessId` →
   `QueryFullProcessImageNameW` (+ window title via `GetWindowTextW`).
4. **Frontend platform-awareness** — the frontend has **zero** platform
   detection today and hardcodes mac glyphs (`⌘⌥⇧⌃`) and mac copy
   ("press ⌘V", "Requires Accessibility", "System Settings"). Needs a single
   `platform` helper and conditional rendering at a handful of sites.
5. **Bundling / deep-links / autostart** — Windows bundle targets (nsis),
   icons (`.ico`), the `open` shell-outs (`commands.rs:415/435/596`) that only
   exist on macOS, and the `"Copied — press ⌘V"` Done-message string
   (`session.rs:1754`) which must say "Ctrl+V" on Windows.

**In scope on Windows (v1):** Whisper STT (CPU or Vulkan backend), Gemma
polishing via the existing `llama-server` + Gemma pipeline, clipboard-default
delivery, opt-in auto-paste (no permission gate), global hotkey via
`tauri-plugin-global-shortcut`, tray + popover + pill + onboarding, focused-app
mode suggestion, auto-updater (NSIS), launch-at-login.

**Out of scope on Windows (v1):** Apple Intelligence / `apple-fm` (deleted at
compile time), Parakeet STT (FluidAudio is arm64-mac-only — same compile-out),
modifier-only chord / double-tap PTT (the `mac_chord.rs` `CGEventTap` path is
already dormant on macOS too; the picker never offers it), the pill floating
*above fullscreen apps* (mac `NSPanel` trick — Windows pill still works, just
as a normal always-on-top window), Continuity-microphone wake UX.

**Top 5 risks** (detail in each section):

1. **whisper-rs backend choice can only be validated on real Windows.** Plain
   CPU is the safe default (zero external runtime deps, always builds). Vulkan
   is faster but the `win-vulkan-x64` llama.cpp binary needs the Vulkan
   loader/ICD present and the whisper-rs `vulkan` feature needs the Vulkan SDK
   at *build* time. Recommendation: ship **CPU whisper** for v1 to de-risk the
   build, keep **Vulkan llama-server** (it's a downloaded binary, not linked).
2. **`whisper-rs-sys` build on Windows needs CMake + a C/C++ toolchain (MSVC).**
   We can't test the native build from a Mac. The crate's `bindgen` step and
   ggml CMake build are the most likely source of a Windows-only compile
   failure.
3. **Code signing + SmartScreen.** Unsigned `Vox.exe` + downloaded
   `llama-server.exe` will trip SmartScreen / Defender. The updater is
   minisign-pinned (fine), but distribution needs an Authenticode cert
   (human/business decision).
4. **`tauri-plugin-global-shortcut` Win key + reserved combos.** The default
   `Cmd+Alt+Period` maps to `Ctrl+Alt+Period` on Windows (via the
   `CommandOrControl` alias) — fine — but some combos are OS-reserved on
   Windows and bind-fails surface differently. The default needs a real-Windows
   smoke test.
5. **Firewall prompt for `llama-server`.** Binding `127.0.0.1` (already done,
   `inference_proc.rs:48`) avoids the public-network prompt in most cases, but
   Defender Firewall behavior on first loopback bind can only be confirmed on
   real Windows.

A note on **`app_data_dir` fallback drift** found during investigation
(not Windows-specific, but it bites the Windows fallback path): three sites use
a literal `"app.vox.vox/…"` fallback when `app.path().app_data_dir()` fails —
`inference_server.rs:177`, `models.rs:289`, `modes.rs:759` — but the bundle id
is `com.vox.app` (`tauri.conf.json:5`). On the happy path Tauri returns the
real dir so this never fires, but it's an inconsistency worth fixing in the
same PR (flagged in Open Questions).

---

## Area 1 — Build system / Cargo features / bundling

### 1a. whisper-rs backend (THE compile-breaker)

**Current (macOS):**
`Cargo.toml:97` — `whisper-rs = { version = "0.16", features = ["coreml","metal"], optional = true }`,
declared in the **platform-agnostic `[dependencies]` table**. Verified against
crates.io: `whisper-rs` `coreml` → `whisper-rs-sys/coreml`, `metal` →
`whisper-rs-sys/metal` + `_gpu`. Both are Apple-only backends; `whisper-rs-sys`
0.15 also exposes `cuda`, `hipblas`, `intel-sycl`, `openblas`, `vulkan`, and a
plain (no-feature) CPU build. The engine code (`whisper_local.rs`) calls
`params.flash_attn(true)` (`whisper_local.rs:50`) which is backend-agnostic.

**Prescribed (Windows):** Split the `whisper-rs` dependency by target so the
Apple backends never reach a Windows build:

```toml
# Remove the whisper-rs line from [dependencies]; replace with two
# target-scoped declarations. The `optional = true` + `local-engines`
# gate must be preserved on BOTH so `--no-default-features` still works.

[target.'cfg(target_os = "macos")'.dependencies]
# (existing objc2 block stays here too)
whisper-rs = { version = "0.16", features = ["coreml", "metal"], optional = true }

[target.'cfg(target_os = "windows")'.dependencies]
# v1: plain CPU backend — no GPU feature. Zero external runtime/SDK deps,
# always builds with the MSVC toolchain + CMake. This is the de-risking
# choice; Vulkan can be a fast-follow once the CPU build is green on real HW.
whisper-rs = { version = "0.16", optional = true }
```

Caveat: Cargo does **not** merge features across two target tables for the same
crate — that's exactly what we want (mac gets coreml+metal, win gets neither).
But because `local-engines = ["dep:whisper-rs"]` (`Cargo.toml:38`) references the
optional dep by the bare crate name, confirm that `dep:whisper-rs` still
resolves when the crate is declared only in target tables. **It does** (Cargo
resolves `dep:` against the union of all dependency tables), but this is a
"verify the `cargo build` + `cargo metadata` on both targets" item.

**APIs/crates:** no new crate. Optionally add a `whisper-vulkan` Vox feature
later that flips the Windows declaration to `features = ["vulkan"]`.

**Edge cases / real-Windows-only:**
- `whisper-rs-sys` builds ggml via CMake + bindgen. Needs Visual Studio Build
  Tools (MSVC) + CMake + LLVM/libclang on the Windows build machine. **Cannot
  be validated from macOS.** This is risk #2.
- CPU-only Whisper is materially slower than Metal. The `audio_ctx` clamp
  (`whisper_local.rs:223`) and physical-core thread count
  (`whisper_local.rs:197`) already help. Acceptable for v1; document it.
- If we ever pick `vulkan`, the *build* needs the Vulkan SDK and the *runtime*
  needs a Vulkan ICD (most GPUs ship one). CPU avoids both.

### 1b. `build.rs` gating

**Current:** `build.rs:main()` already wraps the Swift bridge builds in
`#[cfg(target_os = "macos")]` and `link_clang_rt_osx()` / the
`ApplicationServices` framework link in the same. `emit_tray_icons()` +
`tauri_build::build()` run on all platforms. The Swift `build_apple_fm_bridge`
/ `build_parakeet_bridge` fns are themselves `#[cfg(target_os = "macos")]`.

**Prescribed (Windows):** **No change required.** On Windows the
`#[cfg(target_os = "macos")]` blocks compile out entirely, `emit_tray_icons()`
(pure `png` crate, no platform deps) runs, and `tauri_build::build()` runs.
Verify `build.rs` has no `use` that's mac-only at the top level (it doesn't —
only `std` + `png` via tauri-build). One nicety: the `mod tray_glyphs`
`include!` (`build.rs:11`) pulls `src/tray_glyphs.rs`, which is pure `std::f32`
math — confirmed platform-agnostic.

### 1c. Cargo features

**Current:** `default = ["local-engines", "apple-fm", "parakeet"]`
(`Cargo.toml:37`).

**Prescribed (Windows):** Leave the default list **as-is**. `apple-fm` and
`parakeet` are no-ops on Windows by construction (build.rs skips them; modules
cfg out). The Windows CI/release build can use the default features unchanged;
the feature flags don't need a Windows variant. Document that on Windows
`apple-fm`/`parakeet` are inert.

### 1d. `tauri.conf.json` bundle targets + updater

**Current:** `tauri.conf.json:36–39` targets `["app","dmg"]` (both mac-only);
`bundle.icon` is `["icons/icon.icns"]` (`:40`); `bundle.macOS` block at `:46`;
`plugins.updater.windows.installMode = "passive"` is **already present**
(`:61–63`); endpoints + minisign pubkey are platform-agnostic (`:57–60`).
`createUpdaterArtifacts: true` (`:35`).

**Prescribed (Windows):** Tauri's bundler picks targets per host OS, so the
simplest correct approach is to make `targets` host-aware. Two options:

- **Option A (recommended):** set `"targets": "all"` and let `tauri build` emit
  the right artifacts per host (mac → app/dmg, Windows → nsis). Add a Windows
  icon and an `nsis` config block. `"all"` on macOS still produces app+dmg.
- **Option B:** keep an explicit list but build Windows with
  `tauri build --bundles nsis` on the Windows runner, overriding config.

Concrete changes:
```jsonc
"bundle": {
  "targets": "all",                      // was ["app","dmg"]
  "icon": [
    "icons/icon.icns",                   // mac
    "icons/icon.ico",                    // NEW — Windows (multi-res .ico)
    "icons/128x128.png", "icons/128x128@2x.png"  // tray/window fallbacks
  ],
  "windows": {                            // NEW sibling of "macOS"
    "nsis": {
      "installMode": "perMachine",        // or "currentUser"; see edge cases
      "languages": ["English"]
    }
    // Authenticode signing fields (certificateThumbprint / digestAlgorithm
    // / timestampUrl) go here when a cert exists — human decision.
  }
}
```
`tauri-cli` can generate the full icon set incl. `.ico` from a source PNG via
`npm run tauri icon <path-to-1024.png>`. Prefer that over hand-rolling.

Updater: **NSIS is the only updater-compatible Windows target** (MSI is not
supported by `tauri-plugin-updater`). The `installMode: "passive"` already set
is correct for the updater's silent reinstall. `createUpdaterArtifacts: true`
will emit the `.nsis.zip` + `.sig` that the minisign-pinned `latest.json`
references. **The release pipeline (`scripts/release.sh`) must publish the
Windows artifacts + a Windows entry in `latest.json`** (currently
mac-only — see Ordering).

**Edge cases / real-Windows-only:**
- `perMachine` NSIS needs admin elevation at install; `currentUser` doesn't but
  installs per-user. Per-user is friendlier for an unsigned indie app; decide
  with distribution.
- The updater replaces a running `Vox.exe` — NSIS passive mode + Tauri's
  restart handle this, but the `llama-server.exe` child must be torn down first.
  `quit_app`/`RunEvent::Exit` already call `shutdown_managed`
  (`commands.rs:407`, `lib.rs:286`); the Job Object guarantees teardown. Verify
  the updater's relaunch doesn't orphan a child (the startup reaper
  `reap_orphans_at_startup` is the backstop, `lib.rs:184`).
- SmartScreen on an unsigned installer (risk #3).

### 1e. `entitlements.plist`

Mac-only (`tauri.conf.json:48` references it only inside `bundle.macOS`). **No
Windows action.** Leave the file; it's never read on a Windows build.

### New/changed files (Area 1)
- **Modify** `src-tauri/Cargo.toml` — target-split `whisper-rs`.
- **Modify** `src-tauri/tauri.conf.json` — `targets`, icons, `windows.nsis`.
- **Create** `src-tauri/icons/icon.ico` (+ regenerate PNG set via `tauri icon`).
- `build.rs` — no change.

---

## Area 2 — Hotkeys (`hotkey.rs`, `mac_chord.rs`)

**Current (macOS):**
- The push-to-talk, cancel, and mode-cycle bindings all go through
  `tauri-plugin-global-shortcut` (`hotkey.rs:21`, `register()` at `:293`),
  which is **cross-platform** (RegisterEventHotKey on mac, RegisterHotKey on
  Windows). The `CommandOrControl` token resolves to ⌘ on mac, Ctrl on Windows
  inside `global_hotkey` — so the default `Cmd+Alt+Period` (stored literally,
  `settings.rs:69`) binds as Ctrl+Alt+Period on Windows automatically.
- The **only** mac-specific code in `hotkey.rs` is the chord / double-tap path:
  `#[cfg(target_os = "macos")] use crate::mac_chord` (`:25`), the
  `ActiveHotkeyState.chord` field (`:82`), the `install_chord` /
  `install_double_tap` / `store_chord_handle` / `friendly_chord_error` helpers
  (`:738–850`), and the chord-first branch in `register_push_to_talk`
  (`:546–610`). All already `#[cfg(target_os = "macos")]`.
- `AXIsProcessTrusted()` is logged at `register()` start (`:312–319`) — mac-only,
  already gated.
- `arm_cancel`/`disarm_cancel` (`:447`, `:492`) use only the cross-platform
  `global_shortcut` API — no FFI.

**Prescribed (Windows):** **Almost nothing.** The non-chord path is fully
cross-platform. On Windows:
- `register_push_to_talk` skips both `#[cfg(target_os = "macos")]` chord
  branches and falls straight through to the `shortcut_from_key` + `on_shortcut`
  bind (`hotkey.rs:612`). Correct as-is.
- The `mac_chord` module is `#[cfg(target_os = "macos")]` in `lib.rs:11`, so it
  never compiles on Windows; the dormant chord PTT remains unavailable (out of
  scope, matches macOS where no picker offers it).
- `is_safe_alone` (`hotkey.rs:146`) gates lone F-keys/Escape — platform-agnostic,
  keep.

**APIs/crates:** none new. `tauri-plugin-global-shortcut` (already a dep,
`Cargo.toml:48`) handles `RegisterHotKey` on Windows.

**Edge cases / real-Windows-only:**
- Some chords are OS-reserved on Windows (e.g. `Ctrl+Alt+Del`, certain
  `Win+` combos). `RegisterHotKey` fails for those; the existing
  `try_chain` fallback (`hotkey.rs:181`) + `ActiveHotkey.error_chain` already
  surface bind failures in the diagnostics panel. The default
  `Ctrl+Alt+Period` is not reserved. **Real-Windows smoke test** the default +
  a couple of user picks (risk #4).
- The `Code`/`Modifiers` enums from `global_hotkey` already include `Period`,
  function keys, and the `SUPER`/`META` (Win key) modifier. If we ever want a
  `Win+`-based default we'd extend the picker; v1 keeps `Ctrl+Alt+Period`.
- `try_into_wry` downcast (`hotkey.rs:730`) is runtime-generic and
  platform-agnostic — no change.

### New/changed files (Area 2)
- **No new files.** `hotkey.rs` already compiles on Windows with the existing
  cfg gates. Optional: a one-line doc comment noting the chord path is mac-only.

---

## Area 3 — Clipboard + paste (`inject.rs`, `mac_paste.rs`)

**Current (macOS):**
- `inject.rs` uses `arboard::Clipboard` for `copy_to_clipboard` (`:33`),
  `snapshot_clipboard_text` (`:51`), and the Gatekeeper-priming
  `prewarm()` round-trip (`:76–100`). `arboard` is cross-platform; the only
  *macOS-specific reason* prewarm exists is the NSPasteboard/Gatekeeper
  "Verifying com.vox…" first-use signature check — that delay does not exist on
  Windows.
- `mac_paste.rs` posts a synthetic ⌘V via `CGEventCreateKeyboardEvent` +
  `CGEventPost(kCGHIDEventTap)` and gates on `AXIsProcessTrusted()`
  (`mac_paste.rs:88`). The module is `#[cfg(target_os = "macos")]` in
  `lib.rs:13`.
- The orchestration in `session.rs:1145` already has the non-mac arm:
  `#[cfg(not(target_os = "macos"))] let paste_result = Err("auto-paste is
  macOS-only")`.

**Prescribed (Windows):**

1. **Clipboard read/write/snapshot/restore:** keep `arboard` — **no change**.
   `inject.rs` is already platform-agnostic. The chained-pipeline restore logic
   in `session.rs:1158–1192` is pure Rust over `arboard` and works unchanged.

2. **Synthetic Ctrl+V:** create `src-tauri/src/win_paste.rs` mirroring
   `mac_paste.rs`'s shape (typed error enum, single `post_ctrl_v()` entry
   point). Use `SendInput` from the `windows` crate to send a Ctrl-down,
   V-down, V-up, Ctrl-up sequence with `KEYEVENTF_SCANCODE` (or virtual-key
   `VK_CONTROL` + `VK_V`). Virtual-key is simpler and layout-correct for
   shortcuts (Windows routes Ctrl-shortcuts by VK, like macOS does by physical
   key). **No permission probe** — `SendInput` needs no Accessibility-style
   grant, so `post_ctrl_v()` never returns an `AccessibilityDenied`-style error;
   it can only fail if `SendInput` injects fewer events than requested (rare).

3. **Wire into `session.rs:1145`:** turn the two-arm `cfg` into three:
   ```rust
   #[cfg(target_os = "macos")]
   let paste_result = crate::mac_paste::post_cmd_v().map_err(|e| e.to_string());
   #[cfg(target_os = "windows")]
   let paste_result = crate::win_paste::post_ctrl_v().map_err(|e| e.to_string());
   #[cfg(not(any(target_os = "macos", target_os = "windows")))]
   let paste_result: Result<(), String> = Err("auto-paste unsupported on this OS".into());
   ```

4. **Prewarm:** `inject::prewarm_in_background()` (`inject.rs:76`) is harmless on
   Windows (a clipboard get+set round-trip) but pointless. Cheapest correct
   move: keep calling it unconditionally from `lib.rs:231` (it's a fast no-op-ish
   round-trip), OR gate the call site `#[cfg(target_os = "macos")]`. **Recommend
   gating the call** to avoid a needless clipboard touch at every Windows launch;
   leave the fn itself intact.

**APIs/crates:** add the `windows` crate (Win32 FFI) under a Windows target
table — see Cargo block at the end. Needed crate features:
`Win32_UI_Input_KeyboardAndMouse` (for `SendInput`, `INPUT`, `KEYBDINPUT`,
`VK_CONTROL`, `VK_V`, `KEYEVENTF_KEYUP`).

**Module declaration:** add to `lib.rs` mirroring `mac_paste`:
```rust
#[cfg(target_os = "windows")]
mod win_paste;
```

**Edge cases / real-Windows-only:**
- `SendInput` is blocked from injecting into a higher-integrity (elevated /
  UIPI) foreground window if Vox runs unelevated. The paste silently no-ops into
  an elevated app (e.g. an admin cmd window). The existing fail-open Done
  message ("Copied — press Ctrl+V") covers this gracefully; document it. **Real-
  Windows only.**
- A still-held PTT modifier could leak into the synthetic Ctrl+V. macOS solves
  this by sharing HID system modifier state (`mac_paste.rs:32`). On Windows,
  `SendInput` does not inherit physical modifier state, but if the user is
  *holding* a modifier when the paste fires, it composes. Mitigation: the
  default PTT is press-to-toggle (`hands_free_enabled = true`,
  `settings.rs:299`), so no modifier is held at paste time. For hold-mode,
  consider explicitly clearing modifier state — flag as a polish item, not a v1
  blocker.
- Clipboard contention: another app holding the Windows clipboard open can make
  `arboard` set/get fail transiently. `inject.rs` already maps errors to
  `InjectError`/`None`; the pipeline fail-opens. No new handling needed.

### New/changed files (Area 3)
- **Create** `src-tauri/src/win_paste.rs`.
- **Modify** `src-tauri/src/lib.rs` — `#[cfg(target_os = "windows")] mod win_paste;`.
- **Modify** `src-tauri/src/session.rs` — three-arm `cfg` at `:1145`.
- **Modify** `src-tauri/src/lib.rs` — gate the `inject::prewarm_in_background()`
  call (`:231`) to macOS.

---

## Area 4 — Permissions (`mac_perm.rs`, Accessibility gate)

**Current (macOS):**
- `mac_perm.rs` (AVFoundation `authorizationStatusForMediaType:` /
  `requestAccessForMediaType:`) is `#[cfg(target_os = "macos")]` in `lib.rs:15`.
- `commands.rs` already stubs every permission entry point for non-mac:
  `probe_accessibility() -> true` (`:626`), `probe_microphone() -> true`
  (`:639`), `request_microphone() -> true` (`:657`),
  `check_permissions` returns `{microphone:true, accessibility:true}` via those
  stubs (`:610`).
- `tray_icon.rs:274` — `missing_permissions() -> false` on non-mac, so the tray
  never shows the "needs permission" badge on Windows.

**Prescribed (Windows):**

1. **Microphone:** Windows has no TCC-style pre-grant prompt that blocks cpal
   the way macOS 14+ does. cpal opens the WASAPI device directly. The OS-level
   "Microphone access" privacy toggle (Settings → Privacy → Microphone) can be
   off, in which case cpal fails to open the device at *record* time. That
   failure already surfaces as `AudioError::Cpal` → a friendly pill
   (`audio.rs:77`, surfaced via the existing `BeginOutcome::AudioError` path).
   So `probe_microphone() -> true` and `request_microphone() -> true` are
   **acceptable v1 behavior** — Windows just relies on the record-time error
   instead of a pre-check. **No new code required**, but see onboarding (4c).

2. **Accessibility gate for auto-paste:** there is **no equivalent grant** on
   Windows — `SendInput` works without one. So the auto-paste path must **not**
   show an Accessibility nudge on Windows. The Rust side already returns
   `accessibility: true` from `probe_accessibility()`, which means the
   frontend's `showAccessibilityNudge()` (gated on `accessibility() === false`,
   `GeneralTab.tsx:82` / `TrayPopover.tsx`) **will already never render on
   Windows** — because the probe always reports granted. That's a happy
   accident that works, but the copy is still mac-flavored ("Requires
   Accessibility", `GeneralTab.tsx:194`). See Area 7 for the copy fix.

3. **Onboarding mic step (4c):** the wizard's Mic step
   (`onboarding/steps/MicStep.tsx`) calls `requestMicrophone()`. On Windows that
   returns `true` instantly (stub), so the step auto-advances / shows granted.
   That's fine functionally. **Recommended polish:** on Windows, reframe the
   step copy from "Grant microphone access" to "Vox uses your microphone — pick
   it in Windows Settings → Privacy if recording fails." (Area 7). Not a
   blocker.

**APIs/crates:** none required for v1. (Optional future: query
`Windows.Media.Capture` / the registry for the mic privacy state to drive a
real pre-check — explicitly out of scope.)

**Edge cases / real-Windows-only:**
- If the user has the global mic privacy toggle off, the *first* dictation fails
  with a cpal error. Verify the pill message is intelligible on Windows. **Real-
  Windows only.**
- `mac_perm.rs` must stay un-referenced on Windows. It already is (gated in
  `lib.rs:15`; callers in `commands.rs`/`tray_icon.rs` are behind
  `#[cfg(target_os = "macos")]`). No `mod win_perm` is needed for v1.

### New/changed files (Area 4)
- **No new Rust files.** Frontend copy changes only (Area 7).

---

## Area 5 — Subprocess management (`inference_server.rs`, `inference_proc.rs`)

This is the area with the most pre-existing Windows support. Verify, don't
rebuild.

**Current (already cross-platform):**
- **Windows binary descriptor exists:** `inference_server.rs:96–107` —
  `id: "llama-server-b8953-win-vulkan-x64"`, URL to
  `llama-b8953-bin-win-vulkan-x64.zip`, `expected_sha256` pinned,
  `archive: Zip`, `binary_filename: "llama-server.exe"`. Gated
  `#[cfg(all(target_os = "windows", target_arch = "x86_64"))]`.
- **Zip extraction exists:** `extract_zip` (`:498`) with `enclosed_name` +
  `safe_join` zip-slip hardening, parent-dir creation, and `#[cfg(unix)]`-gated
  permission preservation (so it's a clean no-op on Windows). The `tar.gz` path
  (`extract_tar_gz`) and the unix-only `xattr` quarantine strip (`:361`,
  `:599`) / `ensure_executable` chmod (`:584`) are all `#[cfg(unix)]`-gated, so
  Windows skips them correctly (no chmod/quarantine concept).
- **Job-Object teardown exists:** `inference_proc.rs:317` calls
  `cmd.group_spawn()` from `command-group`, which on Windows assigns the child
  to a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` (documented in the
  `Cargo.toml:99–107` comment and the `inference_proc.rs:306–316` comment). This
  is a **kernel-level kill-on-parent-exit guarantee** — strictly stronger than
  the macOS process-group, which leaks to launchd. The `spawn_async_killer`
  Drop path (`:587`) closes the Job Object on Windows.
- **Orphan reaper is cross-platform:** `reap_orphans_at_startup`
  (`inference_proc.rs:450`) uses `sysinfo` `System::processes()` with
  `with_cmd(UpdateKind::Always)` (`:463`), matches on
  `name.contains("llama-server")` (`:475`) — which still matches
  `llama-server.exe` on Windows — and kills via `proc_.kill_with(Signal::Kill)`
  (`:500`). `sysinfo`'s `system` feature (`Cargo.toml:93`) supports Windows
  process enumeration. The api-key match (`:481`) works identically.
- **Loopback bind:** `HOST = "127.0.0.1"` (`inference_proc.rs:48`) with an
  inline comment that this is the shape that avoids the **Windows Defender
  Firewall** prompt. Ephemeral port via `TcpListener::bind` (`:218`) is
  cross-platform.
- **`llama-server` flags** (`inference_proc.rs:239–298`): `-ngl 99` is clamped
  by llama.cpp to available GPU layers (comment at `:245` already notes
  "On Windows Vulkan / CPU builds llama.cpp clamps"). `--flash-attn on`,
  `--cache-type-k/v q8_0` — all backend-agnostic flags. No change.

**Prescribed (Windows):** **Verify, then leave largely alone.** Concrete items:

1. **Confirm the Vulkan vs CPU llama.cpp asset.** `win-vulkan-x64`
   (`:100`) requires the Vulkan runtime/ICD on the user's machine. Most discrete
   + integrated GPUs ship one, but a headless/VM/older box may lack it, and the
   `.exe` would fail to start → `wait_for_healthy` times out
   (`inference_proc.rs:375`) → `session.rs` fail-opens to the raw transcript.
   **Decision needed (Open Questions):** ship `win-vulkan-x64` (faster, small
   risk of missing ICD) vs `win-cpu-x64` / `win-avx2-x64` (universal, slower).
   Recommendation: **keep Vulkan** but verify the failure path is graceful on a
   no-Vulkan box, and consider a CPU fallback descriptor as a follow-up. (Note:
   the llama.cpp Vulkan zip also bundles its own DLLs incl. the ggml/vulkan
   backend; `walk_for_binary` (`:559`) finds `llama-server.exe` regardless of
   layout, and the sibling DLLs sit next to it so the loader finds them.)
2. **arm64 Windows:** the descriptor is `x86_64`-only (`:96`). On arm64 Windows
   the `#[cfg(not(any(...)))]` unsupported-host descriptor (`:114`) returns an
   empty URL and `descriptor_for_host()` → `None` (`:124`), so Settings shows
   "not supported host" and Gemma is unavailable. x86_64 Windows runs fine under
   emulation on arm64 anyway, but native arm64 is a follow-up. Acceptable v1.
3. **Pinned SHA256 freshness:** the win SHA256 (`:103`) was "computed locally on
   2026-05-04". If `LLAMA_CPP_TAG` is bumped, BOTH the mac and win digests must
   be recomputed (already documented at `:42`/`:101`). No action unless bumping.

**APIs/crates:** none new — `command-group`, `sysinfo`, `zip` already present.

**Edge cases / real-Windows-only:**
- The Job Object is created per-process; if Vox itself is *already* inside a Job
  Object that disallows nested breakaway (rare, e.g. some sandboxes), the child
  assignment can fail. `command-group` handles the common case; **real-Windows
  only.**
- Windows `unlink`-while-open semantics differ from macOS — on Windows you
  generally *cannot* delete an open `.exe`/weights file. `shutdown_if_using`
  (`inference_proc.rs:555`) already kills the server before
  `models::delete_model` removes the file, so the ordering is correct for
  Windows too (it was written for the macOS open-inode case but the kill-first
  ordering is exactly what Windows needs). **Verify delete-after-kill on real
  Windows.**
- First loopback bind firewall behavior (risk #5) — real-Windows only.

### New/changed files (Area 5)
- **No new files.** Possibly **modify** `inference_server.rs` only if we decide
  to add a CPU fallback descriptor (Open Questions).

---

## Area 6 — Tray / windows (`tray.rs`, `tray_icon.rs`, `tray_glyphs.rs`, `pill.rs`, `onboarding.rs`)

**Current (macOS):**
- **Tray popover** (`tray.rs:126`): built with `WebviewWindowBuilder` using
  `.skip_taskbar(true)` (`:141`), `.decorations(false)`, `.transparent(true)`,
  `.always_on_top(true)` — all **cross-platform Tauri APIs**. Monitor
  positioning uses Tauri's `available_monitors`/`primary_monitor` (`:111`,
  `:157`) — cross-platform.
- **Tray icon** (`tray_icon.rs`): `TrayIconBuilder` + `.icon_as_template(true)`
  (`tray.rs:42`). `icon_as_template` is a macOS template-image concept; on
  Windows tray-icon ignores it (the RGBA glyph renders as-is). The animated
  glyphs (`tray_glyphs.rs`, pure procedural RGBA) render identically. The
  "menu bar ink" white-on-dark assumption (`tray_icon.rs:20`) is mac-flavored
  but harmless — a white mic glyph reads fine in the Windows notification area
  (which is dark by default).
- **`missing_permissions()`** is already `#[cfg(not(target_os="macos"))] ->
  false` (`tray_icon.rs:274`).
- **Dock icon / `ActivationPolicy`** (`lib.rs:132–148`, `commands.rs:108`): the
  entire `set_activation_policy(Accessory/Regular)` block is
  `#[cfg(target_os = "macos")]`. On Windows there is no Dock; the equivalent is
  "show in taskbar". Vox is tray-only, so the main windows already set
  `.skip_taskbar(true)` (pill `pill.rs:109`, popover `tray.rs:141`). The
  **Settings/onboarding windows** are normal taskbar windows on Windows — which
  is the right Windows behavior (a tray app's settings window showing in the
  taskbar while open is conventional on Windows).
- **`RunEvent::Reopen`** (`lib.rs:292`) — mac Dock-click handler, already
  `#[cfg(target_os = "macos")]`. Windows has no Reopen event; tray-click already
  opens the popover (`tray.rs:48`), so no Windows equivalent is needed.
- **Pill** (`pill.rs`): `apply_macos_panel_style` (`:655`, NSPanel/NSWindow
  level + collection behavior to float over fullscreen) is
  `#[cfg(target_os = "macos")]` (call site `:120`).
  `active_screen_logical_frame` (`:604`) **already has a cross-platform
  fallback** using `available_monitors` + `cursor_position` (`:612–623`); the
  mac NSScreen path (`:625`) is gated. `spawn_display_follower` is keyed on
  `PILL_GENERATION` and uses Tauri monitor APIs — cross-platform.
- **`show_dock_icon` setting** (`settings.rs:197`) — read only inside
  `#[cfg(target_os = "macos")]` blocks; on Windows it's a stored-but-inert
  field. Fine.

**Prescribed (Windows):** **No new Rust files; minimal/zero changes.**
- Tray, popover, pill, onboarding all build on Windows with the existing
  cross-platform Tauri APIs. The mac-only `apply_macos_panel_style`,
  `RunEvent::Reopen`, and `set_activation_policy` blocks compile out.
- **Verify** the tray icon renders (the procedural RGBA glyph; `icon_as_template`
  is ignored on Windows — confirm it doesn't render inverted/black). If the
  white glyph is invisible on a light-themed notification area, we may need a
  Windows-tinted variant — **real-Windows only**; likely fine since the Windows
  tray flyout is dark.
- **Pill limitation:** on Windows the pill is a normal `always_on_top`
  transparent window. It will **not** float above a fullscreen exclusive game /
  video the way the mac NSPanel does. Acceptable v1 (document it). Tauri's
  transparent + decorations(false) + shadow works on Windows (WebView2).
- The Settings/onboarding windows correctly appear in the Windows taskbar while
  open and disappear when hidden (`lib.rs:259` hides on close) — no change.

**APIs/crates:** none.

**Edge cases / real-Windows-only:**
- WebView2 must be present (it ships with Win10 21H2+/Win11; older needs the
  Evergreen bootstrapper, which Tauri's NSIS can bundle). NSIS config can set
  `"webviewInstallMode"`. **Decision/real-Windows item.**
- Transparent windows on Windows require the WebView2 transparency support;
  verify the pill + popover render with the rounded alpha (not a black box).
  **Real-Windows only.**
- DPI scaling / per-monitor positioning: `tray.rs` and `pill.rs` already divide
  by `scale_factor()`; verify on a mixed-DPI Windows multi-monitor setup.

### New/changed files (Area 6)
- **No new files.** Optional: a Windows tray-glyph tint variant if the icon is
  invisible (deferred pending real-Windows check).

---

## Area 7 — Focused-app probe (`frontmost.rs`)

**Current (macOS):**
- `frontmost.rs:7` `#[cfg(target_os = "macos")] mod imp` — bundle id via
  `NSWorkspace::frontmostApplication().bundleIdentifier`, window title via the
  AX API (`AXUIElementCopyAttributeValue(AXFocusedWindow → AXTitle)`).
- `frontmost.rs:143` — `#[cfg(not(target_os = "macos"))] pub fn detect() -> None`
  stub. So **today on Windows mode auto-suggestion is silently disabled** (always
  returns `None` → no suggestion).
- Consumed by mode auto-suggestion (`commands::suggest_active_mode`,
  `modes.rs`); a `None` is treated as "no suggestion" — fail-open.

**Prescribed (Windows):** Implement a Windows `imp` module so mode
auto-suggestion works (the modes seed with `bundle_glob`/`window-title`
matchers; on Windows the "bundle id" maps to the process **executable name**,
e.g. `chrome.exe`, `code.exe`, `slack.exe`).

```rust
#[cfg(target_os = "windows")]
mod imp {
    // Win32: GetForegroundWindow() -> HWND
    //        GetWindowThreadProcessId(hwnd, &mut pid) -> pid
    //        OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, …, pid)
    //        QueryFullProcessImageNameW(handle, 0, buf, &len) -> full path
    //          -> take file_name() as the "bundle id" (e.g. "chrome.exe")
    //        GetWindowTextW(hwnd, buf, len) -> window title
    pub fn detect() -> Option<(String, Option<String>)> { /* … */ }
}
#[cfg(target_os = "windows")]
pub fn detect() -> Option<(String, Option<String>)> { imp::detect() }
```
Keep the existing `#[cfg(not(target_os = "macos"))]` stub but narrow it to
`#[cfg(not(any(target_os = "macos", target_os = "windows")))]` so the Windows
arm takes precedence and Linux/other still get the `None` stub.

**APIs/crates:** `windows` crate features:
`Win32_UI_WindowsAndMessaging` (`GetForegroundWindow`, `GetWindowThreadProcessId`,
`GetWindowTextW`, `GetWindowTextLengthW`),
`Win32_System_Threading` (`OpenProcess`, `QueryFullProcessImageNameW`,
`PROCESS_QUERY_LIMITED_INFORMATION`),
`Win32_Foundation` (`HWND`, `CloseHandle`, `MAX_PATH`).

**Implementation notes:**
- Fail-open like the mac path: any null HWND / failed call → `None`.
- Lowercase the exe name for stable glob matching (`Chrome.exe` vs `chrome.exe`).
- No special permission needed for `GetForegroundWindow`/`GetWindowTextW` of a
  normal-integrity window. Reading the title of a *higher-integrity* foreground
  window may fail (returns empty) — fall back to `(exe_name, None)`.
- Mode matchers: the built-in modes use bundle-id globs like `com.apple.dt.*`.
  Those won't match Windows exe names — **but that's a content/data issue, not a
  code issue**. v1: auto-suggestion works mechanically; the *shipped* mode
  triggers are mac-centric and will rarely fire on Windows. Optionally seed a
  couple of Windows exe-name triggers in `modes::builtins()` later (out of scope
  for the port itself; flag for PM).

**Edge cases / real-Windows-only:**
- UWP/packaged apps run under `ApplicationFrameHost.exe` for the frame window;
  the *real* app is a child. `QueryFullProcessImageNameW` on the foreground HWND
  may report `ApplicationFrameHost.exe` for UWP apps. Acceptable v1 limitation
  (most target apps — browsers, editors, Slack — are Win32). **Real-Windows
  only.**

### New/changed files (Area 7)
- **Modify** `src-tauri/src/frontmost.rs` — add Windows `imp` module + dispatch;
  narrow the fallback `cfg`.

---

## Area 8 — Frontend (`src/lib/hotkey.ts`, `HotkeyKeys.tsx`, pill copy, onboarding, Settings nudge)

**Current:** The frontend has **no platform detection at all** (verified: zero
`navigator.platform` / `isMac` / OS-plugin usage in `src/`). Mac specifics are
hardcoded:
- `hotkey.ts` glyph maps are mac-only: `MODIFIER_SYMBOLS` (`:39`) maps
  Cmd→⌘, Alt→⌥, etc.; `MODIFIER_LABELS` (`:52`) maps ⌘→"Cmd". `HotkeyKeys.tsx`
  renders `{symbol}{label}` keycaps — on Windows a `Ctrl+Alt+Period` spec would
  render `⌃ Ctrl ⌥ Opt .` (mac glyphs + "Opt"). Wrong for Windows.
- Done-message copy lives in **Rust** (`session.rs:1754`: `"Copied — press ⌘V"`)
  — see Area 8b.
- Settings auto-paste copy: `GeneralTab.tsx:194` "Vox sends ⌘V … Requires
  Accessibility"; the nudge `:204–205` "Open System Settings → Privacy &
  Security → Accessibility". `TrayPopover.tsx:1022/1037` similar.
- Onboarding: `DownloadStep.tsx:503` Apple-Intelligence card "Requires macOS 26
  + Apple Silicon"; `HowToUseStep.tsx`, `MicStep.tsx` likely have mac copy.
- Mic copy: `GeneralTab.tsx:135` / `TrayPopover.tsx:1183` "macOS Sound input".

**Prescribed (Windows):**

**8a. Platform helper (one new file, no new dependency).** Create
`src/lib/platform.ts`:
```ts
// WebView2 UA contains "Windows NT"; WKWebView contains "Macintosh".
// No @tauri-apps/plugin-os needed — keeps the dep surface flat and avoids
// adding a capability. Computed once at module load (platform can't change
// at runtime).
export const isMac = /Macintosh|Mac OS X/.test(navigator.userAgent);
export const isWindows = /Windows NT/.test(navigator.userAgent);
```
(If a more robust source is wanted, `@tauri-apps/api/os`'s `platform()` exists,
but it's async and pulls a capability; UA sniffing is sufficient and synchronous
for render-time branching. Decision flagged in Open Questions.)

**8b. Done-message copy (Rust).** Change `session.rs:1754` + `:1759` from the
hardcoded `"Copied — press \u{2318}V"` to a platform-aware string:
```rust
// helper in session.rs (or a small util)
#[cfg(target_os = "macos")]   const PASTE_HINT: &str = "Copied — press \u{2318}V";       // ⌘V
#[cfg(not(target_os = "macos"))] const PASTE_HINT: &str = "Copied — press Ctrl+V";
```
Also revisit `:1747` `"Copied — grant Accessibility for auto-paste"` — on
Windows auto-paste has no Accessibility concept, and `paste_failed` there means
`SendInput` failed (e.g. elevated target). Use a Windows-appropriate string:
`"Copied — paste didn't go through, press Ctrl+V"`. Gate with the same `cfg`.
("Inserted" at `:1752` is platform-neutral — keep.)

**8c. Hotkey keycap rendering.** Two viable approaches:
- **(Recommended) Render words on Windows, glyphs on mac.** In
  `HotkeyKeys.tsx`, when `isWindows`, render the modifier `label` only
  ("Ctrl", "Alt", "Shift", "Win") and the key as-is, dropping the mac glyph
  column. Add Windows label/symbol tables to `hotkey.ts` keyed off `isMac`:
  on Windows `MODIFIER_SYMBOLS` should map Cmd/CommandOrControl→"Ctrl"
  (text, not ⌘), Alt→"Alt", Shift→"Shift", Super/Meta→"Win". The cleanest
  implementation: keep the mac tables, add parallel Windows tables, and have
  `formatHotkeyTokens` pick the table by `isMac`. Note `formatHotkey` (the
  plain-text joiner, `hotkey.ts:179`) already prefers `label` over `symbol`, so
  once the Windows `label`s are right ("Ctrl"/"Alt"/…), the plain-text contexts
  (onboarding "couldn't be bound" warning, `<option>` labels) are automatically
  correct.
- The shifted-glyph collapse (`Cmd+Shift+/ → ⌘?`, `hotkey.ts:141`) is a mac
  convention; on Windows render `Ctrl+Shift+/` literally (don't collapse).
  Gate the collapse on `isMac`.

**8d. Settings + tray auto-paste copy.** In `GeneralTab.tsx` and
`TrayPopover.tsx`:
- Hint copy: `isMac ? "Vox sends ⌘V for you … Requires Accessibility."
  : "Vox sends Ctrl+V for you and restores your clipboard right after."`
- The Accessibility nudge sub-row (`GeneralTab.tsx:202`, `showAccessibilityNudge`
  `:82`) — on Windows `accessibility()` is always `true` (Rust stub), so the
  nudge **already never renders**. Belt-and-suspenders: also gate
  `showAccessibilityNudge()` on `!isWindows` so a future change to the probe
  can't accidentally surface a mac-only nudge on Windows.
- `openAccessibilitySettings()` (ipc.ts:354) → `commands::open_accessibility_settings`
  is a Windows no-op (Area 8f); the button won't render on Windows anyway.

**8e. Onboarding.** 
- The Apple-Intelligence engine card (`DownloadStep.tsx`, `AppleFmEngineRow.tsx`)
  should be **hidden on Windows** (`<Show when={isMac}>`), matching the existing
  behavior when `apple_fm_availability` reports `framework_missing` — on Windows
  the command returns `framework_missing` (the `#[cfg(not(...))]` path), so the
  card may already disable itself, but explicitly hiding it on Windows is
  cleaner and avoids a dead "macOS 26 + Apple Silicon" card.
- `MicStep.tsx`: on Windows, `requestMicrophone()` returns `true` instantly.
  Reframe copy to a Windows-appropriate "Vox will use your microphone" message
  and skip the "grant permission" framing. Keep the step (it's a nice place to
  confirm a device), or auto-advance.
- `HowToUseStep.tsx` and any "press ⌘V" / hotkey illustrations: use `<HotkeyKeys>`
  (already platform-aware after 8c) and `isMac` for the paste-key glyph.

**8f. Shell-out commands (Rust, supports 8d/8e).** `open_external_url`
(`commands.rs:429`), `open_accessibility_settings` (`:412`),
`open_microphone_settings` (`:593`) only have a `#[cfg(target_os = "macos")]`
body that shells out to `open`. On Windows they currently **no-op silently** —
which breaks the donation/"Support Vox" external links and the (Windows-
irrelevant) settings deep-links. Fix `open_external_url` for Windows:
```rust
#[cfg(target_os = "windows")]
{
    // ShellExecuteW "open" verb, or the simplest portable form:
    std::process::Command::new("cmd").args(["/C", "start", "", &url]).spawn()…;
    // (prefer `windows` crate ShellExecuteW to avoid cmd quoting pitfalls)
}
```
`open_microphone_settings` on Windows can open the Settings privacy page via the
`ms-settings:privacy-microphone` URI (`cmd /C start ms-settings:privacy-microphone`);
`open_accessibility_settings` has no Windows analogue — leave it a no-op (the
button doesn't render on Windows). **Recommend the `windows` crate's
`ShellExecuteW`** (feature `Win32_UI_Shell`) over `cmd /C start` to avoid URL
quoting/injection issues; `open_external_url` already validates `http(s)://`
(`:430`).

**8g. Misc mac copy.** `GeneralTab.tsx:135` / `TrayPopover.tsx:1183`
"macOS Sound input" → on Windows "Windows sound input"/"system default input".
Low priority polish; gate on `isMac`.

**APIs/crates (frontend):** none new (UA sniffing). No `@tauri-apps/plugin-os`.

**Edge cases / real-Windows-only:**
- `noUnusedLocals`/`noUnusedParameters` are on (`tsconfig.json`). Any
  `isMac ? … : …` is fine; just don't leave an unused import. The `tsc --noEmit`
  smoke test catches this on the Mac build (TS is platform-agnostic at
  compile-time), so this is safe to author/verify on macOS.
- `navigator.userAgent` in WebView2 is stable for "Windows NT"; verify exact UA
  on real Windows if we want belt-and-suspenders (real-Windows nicety, not a
  blocker — the regex is conservative).

### New/changed files (Area 8)
- **Create** `src/lib/platform.ts`.
- **Modify** `src/lib/hotkey.ts` — Windows symbol/label tables + `isMac`-gated
  shift collapse.
- **Modify** `src/components/HotkeyKeys.tsx` — Windows keycap rendering.
- **Modify** `src/views/settings/tabs/GeneralTab.tsx` — auto-paste + mic copy,
  gate nudge on `!isWindows`.
- **Modify** `src/views/TrayPopover.tsx` — auto-paste + mic copy.
- **Modify** `src/views/onboarding/steps/DownloadStep.tsx` (+ `AppleFmEngineRow.tsx`)
  — hide Apple-Intelligence card on Windows.
- **Modify** `src/views/onboarding/steps/MicStep.tsx` — Windows mic copy.
- **Modify** `src/views/onboarding/steps/HowToUseStep.tsx` — paste-key glyph via
  `isMac`/`HotkeyKeys`.
- **Modify (Rust)** `src-tauri/src/session.rs` — platform-aware Done strings.
- **Modify (Rust)** `src-tauri/src/commands.rs` — Windows `open_external_url`
  (+ optional `open_microphone_settings`).

---

## New files to create (master checklist)

| File | Purpose |
| --- | --- |
| `src-tauri/src/win_paste.rs` | Synthetic Ctrl+V via `SendInput` (mirrors `mac_paste.rs`); no permission gate. |
| `src-tauri/icons/icon.ico` | Windows app/installer icon (generate via `npm run tauri icon`). |
| `src/lib/platform.ts` | `isMac` / `isWindows` UA-based helpers for render-time branching. |

(Plus the regenerated PNG icon set under `src-tauri/icons/` if `tauri icon` is
used — those overwrite existing files rather than add new ones.)

## Existing files to modify (master checklist)

| File | Change |
| --- | --- |
| `src-tauri/Cargo.toml` | Target-split `whisper-rs` (mac: coreml+metal; win: CPU). Add `[target.'cfg(windows)'.dependencies] windows = …`. |
| `src-tauri/tauri.conf.json` | `bundle.targets` → `"all"` (or host-driven); add `icon.ico`; add `bundle.windows.nsis`. |
| `src-tauri/src/lib.rs` | `#[cfg(target_os = "windows")] mod win_paste;`. Gate `inject::prewarm_in_background()` call to macOS. (Reopen/activation blocks already gated.) |
| `src-tauri/src/session.rs` | Three-arm `cfg` for paste (`:1145`); platform-aware Done strings (`:1747/:1754/:1759`). |
| `src-tauri/src/frontmost.rs` | Windows `imp` (GetForegroundWindow + QueryFullProcessImageNameW + GetWindowTextW); narrow the non-mac stub `cfg`. |
| `src-tauri/src/commands.rs` | Windows body for `open_external_url` (ShellExecuteW); optional `open_microphone_settings` (`ms-settings:`). |
| `src-tauri/src/inference_server.rs` | (Optional) add a CPU fallback descriptor; (housekeeping) fix `app.vox.vox` fallback (`:177`). |
| `src-tauri/src/models.rs`, `src-tauri/src/modes.rs` | (Housekeeping, not Windows-specific) fix `app.vox.vox` fallback (`models.rs:289`, `modes.rs:759`). |
| `src/lib/platform.ts` | (new — see above) |
| `src/lib/hotkey.ts` | Windows modifier tables; gate shift-collapse on `isMac`. |
| `src/components/HotkeyKeys.tsx` | Windows keycap rendering (words, no mac glyphs). |
| `src/views/settings/tabs/GeneralTab.tsx` | Auto-paste/mic copy; gate AX nudge on `!isWindows`. |
| `src/views/TrayPopover.tsx` | Auto-paste/mic copy. |
| `src/views/onboarding/steps/DownloadStep.tsx` + `AppleFmEngineRow.tsx` | Hide Apple-Intelligence card on Windows. |
| `src/views/onboarding/steps/MicStep.tsx` | Windows mic copy. |
| `src/views/onboarding/steps/HowToUseStep.tsx` | Paste-key glyph via `isMac`/`HotkeyKeys`. |
| `scripts/release.sh` | Publish Windows NSIS artifacts + Windows entry in `latest.json` (release-infra; verify against current mac-only script). |

---

## Cargo.toml / dependency changes (exact)

```toml
# 1. REMOVE from [dependencies]:
#   whisper-rs = { version = "0.16", features = ["coreml", "metal"], optional = true }

# 2. ADD whisper-rs to the EXISTING macOS target table (alongside objc2 etc.):
[target.'cfg(target_os = "macos")'.dependencies]
# … existing objc2 / block2 / objc2-app-kit / objc2-foundation / objc2-core-foundation …
whisper-rs = { version = "0.16", features = ["coreml", "metal"], optional = true }

# 3. ADD a Windows target table:
[target.'cfg(target_os = "windows")'.dependencies]
# CPU backend for v1 (no GPU feature → no Vulkan SDK build dep, always compiles).
whisper-rs = { version = "0.16", optional = true }
# Win32 FFI for SendInput (auto-paste), foreground-window probe, ShellExecuteW.
windows = { version = "0.58", features = [
  "Win32_Foundation",
  "Win32_UI_Input_KeyboardAndMouse",   # SendInput, INPUT, KEYBDINPUT, VK_*, KEYEVENTF_*
  "Win32_UI_WindowsAndMessaging",      # GetForegroundWindow, GetWindowThreadProcessId, GetWindowTextW
  "Win32_System_Threading",            # OpenProcess, QueryFullProcessImageNameW
  "Win32_UI_Shell",                    # ShellExecuteW (open_external_url)
] }
```

Notes:
- Pin the `windows` crate to whatever major the rest of the ecosystem in
  `Cargo.lock` prefers; `0.58` is a safe recent baseline. (Verify it doesn't
  conflict with any transitive `windows-sys` already pulled by tokio/reqwest —
  `windows` and `windows-sys` coexist fine; they're distinct crates.)
- `local-engines = ["dep:whisper-rs"]` (`Cargo.toml:38`) is unchanged and still
  resolves because `dep:whisper-rs` matches the optional dep regardless of which
  target table declares it. **Verify with `cargo build` on both targets.**
- No change to `command-group`, `sysinfo`, `zip`, `arboard`, `flate2`, `tar`,
  `reqwest` (native-tls→SChannel comment at `Cargo.toml:52` already covers
  Windows), `tauri-plugin-updater` (`windows.installMode` already in conf).
- `[target.'cfg(unix)'.dependencies] xattr` (`Cargo.toml:150`) stays unix-only —
  correct, Windows skips it.
- `[profile.release] panic = "abort"` etc. (`:153`) is platform-agnostic — keep.

---

## Ordering / dependencies between work items

**Phase A — make it compile on Windows (blocking, do first, sequential-ish):**
1. `Cargo.toml` whisper-rs target split + add `windows` crate. *(Unblocks every
   other Rust item; without it the Windows build doesn't link.)*
2. `win_paste.rs` + `lib.rs` module decl + `session.rs` three-arm cfg. *(Depends
   on `windows` crate from step 1.)*
3. `frontmost.rs` Windows `imp`. *(Depends on `windows` crate; independent of
   step 2 — can be done in parallel with 2 by a second engineer.)*
4. `commands.rs` `open_external_url` Windows body. *(Depends on `windows` crate
   for ShellExecuteW; independent of 2/3.)*

**Phase B — frontend (fully parallel with Phase A; only needs the Mac build to
typecheck):**
5. `src/lib/platform.ts` *(prereq for all other FE items)*.
6. `hotkey.ts` + `HotkeyKeys.tsx` *(depends on 5)*.
7. Settings/tray/onboarding copy *(depends on 5; independent of 6)*.
8. `session.rs` Done-string `cfg` *(Rust; independent; pairs with 7 thematically
   but no code dependency)*.

**Phase C — bundling + release (after A compiles, before shipping):**
9. `tauri.conf.json` targets/icons/nsis + `icon.ico`.
10. `scripts/release.sh` Windows artifacts + `latest.json` Windows entry.

**Independent / parallelizable:** 3, 4, and all of Phase B can run alongside 1–2.
The `app.vox.vox` housekeeping fix is independent of everything.

**Hard gate:** **none of A/B can be *verified* on macOS beyond `cargo build`
(mac target stays green), `cargo test --lib`, `tsc --noEmit`, `vite build`.**
The Windows *native* build (whisper-rs-sys CMake, `windows` crate link, NSIS
bundle, real hotkey/paste/tray behavior) requires a Windows machine or CI
runner. Everything in this plan is structured so the macOS smoke test stays
green (all Windows code behind `#[cfg(target_os = "windows")]` / `cfg(windows)`
target tables, all FE behind runtime `isWindows`).

**Suggested verification ladder:**
1. On Mac: `cargo build`, `cargo test --lib`, `tsc --noEmit`, `vite build` —
   must stay green (proves no macOS regression).
2. On Mac: `cargo build --target x86_64-pc-windows-msvc` will **not** fully work
   (native whisper-rs-sys won't cross-compile), but `cargo check
   --target x86_64-pc-windows-msvc` on the pure-Rust crates can catch obvious
   `cfg(windows)` typos for the *non-sys* code. Treat as best-effort lint, not a
   gate.
3. On a Windows runner (GitHub Actions `windows-latest`): full
   `npm run tauri build --bundles nsis`. This is the real gate and the first
   place the whisper-rs-sys CMake build, `windows`-crate link, and NSIS bundle
   are exercised.

---

## Open questions / decisions made

**Decisions made (with rationale):**

1. **Clipboard: keep `arboard`, don't hand-roll Win32 clipboard.** `arboard` is
   already the dep, already used in `inject.rs`, and is cross-platform. Hand-
   rolling `OpenClipboard`/`SetClipboardData` buys nothing and re-introduces the
   "clipboard held by another app" edge cases `arboard` already handles. Only
   the *synthetic keystroke* (Ctrl+V) needs Win32, hence `win_paste.rs`.

2. **Synthetic paste: `windows` crate `SendInput`, not a cross-platform input
   crate.** Matches the task's "prefer the official `windows` crate" steer and
   the existing `mac_paste.rs` hand-rolled-FFI convention. `enigo`/`rdev` would
   add a heavy dep for one keystroke and obscure the no-permission story.

3. **Whisper backend: CPU for v1, not Vulkan.** De-risks the Windows *build*
   (no Vulkan SDK build dependency, no ICD runtime dependency) which is the
   piece we can't test from a Mac. Vulkan is a clean fast-follow (flip one
   `features = ["vulkan"]`). The **Gemma `llama-server` stays Vulkan** because
   it's a downloaded binary (not linked) and already has a descriptor.

4. **Frontend platform detection: UA sniffing in `platform.ts`, not
   `@tauri-apps/plugin-os`.** Synchronous (needed at render time), zero new
   dependency, no new capability in `capabilities/default.json`. The OS plugin's
   `platform()` is async and heavier.

5. **Keep `apple-fm`/`parakeet` in the default feature set on Windows.** They're
   inert by construction (build.rs + module cfg + Registry-arm cfg). A Windows-
   specific feature list would add maintenance surface for no benefit.

6. **Don't add a Windows mic-permission pre-check for v1.** Windows has no TCC-
   style blocking prompt; cpal opens WASAPI directly and the record-time error
   path already surfaces a friendly pill. A registry/WinRT pre-check is
   gold-plating.

**Flagged for human judgment:**

- **A. Vulkan vs CPU/AVX2 llama.cpp Windows asset.** Current descriptor is
  `win-vulkan-x64` (`inference_server.rs:100`). Confirm whether to keep it
  (faster; needs Vulkan ICD) or switch/add `win-cpu`/`win-avx2` (universal;
  slower). Recommendation: keep Vulkan, verify graceful failure on a no-Vulkan
  box, add CPU fallback descriptor as follow-up. **Needs real-Windows testing to
  decide confidently.**
- **B. NSIS install scope + WebView2 bootstrap.** `perMachine` (admin) vs
  `currentUser`, and whether to bundle the WebView2 Evergreen installer
  (`webviewInstallMode`). Distribution/UX call.
- **C. Code signing (Authenticode).** Unsigned `Vox.exe` + downloaded
  `llama-server.exe` trip SmartScreen/Defender. Business decision (cert
  purchase). The minisign updater pin is unaffected.
- **D. `windows` crate version pin.** Confirm `0.58` (or whatever) doesn't
  duplicate-link awkwardly with transitive `windows-sys`. Quick `cargo tree`
  check on the Windows target.
- **E. arm64 Windows.** v1 ships x86_64 only (runs under emulation on arm64).
  Native arm64 (whisper-rs-sys arm64 build + arm64 llama.cpp asset) is a
  follow-up.
- **F. Windows mode-suggestion triggers.** The built-in modes' bundle-id globs
  are mac-centric (`com.apple.*`). Mechanically auto-suggestion works on Windows
  after Area 7, but shipped triggers won't fire. Seeding Windows exe-name
  triggers (`chrome.exe`, `code.exe`, `slack.exe`) in `modes::builtins()` is a
  PM/content decision, separate from the port.
- **G. `app.vox.vox` fallback drift** (`inference_server.rs:177`, `models.rs:289`,
  `modes.rs:759`) vs bundle id `com.vox.app`. Not Windows-specific and never
  fires on the happy path, but worth fixing in the same PR for correctness.
