# Hotkey capture UI — Staff review

## Verdict

**Block — needs rework.** The user-pickable push-to-talk row is fundamentally broken in production (every app launch silently overwrites the user's chosen PTT spec back to `Cmd+Alt+Period`), and two design-spec items are missing (Enter to save in preview, OS-refused "Try another"). Cargo and tsc are green, so the regressions are functional, not type-level.

## Verification commands

| Command | Result | Notes |
| --- | --- | --- |
| `cargo build --manifest-path src-tauri/Cargo.toml` | **PASS** (exit 0) | Finished in 0.24 s after warm cache. |
| `cargo test --manifest-path src-tauri/Cargo.toml --lib` | **PASS** (exit 0) | 87 passed, 0 failed. *No new tests were added for `shortcut_from_key` despite the closed match being replaced — see Should-fix #4.* |
| `npx tsc --noEmit` | **PASS** (exit 0) | Clean. |
| `npx vite build` | **PASS** (built in 2.75 s) | `settings-*.js` chunk grew to 91 KB / 23 KB gzipped — non-trivial but in scope. |

Out-of-scope changes also landed on this branch (`src/views/Onboarding.tsx`, `src/views/TrayPopover.tsx`, `src/styles.css`, `src/components/SparkleGlyph.tsx`): a new Tips onboarding step + SparkleGlyph extraction. **These are unrelated to the hotkey capture feature** and not covered by this review. They should be split into their own PR before this lands.

---

## Must-fix issues

### M1. PTT row save does not survive app restart — UI promises user customisation that the backend silently destroys

- **Where:** `src-tauri/src/settings.rs:244-247` (unchanged by this PR) interacts catastrophically with the newly-exposed PTT picker in `src/views/Settings.tsx:978-994`.
- **What's wrong:** `settings::load` runs `let hotkey = default_hotkey(); store.set(KEY_HOTKEY, …)` on every invocation. The comment at `settings.rs:244` explicitly says "Hotkey is currently hardcoded; always write the default so the picker reflects reality even if a stored value drifted." This contract dates from when PTT had no UI picker. The new `HotkeyRow kind="ptt" allowEmpty={false} … onChange={(v) => props.update({ hotkey: v })}` block lets the user save a new spec, and `set_settings` does call `settings::save(&app, &settings)` (which writes `KEY_HOTKEY = settings.hotkey`), but the very next call to `settings::load` (which `set_settings` itself runs at line `commands.rs:75` to compute `before`, plus every IPC handler that reads settings) immediately stomps the disk store back to `Cmd+Alt+Period`. On next app launch, `load` runs again and the user's choice is gone. CLAUDE.md still documents this behaviour: *"Hardcoded — `settings::load` always writes the default into the persisted store on every launch."*
- **Why it matters:** This is the headline feature of the PR. The user clicks Change → records `Cmd+Shift+J` → sees keycaps → clicks Save → it appears to work, dictation runs once with the new chord, then somewhere later in the session a `settings::load` runs (any tray click, mode cycle, polish toggle, etc. that paths through `set_active_mode_id` or similar) and the disk value is back to default. The UI re-reads via `settings:changed` and the picker snaps back. The user has no recourse — there's no error, no warning. This makes the capture UI for PTT a lie.
- **Fix:**
  1. Delete the unconditional stomp at `src-tauri/src/settings.rs:246-247`. Replace with:
     ```rust
     let hotkey = get_str(KEY_HOTKEY).unwrap_or_else(default_hotkey);
     ```
     The `#[serde(default = "default_hotkey")]` attribute already handles missing fields, and the tauri-plugin-store path uses these `get_*` helpers, not serde, so we need the explicit branch.
  2. Update CLAUDE.md to remove the "Hardcoded — `settings::load` always writes the default into the persisted store on every launch" sentence under the `hotkey` row of the defaults table; PTT is now user-pickable.
  3. Add a `cargo test` covering `settings::load` round-tripping a non-default hotkey through the store (it should not get rewritten). The existing settings tests live in `settings::tests` and follow the same pattern as `streaming_pill_enabled_roundtrips_through_serde`.

### M2. Enter does not save in preview state

- **Where:** `src/views/Settings.tsx:1181-1213` (`onCaptureKeyDown`) and the preview JSX at `Settings.tsx:1387-1426`.
- **What's wrong:** Both specs explicitly require Enter to activate the primary action in preview state (PM §3 step 4: *"Save: click Save (or Enter) →"*; design §6: *"On Preview, focus moves to the primary action (`Save` valid, `Re-record` invalid). `Enter` activates."*). The Save button has the HTML `autofocus` attribute (`Settings.tsx:1404`), but `autofocus` only fires on initial document load — Solid does not re-apply it when the button mounts during a state transition. Focus actually stays on `captureBox` (set by `enterRecording`'s `queueMicrotask(() => captureBox?.focus())`), so pressing Enter dispatches a keydown on `captureBox`, which falls into the "non-modifier key" branch at `Settings.tsx:1199-1213`, gets `preventDefault`/`stopPropagation`, and tries to re-capture "Enter" as a new chord. The Save handler never runs. The same is true for Re-record on preview-invalid/conflict.
- **Why it matters:** Keyboard-only flow is one of the named success criteria. Right now the user has to mouse over to the Save button. For a settings tab that's nominally about keyboard shortcuts, that's an embarrassing miss.
- **Fix:**
  1. Inside `onCaptureKeyDown`, branch on `mode().kind` BEFORE the modifier/Tab/Esc logic. When not `"recording"`, treat Enter as commit (call `save()` in `"preview"`, `enterRecording()` in `"preview-invalid"`/`"preview-conflict"`).
  2. Remove the `autofocus` attribute on the Save button (`Settings.tsx:1404`) — it never worked anyway. If you want focus to move to Save when entering preview, do it explicitly: in the keyup commit and in `pickCommon`, after `setMode(evaluate(spec))`, schedule `queueMicrotask(() => primaryActionRef?.focus())` so screen readers and visible focus indicator follow the design spec.

