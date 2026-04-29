# Vox — Design Direction

This document captures the *why* behind Vox's visual identity: the icon, the
broader UI, and specific places where competitor research suggests we can
sharpen the product. It pairs with `docs/liquid-glass-light-spec.md` and
`docs/liquid-glass-research.md` (which cover lower-level token mechanics) and
with `CLAUDE.md` (which describes the codebase as it stands).

This is a living direction, not a contract. When you depart from it, write
down why so the next person doesn't reverse the change reflexively.

---

## 1. Brand identity, in one paragraph

Vox is **on-device dictation framed as an authorship tool**, not a recording
tool. You press a hotkey, speak, and the cleaned-up transcript lands on your
clipboard ready for `⌘V`. Audio never leaves the device, no telemetry, no
account, no cloud. The visual identity should communicate three things and
nothing else:

1. **Warm and human** — voice is the most human input; the UI should not feel
   like a security product even though privacy is the marquee feature.
2. **Quiet and pro** — sits next to Things 3, Linear, Raycast in the menu
   bar; should not embarrass itself there.
3. **Words, not waveforms** — the deliverable is *text*. Every visual cue
   should reinforce that voice is just the input method, the output is
   writing.

If a design choice doesn't pull on one of those three threads, cut it.

---

## 2. The icon

### Decision

**A warm peach-to-coral squircle with a cream curly-quote glyph whose tail
extends into a speech-bubble pointer.** Two colors total. Single bold mark
at ~68% bleed. No microphone, no waveform, no padlock, no cloud.

| Token | Value | Role |
|---|---|---|
| `--icon-bg-from` | `#FFB088` | Top-left of background gradient (warm peach) |
| `--icon-bg-to` | `#FF7A5C` | Bottom-right of background gradient (coral) |
| `--icon-glyph` | `#FFF4E8` | Cream foreground — *not* pure white (reads warmer) |
| `--icon-rim` | `#FFFFFF` @ 0.35 | 1px inner highlight on upper half (Liquid Glass cue) |
| `--icon-shadow` | `#A33616` @ 0.18 | Soft inner shadow at bottom; 3px blur, 3px Y-offset |

The master SVG lives at [`src-tauri/icons/source/vox-icon.svg`](src-tauri/icons/source/vox-icon.svg).
The in-app component is [`src/components/VoxLogo.tsx`](src/components/VoxLogo.tsx)
and is used in the Settings sidebar (28 px), the About hero (48 px), and the
Onboarding welcome step (96 px). The component auto-drops the speech-bubble
tail below 32 px to prevent the glyph from muddying.

### Rationale

Built from a research pass over the macOS HIG (Tahoe 26 / Liquid Glass) and
nine competing dictation apps. The decisive findings:

- **6 of 9 competitors use vertical EQ-bar waveforms** (Wispr Flow, Aiko,
  Otter, Whisper Memos, Apple Voice Memos, Whisper Transcription). The
  5-bar pose is borderline genre-cliché.
- **Cool palettes dominate** — cobalts, lavender, onyx. Wispr Flow's cream
  foreground is the only deliberately warm note in the set.
- **Nobody uses a quotation-mark / speech-bubble metaphor** — surprising for
  *dictation*, which is fundamentally "voice → written word."
- **Nobody pairs warmth with privacy** — every privacy-local app dresses
  like a security product (cloud-with-a-slash, padlock, deep navy).

Warm coral + curly quote claims the unoccupied corner of the field. It says
*authorship tool* (the punctuation of speech) and *trust through restraint*
(no padlock theatre). At 16 × 16 it survives because the silhouette is
asymmetric — the speech-bubble tail is what tells you it's a quote and not a
generic comma.

### Rejected alternatives

These were considered and dropped. Re-litigate at your peril.

1. **Typographic "V" lettermark** (paper-cream on deep ink, à la Bear / iA
   Writer). Strong on its own, but a single letter cannot out-distinguish the
   genre — too close to the wordmark category occupied by Vivaldi, Volta,
   Vox Media. Dies in the menu-bar tray as a featureless blob.

2. **Dot-and-orbit** (single luminous dot + concentric ring on dark slate).
   Anti-cliché on paper, but at 16 × 16 it collapses to a generic AI-app
   pulse and shares too much DNA with Siri, ChatGPT, Pi.

3. **Microphone-on-coloured-tile** (Wispr Flow / MacWhisper / Aiko mode).
   Cliché, low-distinctiveness, and reframes Vox as a recording tool when
   the actual workflow is writing.

