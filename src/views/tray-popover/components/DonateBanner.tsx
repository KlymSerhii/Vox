import { HeartGlyph } from "@/components/HeartGlyph";
import { deviceWord } from "@/lib/platform";

/**
 * Milestone donation nudge, shown in place of the passive `DonateRow`
 * once the user has crossed a time-saved tier (1h / 10h / 50h) and is
 * neither opted out nor snoozed. It leads with the quantified value Vox
 * has already delivered (the strongest, most honest conversion lever)
 * and offers three equal-weight, guilt-free exits:
 *
 *   - Support Vox       → opens the donation page (and opts out of
 *                         future nudges; the passive row remains).
 *   - I already donated → honour-system permanent dismissal — believed
 *                         on the user's word, no receipt, no friction.
 *   - Maybe later       → snooze; the next (higher) tier can re-arm
 *                         after the snooze floor.
 *
 * Visual language mirrors `UpdateBanner` (accent-soft card, ring, no new
 * gradients/shadows) but swaps the sparkle for `HeartGlyph` so it reads
 * as "support", not "fresh update". The caller supplies the surrounding
 * `px-4 py-3` section + `border-b`.
 */
export function DonateBanner(props: {
  savedLabel: string;
  onDonate: () => void;
  onAlreadyDonated: () => void;
  onLater: () => void;
}) {
  return (
    <div class="flex items-start gap-2.5 rounded-[8px] bg-[var(--color-accent-soft)] px-3 py-2.5 ring-1 ring-[var(--color-border)]">
      <span
        class="mt-0.5 inline-flex h-5 w-5 shrink-0 items-center justify-center rounded-full text-[var(--color-accent)]"
        aria-hidden="true"
      >
        <HeartGlyph width={14} height={14} />
      </span>
      <div class="flex min-w-0 flex-1 flex-col gap-1">
        <span class="text-[12px] font-semibold text-[var(--color-ink)]">
          You've saved {props.savedLabel} of typing
        </span>
        <span class="text-[11px] leading-snug text-[var(--color-ink-mute)]">
          Vox is free and runs entirely on your {deviceWord}. If it's been useful,
          you can chip in to support development.
        </span>
        <div class="mt-1 flex flex-wrap items-center gap-2">
          <button
            type="button"
            class="inline-flex shrink-0 items-center justify-center rounded-[6px] bg-[var(--color-accent)] px-2.5 py-1 text-[11px] font-medium text-[var(--color-on-accent)] hover:brightness-110"
            onClick={props.onDonate}
          >
            Support Vox
          </button>
          <button
            type="button"
            class="rounded-[6px] px-1.5 py-1 text-[11px] text-[var(--color-ink-mute)] hover:text-[var(--color-ink)]"
            onClick={props.onAlreadyDonated}
          >
            I already donated
          </button>
          <button
            type="button"
            class="rounded-[6px] px-1.5 py-1 text-[11px] text-[var(--color-ink-mute)] hover:text-[var(--color-ink)]"
            onClick={props.onLater}
          >
            Maybe later
          </button>
        </div>
      </div>
    </div>
  );
}
