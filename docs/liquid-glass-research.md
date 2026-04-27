# Liquid Glass — research notes

Synthesised from Apple WWDC25 / HIG materials and post‑June‑2025 CSS recipes
(CSS‑Tricks, LogRocket, kube.io, ekino, Atlas Pup Labs, dev.to/kevinbism,
mycatwrotethis, nikdelvin/liquid-glass, naughtyduk/liquidGL).

## Translucency

Apple publishes no exact alphas. Sampling Tahoe / iOS 26 shots (Macworld,
TidBITS) and the recipes that match them most closely:

- **Panels / sidebars over wallpaper**: white tint at **0.10–0.18 alpha**.
  Atlas Pup ~0.15, Kevin Bism 0.15, LogRocket 0.05 + a heavy inset highlight.
- **Bubble toolbars / pill buttons**: **0.20–0.30** for hit‑target legibility.
- **HIG "thick" alerts/popovers**: **0.30–0.45** — only band where Vox's
  current 0.6–0.78 isn't crazy.

HIG tiers: `ultraThin / thin / regular / thick / chrome`. Sidebar = `regular`.
Toolbar = `thick`. Vox uses thick‑band alpha for sidebars; that's the core
mistake.

## Blur + filter stack

Authentic Liquid Glass is **moderate Gaussian + strong saturate**, not heavy
blur. Blur lets shape through; saturate keeps the desktop *coloured* rather
than greyed; an optional contrast pull‑down stops it going milky.

- Atlas Pup: `blur(20px) contrast(80%) saturate(120%)`.
- ekino: `blur(10px) brightness(0.8)` + SVG displacement.
- Kevin Bism: `blur(2px) saturate(180%)` + `blur(1px)` highlight pseudo.
- LogRocket / DesignFast: `blur(8px)` only — reads flat, not the target.