### What the icon must *not* do

If you find yourself drifting toward any of these, stop:

- Floating glyph centred in a flat circle with negative space around it
  (the current pre-redesign icon — instantly reads "AI-generated")
- Default purple-to-blue mesh gradient (2022 ChatGPT-era; dates the app)
- Three-or-more-color rainbow without a semantic anchor
- Photorealistic mic on a stand (2014 skeuomorphism)
- Padlock, shield, cloud-with-slash — privacy theatre
- Glyph at < 55 % bleed
- Pure white foreground on the coral (reads cold and clinical)
- Drop shadows under the squircle (use the inner shadow only)

### Raster regeneration

The actual macOS app icon (`src-tauri/icons/icon.icns` and the sibling PNGs
used by the Tauri bundler) still ship the legacy blue-circle PNG. To
regenerate from the new SVG:

```bash
brew install librsvg                                  # one-time
rsvg-convert -w 1024 -h 1024 \
  src-tauri/icons/source/vox-icon.svg \
  -o /tmp/vox-1024.png
npx @tauri-apps/cli icon /tmp/vox-1024.png            # emits all sizes
```

For the **16 × 16 entry, hand-tune** afterward — drop the speech-bubble tail
subpath and thicken the glyph by ~8 %. Antialiasing destroys the curly
counter at that size if you simply downscale the master.

For macOS Tahoe's `.icon` layered format (Liquid Glass), ship Default and
Dark variants identical. The icon's warmth *is* the brand — desaturating
for dark mode would defeat the entire differentiator.

---

## 3. UI palette and how it relates to the icon

The icon is intentionally *warmer and more saturated* than the app's UI
chrome. They are not the same palette and shouldn't be.

| Surface | Tone | Reason |
|---|---|---|
| Icon | Saturated coral + cream | Has to sing in the menu bar / Dock against arbitrary wallpapers. Coral is the brand-recall hook. |
| App chrome (Liquid Glass) | Onyx (dark) / warm-cream-over-glass (light), accents in `#7c9cff` blue and `#b49bff` violet | Has to recede so content is legible. Cool accents avoid drawing the eye away from the user's text. |

This is deliberate, not an inconsistency: think of the icon as the *album
cover* and the UI as the *page layout inside the book*. The icon lives in
shared chrome (Dock, Spotlight, About) where it competes with everything;
the UI lives behind a window the user has already opened with intent.

That said, **the warm coral should peek into the UI** in the few places it
naturally belongs (see §5.4 below). It just shouldn't be the default
accent — that role stays with the existing blue/violet tokens.

---

## 4. UI primitives and conventions (current state)

These are the building blocks already in use in `src/views/Settings.tsx`
after the recent redesign pass. New screens should reuse them.

### Layout

- **Card** — `liquid-card` class with `rounded-[14px]` and `ring-1`. Groups
  related rows. `overflow-hidden` so internal dividers tuck under the
  rounded corners. One section = one card.
- **Row** — flex with `flex-1 min-w-0` on the label and `shrink-0` on the
  controls. Label area gets a 13 px label + optional 11 px hint; controls
  area gets the actual interaction. The min-width-zero pattern is
  **load-bearing** — without it, segmented controls and long labels
  collide.
- **SectionHeader** — small-caps title (11 px, 0.08em tracking) outside any
  card, with an optional right-aligned hint. Replaces three older ad-hoc
  patterns. Use for top-level grouping; do not nest.
- **Card with internal dividers** — for related rows under one heading.
  Border-bottom on every row except the last (use `last` prop on `Row`).

### Controls

- **Toggle** — for binary on/off (state, not choice). 22 × 38 px knob.
- **SegmentedControl** — for binary or 3-way *choices* where both options
  deserve equal visual weight. Replaces dropdown for 2-option pickers.
- **Keycap** — for displaying keyboard shortcuts. Monospace, slight inset
  shadow. Use for *display only*, not interactive picking.
- **Native `<select>`** — for 4+ options or anything language-driven. Don't
  rebuild the OS picker; it has correct keyboard nav, search, and
  accessibility for free.

### Typography scale (used; if you need a new size, audit first)

| Size | Use |
|---|---|
| 20 px / 600 | Tab title (header H1) |
| 16 px / regular | Mode emoji icon |
| 14 px / 600 | Card titles inside modals (e.g. mode editor) |
| 13 px / 500 | Row label, sidebar nav, primary buttons |
| 12 px / regular | Hints in dense areas, secondary buttons, help text |
| 11 px / regular | Secondary hints, status chips, small caps section labels |
| 10 px / 600 / 0.08 em | Pills, status chips ("Active", "Local", "v1.0.0") |