### M3. "Try another" affordance on OS-refused state is missing

- **Where:** `src/views/Settings.tsx:1271-1290`. The diagnostic chip and inline error text render, but no button.
- **What's wrong:** PM spec §5b says "Add a Try another button that re-opens capture pre-populated with the failing spec." Design spec §1.F repeats this. The implementation surfaces `Not bound` + `props.diagnostic?.error` but offers no path forward — the user has to spot the (disabled-looking) `Reset` button, click Change, and re-record. That's not a recovery; it's a punishment.
- **Why it matters:** "Conflicts (cross-row, OS-in-use) explained inline within 1 s of save" is in PM §9 success criteria. The OS-refusal branch (e.g., F13 on a keyboard without function-row hardware, or a chord another global app already grabbed via Carbon) was the explicit reason to add the button.
- **Fix:** Inside the `<Show when={failed()}>` block in the right column, render a `Try another` button next to or below the Change/Reset row. Its handler should set the row into recording mode AND pre-populate `pending` with `props.configured` so the previous spec re-renders as a keycap preview the user can re-record over. Wire-up sketch:
  ```tsx
  <Show when={failed()}>
    <button onClick={() => {
      // Re-enter capture but show the failing spec as a preview so
      // the user can decide whether to keep, modify, or replace it.
      setMode(evaluate(props.configured));
      queueMicrotask(() => captureBox?.focus());
    }}>
      Try another
    </button>
  </Show>
  ```
  The display chip can still read "Not bound"; the action sits beside it.

### M4. Bare-key specs from outside the capture UI now bind globally

- **Where:** `src-tauri/src/hotkey.rs:122-124`. The old closed match returned `None` for anything outside the eight permitted ids; the new `Shortcut::from_str` accepts `"M"`, `"A"`, `"Period"`, `"Slash"`, etc. as zero-modifier global hotkeys.
- **What's wrong:** The JS `validateHotkeySpec` is now the only thing keeping bare letters out of `KEY_HOTKEY`, `KEY_CANCEL_HOTKEY`, `KEY_MODE_CYCLE_HOTKEY`. That's fine for the capture path, but:
  - A user who hand-edits `~/Library/Application Support/com.vox.app/vox-settings.json` to set `"hotkey": "M"` now binds bare `M` globally on launch. Carbon `RegisterEventHotKey` will happily install this, and the user's `m` keystroke is captured everywhere. Before this PR, `shortcut_from_key("M")` returned `None` and the bind walk surfaced the error.
  - More worryingly, the persisted-fallback path at `settings::set_hotkey_field` (called from `bind_discrete` when a fallback succeeds) can now write a fallback spec the JS validator never saw, e.g. someone could ship a new fallback array later that contains `"X"` and the backend would happily register it.
