# Vox — Design Specification

> Companion to `product-requirements.md` and `tech-stack.md`.
> Stack target: Tauri 2 + SolidJS + Tailwind CSS 4 + lucide-solid. macOS 13+ primary.
> Last updated: 2026-04-24.

---

## 1. Design principles

1. **Invisible until invoked.** Vox is a hotkey, not a window. Chrome appears only during the pipeline and disappears within 200 ms of completion.
2. **Native macOS feel.** Translucent materials, SF typography, soft elevation. Must not look out of place next to Shortcuts or Raycast.
3. **Never steal focus.** No surface, including errors, activates a window over the user's current app. Recoverable errors live in pill + tray icon.
4. **One-beat feedback.** Every state (listen → transcribe → polish → done) has a single, legible signal. No more than one motion at a time.
5. **Restraint over richness.** Grayscale baseline. Color reserved for semantics: accent = active, red = error, green = done, amber = permission.

---

## 2. Design tokens

### 2.1 Colors

WCAG AA (4.5:1) for body text in both modes. Dark is primary.

**Neutral ramp (shared):** `neutral-0 #FFFFFF`, `50 #F7F7F8` (light canvas), `100 #EFEFF2` (light surface), `200 #E4E4E9` (light hairline), `400 #A1A1AA` (tertiary text), `500 #71717A` (secondary text), `700 #3F3F46` (dark hairline), `800 #27272A` (dark surface), `900 #18181B` (dark canvas), `950 #0B0B0D` (scrim).

**Semantic tokens** (Dark / Light):

- `--color-bg` — `#0B0B0D` 70% + blur / `#F7F7F8` 70% + blur
- `--color-surface` — `rgba(39,39,42,0.72)` / `rgba(255,255,255,0.72)`
- `--color-surface-raised` — `rgba(63,63,70,0.85)` / `rgba(255,255,255,0.95)`
- `--color-border` — `rgba(255,255,255,0.08)` / `rgba(0,0,0,0.08)`
- `--color-border-strong` — `rgba(255,255,255,0.14)` / `rgba(0,0,0,0.12)`
- `--color-text-primary` — `#F4F4F5` / `#18181B`
- `--color-text-secondary` — `#A1A1AA` / `#52525B`
- `--color-text-tertiary` — `#71717A` / `#71717A`
- `--color-accent` — `#7C9CFF` / `#4F6BFF`
- `--color-accent-soft` — `rgba(124,156,255,0.16)` / `rgba(79,107,255,0.10)`
- `--color-danger` — `#FF6B6B` / `#D93A3A`
- `--color-success` — `#4ADE80` / `#16A34A`
- `--color-warning` — `#FBBF24` / `#B45309`

Tailwind 4 wiring (in `styles.css`):

```css
@theme {
  --color-accent: #7c9cff;
  --color-accent-soft: rgb(124 156 255 / 0.16);
  --color-danger: #ff6b6b;
  --color-success: #4ade80;
  --color-warning: #fbbf24;
  --color-ink: #f4f4f5;
  --color-ink-mute: #a1a1aa;
  --color-bg: #0b0b0d;
  --color-surface: rgb(39 39 42 / 0.72);
}
```

### 2.2 Typography

Font stacks — no web fonts, zero bundle cost:

```css
/* sans */  -apple-system, BlinkMacSystemFont, "SF Pro Text", "SF Pro Display", "Inter", "Helvetica Neue", Helvetica, Arial, sans-serif;
/* mono */  ui-monospace, "SF Mono", "JetBrains Mono", Menlo, Consolas, monospace;
/* features */ font-feature-settings: "cv11", "ss01", "ss03";
```

Type scale (`-0.01em` tracking at >16px, `0` at ≤14px):

- `display` — 24/30, 600 — onboarding headlines
- `title` — 17/22, 600 — settings tab titles, modal titles
- `body` — 13/18, 400 — primary UI body text
- `label` — 12/16, 500 — form labels, section headers (caps `0.04em`)
- `caption` — 11/14, 400 — hints, metadata, hotkey reminders
- `pill` — 13/16, 500 — listening pill main text
- `mono-sm` — 12/16, 500 — keycap, version

### 2.3 Spacing, radii, shadows

**Spacing** — 4pt grid; Tailwind defaults match. Pill padding `px-4 py-3`. Settings section gap `gap-6`, card padding `p-5`.

