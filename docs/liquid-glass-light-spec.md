# Vox — Liquid Glass Light Theme Spec

Implementable counterpart to the dark palette. Tone: warm-neutral milky white that lets the desktop bleed through translucent surfaces. Tokens slot into `src/styles.css` under `[data-theme="light"]`.

## 1. Light palette tokens

| Token | Value | Notes |
|---|---|---|
| `--color-accent` | `#5B7BFF` | Darkened from `#7c9cff` for AA on white. |
| `--color-accent-soft` | `rgb(91 123 255 / 0.12)` | Hover/active fills. |
| `--color-violet` | `#8A6DFF` | Slightly deeper for legibility. |
| `--color-danger` | `#E5484D` | Reads on white. |
| `--color-success` | `#22A06B` | Replaces `#4ade80` (too pale on white). |
| `--color-warning` | `#D9870F` | AA on white. |
| `--color-ink` | `#1B1B1F` | Near-black with warmth. |
| `--color-ink-mute` | `#54545A` | Secondary labels. |
| `--color-ink-faint` | `#86868C` | Helper text. |
| `--color-bg` | `#F7F5F1` | Warm cream for opaque windows. |
| `--color-surface` | `rgb(255 252 247 / 0.62)` | Translucent cards. |
| `--color-surface-raised` | `rgb(255 253 250 / 0.78)` | Elevated cards. |
| `--color-border` | `rgb(15 15 20 / 0.08)` | Hairline divider. |
| `--color-border-strong` | `rgb(15 15 20 / 0.14)` | Field outlines. |

Two new light-only tokens for the glass effect:

- `--glass-highlight: rgb(255 255 255 / 0.7)` — 1px inset top edge.
- `--glass-shadow: rgba(40, 35, 60, 0.18)` — elevation, faintly cool to balance the warm fill.

## 2. Per-component treatment

**Settings (opaque).** Body `var(--color-bg)`. Sidebar `rgba(255, 253, 250, 0.6)` with `backdrop-filter: blur(20px)` and 1px right border `var(--color-border)`. Section cards `#FFFFFF`, `1px solid var(--color-border)`, `border-radius: 14px`, `box-shadow: 0 1px 2px rgba(15,15,20,0.04)`. Bump global card radius from 10/12px to 14px.

**TrayPopover (translucent).** Replace inline `rgba(24,24,27,0.88)` with `rgba(255, 252, 247, 0.72)`. Bump filter to `blur(36px) saturate(180%)`. Add `box-shadow: inset 0 1px 0 var(--glass-highlight), 0 12px 40px var(--glass-shadow)`. Inner sections (recents, mode picker) use `--color-surface-raised`. Border `1px solid var(--color-border)`.

**Pill (translucent floating).** Replace `bg-zinc-800/70` with `background: rgba(255, 252, 247, 0.78)`, `backdrop-filter: blur(32px) saturate(180%)`, `box-shadow: inset 0 1px 0 var(--glass-highlight), 0 8px 24px var(--glass-shadow)`, `border: 1px solid var(--color-border)`. Text `--color-ink`. State swatches (Listening red, Done green) keep their saturated dark-mode hue so they remain unmistakable over arbitrary backgrounds; only chrome lightens. Audio meter bars: `--color-accent` at 0.85 alpha.

**Onboarding (opaque).** Body `var(--color-bg)`. Step cards `#FFFFFF` with the same 14px radius and shadow as Settings. Progress dots: filled `--color-accent`, empty `rgb(15 15 20 / 0.12)`.

## 3. Theme toggle UI

Lives in the TrayPopover footer, alongside the dictation toggle, above Settings/Quit. Two-state segmented pill: ~64×24px, `border-radius: 12px`, base `rgb(15 15 20 / 0.06)`. Two equal halves with inline 14×14 SVG glyphs (sun: 8 rays + center dot; moon: crescent via two offset circles), `currentColor` stroke 1.5px, matching existing tray glyph weight. Active half: `background: var(--color-surface-raised)`, `box-shadow: 0 1px 2px rgba(15,15,20,0.08)`, glyph `--color-ink`. Inactive: glyph `--color-ink-faint`. Left-aligned label "Appearance" in `--color-ink-mute`. Persist via new `appearance: 'light' | 'dark'` field on `Settings` (add `'system'` as a follow-up).

## 4. Title bar / window chrome

Yes — call `Window::set_theme(Some(Theme::Light | Theme::Dark))` on Settings and Onboarding when the user toggles. Pill and TrayPopover are decorationless so this is moot for them. Add `commands::set_appearance` that updates the setting, calls `set_theme` on every labelled window, and emits `appearance:changed` so all webviews flip `<html data-theme>` simultaneously. Without this, macOS title bar buttons stay dark while contents go light — looks broken.

## 5. Per-HTML `<meta name="color-scheme">`

**Keep static + override at runtime.** Change all four HTML files from `dark` to `dark light` so form controls, scrollbars, and selection highlights don't fight the theme. The active variant is selected by `<html data-theme="light|dark">` written before paint from the persisted setting and updated on `appearance:changed`. A pure-CSS `@media (prefers-color-scheme)` approach loses the in-app override.

## 6. Accent color

`#7c9cff` fails AA on white (~2.9:1). Use `#5B7BFF` for light mode (≈4.6:1 on `#F7F5F1`); keep `#7c9cff` for dark. Add `--color-accent-strong: #3D5BFF` for filled buttons with white text — clears AA at 14px regular and reads well on dark too.

## 7. Liquid Glass flourishes (two, restrained)

**A. Inset top highlight on every translucent surface.** Add `box-shadow: inset 0 1px 0 var(--glass-highlight)` to TrayPopover, Pill, and the Settings sidebar. The 1px white sliver at the top edge is the signature Liquid Glass "lift" — light catching a beveled glass rim. Free in render cost.

**B. Tinted elevation shadow under TrayPopover only.** `box-shadow: 0 16px 48px -12px rgba(60, 50, 90, 0.22), 0 4px 12px rgba(40, 35, 60, 0.08)`. The faint violet (vs neutral grey) keeps the popover from feeling stamped flat onto the desktop and harmonizes with `--color-violet`. Pill keeps a smaller neutral shadow to stay unobtrusive over arbitrary content.

No gradient sheens, no animated specular — restraint is the aesthetic.
