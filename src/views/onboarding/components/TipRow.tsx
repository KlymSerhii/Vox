import { type JSX, Show } from "solid-js";

export function TipRow(props: {
  label: JSX.Element;
  description: string;
  fellBackNote?: string | null;
  errorAction?: (() => void) | undefined;
  errorActionLabel?: string | undefined;
}) {
  return (
    <li
      class="flex flex-col gap-1 rounded-[14px] bg-[var(--color-surface-raised)] px-3 py-2.5 ring-1 ring-[var(--color-border)]"
      style={{
        "box-shadow":
          "inset 0 1px 0 var(--glass-highlight), 0 1px 2px var(--glass-shadow-soft)",
      }}
    >
      <div class="flex items-baseline gap-2">
        <span class="text-[13px] font-medium text-[var(--color-ink)]">
          {props.label}
        </span>
        <span
          class="text-[13px]"
          classList={{
            "text-[var(--color-ink-mute)]": !props.errorAction,
            "text-[var(--color-danger)]": !!props.errorAction,
          }}
        >
          {props.description}
        </span>
      </div>
      <Show when={props.fellBackNote}>
        <span class="text-[11px] text-[var(--color-ink-faint)]">
          {props.fellBackNote}
        </span>
      </Show>
      <Show when={props.errorAction && props.errorActionLabel}>
        <button
          type="button"
          onClick={props.errorAction}
          class="self-start mt-1 rounded-[6px] bg-[var(--color-danger)]/15 px-2 py-1 text-[11px] text-[var(--color-danger)] hover:bg-[var(--color-danger)]/25"
        >
          {props.errorActionLabel}
        </button>
      </Show>
    </li>
  );
}
