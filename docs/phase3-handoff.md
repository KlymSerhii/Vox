# Vox — Phase 3 Polish Handoff

> Author: Phase-3 polish track (parallel with the local-first STT/LLM
> rewrite and the differentiator-feature track).
> Date: 2026-04-24.
> Branch: `main`. Build verified clean.

This is the coordination doc for the two other Phase-3 agents (the
local-first CTO track that rewrites `stt.rs`/`llm.rs` and the PM
differentiator track). It catalogs every settings key, command, and
TODO this track introduced so the other two don't collide.

---

## 1. Files I created or modified

### Created

- `src-tauri/src/tray_glyphs.rs` — procedurally-generated alpha-only
  glyphs for the tray icon (mic, recording-with-dot, spinner, error,
  lock).
- `src-tauri/src/tray_icon.rs` — public `set_state(app, TrayState)` API
  + the pulse loop driving the recording animation.
- `src-tauri/icons/tray/{idle,recording,recording-off,processing,error,permission}.png`
  + `@2x` variants — written by `build.rs` for design review.
  Runtime path renders directly from `tray_glyphs.rs`; the PNGs are not
  loaded at runtime.
- `docs/signing-and-notarization.md` — full signing / notarization /
  GitHub Actions runbook.
- `docs/phase3-handoff.md` — this file.

### Modified

- `src-tauri/Cargo.toml`
  - Removed `cocoa = "0.26"` and `objc = "0.2"`.
  - Added `objc2 = "0.6"`, `objc2-app-kit = "0.3"`,
    `objc2-foundation = "0.3"`, `objc2-core-foundation = "0.3"` (target
    macOS only).
  - Added `tauri-plugin-autostart = "2"` and
    `tauri-plugin-updater = "2"`.
  - Added `png = "0.17"` to `[build-dependencies]` for tray PNG
    emission.
- `src-tauri/build.rs` — emits the tray PNGs into `icons/tray/`.
- `src-tauri/src/pill.rs` — full rewrite of `apply_macos_panel_style`
  to use `objc2-app-kit` (no more deprecation warnings).
  Added multi-monitor placement: `reposition()` queries
  `NSEvent::mouseLocation()` to find the active screen and converts
  AppKit (bottom-up) coords to Tauri's logical (top-left) frame.
  Falls back to primary monitor on non-mac or when no screen contains
  the cursor. `pill::emit_state` now mirrors pill state into the tray
  icon state machine; `pill::hide` returns the tray to Idle.
- `src-tauri/src/tray.rs`
  - Removed `placeholder_icon()`. Initial icon is sourced from
    `tray_glyphs::idle(32)` via `tray_icon::init`, which then upgrades
    to `Permission` if accessibility/mic isn't granted.
- `src-tauri/src/lib.rs`
  - Registered `tauri-plugin-autostart` and `tauri-plugin-updater`.
  - Registered new commands: `get_launch_at_login`,
    `set_launch_at_login`, `paste_last_transcript`,
    `check_for_updates`.
  - Added `mod tray_glyphs` and `mod tray_icon`.
  - Pass `paste_last_hotkey` into `hotkey::register`.
- `src-tauri/src/commands.rs` — added the four new commands above.
- `src-tauri/src/hotkey.rs` — push-to-talk shortcut now branches on
  `prefs.hands_free_enabled` to support either hold (PTT) or
  press-to-toggle modes. Added paste-last-transcript hotkey. Added
  `Alt+Shift+V` and `Cmd+Shift+V` to `shortcut_from_key`.
- `src-tauri/src/settings.rs` — added settings keys (see §2).
- `src-tauri/tauri.conf.json` — added `plugins.updater` with the
  documented placeholder endpoint.
- `src-tauri/capabilities/default.json` — added `autostart:default`
  and `updater:default`.
- `src/lib/ipc.ts` — typed wrappers for the new commands; extended
  `Settings` interface; added `UpdateCheckResult`.
- `src/views/Settings.tsx` — wired launch-at-login (round-trips
  through the OS), language picker, hotkey-behavior picker
  (PTT / Hands-free toggle), paste-last-transcript hotkey picker, and
  Check-for-updates button in About.

### Did NOT modify (per coordination boundary)

- `src-tauri/src/stt.rs` — reserved for the local-first agent.
- `src-tauri/src/llm.rs` — reserved for the local-first agent.
- The Privacy tab's "Send audio to cloud" toggle — left untouched.
- No new "Engine" / "Model" sub-tabs in Settings.

---

## 2. Settings keys I added

The `Settings` struct (`settings.rs`) gained three new persisted
fields. The `Prefs` struct (the cheap secret-free view used on the
hotkey thread) was extended in lockstep.

