# Hotkey capture UI — Design Spec (v1)

Implements `.tmp/hotkey-pm-spec.md` inside the Hotkeys tab. Three rows: Push-to-talk (newly user-pickable), Cancel, Cycle voice mode. The `<select>` at `Settings.tsx:1099-1112` is replaced by a keycap chip + capture flow on the same row. **Right column only**; left column (label, hint, fallback chips) unchanged. Row grows downward in recording/preview; column rhythm preserved.

## 1. Row anatomy — six states (right column only)

- **A. Idle** — `<HotkeyKeys spec={configured} />` + `Change` + `Reset`.
- **B. Recording** — accent-rimmed glass panel: `Press the shortcut you want… Esc cancels.` Below it: `Or pick:` row of clickable suggestion chips (`[Esc] [F13] [⌥M] [⌥⇧M] [⌘⇧M]`; `[Esc]` only on the cancel row). No `Change`/`Reset`.
- **C. Preview valid** — keycaps + `Save` / `Re-record` / `Cancel`. Verdict `Looks good.`
- **D. Preview invalid** — keycaps + `Re-record` / `Cancel`. Red verdict. `Save` hidden.
- **E. Conflict** — keycaps + `Re-record` / `Cancel`. Red verdict + two stacked resolution buttons (§5). `Save` hidden.
- **F. OS-refused (post-save)** — idle layout; left column gains `Not bound` chip, right column adds `Try another` that re-opens capture pre-populated with the failing spec.

Reset is visible in idle only. One-click — no two-step confirm; the row is one click from being recapturable. Reset writes the hardcoded default (⌘⌥. / Esc / ⌥M).

## 2. Recording surface — inline row expansion

**Choice: inline expansion.** No modal, no popover. The row grows by ~32 px. Capture is per-row; a modal would lift focus off the control and feel heavy for "press one chord," and it keeps the keyboard flow simple (no focus-trap-and-restore beyond the row).

The expanded block is a `liquid-glass` inner card: `var(--color-surface-glass)` background, `border: 1px solid var(--glass-rim)`, `rounded-[10px]`, `box-shadow: var(--glass-shadow-stack-sm)`. Soft-glass subordinates without dimming the rest of the tab.

## 3. Live capture feedback

**Commit on key-up of the first non-modifier**, with the spec's 50 ms repeat-debounce. While only modifiers are held, the panel reads `Press the shortcut you want…` with a subtle 1.2 s opacity breathe (reuse `animate-pulse-mic`). **Do not render keycaps for partial chords** — live modifier chips invite users to "lift a finger to check," which resets the tracker and flickers. Once the non-modifier resolves, transition to Preview rendering keycaps via `<HotkeyKeys spec={captured} />`. Reuse this component exclusively — no new chip primitive.

## 4. Microcopy (match the pill voice: tight, instructional)

| Surface | Copy |
|---|---|
| Recording prompt | `Press the shortcut you want… Esc cancels.` |
| Preview, valid | `Looks good.` |
| Shift-only key | `Needs a regular key, not Shift on its own.` |
| Bare letter | `Add Cmd, Opt, Ctrl, or Shift to a regular key.` |
| Escape on non-cancel row | `Esc is reserved for cancelling capture. Pick another.` |
| Unsupported by backend | `Not supported yet. Try another chord.` |
| Cross-row conflict | `Already used by <Other row>.` |
| OS registration failed | `<spec> couldn't be bound on this Mac.` (existing fallback chip + new `Try another`) |

Verdicts sit under the chip, `text-[11px]`, `text-[var(--color-ink-mute)]` (valid) or `text-[var(--color-danger)]` (invalid/conflict). The cancel row exposes a clickable `[Use Escape]` chip; the key-press path never accepts Escape. Voice matches the pill ("Copied — press ⌘V to paste."): lowercase, no marketing fluff, no exclamation points.

## 5. Conflict resolution — explicit pick, no auto-clear

Cross-row collision renders the verdict + **two stacked buttons**:

- **Use this and clear that one** — primary (`bg-[var(--color-accent)] text-[var(--color-on-accent)]`). Writes the new spec to this row AND the row's hardcoded default (or `""` for Cancel/Cycle) to the conflicting row. The cleared row flashes its keycap with `animate-flash-done`.
- **Change other shortcut** — secondary (`bg-[var(--color-control)]`). Closes this capture, scrolls the conflicting row into view, auto-opens its capture panel.

`Save` is **hidden** in conflict state. The user must resolve — auto-clearing would silently strip a customised binding; explicit choice is right for a settings surface.

## 6. Keyboard-only flow (a11y)

Tab order: `Change` → `Reset` → next row's `Change`. The keycap chip is decorative (not focusable). `Space`/`Enter` on `Change` enters Recording. While recording:

- Capture div: `role="dialog" aria-modal="false" aria-label="Capture <row> shortcut"`; receives focus on entry.
- **`Tab` is `preventDefault`'d** so the user can't ratchet Tab+Shift into a binding.
- **`Esc`** exits and restores focus to `Change`. Listener uses `capture: true` + `preventDefault` so Esc never reaches the OS hotkey system or Settings' outer Esc-to-close.
- Window `blur` cancels and restores focus to `Change`.
- On Preview, focus moves to the primary action (`Save` valid, `Re-record` invalid). `Enter` activates.

`aria-live="polite"` on the verdict line so screen readers hear "Looks good." / "Already used by Cycle voice mode."

## 7. Reset + Test affordance

- **Reset** in every row (next to `Change` in idle). PTT had no Reset because it was hardcoded — that disappears as PTT becomes pickable.
- **Test binding** stays on the PTT row only. **Not** added to Cancel/Cycle: those already show bound state via `Bound to …` / `Not bound` chips from `diagnoseHotkeys`; per-row Test would duplicate signal.

## 8. Dark + light tokens

**No new tokens.** The "armed" cue is the inset glass card + `ring-1 ring-[var(--color-accent)]` — same accent used everywhere for active state, which already inverts across themes (teal `#3fb8b0` dark / `#0f8c8c` light). Keycap chips reuse `.keycap` unchanged. Tokens: `--color-surface-glass` (panel), `--color-accent` (rim), `--color-ink-mute` (valid verdict), `--color-danger` (invalid/conflict), `--color-control`/`--color-control-hover` (secondary buttons + suggestion chips). No keycap glow — Vox is quiet; the 1 px accent rim is the only armed cue.

## 9. Motion

- Enter Recording: right column height `0 → auto` over 180 ms `cubic-bezier(0.2,0,0,1)` (matches `pill-in`); inner panel fades in over 120 ms.
- Recording → Preview: 100 ms verdict cross-fade; keycaps in over 180 ms.
- Exit: collapse over 140 ms, rim fades.
- Conflict-clear flash on other row: 180 ms `animate-flash-done` (existing keyframe).

`prefers-reduced-motion` already collapses these via the global `*` override in `styles.css`. No new plumbing.

## 10. Hotkeys tab wireframe (one row in each state)

```
┌── PRIMARY SHORTCUT ───────────────────────────────────────────────┐
│ Push-to-talk  Built-in           [⌘ Cmd]+[⌥ Opt]+[.]    [IDLE]    │
│ Hold the key to dictate.         Change   Reset                   │
│                                  Test binding · Bound to ⌘⌥.      │
└───────────────────────────────────────────────────────────────────┘
┌── SECONDARY SHORTCUTS ────────────────────────────────────────────┐
│ Cancel while recording           ┌── armed glass card ───┐ [REC]  │
│ Abort the pipeline.              │ Press the shortcut…   │        │
│                                  │ Esc cancels.          │        │
│                                  └───────────────────────┘        │
│                           Or pick: [Esc] [F13] [F14] [F15]        │
├───────────────────────────────────────────────────────────────────┤
│ Cycle voice mode                 [⌥ Opt]+[M]            [CONFLICT]│
│ Advance through Voice Modes.     Re-record   Cancel               │
│                                  Already used by Cancel.          │
│                                  [Use this & clear that one]      │
│                                  [Change other shortcut]          │
└───────────────────────────────────────────────────────────────────┘
Changes apply immediately. If a key fails to bind, Vox falls back…
```
