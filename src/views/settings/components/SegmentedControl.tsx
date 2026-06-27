import { For } from "solid-js";

export function SegmentedControl<T extends string>(props: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (v: T) => void;
  ariaLabel?: string;
}) {
  return (
    <div
      role="radiogroup"
      aria-label={props.ariaLabel}
      class="inline-flex shrink-0 items-center rounded-[8px] bg-[var(--color-control)] p-[2px] ring-1 ring-inset ring-[var(--color-border)]"
    >
      <For each={props.options}>
        {(o) => {
          const active = () => props.value === o.value;
          return (
            <button
              type="button"
              role="radio"
              aria-checked={active()}
              onClick={() => props.onChange(o.value)}
              class="rounded-[6px] px-3 py-[3px] text-[12px] font-medium leading-[16px] transition-all"
              classList={{
                "text-[var(--color-ink)]": active(),
                "text-[var(--color-ink-mute)] hover:text-[var(--color-ink)]":
                  !active(),
              }}
              style={
                active()
                  ? {
                    background: "var(--color-surface-raised)",
                    // Contained shadow: a 1px inset highlight + a 1px
                    // hairline drop. The full glass-shadow-stack-sm
                    // (which has a 22px outer blur) was leaking past
                    // the pill's 2px bottom padding and reading as
                    // asymmetric padding on the active segment.
                    "box-shadow":
                      "inset 0 1px 0 var(--glass-highlight), 0 1px 1px var(--glass-shadow-contact)",
                  }
                  : {}
              }
            >
              {o.label}
            </button>
          );
        }}
      </For>
    </div>
  );
}