**Radii** — `r-sm 6px` (inputs, small btns), `r-md 10px` (cards, rows), `r-lg 14px` (popover), `r-xl 18px` (settings window), `r-pill 28px` (listening pill; fully-rounded on 56px height).

**Elevation** (layered for translucent surfaces):

- `e-1` cards: `0 1px 2px rgb(0 0 0 / 0.08), 0 0 0 1px rgb(0 0 0 / 0.04)`
- `e-2` popover/modal: `0 4px 16px rgb(0 0 0 / 0.12), 0 0 0 1px rgb(255 255 255 / 0.04)`
- `e-3` pill dark: `0 8px 32px rgb(0 0 0 / 0.24), 0 0 0 1px rgb(255 255 255 / 0.06)`
- `e-3l` pill light: `0 8px 32px rgb(0 0 0 / 0.12), 0 0 0 1px rgb(0 0 0 / 0.06)`

### 2.4 Motion

- `m-instant` — 80ms `cubic-bezier(0.2, 0, 0, 1)` — hover, press feedback
- `m-quick` — 150ms `cubic-bezier(0.2, 0, 0, 1)` — pill fade-in, tooltip
- `m-smooth` — 240ms `cubic-bezier(0.32, 0.72, 0, 1)` — popover, state transitions
- `m-slow` — 400ms `cubic-bezier(0.32, 0.72, 0, 1)` — onboarding screen transitions

**Named animations**:

- `pulse-mic` — opacity 1 → 0.55 → 1 on a 1.2s `ease-in-out` loop. Mic glyph during Listening.
- `pill-in` — `translateY(8px) scale(0.98) opacity 0` → `0/1/1` over `m-quick`. `pill-out` reverses over 200ms.
- `spinner` — 360° rotation, 900ms linear, infinite.
- `flash-done` — bg transitions to `--color-success` alpha 0.2 for 180ms, then fades.
- `waveform` — 24 bars driven by real RMS from Rust at 30fps. Silent mic = flat 2px bars (no synthetic idle).

**Reduced motion** (`prefers-reduced-motion: reduce`): disable `pulse-mic`, `pill-in/out` (snap); waveform still updates (real data); spinner → static dot.

---

## 3. Component specs

### 3.1 Listening pill

**Window.** 280 × 56 px at 1×. Borderless, transparent, non-activating, click-through, `NSScreenSaverWindowLevel`. Bottom-center of active display, 24px above Dock (or screen bottom if Dock hidden). Configurable to `bottom-right` / `top-center`. Never overlap menu bar / notch; `top-center` anchors 8px below menu bar.

**Visual (dark).** Bg `rgba(11,11,13,0.72)` + `backdrop-filter: blur(24px) saturate(180%)`. Inner border 1px `rgba(255,255,255,0.08)`. Shadow `e-3`. Radius 28px (fully pilled on the 56px axis).

**Layout**

```
+--------------------------------------------------------------+
|  [ mic ]   ||||  |||  ||  |||||  ||  ||||   Listening…       |
|   20×20   <------  waveform 144×24  ------>   13px 500       |
+--------------------------------------------------------------+
    px-4 (16)                                           px-4
```

- Left slot (28×28): icon (mic / spinner / check / alert).
- Middle slot (flex-1, max 160px): waveform canvas OR status text.
- Right slot: status text OR timer OR flag code for pinned language (e.g. `EN`, `UA`).
- Gap: `gap-3` (12px).

**States** (icon / middle / right text / accent):

- Hidden — window not rendered.
- Listening — mic (accent, pulsing) / live waveform / `Listening…` / accent glow `shadow-[inset_0_0_0_1px_theme(colors.accent/30)]`.
- Listening, pinned lang — mic / waveform / `EN · 00:04` / same.
- Hands-free — mic + small infinity / waveform / `Hands-free` / subtle accent border.
- Transcribing — spinner (accent) / text-only / `Transcribing…` / neutral.
- Polishing — spinner (violet `#B49BFF`) / text-only / `Polishing…` / violet accent.
- Done — check (success) / — / `Done` / `flash-done` 180ms then fade.
- Paste-fallback — clipboard (warning) / — / `Copied — ⌘V to paste` / warning outline.
- Error network — `alert` (danger) / — / `No connection` / danger ring.
- Error API key — `alert` (danger) / — / `API key invalid` / danger ring.
- Error mic — `mic-off` (danger) / — / `No mic access` / danger ring.
- Canceled — `x` / — / `Canceled` / neutral, 150ms fade.
- Offline — `cloud-off` (warning) / — / `Offline` / warning.