Don't add new sizes lightly. Variance kills calm.

### Motion

- 150 ms ease for hover/focus state changes.
- 200 ms ease for the Saved badge fade.
- 250 ms ease for theme transitions (background-color + box-shadow only).
- `prefers-reduced-motion: reduce` already kills all animations.
- **No springy / bouncy / playful animations.** Vox is a tool, not a toy.

---

## 5. UI improvement suggestions, derived from competitor research

These are *opportunities*, not commitments. Each one names the competitor
that does it well so you can study the implementation directly. Ranked by
expected impact.

### 5.1 Onboarding: copy the Wispr Flow / Linear cadence

Wispr Flow's onboarding teaches you the keyboard shortcut by making you
*use it* before the wizard finishes. Vox's current onboarding tells you
the shortcut and trusts you to remember.

Concrete: replace the final "How to use" step with an inline practice
moment — "Try it now: hold ⌘ ⌃ Space and say *hello*." Watch for the
first successful transcript and only then enable the "Done" button. This
both validates the install and creates a memorable first-success moment.

### 5.2 Pill: borrow the Raycast HUD vocabulary

Raycast's command-bar HUDs use a single horizontal pill with a status
glyph on the left, status text in the middle, and a keycap hint on the
right. Vox's pill currently does most of this but the state transitions
(Listening → Transcribing → Polishing → Done) are not given equal visual
weight, and "Polishing" sometimes flashes too fast to read.

Concrete: every state should be readable for at least 250 ms even if the
underlying step finishes faster (artificially hold the *display*, not the
work). Add a thin 1-pixel progress hairline along the bottom of the pill
that grows across stages so users can see *where* in the pipeline they
are without having to read.

### 5.3 Voice modes: match the Things 3 inline-edit pattern

Things 3 lets you tap a row to expand it inline with all editing controls
in place; Vox currently opens a modal for mode editing. Modals are the
right tool for *creating* a mode, but for *editing* an existing one,
inline expansion would feel less heavy. The mode list is short (5
defaults + custom), so no scroll-state issues.

Concrete: keep the modal for "+ New mode" (clean slate, deserves focus);
switch "Edit" to inline expansion that reveals the system prompt + post-
processing controls underneath the row.

### 5.4 Recording state: let the warm coral peek through

Right now the listening-state visual cue uses the same cool blue accent
as the rest of the UI. The icon's coral is unused inside the app.

Concrete: the **listening state** (mic actively recording) is the one
moment where the brand color earns its keep. The pill background (or its
mic-level meter) should warm to coral when actively listening, then
cool back to blue/violet during transcription and polishing. The user
*feels* the device is recording without having to read text.

This also makes the warm coral icon feel earned in the UI rather than
floating in from the wallpaper.

### 5.5 Privacy story: take a page from Aiko's confidence

Aiko's marketing one-liner is "your audio never leaves your device, full
stop" — and it doesn't make you tap through a privacy explainer. Vox
already has a strong Privacy tab, but the *menu-bar popover* (the most
visited surface) doesn't echo it.

Concrete: a single "Local" badge in the popover footer, identical to the
one I added to the Settings sidebar. No popover, no tooltip, no
explanation — just a visible reminder every time the user opens the
menu. If they care, they'll click it. If they don't, the silence is the
message.

### 5.6 Models tab: borrow the Linear "Plan" picker pattern

Linear shows tier upgrades with a single sentence comparing each plan's
*ceiling capability* rather than feature checklists. Vox's Models tab
currently shows three Gemma options with size and tier labels but
doesn't help the user pick.

Concrete: a one-liner per model that explains *what changes* for the
user. "E2B (3.5 GB) — fast, basic punctuation. E4B (5.4 GB) — balanced,
good filler-word removal. 26B-A4B (17 GB) — slowest, best at preserving
your voice in long dictations." Tier labels stay; recommendations
become *for whom*, not *what RAM*.

### 5.7 Mode picker on the pill: copy the Notion Mail switcher

Notion Mail's account switcher is a 2-row × 2-col grid of icons with the
active one ringed in accent. Vox's mode-cycle hotkey is great for
rotating but discovering modes requires tray → settings → modes. The
listening pill could show the active mode as an inline emoji + name, and
holding the cycle hotkey could pop a 2 × 2 grid for direct selection.

