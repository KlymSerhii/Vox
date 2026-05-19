import { type InProgressEntry } from "@/lib/ipc";
import { stageLabel } from "../helpers";
import { Spinner } from "./Spinner";

export function InProgressRow(props: {
  entry: InProgressEntry;
  onCancel: () => void;
}) {
  return (
    <div class="group flex items-center gap-2 rounded-[8px] px-2 py-2 transition-colors hover:bg-[var(--color-overlay)]">
      <span aria-hidden="true" class="text-[14px] leading-none">
        {props.entry.mode_icon}
      </span>
      <Spinner />
      <span
        class="pill-shimmer flex-1 truncate text-[12px] font-medium leading-none"
        style={{
          "--shimmer-base": "var(--color-ink-mute)",
          "--shimmer-tint": "var(--color-violet, #a78bfa)",
          "letter-spacing": "0.01em",
        }}
      >
        {stageLabel(props.entry.stage)}
      </span>
      <span class="shrink-0 text-[11px] text-[var(--color-ink-faint)]">
        {props.entry.mode_name}
      </span>
      <button
        type="button"
        class="inline-flex h-7 w-7 items-center justify-center rounded p-1.5 text-[var(--color-ink-mute)] hover:bg-[var(--color-overlay)] hover:text-[var(--color-danger)]"
        title="Cancel session"
        aria-label="Cancel session"
        onClick={(e) => {
          e.stopPropagation();
          props.onCancel();
        }}
      >
        <svg
          width="14"
          height="14"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true"
        >
          <line x1="6" y1="6" x2="18" y2="18" />
          <line x1="6" y1="18" x2="18" y2="6" />
        </svg>
      </button>
    </div>
  );
}
