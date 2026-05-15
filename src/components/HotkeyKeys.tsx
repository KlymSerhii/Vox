import { For, Show } from "solid-js";

import { formatHotkeyTokens } from "../lib/hotkey";

/**
 * Render a hotkey spec as a row of keycap badges. Modifier badges
 * show both the macOS glyph and a short word — `⌘ Cmd`, `⌥ Opt`,
 * `⇧ Shift`, `⌃ Ctrl` — so the chord is legible to users who don't
 * recognise the symbols. Trailing key glyphs (`.`, `M`, `?`) render
 * on their own.
 *
 * `Mods:Cmd+Alt` renders as the bare modifiers; `Tap2:Alt` renders
 * the modifier followed by a small `×2` suffix attached to the last
 * keycap so the double-tap is visually compact.
 */
export function HotkeyKeys(props: { spec: string | null | undefined }) {
  const tokens = () => formatHotkeyTokens(props.spec);
  return (
    <span class="inline-flex items-center gap-1 align-middle">
      <For each={tokens()}>
        {(t, i) => (
          <Show
            when={t.kind !== "tap2"}
            fallback={
              <span class="text-[9px] font-medium text-[var(--color-ink-faint)]">
                {t.label}
              </span>
            }
          >
            <Show when={i() > 0 && tokens()[i() - 1].kind !== "tap2"}>
              <span class="text-[10px] text-[var(--color-ink-faint)]">+</span>
            </Show>
            <span class="keycap">
              <span aria-hidden="true">{t.symbol}</span>
              <Show when={t.label}>
                <span class="text-[10px] font-medium text-[var(--color-ink-mute)]">
                  {t.label}
                </span>
              </Show>
            </span>
          </Show>
        )}
      </For>
    </span>
  );
}