Concrete: defer until there's signal users want it. Mode discovery is
already addressed by the Auto-suggest HUD; this is a nice-to-have for
power users who maintain 6+ custom modes.

### 5.8 Brand voice: prune "Talk to any app." everywhere

The pre-redesign tagline survives in a couple of places that were not in
the screenshots — search before shipping copy. Default to two acceptable
phrasings:

- **Hero / first impression**: "On-device dictation."
- **Tagline / subtitle**: "Press, speak, paste."

Avoid: "Talk to any app" (misleading post-pivot — there's no auto-paste),
"AI-powered dictation" (every app says this), "Voice typing" (genericist),
"Whisper for Mac" (positions Vox as a wrapper, which it isn't).

### 5.9 Empty states: match Linear's micro-copy energy

The Dictionary tab's empty state currently reads "Add names or technical
words Vox should spell correctly." Functional, but a missed chance.

Concrete: empty states should hint at *value*, not *function*. Try "Vox
will spell 'Anthropic' even if it sounds like 'Anthropik'." A single
real example beats five abstract bullets.

### 5.10 The 16 × 16 problem

The new icon's 16 × 16 export needs a hand-tuned variant (drop the tail,
thicken the glyph). The same constraint applies to the **menu-bar tray
icon**, which lives at this exact size and changes state (idle,
listening, transcribing, error).

Concrete: tray glyphs should *not* try to mimic the app icon at small
size — they should be functional state indicators. A small filled circle
that pulses warm-coral while listening is plenty. The brand identity
lives in the dropdown popover header, not in the 16-px glyph.

---

## 6. What we're explicitly *not* doing

These came up during research and were considered and rejected. Naming
them here so they don't keep returning.

- **No "AI" branding.** No sparkle glyph, no "powered by AI" pill, no
  "✨" in copy. Vox uses Whisper and Gemma; that's an implementation
  detail, not a positioning.
- **No subscriptions / pricing UI.** Vox is local-first and key-free. If
  a future build introduces a paid tier, design that surface from
  scratch — don't retrofit a modal into the current flow.
- **No cloud iconography anywhere.** Even ironically. The whole point.
- **No dark mode for the icon.** The warm coral *is* the brand
  recognition; desaturating to navy in dark mode would defeat the
  differentiator. UI chrome themes; the icon does not.
- **No skeuomorphic anything.** No fake bevels, no realistic glass
  reflections, no "tape recorder" metaphors. We use real Liquid Glass
  via `backdrop-filter`, which is the only acceptable form of depth.

---

## 7. Open questions

Things worth deciding before they decide themselves.

- **Tray icon state vocabulary.** Should the listening state be a coral
  pulse (matches brand) or a red dot (matches macOS system convention
  for "recording in progress")? Probably brand-coral with a 1-pixel red
  outer ring — communicates both.
- **Light-theme icon.** Currently the icon is theme-invariant. Should
  the cream glyph deepen slightly on light backgrounds for contrast?
  Tested: at 1024 px the warm cream reads fine on cream; at 16 px the
  glyph reads against the coral background, not the page.
- **Replacement of the placeholder shadow on the gradient logo
  variant.** The old `box-shadow: inset 0 1px 0 rgba(255,255,255,0.35)`
  was tuned for a flat purple-blue gradient; the new SVG uses a
  filtered inner shadow instead. Audit other places where the old
  shadow was used (currently only Settings, but check Onboarding's
  Welcome card and any future pill-state cards).
- **Wordmark.** Vox does not currently have a custom wordmark — the
  word "Vox" everywhere is system font (SF Pro). A modest custom
  wordmark in the About card and onboarding hero would tighten the
  identity. Not urgent; lives at the bottom of the priority list until
  a typographer is involved.

---

## 8. Sources

Captured here so the next person doesn't redo the search.

- Apple — *App Icons* (Human Interface Guidelines, current).
- Apple Newsroom — "Apple introduces a delightful and elegant new
  software design", June 2025 (Liquid Glass announcement).
- *Updating application icons for macOS 26 Tahoe and Liquid Glass*,
  Successful Software, Sept 2025.
- *macOS Tahoe forces all app icons into iOS squircles*, Lap Cat
  Software, June 2025.
- macOS Icon Gallery — submissions from 2024–2026 (peer reference).
- Wispr Flow, Superwhisper, Aiko, MacWhisper, Whisper Memos, Otter.ai,
  Krisp, Talon Voice, Whisper Transcription — competitive sweep
  conducted April 2026.
