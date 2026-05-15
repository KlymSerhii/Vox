# Hotkey capture UI v2 — Staff review

## Verdict

**Ship with the noted fixes.** The v2 work is solid: every v1 must-fix is closed, the new wire formats are well-tested (12 new `mac_chord` tests, 11 new `shortcut_from_key` tests), the CGEventTap lifecycle is sound, and the AX deep-link is correct. There are no regressions of v1 behaviours the user described as "basically working." The blocker bar isn't met. There are, however, a handful of must-fix correctness items (M1–M3) that are cheap to land before this ships, plus a couple of meaningful cleanups.

## Verification commands

| Command | Result | Notes |
| --- | --- | --- |
| `cargo build --manifest-path src-tauri/Cargo.toml` | **PASS** (exit 0) | 3.71 s warm. |
| `cargo test --manifest-path src-tauri/Cargo.toml --lib` | **PASS** (114 / 114) | Up from 87 in v1. 27 new tests including `hotkey_roundtrips_through_serde`, all 11 `shortcut_from_key::tests`, and 12 new `mac_chord::tests` covering double-tap state. |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` | **FAIL** with 4 errors | All 4 errors are PRE-EXISTING on `main`. I verified by `git stash`ing the diff and re-running clippy on `main`: identical 4 errors (`build.rs:42/48/49` redundant_closure × 3, `tray_glyphs.rs:369` unnecessary_cast × 1). The engineer's claim is **correct** — v2 introduced zero new clippy errors. The pre-existing errors are out-of-scope for this PR. |
| `npx tsc --noEmit` | **PASS** (exit 0) | Clean. |
| `npx vite build` | **PASS** (built in 2.68 s) | `settings-DOybdQ55.js` is 93.09 kB / 24.11 kB gzipped — up from v1's 91 / 23 kB. Bloat is roughly proportional to the new state machine and parser. Within scope. |

### Pre-existing clippy errors (NOT this PR's responsibility)

```
src/tray_glyphs.rs:369  unnecessary_cast  (f32 -> f32)
build.rs:42             redundant_closure (tray_glyphs::idle)
build.rs:48             redundant_closure (tray_glyphs::error)
build.rs:49             redundant_closure (tray_glyphs::permission)
```

These should be fixed in a follow-up PR. Calling them out separately because future PRs will keep getting "clippy fails on main" reports until the source of truth gets fixed.

---

## Must-fix issues

### M1. `aria-modal="true"` on a non-modal dialog still misleads screen-readers; the capture box has neither focus trap nor screen-reader background suppression

- **Where:** `/Users/serhiiklymenko/Projects/Vox/src/views/Settings.tsx:1754-1762`.
- **What's wrong:** The engineer addressed v1's S8 by flipping `aria-modal="false"` → `aria-modal="true"`. But the runtime is *not* a modal dialog — focus can leave the capture box if the user clicks anywhere else in the Settings window (the `mousedown` listener exits to idle, which is correct), and the rest of the Settings UI is *not* aria-hidden. Screen-reader users with `aria-modal="true"` will be told "you're in a dialog and the rest is suppressed," which is false — they can navigate to other rows but the AT thinks they can't. This is more misleading than v1's `aria-modal="false"`.
- **Why it matters:** Lying to AT users is an accessibility regression. The "use this and clear that one" / "change other shortcut" buttons need consistent semantics so JAWS / VoiceOver can announce them.
- **Fix:** Change `aria-modal="true"` to **drop the attribute entirely** at `Settings.tsx:1757`. Keep `role="dialog"` + the existing `aria-label`. Add `aria-describedby` pointing to the slot 2 verdict span (id it: `<span id={`hotkey-capture-${props.kind}-verdict`} ...>` and reference it). Result: screen-readers announce "dialog, Capture Push-to-talk shortcut" + the live verdict, without falsely promising modal isolation.

### M2. `register_push_to_talk` uses `log::error!` for normal-lifecycle events (`installed via CGEventTap`)

- **Where:** `/Users/serhiiklymenko/Projects/Vox/src-tauri/src/hotkey.rs:550, 584` (install-success branches) and `/Users/serhiiklymenko/Projects/Vox/src-tauri/src/mac_chord.rs:210-211, 355-359, 378, 398, 416, 446, 474-476, 485, 490-492, 503, 727-729, 879-882, 887-891` (every flagsChanged tap event).
- **What's wrong:** The chord install path logs `log::error!` for *successful* installs and even for normal `flagsChanged` raise/release transitions ("mac_chord: raise (flags=…)", "mac_chord: tap enabled; entering CFRunLoopRun"). Per CLAUDE.md, the env_logger default is `info` — `error!` means these will always be in user-facing logs, **including the "noisy 10/sec flagsChanged" log line** that fires every time the user types any modifier key (Cmd+C, Shift, etc.) for the entire app lifetime once a chord is bound. With Cmd+Alt PTT, every shift press while typing fills the log.
- **Why it matters:** Even the throttled "≤10/sec" is 600/min during active typing. Logs become unreadable and the user thinks something is wrong. The throttle was added in the dormant-debug phase; with v2 making chords user-facing, this is now production behaviour.
- **Fix:** Downgrade in `mac_chord.rs`:
  - Line 210, 355, 378, 398, 416, 727 → `log::debug!` (install / teardown lifecycle).
  - Line 446 → `log::warn!` (tap disabled by macOS is genuinely abnormal).
  - Lines 474-476, 485, 490-492, 503-518, 879-882, 887-891 → `log::trace!` (per-event firing detail; throttle stays).
  - In `hotkey.rs`, lines 550, 584 should be `log::info!` (success), 565-566 / 598-599 should stay `log::error!` (genuine failures).

### M3. The 1.5 s AX poll runs while the user is reading or editing on the Hotkeys tab even when no row is in capture — wasted wakeups

- **Where:** `/Users/serhiiklymenko/Projects/Vox/src/views/Settings.tsx:964, 971-973, 985`.
- **What's wrong:** `axPoll = setInterval(..., 1500)` is created in `HotkeysTab.onMount` and only cleared on `HotkeysTab` unmount. Per the PM spec §3c the row is supposed to poll *"while in capturing with a non-empty mod-only set"* — not while the tab is mounted. The engineer flagged this themselves as a deviation. In practice it's bounded (HotkeysTab unmounts when the user switches tabs or closes Settings, verified via `<Show when={tab() === "hotkeys"}>` at `Settings.tsx:265-267`), but while open it costs ~40 IPC calls per minute crossing the Tauri command boundary into `AXIsProcessTrusted` for no behaviour change. On a Mac on battery with Settings → Hotkeys left open, that's measurable.
- **Why it matters:** The deviation is described as "is this fine?" by the engineer. The poll's only purpose is the inline upsell unlock in `evaluate()`. If no row is in capture or capturing with an AX-required spec, polling is pure overhead. Worse: every poll fires `setAxGranted(...)`; with `axGranted` flowing into three `HotkeyRow` instances' `props.axGranted`, every poll runs the `createEffect` at `Settings.tsx:1292-1306` in each row — three effect reruns per 1.5 s with no work to do.
- **Fix:** Lift a `pollingActive()` memo derived from any of the three rows being in `capturing` with `isModsSpec || isTap2Spec`. Gate the interval setup so we only poll while that's true. The simplest version: bubble each row's "capturing-with-ax-spec" state up via a callback (sibling `capturingNeedsAx` signal in `HotkeysTab`), and switch:
  ```ts
  createEffect(() => {
    if (!anyRowNeedsAx()) {
      if (axPoll != null) { clearInterval(axPoll); axPoll = null; }
      return;
    }
    if (axPoll == null) axPoll = setInterval(() => void refreshAx(), 1500);
  });
  ```
  Keep the initial `void refreshAx()` on mount so first-paint has a fresh value; drop the always-on `setInterval`.

---

## Should-fix issues

### S1. `tap2_run_loop_thread` is a near-verbatim clone of `run_loop_thread` — ~80 lines of FFI scaffolding duplicated

- **Where:** `/Users/serhiiklymenko/Projects/Vox/src-tauri/src/mac_chord.rs:352-419` vs `785-834`.
- **What's wrong:** The two functions differ only in (a) the typed `Inner` / `Tap2Inner` pointer cast, (b) which `tap_callback` is registered with `CGEventTapCreate`, (c) which `notify_ready` helper is called. Every other line is identical FFI boilerplate. Same goes for `notify_ready` (l. 421-427) vs `tap2_notify_ready` (l. 836-842).
- **Why it matters:** The duplication invites subtle drift — already visible: `run_loop_thread` is dense with `log::error!` debug calls; `tap2_run_loop_thread` is silent. If a future bug needs the install / teardown logs to debug a tap2 failure, the asymmetry will cost time.
- **Fix:** Extract a generic helper that runs over a `RunLoopHook` trait:
  ```rust
  trait RunLoopHook: 'static {
      fn shared_handles(&self) -> &Arc<TapHandles>;
      fn notify_ready(&self, result: Result<(), ChordError>);
  }
  fn run_loop_thread_generic<T: RunLoopHook>(
      inner_ptr: *mut T,
      callback: CGEventTapCallback,
  ) { ... }
  ```
  Both `Inner` and `Tap2Inner` already carry an `Arc<TapHandles>` and a `Mutex<Option<Sender>>` — the trait is a 10-line refactor. The two `tap_callback` / `tap2_callback` extern functions stay separate because they have different behaviour (the matching state machine differs), but the bootstrap collapses to one function.

### S2. The "Bound to …" Test status in the Test binding button uses the raw wire spec, not `formatHotkey`

- **Where:** `/Users/serhiiklymenko/Projects/Vox/src/views/Settings.tsx:992-996`.
- **What's wrong:** `setTestStatus(`Bound to ${a.bound}`)` shows raw `Bound to Tap2:Alt` to users. The keycap chip in idle correctly uses `formatHotkey(props.diagnostic?.bound)` at `Settings.tsx:1642, 1657` — this is the only place that exposes the wire format.
- **Why it matters:** Inconsistent UX. Users see `[⌥ Opt ×2]` in the chip and `Bound to Tap2:Alt` in the Test status row right next to it. The shared formatter exists; failing to call it is drift.
- **Fix:** Change `Settings.tsx:994` to `setTestStatus(`Bound to ${formatHotkey(a.bound)} (fallback from ${formatHotkey(a.configured)})`)`, and `Settings.tsx:996` to `setTestStatus(`Bound to ${formatHotkey(a.bound)}`)`.

### S3. v1's S5 (three duplicate `mousedown`/`blur` document-level listeners) was not addressed; v2 added a separate AX poll on top

- **Where:** `/Users/serhiiklymenko/Projects/Vox/src/views/Settings.tsx:1320-1327` × 3 row instances + the AX poll at `964-986`.
- **What's wrong:** Each of the three `HotkeyRow` mounts attaches `document.addEventListener("mousedown", …, true)` and `window.addEventListener("blur", …)`. All three fire on every click in the Settings window. The v1 staff review (item S5) flagged this; v2 added the AX poll on top of it. Net: per-click work in Settings is now 3 × mousedown listeners + the AX poll fires on its 1.5 s timer.
- **Why it matters:** Bloat in a hot path. Each listener does `mode().kind === "capturing"` early-out, so the no-op cost is minor — but combined with M3's poll-while-not-needed, the Settings UI does steady wasted work whenever Hotkeys is open.
- **Fix:** Hoist a single mousedown + blur pair to `HotkeysTab`. Track which row's `mode()` is capturing via a shared signal, and dispatch the click-outside / blur logic to that row. Combine with the M3 fix — both M3 and S3 share the same "lift state to the tab" structural change.

### S4. The `configuredDormant` warning at `Settings.tsx:1665-1670` doubles with the existing `failed()` chip + `props.diagnostic?.error` (chord-error-friendly text)

- **Where:** `/Users/serhiiklymenko/Projects/Vox/src/views/Settings.tsx:1645-1670` plus `1917-1930`.
- **What's wrong:** When a `Mods:Cmd+Alt` PTT is saved with AX denied:
  1. `failed()` is true → red `Not bound` chip.
  2. `props.diagnostic?.error` renders the friendly chord error: "Vox can't listen for the chord because Accessibility permission isn't granted…"
  3. `configuredDormant(...)` is true → "Needs Accessibility — modifier-only and double-tap chords are dormant until you grant it."

  That's three pieces of UI saying the same thing. The `configuredDormant` line is redundant — its precondition (`!ax && !diag.bound && (isMods || isTap2)`) is a strict subset of what the friendly error already explains.
- **Why it matters:** Users have to read three messages to understand one fact. The design v2 §11 "Net new visible elements: 0" implicitly forbids this.
- **Fix:** Delete the `<Show when={configuredDormant(...)}>` block at `Settings.tsx:1665-1670` and the `configuredDormant` function at `1917-1930`. The friendly error from `friendly_chord_error` in `hotkey.rs::739-758` already says everything the new line said.

### S5. `Mods:Alt` shows as a captured chip during the first tap of a double-tap

- **Where:** `/Users/serhiiklymenko/Projects/Vox/src/views/Settings.tsx:1488-1498` (keydown lone-mod branch) + `1565-1585` (keyup rewind).
- **What's wrong:** PM spec §4 table row "Tap ⌥, release within 400 ms, set empty" says: "tap tracker armed, **no chip**". The implementation renders `[⌥ Opt]` on keydown, then rewinds to empty on keyup. The user sees the chip flash briefly. The engineer flagged this deviation in their report.
- **Why it matters:** Visually noisy on the first tap before the second tap promotes. Less obvious — for slow users with `prefers-reduced-motion: reduce` (transition collapse) the chip's appear-then-disappear is harsh.
- **Fix:** Don't render the chip on the first lone-modifier keydown. The keydown handler at `Settings.tsx:1488-1498` should set a transient flag `firstTapPending = true` without writing the spec into the snapshot. The chip slot 1 should render only when the snapshot's spec is non-empty *or* a `>1 modifier` chord is held (verified via `e.altKey + e.metaKey + …` count). On `aloneCount === 1` keydown, set verdict TAP1_HINT but leave `snap.spec === ""`. The slot 1 fallback at `Settings.tsx:1766-1773` then renders `—`. Result: PM-spec-compliant "no chip" first-tap state, second-tap detection unchanged.

### S6. `validateHotkeySpec` accepts `Mods:` and `Tap2:` shapes with `ok-needs-ax` even when no modifier was parsed

- **Where:** `/Users/serhiiklymenko/Projects/Vox/src/lib/hotkey.ts:357-369`.
- **What's wrong:** `isModsSpec("Mods:")` returns `true` (prefix matches). The validator catches the empty case at `364-368` and rejects. But `isModsSpec("Mods:Garbage")` also returns `true` because the prefix matches; `parseSpec` returns shape: "mods" with all modifiers `false` because `bucketModifier("Garbage")` returns `null`. The validator then takes the "needs at least one modifier" branch and returns invalid. Correct outcome, but the error message says "Modifier-only chord needs at least one modifier" rather than "Unrecognised modifier token 'Garbage'". A user with a hand-edited settings doc gets a confusing error.
- **Why it matters:** Settings JSON is the only path that can land malformed specs into validate. Edge case but the spec's own example list in PM spec §6 implies the shape is rigorously validated.
- **Fix:** In `parseSpec` at `src/lib/hotkey.ts:260-264` / `268-273`, track whether *every* token was a recognised modifier. If not, set a sentinel (e.g. `out.shape = "empty"` with a side-channel error) or add a `valid: boolean` to `ParsedSpec`. Either way the validator should say "Unrecognised modifier 'Garbage' in Mods: spec" rather than the generic "needs at least one modifier" message.

### S7. `DoubleTapState::step` is missing a test for the case the spec implicitly calls out: tap, hold, lift, tap-again

- **Where:** `/Users/serhiiklymenko/Projects/Vox/src-tauri/src/mac_chord.rs:980-1031`.
- **What's wrong:** Tests cover (a) within-window double-tap fires, (b) outside-window double-tap doesn't fire, (c) other-modifier joining cancels, (d) armed-release. Missing: the user holds the target for an *extended* time, then releases and re-presses within 300 ms — should fire `Press` (it's a tap, not a hold-from-baseline, but the spec says "second tap within window"). Trace through the existing state machine: first press at t=0 → Idle, target_down=true. Release at t=2000 → Idle, first_release_at=Some(2000). Second press at t=2100 → 2100-2000=100ms ≤ 300ms → Press. Should work. Add a test to lock in the behaviour:
  ```rust
  #[test]
  fn double_tap_state_fires_after_long_hold_release() {
      let mut s = DoubleTapState::new(DoubleTapModifier::Option);
      assert_eq!(s.step(FLAG_ALTERNATE, false, 0), DoubleTapAction::Idle);
      // Held for 2 s, then released.
      assert_eq!(s.step(0, false, 2000), DoubleTapAction::Idle);
      // Tap within 300 ms of the release fires Press.
      assert_eq!(s.step(FLAG_ALTERNATE, false, 2100), DoubleTapAction::Press);
  }
  ```
- **Why it matters:** Locks in the spec-defined behaviour against future state-machine refactors.
- **Fix:** Add the test at `mac_chord.rs:1030`.

---

## Nice-to-haves

### N1. The "Try another" button at `Settings.tsx:1692-1704` re-enters capture with an `evaluate(props.configured)` snapshot

- That's exactly the v1 staff review M3 fix. Verified working. No change needed.

### N2. `pickCommon` writes `lastSingleModRelease = null` which is correct — picking `Tap2:Alt` from the suggestion chip is a hand-pick, not a tap sequence

- Verified correct.

### N3. `mac_chord::DoubleTapState::tainted` field handles the "lift target, then someone joins, then lift" case correctly

- Pure-function tests cover the cancel-on-other-modifier case. The `tainted` flag prevents a false first-tap arming after a chord release. Sound.

### N4. The chord lifecycle (drop-old-before-install-new) at `hotkey.rs:306-310` is correct and well-commented

- The comment "Drop any existing chord listener BEFORE installing a new one so two taps never run in parallel after a re-register" is exactly the invariant.

### N5. `tap2`-token rendering in `HotkeyKeys.tsx:30-40` correctly avoids double-rendering the `+` separator before the `×2` suffix

- Verified by reading the `<Show when={i() > 0 && tokens()[i() - 1].kind !== "tap2"}>` guard.

---

## Items checked and clean

The following I verified explicitly; the fix engineer can leave them alone:

1. **v1 M1 (`settings::load` rewriting hotkey)** is fixed at `src-tauri/src/settings.rs:244`. New `hotkey_roundtrips_through_serde` test at `settings.rs:530-545` locks in the round-trip.
2. **v1 M2 (Enter doesn't save in preview)** is fixed at `src/views/Settings.tsx:1420-1425`. Enter is intercepted in `onCaptureKeyDown` before the mode check; works in any state.
3. **v1 M3 ("Try another" missing)** is fixed at `src/views/Settings.tsx:1691-1704`. The button renders inside `<Show when={failed()}>`, calls `enterCapture(evaluate(props.configured))`.
4. **v1 M4 (bare-key specs bind globally)** is fixed at `src-tauri/src/hotkey.rs:130-144` with a defense-in-depth `shortcut_from_key` guard. 11 new tests at `hotkey.rs:894-973` lock in: bare letter rejected, F13/Escape accepted, Mods:/Tap2: refused at the shortcut-plugin path, `CommandOrControl` alias works.
5. **v1 S1 (flash on cleared row)** is fixed via `triggerFlash` at `Settings.tsx:931-938` + `props.flash` propagation to `HotkeyRow:1680, 1686`.
6. **v1 S4 (no Rust tests for `shortcut_from_key`)** is fixed — 11 new tests added.
7. **`isModsSpec` / `isTap2Spec` shape detection** at `src/lib/hotkey.ts:78-85` correctly accepts `Mods:Cmd+Alt` and `Tap2:Alt`. Validator rejects malformed `Tap2:Cmd+Alt` (multi-modifier) per PM spec §9 at `lib/hotkey.ts:347-353`.
8. **Backward compat for legacy `Cmd+Alt+Period`** confirmed — `parseSpec` returns `shape: "single"`, `canonicalSpec` round-trips identically, `formatHotkeyTokens` renders the keycap row unchanged.
9. **`shortcut_from_key` defense rejects `Mods:`/`Tap2:` prefixes** at `hotkey.rs:131-133` — the shortcut plugin can never accidentally try to bind a chord spec. Tests at `hotkey.rs:959-973`.
10. **Dispatch in `register_push_to_talk`** at `hotkey.rs:546-578` correctly routes `Tap2:` to `install_double_tap`, `Mods:` to `install_chord`, and the legacy path stays unchanged.
11. **Cancel / mode-cycle paths** at `hotkey.rs:325-385` go exclusively through `bind_discrete` → `gs.register` / `gs.on_shortcut`. They never touch `mac_chord`. The UI mirrors this: `COMMON_CHORDS` at `Settings.tsx:158-168` flags `Mods:`/`Tap2:` entries with `pttOnly: true` and they only render on the PTT row.
12. **`validateHotkeySpec` rejects `Mods:`/`Tap2:` for cancel/cycle** at `lib/hotkey.ts:341-345` and `lib/hotkey.ts:358-362` with friendly messages.
13. **`mac_chord` lifecycle on re-register** is correct: `hotkey.rs:306-310` drops the existing chord handle BEFORE installing the new one. The drop runs `CFRunLoopStop` + thread.join + CFRelease in the right order at `mac_chord.rs:271-327`.
14. **`DoubleTapState::step` pure-function tests** at `mac_chord.rs:980-1030` cover the spec-mandated cases: within-window fires, outside-window doesn't fire, other-modifier-joining cancels, armed-release works.
15. **`strict_all_or_none = true` for `ChordSpec`** at `mac_chord.rs:547` correctly prevents `Mods:Cmd+Alt` from firing on `Cmd+Alt+M` (a real text shortcut): the flagsChanged event with `flags = Cmd|Alt` matches, but the C base key generates no flagsChanged so the comparison stays exact.
16. **`AXIsProcessTrusted`** (the non-prompting variant) is used everywhere — `commands.rs:546`, `hotkey.rs:316`, `mac_chord.rs:209, 726`. Critically, `AXIsProcessTrustedWithOptions(kAXTrustedCheckOptionPrompt: true)` is NOT called from anywhere — the never-prompt invariant from CLAUDE.md is preserved.
17. **Deep-link URL** at `commands.rs:341` is the canonical `x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility`. Verified opens the right pane.
18. **IPC surface count** — `open_accessibility_settings` already existed pre-v2 (it was added by an earlier change, used by Onboarding + the auto-paste Accessibility nudge in Settings → General). v2 only added the new `axGranted` polling pattern in JS via the existing `check_permissions` IPC. **Zero net new IPC commands.** PM spec §3c's "new `open_accessibility_settings` IPC" is implemented in spirit (the IPC exists and serves the upsell button).
19. **Session-start fail-open** at `hotkey.rs:415-423` correctly returns `Err` to the caller when PTT bind failed, but `lib.rs:170-182` only logs the error and continues — the app boots. The UI surfaces "Not bound" + the friendly error via the diagnostic. Users with revoked AX who saved `Mods:Cmd+Alt` see the upsell, can grant AX, click "Try another" (or just open System Settings and re-save). No crash. Matches PM spec §5 "follow v1's chord-error path."
20. **`HotkeyDiagnostic.mechanism`** is set to `"chord"` for both `install_chord` (`hotkey.rs:557`) and `install_double_tap` (`hotkey.rs:557`). Tray + Settings consumers can tell the difference between chord and shortcut bindings.
21. **Solid idioms** — accessor patterns match the rest of `Settings.tsx`: `() => props.foo` for derived values, `createSignal`/`createEffect`/`createMemo` consistent, `<Show>` / `<For>` over JSX conditionals, `class:` for classlist toggles (via `classList={{ ... }}`). No drift.
22. **Rust FFI safety patterns** match `mac_perm.rs`: `extern "C" { fn AXIsProcessTrusted() -> bool; }`, `unsafe { … }` blocks tightly scoped, raw pointer ownership documented in module comments.
23. **`cancel_hotkey` mid-recording rebind** at `hotkey.rs:430-432` correctly re-arms the cancel grab after `unregister_all()` wiped it. Unchanged from v1.
24. **Conflict resolution** correctly compares both legacy and `Mods:`/`Tap2:` specs via `specsEqual` → `canonicalSpec` round-trip. Cross-row conflict detection at `Settings.tsx:1270` works for all spec shapes.

---

## Out-of-scope changes on this branch

The diff includes substantial **non-hotkey** work:

- `src-tauri/src/audio.rs` (+219 lines): splits `stop_and_encode` into `stop()` + `encode_recorded()` so the cpal stream drops the moment recording ends — independent feature aimed at releasing the macOS mic indicator early.
- `src-tauri/src/session.rs` (+113 lines): calls `audio.stop()` synchronously at `end_recording` / `cancel_session` to amortise the mic-indicator release latency.
- `src-tauri/src/engines/mod.rs` + `src-tauri/src/engines/whisper_local.rs`: doc-comment renames following audio.rs.

These are NOT covered by this review. The audio refactor changes the recording pipeline's hot path — it deserves its own PR and review. The v1 staff review made the same call-out about onboarding / SparkleGlyph drift; the pattern is repeating. **Recommendation: split the audio.rs / session.rs work into a separate commit / PR before this lands.** Until that happens, this branch is doing two unrelated things and the merge log will conflate them.

---

## Summary for the fix engineer

Three blocking changes (M1, M2, M3) plus six should-fixes. All build / test / type-check / build commands pass; clippy fails on 4 errors that are pre-existing on `main` and were correctly identified by the engineer. The audio.rs / session.rs work is out of scope for this review and should be split.

After M1 lands, manually test VoiceOver on Settings → Hotkeys → Change. After M2, run a `Mods:Ctrl+Alt` PTT with `VOX_LOG=info npm run tauri dev` and confirm the log is quiet during normal modifier typing. After M3, leave Settings → Hotkeys open with the system idle for 60 s and confirm the Hotkeys tab makes zero IPC calls (use the dev tools network panel or add a temporary `console.log` to `refreshAx`). The hot path that matters for the user is captured-with-Mods-set — that should still poll.

The headline assessment is that v2 closed every v1 must-fix, added comprehensive tests, and kept the surface area honest. The conditional slot 3 / single capturing state is genuinely cleaner than v1's discriminated-union of preview states. With M1–M3 fixed, this is ready to ship.