**Recommended:** `blur(24px) saturate(180%) contrast(92%)`. Vox's `blur(36px)
saturate(180%)` over‑blurs (kills wallpaper read‑through) and lacks contrast.

## Shadow stack

Liquid Glass shadows are deeper, larger, slightly cool‑tinted vs classic
frosted glass; alpha stays restrained.

- Atlas Pup (multi‑direction inner specular, no outer):
  ```
  inset 10px 10px 20px rgba(153,192,255,0.10),
  inset  2px  2px  5px rgba(195,218,255,0.20),
  inset -10px -10px 20px rgba(229,253,190,0.10),
  inset -2px -2px 30px rgba(247,255,226,0.20);
  ```
- LogRocket: `0 6px 24px rgba(0,0,0,0.20)` + `inset 0 0 20px -5px rgba(255,255,255,0.60)`.
- Kevin Bism: `0 8px 32px rgba(31,38,135,0.20)` (cool‑tinted ambient).

**Target stack (light, panel‑sized):**
```
0  1px  2px rgba(20,24,40,0.06),  /* contact */
0  8px 24px rgba(20,24,40,0.12),  /* mid lift */
0 28px 64px rgba(20,24,40,0.22);  /* ambient */
```
Vox's `0 16px 48px rgba(40,35,60,0.18)` is roughly layer 2 only — missing
contact line and ambient.

## Edge treatments

Three discrete elements; the bright top inner edge is load‑bearing.

1. **Top inner highlight (rim light):**
   `inset 0 1px 0 rgba(255,255,255,0.75)` (Kevin Bism 0.75, Atlas Pup ~0.6).
2. **Inner darker bottom edge:** `inset 0 -1px 0 rgba(20,24,40,0.10)`.
3. **1px outer rim:** `border: 1px solid rgba(255,255,255,0.55)` on light, or
   `outline: 1px solid rgba(255,255,255,0.35); outline-offset: -1px;` so the
   highlight composites outside the radius.

Stack all three. Vox has none of them.

## Color tint

Apple's material is **not** pure white — the system samples wallpaper and
shifts hue/saturation. In static CSS, approximate with warm‑neutral (light)
or cool‑neutral (dark). Vox's `rgba(255,252,247,…)` cream base is fine; just
lower the alpha.

- Light: `rgba(252,250,247,0.16)` panel / `rgba(252,250,247,0.28)` raised.
- Dark: `rgba(28,30,38,0.36)` panel / `rgba(28,30,38,0.55)` raised. Slight
  blue‑grey reads as glass over wallpaper.

## Refraction effects

- **CSS‑only refraction is not real.** Needs SVG `feDisplacementMap` of
  turbulence (`baseFrequency≈0.008 0.008, numOctaves=2, scale 30–77`)
  referenced via `backdrop-filter: url(#filter)`. **Chromium‑only.** Safari /
  WKWebView ignores SVG in `backdrop-filter`. **Tauri = WKWebView = no go.**
- **Chromatic aberration**: three `feColorMatrix` channel filters offset
  ~3–6px each (R −3, G 0, B +3, recompose). Same Chromium‑only constraint.
- **For Vox: drop SVG.** Fake the rim cue with the inset highlight + a
  `::before` `linear-gradient(135deg, rgba(255,255,255,0.18) 0%, transparent 40%)`.

## Reference images

- [Apple newsroom hero](https://www.apple.com/newsroom/2025/06/apple-introduces-a-delightful-and-elegant-new-software-design/) — canonical Control Center / Lock Screen panels; very low‑alpha tint, strong wallpaper bleed.
- [Apple macOS Tahoe page](https://www.apple.com/os/macos/) — sidebar + toolbar over wallpaper showing the rim highlight.
- [TidBITS comparison](https://tidbits.com/2025/10/09/how-to-turn-liquid-glass-into-a-solid-interface/) — default vs Reduce Transparency side‑by‑side; shows how *little* tint Apple uses.
- [Macworld tone‑down](https://www.macworld.com/article/2858361/how-to-reduce-the-liquid-glass-transparency-effect-in-macos-tahoe.html) — menu‑bar near‑clear default.
- [iDownloadBlog Control Center](https://www.idownloadblog.com/2025/06/27/customize-mac-control-center/) — clean popover shots.

## CSS recipes

**A. Authentic panel — recommended baseline:**
```css
background: rgba(252,250,247,0.16);
backdrop-filter: blur(24px) saturate(180%) contrast(92%);
-webkit-backdrop-filter: blur(24px) saturate(180%) contrast(92%);
border-radius: 18px;
border: 1px solid rgba(255,255,255,0.45);
box-shadow:
  inset 0  1px 0 rgba(255,255,255,0.75),
  inset 0 -1px 0 rgba(20,24,40,0.08),
  0  1px  2px rgba(20,24,40,0.06),
  0  8px 24px rgba(20,24,40,0.12),
  0 28px 64px rgba(20,24,40,0.22);
```

**B. Bubble toolbar / pill button:**
```css
background: rgba(255,255,255,0.28);
backdrop-filter: blur(18px) saturate(200%);
border-radius: 999px;
border: 1px solid rgba(255,255,255,0.55);
box-shadow:
  inset 0 1px 0 rgba(255,255,255,0.85),
  0 1px  2px rgba(20,24,40,0.10),
  0 6px 16px rgba(20,24,40,0.18);
```

**C. Atlas Pup multi‑specular (ornamental — hero card only):**
```css
background: rgba(255,255,255,0.18);
backdrop-filter: blur(20px) contrast(80%) saturate(120%);
box-shadow:
  inset 10px 10px 20px rgba(153,192,255,0.10),
  inset  2px  2px  5px rgba(195,218,255,0.20),
  inset -10px -10px 20px rgba(229,253,190,0.10),
  inset -2px -2px 30px rgba(247,255,226,0.20),
  0 28px 64px rgba(20,24,40,0.20);
```

**Recipe A is the most authentic match** to Apple's shipped surfaces.

## Recommendations for Vox

Token edits, light theme (`src/styles.css` lines 47–60):

| Token | Current | Change to |
| --- | --- | --- |
| `--color-surface` | `rgb(255 252 247 / 0.62)` | `rgba(252,250,247,0.18)` |
| `--color-surface-raised` | `rgb(255 253 250 / 0.78)` | `rgba(252,250,247,0.32)` |
| `--color-surface-glass` | `rgba(255,252,247,0.72)` | `rgba(252,250,247,0.22)` |
| `--color-sidebar` | `rgba(255,253,250,0.6)` | `rgba(252,250,247,0.14)` |
| `--glass-highlight` | `rgb(255 255 255 / 0.7)` | `rgba(255,255,255,0.75)` |
| `--glass-shadow` | `rgba(40,35,60,0.18)` | `rgba(20,24,40,0.22)` |
| `--glass-shadow-soft` | `rgba(40,35,60,0.08)` | `rgba(20,24,40,0.12)` |

Add tokens:
```
--glass-shadow-contact: rgba(20,24,40,0.06);
--glass-edge-bottom:    rgba(20,24,40,0.08);
--glass-blur:           blur(24px) saturate(180%) contrast(92%);
```

Apply on every glass panel:
```
backdrop-filter: var(--glass-blur);
border: 1px solid rgba(255,255,255,0.45);
box-shadow:
  inset 0  1px 0 var(--glass-highlight),
  inset 0 -1px 0 var(--glass-edge-bottom),
  0  1px  2px var(--glass-shadow-contact),
  0  8px 24px var(--glass-shadow-soft),
  0 28px 64px var(--glass-shadow);
```

Ship this and the light theme reads as Liquid Glass over wallpaper, not
"white card on grey." Skip SVG refraction — WKWebView won't render it.
