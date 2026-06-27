import { Show } from "solid-js";

export function SectionHeader(props: { title: string; hint?: string }) {
  return (
    <div class="mt-1 mb-2 flex items-baseline justify-between gap-3 px-1">
      <h2 class="text-[11px] font-semibold uppercase tracking-[0.08em] text-[var(--color-ink-mute)]">
        {props.title}
      </h2>
      <Show when={props.hint}>
        <span class="text-[11px] text-[var(--color-ink-faint)]">
          {props.hint}
        </span>
      </Show>
    </div>
  );
}