- **Why it matters:** Loss of the "the backend is the safety net" property. The PM spec §4 lists "Bare letters would swallow typing globally" as the FIRST reason for the modifier-required rule. Right now that rule is enforced only at the UI.
- **Fix:** Keep `Shortcut::from_str` (the engineer's deviation is correct in intent — open the vocabulary), but add a single-spot guard in `shortcut_from_key` that rejects modifier-less specs unless the key is a function key or Escape, mirroring `validateHotkeySpec`. Concretely:
  ```rust
  fn shortcut_from_key(id: &str) -> Option<Shortcut> {
      let sc = Shortcut::from_str(id).ok()?;
      // Defense in depth: refuse bare letters/digits/punctuation. The
      // capture UI already validates, but hand-edited settings docs or
      // future fallback arrays must not be able to bind a bare key
      // globally (PM spec §4).
      if sc.mods.is_empty() {
          use tauri_plugin_global_shortcut::Code;
          let safe_alone = matches!(
              sc.key,
              Code::F1 | Code::F2 | Code::F3 | Code::F4 | Code::F5 | Code::F6
              | Code::F7 | Code::F8 | Code::F9 | Code::F10 | Code::F11 | Code::F12
              | Code::F13 | Code::F14 | Code::F15 | Code::F16 | Code::F17 | Code::F18
              | Code::F19 | Code::F20 | Code::Escape
          );
          if !safe_alone {
              return None;
          }
      }
      Some(sc)
  }
  ```
  Add a `#[cfg(test)] mod tests { #[test] fn rejects_bare_letter() … #[test] fn accepts_f13() … #[test] fn rejects_shift_only() }` in `hotkey.rs` next to it.

---

## Should-fix issues

### S1. Conflict resolution does not flash the cleared row

- **Where:** `src/views/Settings.tsx:1491-1500` ("Use this and clear that one").
- **What's wrong:** Design spec §5 explicitly calls out: *"The cleared row flashes its keycap with `animate-flash-done`."* The implementation only writes both settings fields and exits to idle. There's no flash signal.
- **Why it matters:** Without the flash, the user sees the conflict UI vanish but doesn't immediately notice the *other* row changed. They re-read the affected row to confirm — slow, easy to overlook.
- **Fix:** Add a `flashRow` signal in `HotkeysTab` keyed by `HotkeyKind`. After `onResolveConflict` writes both fields, set `flashRow` to the cleared row's kind for ~200 ms. Each `HotkeyRow` reads its kind's flash state and applies `animate-flash-done` to the keycap chip (`Settings.tsx:1303`'s `<HotkeyKeys spec={props.configured} />`, wrap in a `<span class:animate-flash-done={...}>`). Class lookup table is already in `styles.css`.

### S2. `tokenFromCode` has dead branches for Tab and CapsLock

- **Where:** `src/views/Settings.tsx:1617` and `1625`.
- **What's wrong:**
  - `case "Tab": return code;` is dead because `onCaptureKeyDown:1184-1188` intercepts Tab and returns before reaching `specFromKeyEvent`.
  - `case "CapsLock": return code;` is dead because `isModifierCode:1588` classifies CapsLock as a modifier and the handler returns early at `:1199-1202`.
- **Why it matters:** Tiny but misleading — a future reader looks at `tokenFromCode` and assumes Tab is a valid binding, then chases the contradiction.
- **Fix:** Delete both case arms. Add a one-line comment in the `default` arm: `// Tab + CapsLock are intentionally unreachable here (Tab is preventDefault'd before specFromKeyEvent, CapsLock is treated as a modifier).`

### S3. `validateHotkeySpec` re-parses modifiers from scratch instead of reusing `canonicalSpec`

- **Where:** `src/views/Settings.tsx:1648-1703` vs `1532-1564`.
- **What's wrong:** Both functions iterate `spec.split("+")`, normalise tokens to upper-case, and bucket into `cmd/ctrl/alt/shift/key`. `canonicalSpec` carries a few more aliases (`SUPER`, `META`, `CMDORCTRL`, etc.) that `validateHotkeySpec` lacks. If a future spec arrives via the disk file with `Super+J`, `validateHotkeySpec` would treat `Super` as `key = "Super"`, set `cmd = false`, and return `"Add Cmd, Opt, Ctrl, or Shift to a regular key."` — wrong.
- **Why it matters:** Two parsers, two truth tables, guaranteed drift.
- **Fix:** Refactor `canonicalSpec` to return both the canonical string AND the parsed modifier/key buckets (or extract a shared `parseSpec(spec): { cmd, ctrl, alt, shift, key }` helper used by both). The validator then becomes a thin check on the parsed struct. Net code shrinks ~40 lines.

### S4. No Rust tests cover the new `shortcut_from_key` behaviour

- **Where:** `src-tauri/src/hotkey.rs:122-124`.
- **What's wrong:** The previous closed match was tested implicitly via `try_chain` behaviour, but the broader vocabulary (any letter, digit, punctuation, F1–F24) has zero coverage. If `global_hotkey::HotKey::from_str` ever regresses on `Cmd+Shift+J` parsing during a dep bump, we won't catch it.
- **Why it matters:** A bump of `tauri-plugin-global-shortcut` (or its `global-hotkey` subdep) on a future Tauri major could silently break previously-saved user customisations. CI would still be green.
- **Fix:** Add unit tests:
  ```rust
  #[cfg(test)] mod tests {
      use super::*;
      #[test] fn parses_default_ptt() { assert!(shortcut_from_key("Cmd+Alt+Period").is_some()); }
      #[test] fn parses_cmd_shift_letter() { assert!(shortcut_from_key("Cmd+Shift+J").is_some()); }
      #[test] fn parses_function_key() { assert!(shortcut_from_key("F13").is_some()); }
      #[test] fn parses_escape() { assert!(shortcut_from_key("Escape").is_some()); }
      #[test] fn rejects_bare_letter_after_M4_fix() { assert!(shortcut_from_key("M").is_none()); }
      #[test] fn rejects_garbage() { assert!(shortcut_from_key("NotARealKey").is_none()); }
      #[test] fn parses_commandorcontrol_alias() {
          assert_eq!(shortcut_from_key("Cmd+Alt+Period"), shortcut_from_key("CommandOrControl+Alt+Period"));
      }
  }
  ```
  The last one is important because the JS `specsEqual` treats them as equal — the Rust parser must agree.

### S5. Three duplicate `mousedown`/`blur` listeners installed at the document level

- **Where:** `src/views/Settings.tsx:1160-1167`.
- **What's wrong:** Every `HotkeyRow` mounts adds its own `document.addEventListener("mousedown", onDocPointer, true)` and `window.addEventListener("blur", onWinBlur)`. Three rows = three listeners, each firing on every click in the Settings window. Each listener does a `mode().kind === "idle"` guard, so the no-op cost is fine, but it's still wasteful and the cleanup order on tab switch / unmount is fragile (each row has its own state — only one is in capture at a time, but listeners 2 and 3 walk three reactive signals on every click).
- **Why it matters:** Bloat in a hot path. Tab switches in Settings happen often; mousedown global listeners are common to debug.
- **Fix:** Hoist the listener to `HotkeysTab` and dispatch by reading which row's `mode()` is non-idle, or use a single shared "currently capturing" signal at the tab level (rows publish their capture state to it on enter/exit). The latter also makes click-outside semantics consistent (no row's listener fires when another row is in capture).

### S6. `<Show when={mode().kind === "preview"}>` + IIFE-with-cast pattern repeats 3x

- **Where:** `Settings.tsx:1387-1426`, `1428-1459`, `1461-1516`.
- **What's wrong:** Three nearly identical render blocks gated by string-discriminator checks, each opening an IIFE with `as Extract<CaptureMode, { kind: "…" }>` to coerce the type. The body of each is 80% the same (HotkeyKeys + verdict + Re-record/Cancel buttons), differing only in the verdict colour, the action set, and the conflict-only resolution buttons.
- **Why it matters:** ~130 lines of duplicated JSX that drifts if anyone updates spacing / button class.
- **Fix:** Extract a `<PreviewBody spec={m.spec} verdict={…} verdictTone="ok"|"danger" primary={…} resolution={…} onCancel={…}>` component. Or use Solid's `<Show keyed when={previewMode()}>` pattern with a typed accessor:
  ```tsx
  const previewMode = createMemo(() => {
    const m = mode();
    return m.kind === "preview" || m.kind === "preview-invalid" || m.kind === "preview-conflict" ? m : null;
  });
  <Show when={previewMode()}>{(m) => <PreviewBody mode={m()} … />}</Show>
  ```
  Loses some discriminator narrowing but pays for itself in cohesion.

### S7. PTT row's `extra` slot decouples Test binding rendering from row layout

- **Where:** `Settings.tsx:997-1021`.
- **What's wrong:** The `extra` prop is only used by PTT, yet `HotkeyRow` accepts it generically. The decoupling makes the row component slightly more flexible but also exposes a hole in the design spec (§7 says "Test binding stays on the PTT row only"). With a single use site, it'd be clearer to inline the Test button into a `<Show when={props.kind === "ptt"}>` block inside `HotkeyRow`, or pull the PTT row's right-column layout into a sibling structure rather than threading it via prop.
- **Why it matters:** Mild — but it makes the row signature visually heavier than it needs to be.
- **Fix:** Either drop `extra` and inline `<Show when={props.kind === "ptt"}>` inside the row, or extract a separate `<PttRow>` component that composes `<HotkeyRow>` and the test-binding cluster. Light preference for the latter — `HotkeyRow` becomes a pure-presentational component, all "is this PTT" specialness lives in one place.

### S8. `aria-modal="false"` paired with `role="dialog"` is non-standard

- **Where:** `Settings.tsx:1340-1341`.
- **What's wrong:** `aria-modal="false"` on a `role="dialog"` is technically valid but signals "this dialog does not trap focus". The implementation DOES intercept Tab (and the spec calls for trapped focus). Set `aria-modal="true"` to match runtime behaviour, or drop the attribute entirely (the default is `false`).
- **Why it matters:** Screen-reader users may be told "dialog" but not have screen-reader focus reset to the dialog's first focusable element.
- **Fix:** Change `aria-modal="false"` to `aria-modal="true"`. Optionally set `aria-describedby="<id of recording prompt span>"` so the "Press the shortcut…" line is announced on entry.

---

## Nice-to-haves

### N1. Conflicting common-chord chips render as primary-coloured even when they'd collide

- The "Or pick:" strip shows `Alt+M` to the PTT row even when cycle is bound to `Alt+M` (the default). Clicking it lands in conflict-state, which is correct but anticlimactic. Optional polish: dim/strike the chip if `props.others.find(o => specsEqual(o.spec, c.spec))` is truthy.
- Deferrable because conflict resolution still works correctly; this is hint-quality UX.

### N2. The 50 ms repeat debounce isn't actually doing anything

- macOS Key Repeat default initial delay is 250 ms, repeat interval ~30 ms. The first auto-repeat fires well past the 50 ms window, and `pending.spec` is identical across firings anyway because modifier state hasn't changed. The PM spec asks for it, so leave it in for safety, but the comment at `Settings.tsx:1205-1209` could be honest about that: it's defensive, not load-bearing.
- Deferrable because there's no harm.

### N3. `canonicalSpec` does case-insensitive comparisons via `toUpperCase()` but the parser is `.to_uppercase().as_str()` Rust-side

- Both work, just noting that they're equivalent. No change.

### N4. The `extra` prop on `HotkeyRow` renders after the capture panel

- When the capture panel is open, the Test binding button hangs awkwardly below it (`Settings.tsx:1517-1519`). Per the design spec wireframe, Test should disappear during capture and reappear in idle. Optional: gate `{props.extra}` on `mode().kind === "idle"`.

### N5. `lib.rs::run` and CLAUDE.md description of "PTT … hardcoded to Cmd+Alt+Period" need an update

- CLAUDE.md says: *"`hotkey` | `Cmd+Alt+Period` (⌘⌥.) | Hardcoded — `settings::load` always writes the default into the persisted store on every launch."* After M1 is fixed, update this row to "User-pickable in Settings → Hotkeys; defaults to `Cmd+Alt+Period`."
- Defer until M1 lands.

---

## Items checked and clean

The following I verified explicitly — the fix engineer can leave them alone:

1. **`set_settings` re-bind path** (`src-tauri/src/commands.rs:64-139`). Hotkey changes correctly trigger `hotkey::register` and the `settings:changed` event fires. No new IPC was added. The webview's `update` → `setSettings` → backend `set_settings` chain is intact.
2. **Settings schema** (`src-tauri/src/settings.rs:101-198`). No struct fields added or removed. Per-field `#[serde(default)]` is in place — an older on-disk doc that's missing the new fields still loads. (The PR doesn't add new fields anyway.)
3. **`prefers-reduced-motion` coverage** (`src/styles.css:440-462`). `animate-pulse-mic` and `animate-pill-in` are both in the suppression list. `scroll-behavior: auto !important` overrides the `scrollIntoView({ behavior: "smooth" })` call.
4. **`HotkeyDiagnostic` type alignment** (`src/lib/ipc.ts:728-738`). The TS type still matches the Rust `ActiveHotkey` serde Output — `purpose: string` (`commands.rs:1143`), `bound`, `fell_back`, `mechanism`, `error`, `error_chain` all present.
5. **`onSettingsChanged` subscription in HotkeysTab** (`Settings.tsx:917-930`). Still subscribes once on mount, cleanups on unmount, calls `refreshDiagnostics` on event — matches the cross-window broadcast contract in CLAUDE.md.
6. **`onMount` / `onCleanup` symmetry for capture listeners** (`Settings.tsx:1160-1167`). Mousedown listener uses `capture: true` and unsubscribes on cleanup; window blur unsubscribes on cleanup. No leaks.
7. **`Shortcut::from_str` round-trips the previous closed set** — verified against `global-hotkey-0.7.0/src/hotkey.rs::parse_hotkey`:
   - `"Cmd+Alt+Period"` → `Modifiers::SUPER|ALT, Code::Period`. ✓
   - `"Alt+M"` → `Modifiers::ALT, Code::KeyM`. ✓
   - `"Alt+Shift+M"` → `Modifiers::ALT|SHIFT, Code::KeyM`. ✓
   - `"CommandOrControl+Shift+M"` → `Modifiers::SUPER|SHIFT, Code::KeyM`. ✓ (parser explicitly handles `COMMANDORCONTROL` → `SUPER` on macOS at `hotkey.rs:212`).
   - `"F13"`/`"F14"`/`"F15"` → matching `Code::F1x`. ✓
   - `"Escape"`/`"Esc"` → `Code::Escape`. ✓
   - All previously-supported specs continue to bind; no regressions in the closed set.