| Key | Type | Default | Where it's read |
| --- | --- | --- | --- |
| `paste_last_hotkey` | `String` | `"Alt+Shift+V"` | `hotkey::register` registers a third global shortcut that calls `commands::paste_last_transcript`. |
| `hands_free_enabled` | `bool` | `false` | `hotkey::register`'s shortcut callback branches on this each press: `true` → toggle mode, `false` → PTT (hold). |
| `language` | `String` (BCP-47-ish: `"auto"`, `"en"`, `"es"`, `"fr"`, `"de"`, `"it"`, `"pt"`, `"ja"`, `"zh"`, `"ru"`, `"ar"`) | `"auto"` | **TODO local-first.** Currently UI-only — see §3. |

`launch_at_login` already existed in `Settings` as a stub. It now
mirrors `tauri-plugin-autostart`'s state. The OS is the source of
truth on settings open; the toggle's `set_launch_at_login` call
flips the OS state and updates the cached settings field.

`Prefs` (consumed on the hotkey thread for cancel-cheap reads):

```rust
pub struct Prefs {
    pub cleanup_enabled: bool,
    pub send_audio_to_cloud: bool,
    pub dictation_enabled: bool,
    pub hands_free_enabled: bool,    // new
    pub language: String,            // new (marked #[allow(dead_code)] until local-first wires it)
    pub dictionary: Vec<String>,
}
```

---

## 3. `// TODO local-first` markers I left for the CTO track

Search the tree with:

```bash
rg -n "TODO local-first" src-tauri/ docs/
```

| Location | What needs to happen |
| --- | --- |
| `src-tauri/src/settings.rs:404` (`Prefs.language`) | When the local-first STT engine is wired, read `prefs.language` and pass it to the Whisper/local-Whisper request. The field is already populated and persisted; you just need to consume it. Drop the `#[allow(dead_code)]` once consumed. |
| `src-tauri/src/hotkey.rs` — toggle mode comment | I implemented "press-to-start, press-to-stop" plus the existing 60 s safety cap. Real VAD-based silence auto-stop (PRD §4.1: 30 s of silence in hands-free) is still TODO and naturally lives in your VAD/audio rewrite. |

I deliberately did **not** add a hidden VAD stub — leaving that turf
clean for the local-first track.

---

## 4. New IPC commands (Rust → frontend)

All registered in `lib.rs::generate_handler!`:

- `get_launch_at_login(app) -> Result<bool, String>` — reads the OS
  autostart state.
- `set_launch_at_login(app, enabled: bool) -> Result<(), String>` —
  enables or disables autostart and mirrors to the settings cache.
- `paste_last_transcript(app) -> Result<(), String>` — pulls the latest
  entry from the `transcripts` ring buffer and pastes it via the same
  `inject::paste_text` machinery the main pipeline uses. Errors with
  `"No recent transcript."` if the ring is empty (and surfaces an
  Error pill). Errors with `"Vox is busy with another dictation."` if a
  pipeline is mid-flight.
- `check_for_updates(app) -> UpdateCheckResult` — async, returns
  `{ status, current_version, available_version, message }`. `status`
  is `"available" | "up-to-date" | "error" | "unconfigured"`.

TypeScript wrappers are in `src/lib/ipc.ts` and follow the same
naming.

---

## 5. Tray icon state machine (for consumers)

`tray_icon::set_state(&app, TrayState::*)` accepts:

```
TrayState::Idle          // mic outline (default)
TrayState::Recording     // pulses between mic-with-dot and mic-no-dot @ 2 Hz
TrayState::Processing    // 3/4-circle spinner glyph (transcribing or polishing)
TrayState::Error         // mic + exclamation
TrayState::Permission    // lock glyph (accessibility or mic TCC missing)
```

The pulse loop is reentrancy-safe — calling `set_state(Recording)`
while already recording does **not** spawn a second loop. Transitions
out of `Recording` cause the existing loop to self-terminate within
its next 500 ms tick.

`pill::emit_state` already wires the pipeline states into this map
(Listening → Recording, Transcribing/Polishing → Processing,
Done/Canceled → Idle, Error → Error). If the local-first track adds
new pipeline states, route them through `tray_icon::set_state` rather
than touching the tray crate directly.

---

## 6. Multi-monitor pill

`pill::reposition()` now picks the active display by:

1. macOS: query `NSEvent::mouseLocation()` (bottom-up coords, primary
   origin) → walk `NSScreen::screens()` → find the screen containing
   the cursor → translate to Tauri's logical (top-down, primary-origin)
   frame. Falls back to the primary screen when no screen contains the
   cursor.
2. Cross-platform fallback: walk `available_monitors()` and match
   against `cursor_position()`.
3. Last-resort: primary monitor (the original Phase-2 behavior).

The pill always lands bottom-center (96 px above the bottom edge) of
the chosen screen. Verified visually that the AppKit↔Tauri coordinate
flip is correct: a screen physically above the primary translates to
a *negative* Tauri Y, which is what Tauri expects for "monitors above
the primary."

---

## 7. cocoa → objc2 migration notes

- All `cocoa::*` and `objc::msg_send!` calls in `pill.rs` are gone.
  Window-level constants now come from `objc2_app_kit::NSStatusWindowLevel`,
  collection-behavior bits from `objc2_app_kit::NSWindowCollectionBehavior`,
  and the style mask from `NSWindowStyleMask::Borderless |
  NSWindowStyleMask::NonactivatingPanel`.
