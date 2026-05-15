# Hotkey capture UI — Design Spec (v2)

Implements `.tmp/hotkey-pm-spec-v2.md`. Adds live keycap feedback, explicit Save / Clear, and modifier-only + double-tap chords gated by an inline Accessibility upsell. Reuses the v1 panel and `<HotkeyKeys>`. **No new tokens, no new visual primitives** — one conditional sub-row inside the existing panel.

## 1. State machine

v1's `recording / preview / preview-invalid / preview-conflict` collapse into one `capturing` state carrying `{ set, verdict, ax_needed }`. Idle and post-save OS-refused (`Not bound` + `Try another`) unchanged.

## 2. Panel — four ordered slots inside the v1 container

Same `liquid-glass` panel (260 px, `rounded-[10px]`, `ring-1 ring-[var(--color-accent)]`, `animate-pill-in`):

```
┌── liquid-glass, accent rim ──────────────┐
│ ① keycap row     chips or placeholder    │
│ ② verdict line   one sentence            │
│ ③ permission row conditional             │
│ ④ action row     Save · Clear · Cancel   │
└──────────────────────────────────────────┘
                  Or pick: [⌥M] [F13] …
```

Slot ③ is the **only** new sub-element, `display:none` for ~95% of captures. When present: either the AX upsell button OR v1's two conflict-resolution buttons (v1 placed them below the action row — v2 folds them into ③ so conflict stops needing its own layout). "Or pick" chips drop **outside** the panel to its right.

## 3. Wireframes (slot ① / ② / ③ if shown / ④)

Idle row (unchanged): `[⌘ Cmd]+[⌥ Opt]+[.]   Change   Reset`.

```
(A) empty                              Save off
①  — (placeholder)
②  Press the shortcut you want… Esc cancels.   (animate-pulse-mic)
④  [Save•off] [Clear•off] [Cancel]   Or pick: [⌥M] [F13] …

(B) mods only, AX granted              Save on
①  [⌘ Cmd]+[⌥ Opt]
②  Hold a base key, or click Save to bind modifiers alone.
④  [Save] [Clear] [Cancel]

(C) mods + base                        Save on
①  [⌘ Cmd]+[⌥ Opt]+[M]
②  Looks good.
④  [Save] [Clear] [Cancel]

(D) mods only, AX missing              Save off
①  [⌃ Ctrl]+[⌥ Opt]
②  Needs Accessibility — Vox can't hear modifier-only chords.
③  [Open System Settings]
④  [Save•off] [Clear] [Cancel]

(E) double-tap                         Save on (AX-gated)
①  [⌥ Opt ×2]
②  Looks good. Tap twice within 300 ms to trigger.
④  [Save] [Clear] [Cancel]

(F) invalid                            Save off
①  [Shift]
②  Needs a regular key, not Shift on its own.
④  [Save•off] [Clear] [Cancel]

(G) second base key                    Save on, soft hint
①  [⌘]+[⌥]+[X]
②  One base key only — using X. Click Clear to start over.
④  [Save] [Clear] [Cancel]

(H) conflict                           Save off
①  [⌥ Opt]+[M]
②  Already used by Cycle voice mode.
③  [Use this & clear that one]  [Change other shortcut]
④  [Save•off] [Clear] [Cancel]
```

## 4. Live keycap feedback

- Modifiers-only set: `<HotkeyKeys spec="Cmd+Alt" />`. Existing tokeniser already handles a spec without a trailing key — **zero change**.
- Double-tap: add `tap2?: string`. One `.keycap` for the modifier + `<span class="text-[9px] text-[var(--color-ink-faint)] ml-0.5">×2</span>`. Chosen over two stacked chips (noisy) and animation (Vox is quiet); smallest API change, no new primitive.
- Chips appear per-`keydown` via Solid's `<For>`; each fades in 100 ms (`animate-pill-in`), 30 ms stagger on initial paint, none on sequential presses. Removal only on Clear.
- **No held-vs-captured glow.** PM spec: set is captured-so-far; releasing does not drop. An overlay would lie about state.

## 5. Slot ② — one home for every inline message

All inline copy renders in slot ②, `aria-live="polite"`. `--color-ink-mute` neutral, `--color-danger` invalid/conflict/AX, `--color-ink` + `animate-pulse-mic` for the initial placeholder. AX *sentence* shares ②; only its *button* gets ③.

## 6. Action row

Bottom, left-aligned, gap `1.5`, identical every variant. Tab order `Save → Clear → Cancel`. Enter = Save (when enabled); Escape = Cancel (`capture:true`, `preventDefault`).

