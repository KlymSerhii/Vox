import { Show } from "solid-js";

export function Row(props: {
  label: string;
  hint?: string;
  children: any;
  last?: boolean;
}) {
  return (
    <div
      class="flex items-center justify-between gap-4 px-4 py-3"
      classList={{
        "border-b border-[var(--color-border)]": !props.last,
      }}
    >
      <div class="flex min-w-0 flex-1 flex-col">
        <span class="text-[13px] text-[var(--color-ink)]">{props.label}</span>
        <Show when={props.hint}>
          <span class="text-[11px] text-[var(--color-ink-mute)]">
            {props.hint}
          </span>
        </Show>
      </div>
      <div class="flex shrink-0 items-center gap-2">{props.children}</div>
    </div>
  );
}