**Interactions.** Click-through by default (`pointer-events: none`). No hover state. `Esc` cancels (handled by global shortcut). Error state dwell 2000ms then auto-hide.

**Tailwind — main container**

```html
<div class="pointer-events-none fixed bottom-6 left-1/2 -translate-x-1/2
            flex items-center gap-3 w-[280px] h-14 px-4
            rounded-full
            bg-[rgba(11,11,13,0.72)] backdrop-blur-xl backdrop-saturate-150
            ring-1 ring-inset ring-white/8
            shadow-[0_8px_32px_rgb(0_0_0/0.24),0_0_0_1px_rgb(255_255_255/0.06)]
            text-[13px] font-medium text-zinc-100
            transition-all duration-150 ease-[cubic-bezier(0.2,0,0,1)]">
```

Light-mode override: `bg-[rgba(247,247,248,0.72)] ring-black/8 text-zinc-900 shadow-[0_8px_32px_rgb(0_0_0/0.12),0_0_0_1px_rgb(0_0_0/0.06)]`.

**Waveform renderer.** Canvas 144×24 @devicePixelRatio. 24 bars, 4px wide, 2px gap, radius 2px. `h = clamp(rms * 24, 2, 24)`, vertically centered. Color `var(--color-accent)` at 0.9; tail bars fade to 0.4.

---

### 3.2 Menu-bar tray icon

macOS requires a **template image** (monochrome, alpha-only) so the system tints for light/dark menu bars. Dimensions **18 × 18 pt** (36×36 @2×), 1.5px stroke, fits within 16×16 safe area.

State variants:

- **Idle** — mic outline.
- **Recording** — mic filled + 3px red dot bottom-right, `pulse-mic`.
- **Processing** — mic outline + 270° arc spinner overlay.
- **Error** — mic outline + red filled circle badge with white `!`.
- **Missing permission** — same badge shape in amber.
- **Disabled** — mic with diagonal slash.

**Click behavior.** Left/right/control-click all open the popover (§3.3). No separate native context menu in v1.

---

### 3.3 Tray popover

**Window**

- Size: **320 × 400 px**, arrow-anchored to tray icon (8px gap below menu bar).
- Decorations off, transparent, always-on-top, not in taskbar.
- Radius: `14px`. Shadow `e-2`. Background: `--color-surface` + `backdrop-blur-2xl`.
- Border: 1px `--color-border`.

**Layout (top to bottom)**

```
┌───────────────────────────────────── 320 ─────────┐
│  ● Listening ready              [ ⚙ ] [ × ]       │  48 — header, hairline border bottom
│                                                   │
│  Dictation                            [ on  ● ]   │  56 — big toggle row
│  Hold Right ⌥ to speak                            │
│                                                   │
│  ───────────────────────────────────────          │
│  Recent (cleared on quit)                         │  24 — section label
│  ┌─────────────────────────────────────────────┐  │
│  │ 2m ago   Hey team, let's push the launch… │  │  44 each, up to 5 rows
│  │ 12m ago  The staging environment is still… │  │
│  │ 28m ago  Thanks, will review after lunch.  │  │
│  │ 1h ago   …                                  │  │
│  │ 1h ago   …                                  │  │
│  └─────────────────────────────────────────────┘  │
│                                                   │
│  ───────────────────────────────────────          │
│  ⌘ ⇧ V  Paste last transcript                      │  28 — hotkey row
│  Esc    Cancel                                    │  28
│                                                   │
│  ┌─────────────────────────────────────────────┐  │
│  │  Settings…              Quit                │  │  44 — two-column footer
│  └─────────────────────────────────────────────┘  │
└───────────────────────────────────────────────────┘
```

**States.** Ready — green dot + "Listening ready". Recording — red pulsing dot + "Recording…" + mini waveform in header strip. Processing — amber dot + "Transcribing…" / "Polishing…". Error — red dot + first line + deep-link second line ("Open System Settings", "Re-enter API key"). Missing permission — amber dot + "Grant access to continue" + CTA. Disabled — muted dot + "Dictation off", toggle `off`. Empty recents — placeholder "Your last 5 transcripts will appear here.". Offline — warning banner under header: "No internet — transcripts will fail".

