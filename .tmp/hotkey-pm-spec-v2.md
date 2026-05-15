# Hotkey capture UI (v2) — PM Spec

## 1. Summary
Capture shows keycaps the moment a modifier is held, commits only on **Save**, and accepts modifier-only chords and double-tap modifiers — gated by Accessibility with an inline upsell.

## 2. User stories
1. *Chord builder.* Press several keys, see each land in the row, lift, click **Save**.
2. *Mod-only PTT.* Hold `Ctrl+Alt` to dictate — fingers stay on the home row.
3. *Double-tap PTT.* Tap `⌥` twice to dictate while typing alphanumeric codes without base-key collision.
4. *Permission-aware.* Without Accessibility, mod-only and double-tap are visibly disabled with one-click grant.
5. *Live feedback.* Adding `⌘` then `⌥` updates keycaps on each press so I know capture is alive.

## 3. Sub-specs

### 3a. Live feedback
- **Update on every modifier `keydown`, no debounce.** Modifier state on macOS is monotonic; non-modifier keys keep the 50 ms repeat-debounce.
- **Row shows every captured key** (`⌘ + ⌥ + M`).
- **Releasing a modifier does NOT drop it.** The row reflects the *captured-so-far set*, not currently-held keys — lifting fingers to read must not destroy work.
- **Empty state** returns only via `[Clear]` or fresh capture entry.
- **Verdict** updates live: `Hold a base key, or click Save to bind modifiers alone.` → `Looks good.` → `Already used by <row>.`

### 3b. Explicit save
- **Commit: Save button + Enter.** Manual-only feels broken vs v1 auto-commit; auto-after-stillness reintroduces the "I lifted a finger and it committed" failure. Button is the source of truth; Enter is a keyboard convenience.
- **State machine flattens.** v1's `recording` / `preview` / `preview-invalid` / `preview-conflict` collapse to one `capturing` state carrying `{ set, verdict }`. Save enabled iff `verdict === "ok"`.
- **Buttons:** `[Save]` (primary, disabled unless valid) · `[Clear]` (wipes set, stays in capture) · `[Cancel]` (exits). `Re-record` is gone — `Clear` replaces it.
- **Idle** still shows the configured binding via `<HotkeyKeys>` + `[Change]` + `[Reset]`.

### 3c. Mod-only & double-tap + Accessibility
- **Both ship in v2.** `mac_chord.rs` already handles mod-only single-press; double-tap is ~30 lines on the same `kCGEventFlagsChanged` callback.
- **Double-tap detection** in the capture handler: on a modifier `keydown`, if the same modifier fired within **400 ms** *and the set was empty between them*, promote to `Tap2:<Modifier>`. UI shows a ghost chip after tap 1, solid `[⌥⌥]` after tap 2.
- **Wire format:** `Mods:Cmd+Alt` for modifier-only · `Tap2:Alt` for double-tap (single mod only in v2) · legacy `Cmd+Alt+Period` loads unchanged.
- **Accessibility flow: hybrid (option iii).** Capture is always available. With AX denied, mod-only / double-tap verdict reads `Needs Accessibility — Vox can't hear modifier-only chords. [Grant Accessibility]`. The button calls a new `open_accessibility_settings` IPC that runs `open "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"` — macOS offers no programmatic prompt analogous to `requestAccessForMediaType:`; `AXIsProcessTrusted(options:{kAXTrustedCheckOptionPrompt:true})` only nudges. Save stays disabled until a fresh poll returns true; the row polls `ax_is_trusted` every 1.5 s while in `capturing` with a non-empty mod-only set. Justification: (i) breaks the v1 "no new TCC prompts" promise for the 95% who never need it; (ii) traps users mid-chord.

## 4. Disambiguation
State: `{ mods, key, last_mod_tap_at, last_tap_mod }`.

