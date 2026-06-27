import { PrewarmBars } from "./PrewarmBars";

/// Visible indicator that bridges the silent gap between every download
/// bar reading "Done" and the pre-warm finishing. Mirrors the liquid-card
/// surface used elsewhere on this step; the 3-bar audio-EQ loader is the
/// same voice/audio language as the recording pill, so it reads as "the
/// app is listening to its own models warm up" rather than a generic
/// spinner. The shimmer on the headline ties the wait to the pill's
/// `pill-shimmer` processing-state language without coupling to it.
export function PrewarmCard() {
  return (
    <div
      role="status"
      aria-live="polite"
      class="liquid-card mt-1 flex items-center gap-3 rounded-[10px] bg-[var(--color-surface-raised)] px-3 py-2.5 ring-1 ring-[var(--color-border)]"
    >
      <PrewarmBars />
      <div class="flex min-w-0 flex-col gap-0.5">
        <span class="pill-shimmer text-[13px] font-medium leading-tight">
          Warming up models…
        </span>
        <span class="text-[11px] leading-tight text-[var(--color-ink-mute)]">
          First launch only — usually 15-30 seconds.
        </span>
      </div>
    </div>
  );
}
