import { DiscordGlyph } from "@/components/DiscordGlyph";

/**
 * Permanent "Join the Discord" community row, pinned directly below the
 * "Support Vox" surface. It's the entry point to the user community —
 * where people request features, report bugs, and help each other out.
 * Clicking it just opens the Discord invite; it carries no nudge/opt-out
 * state of its own.
 *
 * Mirrors `DonateRow` exactly (full-width hover button, leading accent
 * glyph, two-line label, trailing external-link arrow) so the two rows
 * read as a single stacked "support & community" group; the caller
 * supplies the surrounding `border-b`.
 */
export function CommunityRow(props: { onClick: () => void }) {
  return (
    <button
      type="button"
      class="flex w-full items-center gap-2.5 px-4 py-3 text-left transition-colors hover:bg-[var(--color-overlay)]"
      onClick={props.onClick}
      title="Join the Vox community on Discord"
    >
      <span
        class="inline-flex h-5 w-5 shrink-0 items-center justify-center text-[var(--color-accent)]"
        aria-hidden="true"
      >
        <DiscordGlyph width={15} height={15} />
      </span>
      <span class="flex min-w-0 flex-1 flex-col">
        <span class="text-[13px] font-semibold text-[var(--color-ink)]">
          Join the Discord
        </span>
        <span class="text-[11px] leading-snug text-[var(--color-ink-mute)]">
          Request features, report bugs, or say hi.
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