**Interactions.** Toggle: `Space`/click, 180ms. Recent row hover reveals "Copy" pill (keyboard arrow-nav + `Enter` copies). Footer: `Settings…` opens window; `Quit` confirms only if recording active. Popover dismisses on Esc, outside-click, or re-click of tray icon. Tab order: header → toggle → recent list → settings → quit.

**Recent row.** 44px tall, `px-4 py-2 rounded-[10px]`, hover `bg-white/4` (dark) / `bg-black/4` (light). Left: age (`text-caption` tertiary, 48px column). Right: 1-line truncated preview (body size).

**Tailwind — popover container**

```html
<div class="w-80 h-[400px] overflow-hidden
            rounded-[14px] bg-[rgba(24,24,27,0.82)] backdrop-blur-2xl
            ring-1 ring-white/8
            shadow-[0_4px_16px_rgb(0_0_0/0.12),0_0_0_1px_rgb(255_255_255/0.04)]
            text-[13px] text-zinc-100">
```

---

### 3.4 Settings window

**Window**

- Size: **760 × 560 px**, resizable with min `720 × 520`.
- Native macOS traffic lights, translucent title-bar (`titleBarStyle: 'hiddenInset'` in Tauri config; use `window-vibrancy` for sidebar material).
- Radius handled by OS (Sonoma `r-xl`).

**Layout — split view**

```
┌──────────────────────────────────────────────────── 760 ────────┐
│ ◉ ◉ ◉                                                           │  28 — draggable title bar
├───────────── 220 ─────────────┬─────────── 540 ─────────────────┤
│                               │                                 │
│  [icon] General          ▸    │   General                       │
│        Hotkeys                │   ──────────────────────────    │
│        API keys               │                                 │
│        Dictionary             │   ┌───────────────────────────┐ │
│        Privacy                │   │ Launch at login      [● ] │ │
│        About                  │   │ Start hidden         [ ●] │ │
│                               │   └───────────────────────────┘ │
│                               │                                 │
│                               │   Indicator position            │
│                               │   ┌──────┬──────┬──────┐        │
│                               │   │  ↓   │  ↘   │  ↑   │        │
│                               │   │bottom│bottom│ top  │        │
│                               │   │center│right │center│        │
│                               │   └──────┴──────┴──────┘        │
│                               │                                 │
│                               │   Language                      │
│                               │   [ Auto (detect)          ▾ ]  │
│                               │                                 │
│ Vox 1.0.0 (b4a91c)            │                                 │
└───────────────────────────────┴─────────────────────────────────┘
```

**Sidebar**

- Width 220. Translucent vibrancy (`NSVisualEffectView` sidebar material via `window-vibrancy`). In Tailwind: `bg-transparent` + CSS background fallback `rgba(24,24,27,0.6)` with backdrop-blur-2xl.
- Each tab row: 36px tall, `px-3`, `gap-2`, 14px icon + 13px label.
- Active: `bg-white/6` (dark) / `bg-black/5` (light), `rounded-[6px]`, accent-colored icon.
- Focus ring: 2px accent, offset 2px.
- Footer: version + build hash (`text-caption tertiary`), `text-center`.

**Main pane**

- `px-8 py-6`, `bg-[rgba(39,39,42,0.4)]` dark / `bg-[rgba(255,255,255,0.5)]` light.
- Title row: `text-title` + divider (1px border below, mb-5).
- Sections use **cards**: `rounded-[10px] bg-surface-raised ring-1 ring-border p-0` with internal rows `px-4 py-3` separated by hairlines.
- Controls aligned right. Labels on left, helper text below label in `text-caption text-secondary`.

**Per-tab content**

- **General.** Launch at login (toggle). Start hidden (toggle). Indicator position (segmented 3-up w/ glyphs). Language (dropdown: Auto + top 12 + "More…"). "Fast mode" toggle (hidden unless latency SLO missed — PRD 6.1).
- **Hotkeys.** 3 rows: Push-to-talk, Paste last transcript, Cancel (read-only `Esc`). Each row: label + keycap pills + "Change…". Keycap pill: `mono-sm px-2 py-0.5 rounded-[6px] ring-1 ring-border bg-surface`. Capture modal prompt: "Press the keys you want to use" + `Esc` cancels. Live conflict check.
- **API keys.** Provider cards: Groq, Anthropic, OpenAI (optional). Each: masked `password` input, Show toggle, Test button (spinner → inline check/cross), status caption ("Stored in Keychain" / "Last tested 2m ago" / "Invalid key — check prefix").
- **Dictionary.** Search + chip list. Add-input with `Enter` to commit. Remove on chip click. Counter `12 / 100`.
- **Privacy.** "Do not send audio if offline" (on). "Never touch clipboard, type instead" toggle. "Open logs folder" link. Danger card: "Delete all local data" (red outline, confirm modal).
- **About.** Wordmark, version, build hash, "Check for updates" button, links: Documentation, Privacy policy, Report an issue, Open-source licenses.