| Button | Style | Enabled |
| --- | --- | --- |
| **Save** | `bg-[var(--color-accent)] text-[var(--color-on-accent)]`, `disabled:opacity-40` | `verdict==="ok"` AND (set has base key OR AX granted) |
| **Clear** | `bg-[var(--color-control)]` | set non-empty |
| **Cancel** | `text-[var(--color-ink-mute)]` | always |

Clear replaces v1's `Re-record` — empties the set in place. Focus moves to Save when verdict flips `ok`; else stays on panel (`tabIndex=-1`).

## 7. Accessibility upsell

Slot ③ appears iff set is modifier-only or `tap2` AND `ax_is_trusted === false`. Button `Open System Settings` → new `open_accessibility_settings` IPC running `open "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"`. Polling silent, no spinner: row polls `ax_is_trusted` every 1.5 s; on grant, slot ③ collapses 140 ms, slot ② flips, Save enables. If user dismisses and clicks Save: button is already disabled, hover title `Needs Accessibility — open System Settings to grant.`. No toast.

## 8. Microcopy — final strings

| Surface | Copy |
| --- | --- |
| Empty | `Press the shortcut you want… Esc cancels.` |
| Mods only, AX granted | `Hold a base key, or click Save to bind modifiers alone.` |
| Mods only, AX missing | `Needs Accessibility — Vox can't hear modifier-only chords.` |
| Tap 1 detected | `Tap Opt again for double-tap, or hold for a chord.` |
| Double-tap captured | `Looks good. Tap twice within 300 ms to trigger.` |
| Double-tap, AX missing | `Needs Accessibility — Vox can't hear double-tap chords.` |
| Valid chord | `Looks good.` |
| Second base key | `One base key only — using X. Click Clear to start over.` |
| Shift-only | `Needs a regular key, not Shift on its own.` |
| Bare letter | `Add Cmd, Opt, Ctrl, or Shift to a regular key.` |
| Esc on non-cancel row | `Esc is reserved for cancelling capture. Pick another.` |
| Cross-row conflict | `Already used by <Other row>.` |
| AX upsell button | `Open System Settings` |
| Save disabled (AX) `title` | `Needs Accessibility — open System Settings to grant.` |
| OS-refused (unchanged) | `<spec> couldn't be bound on this Mac.` |

## 9. Tokens + classes

| Use | Token / class |
| --- | --- |
| Panel | `.liquid-glass`, `ring-1 ring-[var(--color-accent)]`, `rounded-[10px]` |
| Keycaps (+ `×2` sub-span) | `.keycap` |
| Verdict | `--color-ink-mute` / `--color-danger` / `--color-ink`+`animate-pulse-mic` |
| Save / Clear / Cancel | `--color-accent` / `--color-control` / `--color-ink-mute` |
| Panel & chip enter | `animate-pill-in` |
| Slot ③ collapse | inline `transition-[height]` 140 ms `cubic-bezier(0.2,0,0,1)` |

**No new tokens, no new utility classes.** Only new code primitive: `HotkeyKeys`'s `tap2` prop.

## 10. Motion — quiet

Chip enter 100 ms fade-up. Verdict swap 100 ms cross-fade. Slot ③ enter/exit 140 ms height. No glow, no shake, no bounce. `prefers-reduced-motion` collapses via the existing `*` override.

## 11. Bloat audit

| | v1 | v2 |
| --- | --- | --- |
| Keycap row | 1 (preview only) | 1 (always) |
| Placeholder | 1 (recording only) | merged into ② |
| Verdict | 1 | 1 |
| Conflict cluster | 1 (separate) | merged into ③ |
| AX upsell | — | slot ③ (conditional) |
| Action row | 1 (varied) | 1 (fixed) |
| "Or pick" chips | 1 | 1 |
| **Steady visible** | **4** | **4** |
| **Worst case** | **5** | **5** |

**Net new visible elements: 0.** Three new behaviours fit one *conditional* slot.

## 12. Keyboard / a11y

Tab during capturing → `preventDefault`. Esc → cancel (`capture:true`). Window blur → exit. Verdict `aria-live="polite"`. Action-row tab order `Save → Clear → Cancel`. Enter = Save; Escape = Cancel.

## 13. Do not change from v1

1. Idle layout — keycaps + `Change` + `Reset` (+ `Off` for cancel/cycle).
2. OS-refused post-save — `Not bound` chip + `Try another`.
3. `Test binding` — PTT-row-only.
4. Left-column diagnostic chips — `Bound to …` / `Not bound`.
5. The liquid-glass panel container — same class, 260 px, accent rim, `animate-pill-in`. v2 changes what's *inside*, not the container.
