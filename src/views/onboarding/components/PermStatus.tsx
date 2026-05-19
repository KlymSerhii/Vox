import { Show } from "solid-js";

export function PermStatus(props: {
  granted: boolean;
  grantedLabel: string;
  pendingLabel: string;
}) {
  return (
    <div
      class="flex items-center gap-2 text-[12px]"
      classList={{
        "text-[var(--color-success)]": props.granted,
        "text-[var(--color-ink-mute)]": !props.granted,
      }}
    >
      <Show
        when={props.granted}
        fallback={
          <svg
            width="12"
            height="12"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
          >
            <circle cx="12" cy="12" r="9" />
          </svg>
        }
      >
        <svg
          width="12"
          height="12"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2.5"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true"
        >
          <polyline points="4 12 10 18 20 6" />
        </svg>
      </Show>
      <span>{props.granted ? props.grantedLabel : props.pendingLabel}</span>
    </div>
  );
}
