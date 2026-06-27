import { createMemo } from "solid-js";
import { TABS, type Tab } from "../constants";

export function Header(props: { tab: Tab; savedFlash: boolean }) {
  const meta = createMemo(() => TABS.find((t) => t.id === props.tab));
  return (
    <header class="mb-5 flex items-end justify-between gap-4 border-b border-[var(--color-border)] pb-4">
      <div class="flex flex-col">
        <h1 class="text-[20px] font-semibold tracking-[-0.015em] text-[var(--color-ink)]">
          {meta()?.label ?? ""}
        </h1>
        <p class="mt-0.5 text-[12px] text-[var(--color-ink-mute)]">
          {meta()?.subtitle ?? ""}
        </p>
      </div>
      <div class="flex h-[22px] items-center">
        <span
          class="inline-flex items-center gap-1 rounded-full bg-[var(--color-success)]/15 px-2 py-0.5 text-[11px] font-medium text-[var(--color-success)] transition-opacity duration-200"
          classList={{
            "opacity-100": props.savedFlash,
            "opacity-0 pointer-events-none": !props.savedFlash,
          }}
          aria-live="polite"
        >
          <svg
            width="11"
            height="11"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="3"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
          >
            <polyline points="5 12 10 17 19 7" />
          </svg>
          Saved
        </span>
      </div>
    </header>
  );
}
