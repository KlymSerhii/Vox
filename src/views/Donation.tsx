import { createSignal } from "solid-js";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { HeartGlyph } from "@/components/HeartGlyph";
import { donationOptOut, openExternalUrl } from "@/lib/ipc";
import { deviceWord } from "@/lib/platform";
import { formatDuration } from "@/views/tray-popover/helpers";
import { DONATE_URL } from "@/views/tray-popover/constants";

/**
 * Dedicated milestone-donation window. Auto-appears once after a
 * dictation completes when the user crosses a time-saved tier (1h / 10h /
 * 50h); the Rust side marks the tier shown before this renders, so the
 * window fires exactly once per tier even if the user just closes it.
 *
 * Mirrors the tray popover's `DonateBanner` — same quantified-value copy
 * (`HeartGlyph`, accent-soft card, no new gradients/shadows) and the same
 * three guilt-free actions — scaled up to fill the pop-up window:
 *
 *   - Support Vox       → open the donation page, then opt out of future
 *                         nudges (engaging with donating stops them).
 *   - I already donated → honour-system permanent dismissal.
 *   - Maybe later       → just close; the backend already recorded this
 *                         tier as shown, so no snooze is needed.
 *
 * `saved_secs` arrives via the `?s=` query param the Rust window builder
 * appends, so the copy renders without re-fetching (the gating prompt
 * returns `null` once the tier is marked shown).
 */
export default function DonationView() {
  const savedSecs = Number(new URLSearchParams(location.search).get("s")) || 0;
  // Guard against double-fires from an impatient double-click while the
  // best-effort awaits are in flight.
  const [closing, setClosing] = createSignal(false);

  function close() {
    void getCurrentWindow().close();
  }

  async function support() {
    if (closing()) return;
    setClosing(true);
    // Best-effort: open the page and opt out, but always close even if
    // either step fails — same semantics as the banner's nudge action.
    try {
      await openExternalUrl(DONATE_URL);
    } catch (err) {
      console.warn("donation: open page failed", err);
    }
    try {
      await donationOptOut();
    } catch (err) {
      console.warn("donation: opt-out failed", err);
    }
    close();
  }

  async function alreadyDonated() {
    if (closing()) return;
    setClosing(true);
    try {
      await donationOptOut();
    } catch (err) {
      console.warn("donation: opt-out failed", err);
    }
    close();
  }

  return (
    <div class="flex h-screen w-screen flex-col items-center justify-center gap-6 bg-[var(--color-bg)] px-8 py-10 text-center text-[var(--color-ink)]">
      <span
        class="inline-flex h-14 w-14 shrink-0 items-center justify-center rounded-full bg-[var(--color-accent-soft)] text-[var(--color-accent)] ring-1 ring-[var(--color-border)]"
        aria-hidden="true"
      >
        <HeartGlyph width={26} height={26} />
      </span>

      <div class="flex flex-col gap-2">
        <h1 class="text-[19px] font-semibold leading-tight">
          You've saved {formatDuration(savedSecs)} of typing
        </h1>
        <p class="max-w-[300px] text-[13px] leading-relaxed text-[var(--color-ink-mute)]">
          Vox is free and runs entirely on your {deviceWord}. If it's been useful,
          you can chip in to support development.
        </p>
      </div>

      <div class="mt-1 flex w-full max-w-[260px] flex-col items-stretch gap-2.5">
        <button
          type="button"
          class="inline-flex h-10 items-center justify-center rounded-[8px] bg-[var(--color-accent)] px-4 text-[13px] font-medium text-[var(--color-on-accent)] transition-[filter] hover:brightness-110 disabled:cursor-not-allowed disabled:opacity-40"
          disabled={closing()}
          onClick={() => void support()}
        >
          Support Vox
        </button>
        <button
          type="button"
          class="rounded-[8px] px-4 py-2 text-[12px] text-[var(--color-ink-mute)] transition-colors hover:text-[var(--color-ink)] disabled:cursor-not-allowed disabled:opacity-40"
          disabled={closing()}
          onClick={() => void alreadyDonated()}
        >
          I already donated
        </button>
        <button
          type="button"
          class="rounded-[8px] px-4 py-2 text-[12px] text-[var(--color-ink-mute)] transition-colors hover:text-[var(--color-ink)]"
          onClick={close}
        >
          Maybe later
        </button>
      </div>
    </div>
  );
}