**Interactions**

- Changes apply instantly (no Save). On any change, a 240ms green check flashes next to the field.
- Sidebar keyboard nav: Up/Down arrows; `Tab` enters main pane.
- `Cmd+,` opens window. `Cmd+W` closes.
- `Cmd+1..6` jumps to each tab.

**Tailwind — card row**

```html
<div class="flex items-center justify-between px-4 py-3 border-b border-white/6 last:border-0">
  <div class="flex flex-col">
    <span class="text-[13px] text-zinc-100">Launch at login</span>
    <span class="text-[11px] text-zinc-400">Vox will start quietly in your menu bar.</span>
  </div>
  <button role="switch" …></button>
</div>
```

---

### 3.5 Onboarding flow

**Window**

- Size: **560 × 520 px**, centered, non-resizable, traffic lights only (no maximize).
- Same vibrancy treatment as Settings. No sidebar — single pane with progress dots at the top.
- On close mid-flow, state is persisted; relaunch resumes.

**Shared chrome**

```
┌────────────── 560 ─────────────┐
│ ◉ ◉ ◉                          │
│                                │
│        ●  ○  ○  ○  ○           │  progress dots — filled = current, 4px dots 8 gap
│                                │
│                                │
│       [ large illustration     │  180 tall
│         or SF Symbol glyph ]   │
│                                │
│     Talk to any app.           │  display type
│     Vox types what you say.    │  body, secondary, max-w 360
│                                │
│                                │
│                                │
│    [Skip]           [Get started ▸]  │  footer, 48 bottom
└────────────────────────────────┘
```

**5 screens**

1. **Welcome.** Mic on soft accent-gradient disc. "Talk to any app." / "Vox types what you say — in Slack, Cursor, Gmail, anywhere." Primary: `Get started`. No skip.
2. **Microphone.** Mic glyph. "Let Vox hear you." / "Vox only captures audio while you hold the hotkey. Nothing is saved to disk." Button: `Allow microphone` (triggers OS prompt). Live status: `○ Waiting…` → `✓ Granted`. Primary enabled after grant; skip disabled.
3. **Accessibility.** Keyboard glyph. "Let Vox type for you." / "Vox needs Accessibility to send the finished text to the app you're using." Button: `Open System Settings` (deep-link). Poll status every 500ms. Primary `Continue` disabled until granted.
4. **API keys.** Key glyph. "Bring your own keys." / "Vox runs on your accounts. Keys stay in your Keychain." Groq + Anthropic fields with inline `Test` (green on success). `Get a key ↗` helper link under each. Primary `Continue` disabled until both valid; skip = "Set up later".
5. **Try it now.** Hand-holding-`⌥` glyph. "Try it." / "Hold `Right ⌥` and say something. Your words will appear below." Large in-window text field (2 rows, mono body). On first successful paste: green flash + corner check, auto-advance. Primary `Finish` (or `Skip for now` until any success).

**Success micro-screen** (1.2s auto-dismiss): check glyph + "You're set. Vox lives in your menu bar."

**Interactions.** `Enter` advances (when primary enabled). `Esc` closes (persists state). Back chevron top-left on screens 2–5. Progress dots are indicators, not interactive.

**Tailwind — primary button**

```html
<button class="inline-flex items-center gap-1.5 h-9 px-4 rounded-[8px]
               bg-accent text-white text-[13px] font-medium
               shadow-[inset_0_0_0_1px_rgb(255_255_255/0.12),0_1px_2px_rgb(0_0_0/0.24)]
               hover:brightness-110 active:brightness-95
               disabled:opacity-40 disabled:cursor-not-allowed
               transition-[filter] duration-80">
```

---

### 3.6 Error / permission states

All errors surface in **two** places per PRD 7.6: the pill (transient, 2s) and the tray icon (persistent badge). Settings/Onboarding also show them in-context.

**Matrix**

Condition — Tray / Pill on hotkey / Settings badge / Onboarding block:

- Mic denied — amber `!` / "No mic access — open Settings" / General: red banner + "Grant…" / screen 2 blocked.
- Accessibility denied — amber `!` / "Can't type here — grant Accessibility" / General: red banner + deep-link / screen 3 blocked.
- API key missing — amber `!` / "Add your Groq key" / API Keys card outlined red / screen 4 blocked.
- API key invalid — red / "API key invalid" / inline under field / inline on test.
- Network down — red / "Offline — Vox needs internet" / About: "Last checked offline" / screen 4 warn.
- LLM timeout — red brief / "Polish skipped" / — / —.
- Audio device gone — amber / "No input device" / General: picker highlighted / —.
- Paste target unknown — — / "Copied — ⌘V to paste" / — / —.

**Visual rules**

- Red banner: `bg-danger/12 ring-1 ring-danger/30 text-danger rounded-[10px] px-4 py-3`.
- Amber banner: same with `--color-warning`.
- Icon inside banner: 16px from lucide (`alert-triangle`, `mic-off`, `wifi-off`, `key-round`, `lock`).
- Primary CTA inside banner: text button in matching hue, underline on hover.

---

## 4. Microcopy

Voice: terse, first-person-plural avoided, no exclamations except one (`You're set.`). Sentence case. No emoji.

**Global**

- App name: `Vox`
- Tagline (About): `Talk to any app.`

**Tray / pill status**

- Ready (tray): `Listening ready`
- Recording: `Listening…`
- Transcribing: `Transcribing…`
- Polishing: `Polishing…`
- Done: `Done`
- Canceled: `Canceled`
- Hands-free: `Hands-free`
- Offline: `Offline`
- Paste fallback: `Copied — press ⌘V to paste`

**Errors (pill short / tray tooltip long)**

- Mic denied: `No mic access` / `Vox can't use your microphone. Open System Settings to allow it.`
- Accessibility denied: `Can't type here` / `Vox needs Accessibility permission to send text to other apps.`
- API key missing: `Add your Groq key` / `Open Settings and paste a Groq API key to start.`
- API key invalid: `API key invalid` / `Your Groq key was rejected. Check it in Settings.`
- Network: `Offline` / `Vox needs an internet connection to transcribe.`
- LLM timeout: `Polish skipped` / `The cleanup step timed out. We pasted the raw transcript.`
- No input device: `No input device` / `No microphone was detected. Connect or select one in System Settings → Sound.`
- Paste target unknown: `Copied — ⌘V to paste` / `We couldn't find a text field, so the transcript is on your clipboard.`

**Permission prompts (in onboarding)**

- Mic title: `Let Vox hear you.`
- Mic body: `Vox only captures audio while you hold the hotkey. Nothing is saved to disk.`
- Mic button: `Allow microphone`
- Accessibility title: `Let Vox type for you.`
- Accessibility body: `Vox needs Accessibility to send the finished text to the app you're using.`
- Accessibility button: `Open System Settings`
- API keys title: `Bring your own keys.`
- API keys body: `Vox runs on your accounts. Keys stay in your Keychain.`

**Buttons**

- `Get started`, `Continue`, `Back`, `Skip for now`, `Finish`
- `Allow microphone`, `Open System Settings`, `Test`, `Show`, `Hide`, `Copy`, `Change…`, `Remove`, `Add`
- `Settings…`, `Quit`, `Check for updates`

**Empty states**

- Recents empty: `Your last 5 transcripts will appear here.`
- Dictionary empty: `Add names or technical words Vox should spell correctly.`
- No pinned language: `Auto — Vox detects the language.`

**Confirm modals**

- Delete data title: `Delete everything?`
- Delete data body: `Your settings, dictionary, and saved API keys will be removed. This can't be undone.`
- Delete buttons: `Delete` (danger) / `Keep`

**Hotkey reminders (popover)**

- Row: `Hold ⌥ (Right Option) — Speak`
- Row: `⌘⇧V — Paste last`
- Row: `Esc — Cancel`

---

## 5. Accessibility

**Contrast.** Body text ≥ 4.5:1 in both modes. Pill text on translucent bg uses `font-medium` + solid foreground to maintain ratio despite blur. Accent on surface ≥ 3:1 for icons; ≥ 4.5:1 for any accent-colored text (rare — only "Get a key ↗"). Danger/warning banners: solid 800/400 text tone over low-alpha bg.