8. **Conflict cross-row gating** (`Settings.tsx:953-960`). `otherSpecs` filters out empty specs (so an `""`-disabled row doesn't claim a conflict against any captured spec) and filters out self.
9. **No new hex colours in Settings.tsx hotkey block.** All colours go through `var(--color-…)` tokens.
10. **`Reset` button disabled state** (`Settings.tsx:1317`). Correctly uses `specsEqual` (Cmd vs Command vs CommandOrControl alias handling).
11. **Reduced-motion `scrollIntoView`** is collapsed by the global `scroll-behavior: auto !important` rule.
12. **The `commands::set_settings` `before` snapshot** still detects hotkey changes correctly via `before.hotkey != settings.hotkey` (`commands.rs:76-78`), so `hotkey::register` is invoked on every save.

---

## Summary for the fix engineer

Five blocking changes (M1–M4 + S4) plus four medium-priority cleanups (S1–S3, S5). All build/lint/test commands pass — the rework is functional, not type-level. Total estimated effort: ~3 hours of focused work. After M1 lands, hand-test by setting PTT to `Cmd+Shift+J`, quitting Vox, relaunching, and verifying the chord still binds (and the picker still shows `Cmd+Shift+J`). After M2, hand-test Enter from preview/preview-invalid/preview-conflict. After M4, drop `{"hotkey": "M"}` into the on-disk settings JSON, launch, and confirm `hotkey::register` rejects it instead of binding bare M.