- One `msg_send!` survives — `setCanHide:` — because it is not on the
  typed `NSWindow` surface that `objc2-app-kit` exposes. It's routed
  through `objc2`'s own `msg_send!` macro (via the runtime's
  `AnyObject` view), so the lint cleanup is consistent.
- `Cargo.toml` declares `objc2-foundation`, `objc2-app-kit`,
  `objc2-core-foundation` as direct deps with explicit feature
  lists. Adding new selectors? Add the corresponding feature flag.
- Verified zero deprecation warnings: `cargo clippy --all-targets --
  -D warnings` exits 0. The 17 `cocoa` deprecations from the staff
  review are gone.

---

## 8. Auto-update foundation

- Plugin registered with a documented placeholder endpoint
  (`https://updates.vox.local/...`). The pubkey is empty — the actual
  Ed25519 key gets generated per the runbook in
  `docs/signing-and-notarization.md` §6.
- We do **NOT** auto-check at startup. The runbook is explicit that
  signed releases are a prerequisite. Settings → About → Check for
  updates is the only entry point today.
- When `app.updater()` fails (because a real endpoint isn't
  configured), the command returns `status: "unconfigured"` rather than
  blowing up. The UI surfaces the friendly message.

---

## 9. Build status

Verified at the moment of writing:

| Check | Result |
| --- | --- |
| `cargo check --manifest-path src-tauri/Cargo.toml` | clean |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` | clean (zero warnings) |
| `npm run build` (`tsc --noEmit && vite build`) | clean |
| `npm run tauri build -- --bundles app` | OK — bundles `Vox.app` (7.2 MB) |

The 17 cocoa deprecation warnings called out by the staff review are
gone. No new clippy lints introduced.

---

## 10. Per-task status

Per the Phase-3 polish brief:

1. **`cocoa` → `objc2-app-kit` migration (T4)** — **done.** Zero
   deprecation warnings; clippy `-D warnings` clean.
2. **Tray-icon state animation (T1 / I5)** — **done.** Five states +
   pulse loop on Recording. PNGs written to disk by `build.rs` for
   design review; runtime renders procedurally.
3. **Launch-at-login (T2)** — **done.** OS state is the source of
   truth; the settings field is a cache. Reconciled on Settings open.
4. **Multi-monitor pill placement (T9)** — **done.** Active screen
   chosen by cursor location; AppKit↔Tauri coordinate translation
   handled.
5. **Paste-last-transcript hotkey (T3 / 4.4)** — **done.** Default
   `Alt+Shift+V`. Empty-buffer surfaces an Error pill; mid-pipeline
   presses are ignored.
6. **Hands-free dictation (T1 in PRD §4.1)** — **partial.** Toggle
   mode + 60 s safety cap shipped; real VAD-based silence auto-stop is
   left as `// TODO Phase 4` for the local-first agent. UI picker
   added.
7. **Language pinning (T7 / PRD §4.3)** — **partial (intentionally).**
   Setting persisted; UI picker added; `Prefs.language` populated.
   `stt.rs` is reserved for the local-first agent; consuming the value
   is their handoff (see §3).
8. **Auto-update foundation (T11)** — **done.** Plugin wired,
   endpoint stubbed, command + button operational; signed releases are
   the prerequisite documented in `docs/signing-and-notarization.md`.
9. **Signing & notarization runbook** — **done.**
   `docs/signing-and-notarization.md` covers Apple Developer
   prerequisites, `tauri.conf.json` config, hardened-runtime
   entitlements (no changes needed), `notarytool` workflow, stapling,
   GitHub Actions sketch, and the first-run "open anyway" workaround.

---

## 11. Things to coordinate on

- **`stt.rs` rewrite** — please consume `prefs.language` when wiring
  the new STT engine. Drop the `#[allow(dead_code)]` on
  `Prefs.language` once you do. No need to touch the UI; the picker
  already drives it.
- **Hotkey rebind without restart (I2)** — still deferred. The
  Settings UI footer warns the user. If the local-first track adds
  the live re-register path, the existing `hotkey::register` is the
  function to call (just `unregister_all` + re-`register` with the
  new strings).
- **Privacy tab — Send audio to cloud** — left intact. Once the
  local-first STT lands and we have offline mode, this toggle's wiring
  semantics will likely flip ("on" means "use cloud STT instead of
  local"), which is your turf.

---

## 12. Files touched but not added to the Cargo manifest

Just for the search-the-repo crowd:

- `objc2-core-graphics` is *not* a direct dep — we use cursor location
  through `NSEvent::mouseLocation()`, which lives in `objc2-app-kit`.
  Listing this so nobody adds an unused dep "for cursor location" later.
- `image` crate is *not* added — PNG generation in `build.rs` uses
  the `png` crate (smaller, less compile time). Runtime tray drawing
  is RGBA-only and goes straight into `Image::new_owned`.

End of handoff.
