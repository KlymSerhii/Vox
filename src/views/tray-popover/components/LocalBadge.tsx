import { openSettingsWindow } from "@/lib/ipc";
import { deviceWord } from "@/lib/platform";

export function LocalBadge() {
  // Quiet reminder, no popover, no tooltip on the badge itself —
  // if the user cares they'll click through to the Privacy tab.
  // DESIGN.md §5.5.
  return (
    <button
      type="button"
      class="inline-flex items-center gap-1.5 rounded-full px-2 py-1 text-[10px] font-semibold uppercase tracking-[0.08em] text-[var(--color-ink-mute)] transition-colors hover:bg-[var(--color-overlay)] hover:text-[var(--color-ink)]"
      onClick={() => openSettingsWindow()}
      title={`Audio never leaves this ${deviceWord}`}
    >
      <svg
        width="11"
        height="11"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
        stroke-linecap="round"
        stroke-linejoin="round"
        aria-hidden="true"
      >
        <path d="M12 21s-7-4-7-10V6l7-3 7 3v5c0 6-7 10-7 10z" />
      </svg>
      <span>Local</span>
    </button>
  );
}
