# Hotkey capture UI (v1) — Product Spec

## 1. Problem
Users want to bind any chord for push-to-talk, cancel, and mode-cycle. Today the Hotkeys tab uses a 4–7 item `<select>` (`Settings.tsx:143-161`), and PTT isn't even pickable — it's hardcoded to ⌘⌥. (`Settings.tsx:971`). The fix: a "press the keys you want" capture UI on every row.

## 2. Scope
**In:** Capture UI for all three rows — push-to-talk (newly user-pickable), `cancel_hotkey`, `mode_cycle_hotkey`. **Out:** modifier-only chord PTT (`mac_chord.rs` stays dormant); per-mode hotkeys; multiple bindings per action; mid-pipeline switching; string import. Accessibility-gated chords are excluded from v1 capture output (§4).

## 3. User flow
Each row replaces the `<select>` with a **keycap chip + buttons**.

1. **Idle:** chip shows current binding via `<HotkeyKeys spec={configured} />`. Buttons: **Change**, **Reset**. Fallback "Bound to …" pill stays.
2. **Capturing:** click Change (or Space/Enter on focused row). Chip becomes a focus-trapped panel: "Press the shortcut you want… *Esc cancels.*" A global `keydown`/`keyup` listener (`capture: true`, `preventDefault`) records the first non-modifier press while tracking modifier state.
3. **Preview:** valid combo (§4) renders as keycaps with a verdict ("⌘⇧K — looks good" / "Conflicts with cancel"). Buttons: **Save**, **Re-record**, **Cancel**.
4. **Save:** click Save (or Enter) → existing `setSettings({ hotkey | cancel_hotkey | mode_cycle_hotkey: spec })`. Backend re-registers, `settings:changed` refreshes diagnostic chips.
5. **Cancel:** Esc, click outside, Cancel button, or blur exits without saving.

Keyboard-only: Tab cycles rows; Space/Enter enters capture; Esc exits; Tab is **disabled while capturing** (otherwise Tab+Shift becomes a binding nobody wants).

## 4. Capture rules
A binding is accepted iff it serialises to a Tauri spec the existing `shortcut_from_key` in `hotkey.rs:116` (or its v1.1 expansion) can resolve:

- **At least one modifier** (Cmd / Ctrl / Alt / Shift) **plus** a non-modifier — **OR** a single function key (F1–F20). Bare letters would swallow typing globally; modifier-only chords need Accessibility and are out of scope.
- **Shift-only** combos rejected (collide with typing).
- **Escape** is reserved unless the row IS `cancel_hotkey`, where it is the default and explicitly allowed. Elsewhere, Esc means "cancel capture."
- We read **physical `event.code`**, not `event.key` — matches Tauri's `Code::KeyM` model, sidesteps international layouts and dead keys.
- Emitted spec uses the same vocabulary the backend already accepts (`CommandOrControl+Alt+Period`, etc.). Specs the backend can't yet parse are rejected at preview with "Not supported"; `shortcut_from_key` expands in lockstep with the capture vocabulary.

## 5. Validation & conflict handling
Client-side before save:

a. **Cross-row collision** — captured spec equals either *other* row's value → block save with "Already used by `<row>` — pick another." Mirrors backend checks at `hotkey.rs:298,338`.
b. **OS registration failure** — backend `register()` runs on save; the existing "Not bound" chip + `error` text already renders. Add a **Try another** button that re-opens capture pre-populated with the failing spec.
c. **Accessibility missing** — unreachable in v1 because §4 excludes Accessibility-gated bindings.

## 6. Defaults & "Common chords"
**Replace the dropdown entirely.** A dropdown beside a capture button is two ways to do the same thing — bad UX. Every row gets: keycap chip + **Change** + **Reset** (writes the row's hardcoded default: ⌘⌥. / Escape / ⌥M).

For discoverability, the capture panel shows a small "Or pick a common shortcut" list (F13, ⌥M, ⌥⇧M, ⌘⇧M, plus Esc for the cancel row only) the user can click. Same data as today's option lists, but presented as suggestions *inside* capture rather than a parallel path.

## 7. Edge cases
- **Escape during capture** always cancels. To bind Escape *as* a hotkey, the row must be `cancel_hotkey` and the user clicks the explicit "Use Escape" chip — no key-press path.
- **Sticky modifiers:** track only physically-held modifiers at the moment the non-modifier fires; release-then-press resets state.
- **Dead keys / IME:** bypassed because we read `event.code`.
- **Repeat keydowns:** first non-modifier wins; 50 ms debounce.
- **Window blur:** auto-cancels (focus loss = ambiguous state).

## 8. Persistence & live-rebind
No new IPC. Capture writes through existing `setSettings` → `hotkey::register` → `settings:changed` (`hotkey.rs:259`). `HotkeysTab` already subscribes via `onSettingsChanged` (`Settings.tsx:928`); fallback chips and **Test binding** keep working unchanged.

## 9. Success criteria
- All three rows accept any valid combo (e.g. ⌘⇧J) without source changes.
- Bindings survive restart and live-rebind without restarting Vox.
- Conflicts (cross-row, OS-in-use) explained inline within 1 s of save.
- Onboarding unaffected — capture is additive; default specs ship unchanged.
- Zero new Accessibility prompts in v1 (PTT default stays `Cmd+Alt+Period` via Carbon).

## 10. Out of scope (v1)
- Modifier-only chord PTT (e.g. bare ⌥).
- Per-mode hotkeys.
- Multiple bindings per action.
- Importing a spec from a typed string.
- Customising the cancel-hotkey session-only arming window.
