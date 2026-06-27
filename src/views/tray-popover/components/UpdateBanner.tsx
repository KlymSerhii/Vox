import { Show } from "solid-js";
import { SparkleGlyph } from "@/components/SparkleGlyph";

/**
 * Pinned row above the Recent list in the tray popover. Renders only
 * while an update is available AND not "Later"-dismissed for the
 * current version (caller gates with `state.banner_visible` /
 * `dismissed_version`). Reuses the same accent-coloured affordance
 * the rest of the app uses for "polish-ish" interactions —
 * `SparkleGlyph` from `src/components/SparkleGlyph.tsx` — so the
 * cue reads as a Vox-branded "fresh thing arrived" moment rather
 * than a generic notification.
 *
 * Colour tokens: only `--color-accent`, `--color-accent-soft`,
 * `--color-on-accent`, `--color-ink`, `--color-ink-mute`. No new
 * gradients or shadows — the existing `liquid-card` rim from the
 * surrounding popover sells the elevation; the banner sits inside
 * the same border-bordered surface as the Recent list, just
 * pinned above it.
 */
export function UpdateBanner(props: {
  availableVersion: string;
  installing: boolean;
  onInstall: () => void;
  onLater: () => void;
}) {
  return (
    <div class="mx-2 mt-2 mb-1 flex items-start gap-2.5 rounded-[8px] bg-[var(--color-accent-soft)] px-3 py-2.5 ring-1 ring-[var(--color-border)]">
      <span
        class="mt-0.5 inline-flex h-5 w-5 shrink-0 items-center justify-center rounded-full text-[var(--color-accent)]"
        aria-hidden="true"
      >
        <SparkleGlyph width={14} height={14} />
      </span>
      <div class="flex min-w-0 flex-1 flex-col gap-1">
        <span class="truncate text-[12px] font-semibold text-[var(--color-ink)]">
          Update available — v{props.availableVersion}
        </span>
        <div class="flex items-center gap-2">
          <button
            type="button"
            class="inline-flex shrink-0 items-center justify-center rounded-[6px] bg-[var(--color-accent)] px-2.5 py-1 text-[11px] font-medium text-[var(--color-on-accent)] hover:brightness-110 disabled:cursor-wait disabled:opacity-70"
            onClick={props.onInstall}
            disabled={props.installing}
          >
            <Show when={!props.installing} fallback={<span>Installing…</span>}>
              <span>Install</span>
            </Show>
          </button>
          <button
            type="button"
            class="rounded-[6px] px-1.5 py-1 text-[11px] text-[var(--color-ink-mute)] hover:text-[var(--color-ink)]"
            onClick={props.onLater}
            disabled={props.installing}
          >
            Later
          </button>
        </div>
      </div>
    </div>
  );
}