**Keyboard navigation.** Tray popover, Settings, and Onboarding fully keyboard-reachable. Focus ring: `outline-2 outline-offset-2 outline-accent`. Settings sidebar: `↑/↓` selects, `Enter` enters main pane. Onboarding: `Enter` advances, `Esc` closes. The pill is non-interactive and excluded.

**VoiceOver labels.**

- Tray icon: `Vox, {state}. Activate to open menu.` ({state} = Ready / Recording / Processing / Error / Permission needed / Off)
- Pill: `role="status" aria-live="polite"` — announces "Listening", "Transcribing", "Polishing", "Done", "Error: …" on state change.
- Toggle: `Dictation, {on|off}`
- API key input: `{Provider} API key. {Status}.` (Status = Not set / Stored / Invalid).
- Keycap pill: `aria-label="Right Option"` (expand glyphs to words).
- Waveform canvas: `aria-hidden="true"` (decorative; text label carries meaning).

**Reduced motion** (`prefers-reduced-motion: reduce`). Pill `pulse-mic` → static glyph. `pill-in/out` snap (no translate/scale). Spinners → single dot fading 1 → 0.4 → 1 on 1.5s. `flash-done` + onboarding check: instant. Waveform still updates (real data).

**Other.** All hotkeys rebindable (PRD 6.7). Settings layout respects system text size up to `xxLarge` (flex, not fixed row heights). No state is color-only — every state has icon + text.

---

## 6. Asset list

All icons from **lucide-solid** unless flagged custom. Stroke `1.5`, `size={16}` default in UI, `size={20}` in pill, `size={14}` in sidebar.

**Lucide icons used** (exact lucide names):

- `mic`, `mic-off` — pill + tray, permission
- `activity` — (fallback waveform glyph, unused if canvas works)
- `loader-2` — spinner (apply `animate-[spinner_900ms_linear_infinite]`)
- `check`, `check-circle-2` — success
- `x`, `circle-x` — cancel / close
- `alert-triangle`, `alert-circle` — error
- `wifi-off`, `cloud-off` — offline
- `key-round` — API keys tab, missing-key state
- `lock`, `shield` — privacy tab
- `settings`, `settings-2` — settings tab + gear buttons
- `sliders-horizontal` — general tab
- `command`, `keyboard` — hotkeys tab
- `book-open-text` — dictionary tab
- `info` — about tab
- `arrow-right`, `chevron-right`, `chevron-left` — nav
- `copy` — copy recent transcript
- `eye`, `eye-off` — show/hide API key
- `plus`, `trash-2` — dictionary add/remove
- `sparkles` — LLM polishing stage accent (optional — or use custom `wand`)
- `moon`, `sun` — theme toggle (About tab, if we expose override)
- `refresh-cw` — check for updates
- `external-link` — doc links

**Custom SVGs required** (hand-authored):

- `tray-idle.svg` (18×18 template) — rounded-rect mic body 10×6, 3px-radius stand, 2px stem. Alpha-only.
- `tray-rec.svg` — idle + 3px solid circle bottom-right (Rust overlays red; or animate opacity).
- `tray-proc.svg` — mic outline + 270° arc, stroke 1.25.
- `tray-err.svg` — mic outline + filled-circle badge with `!` cut-out bottom-right.
- `tray-perm.svg` — same geometry as err, separate asset for clean runtime swap.
- `tray-off.svg` — mic with diagonal slash.
- `waveform-flat.svg` — static placeholder (24 bars @ 2px) before first RMS frame.
- Onboarding glyphs, 120×120, flat-fill, accent tint: `onboard-welcome.svg` (mic disc + radial gradient), `onboard-mic.svg`, `onboard-access.svg` (keyboard + cursor arrow), `onboard-keys.svg` (key-round in a ring), `onboard-try.svg` (hand holding `⌥`), `onboard-success.svg` (check in a ring).
- `vox-wordmark.svg` — "Vox" set in SF Pro Display Semibold, outlined, 96×32. Used on About tab + Welcome.
- `AppIcon.icns` — standard macOS set; cue: soft accent-gradient rounded-square with a mic cut-out. (Out of scope here beyond noting it must exist.)

**File locations.** Tray templates: `/src-tauri/icons/tray/*.png` (1× + 2×, generated from the SVGs via `svgexport` in build). Onboarding + wordmark: `/src/assets/art/*.svg`, imported as components. No raster assets for UI chrome — SVG/canvas only.

---

End of spec. Questions or revisions go in the engineer thread, not inline here.
