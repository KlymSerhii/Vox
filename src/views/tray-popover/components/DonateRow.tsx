import { HeartGlyph } from "@/components/HeartGlyph";

/**
 * Permanent, low-key "Support Vox" row, pinned directly below the Time
 * saved card. Always present — it's the passive tip-jar entry that lets
 * motivated users give without ever seeing the milestone nudge, and it
 * stays put even after the user has opted out of nudges (so repeat
 * giving is always one click away). Clicking it just opens the donation
 * page; it deliberately does NOT touch nudge state, so the higher-
 * converting milestone moment stays intact for everyone else.
 *
 * Full-width hover button matching the popover's "Quick Settings"
 * header affordance (`px-4 py-3`, hover overlay); the caller supplies
 * the surrounding `border-b`.
 */
export function DonateRow(props: { onClick: () => void }) {
  return (
    <button
      type="button"
      class="flex w-full items-center gap-2.5 px-4 py-3 text-left transition-colors hover:bg-[var(--color-overlay)]"
      onClick={props.onClick}
      title="Support Vox"
    >
      <span
        class="inline-flex h-5 w-5 shrink-0 items-center justify-center text-[var(--color-accent)]"
        aria-hidden="true"
      >
        <HeartGlyph width={15} height={15} />
      </span>
      <span class="flex min-w-0 flex-1 flex-col">
        <span class="text-[13px] font-semibold text-[var(--color-ink)]">
          Support Vox
        </span>
        <span class="text-[11px] leading-snug text-[var(--color-ink-mute)]">
          Free &amp; fully on-device — chip in if it helps.
        </span>
      </span>
      <svg
        width="14"
        height="14"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="1.75"
        stroke-linecap="round"
        stroke-linejoin="round"
        class="shrink-0 text-[var(--color-ink-mute)]"
        aria-hidden="true"
      >
        <path d="M7 17 17 7M9 7h8v8" />
      </svg>
    </button>
  );
}