| Sequence | Result | Verdict |
| --- | --- | --- |
| Tap ⌥, release within 400 ms, set empty | tap tracker armed, no chip | `Tap Opt again for double-tap, or hold for a chord.` |
| Tap ⌥, tap ⌥ within 400 ms, set empty between | `Tap2:Alt` | `Looks good.` (AX-gated) |
| Tap ⌥, wait 400+ ms | tracker resets | empty-state |
| Hold ⌥, then ⌘ | `mods={Alt,Cmd}`; tracker cleared | `Hold a base key, or click Save to bind modifiers alone.` |
| Hold ⌥+⌘, release ⌥ | `mods` unchanged (captured-so-far) | unchanged |
| Hold ⌥+⌘, press M | `key=KeyM` | `Looks good.` |
| Press ⌥⌘M, lift M, press X | `key=KeyX` (last wins) | first time: `One base key only — using X. Click Clear to start over.` |
| Click Clear | empty | empty-state |

Critical: the second mod tap must find the set empty between, otherwise it folds into the existing capture. This is the only rule that disambiguates `Hold Cmd, release, hold Cmd+Alt` from `tap Cmd twice`.

> **User-asked correction.** The brief's `⌘⌥M+X` implies multi-base chords. Carbon `RegisterEventHotKey` and `kCGEventFlagsChanged` taps both key on a single base — v2 does **not** support multi-base. The user may press several base keys but only the last counts, with a soft hint the first time. v3 candidate.

## 5. Permission flow
- **Capture entry:** no check, no prompt.
- **Mod-only / Tap2 in capture with AX denied:** Save disabled, verdict shows upsell + `[Grant Accessibility]` → `open_accessibility_settings`. Row polls `ax_is_trusted` every 1.5 s and unlocks Save on grant.
- **At `hotkey::register`:** if saved PTT is `Mods:` / `Tap2:` and AX is false, follow v1's chord-error path — `Not bound` chip + `[Try another]`. PTT does **not** silently fall back (same v1 no-fallback policy).
- **Revocation while running:** detected next session start; Vox doesn't watch AX live (no public KVO).

## 6. Persistence schema

| Shape | Example | Handling |
| --- | --- | --- |
| Single-key (legacy) | `Cmd+Alt+Period`, `F13` | unchanged — `shortcut_from_key` |
| Modifier-only (new) | `Mods:Cmd+Alt`, `Mods:Ctrl+Alt+Shift` | new `parse_mods_spec` → `mac_chord::ChordSpec` |
| Double-tap (new) | `Tap2:Alt`, `Tap2:Cmd` | new `parse_double_tap_spec` → `mac_chord::DoubleTapSpec` |
| Disabled (cancel/cycle) | `""` | unchanged |

`validateHotkeySpec` rejects `Mods:` / `Tap2:` for cancel/cycle — those bind via the shortcut plugin, not CGEventTap. PTT only. No settings migration — wire stays a string, new fields additive.

## 7. v1 integration
- **"Try another"** still applies to OS-refused mod-only chords (same diagnostic chip + button).
- **Conflict resolution** identical — cross-row spec equality wins regardless of `Cmd+Alt+M` vs `Mods:Cmd+Alt`.
- **Test binding** extends `commands::test_hotkey` to consult `ActiveHotkeyState.chord` (and a new `double_tap` slot) — if the handle exists and `bound == configured`, return success. PTT row only.

## 8. Success criteria
1. Holding `⌘` shows a keycap within one frame; adding `⌥`, `M` updates live.
2. `Save` is the only commit path; no chord auto-commits.
3. Mod-only `Ctrl+Alt` and double-tap `⌥⌥` both bind, trigger PTT, survive restart.
4. AX upsell renders within 100 ms of a mod-only set; recovery on grant is ~2 s without restart.
5. Legacy `Cmd+Alt+Period` settings load unchanged.

## 9. Out of scope (v2)
1. Per-mode hotkeys.
2. Chord sequences across press cycles (`⌘K T`).
3. Multi-base-key chords (`⌘⌥M+X`).
4. Multi-modifier double-tap (`Tap2:Cmd+Alt`).
5. Per-binding hold-vs-toggle (`hands_free_enabled` stays global).
6. Live AX watcher (whole-app re-arm on grant without re-entering capture).
