import { Show } from "solid-js";
import { type PolishChoice } from "../constants";
import { RadioCard } from "./RadioCard";

export function PolishEngineCard(props: {
  engine: PolishChoice;
  title: string;
  subtitle: string;
  recommended: boolean;
  /// Optional caveat pill rendered below the subtitle. Hidden when the
  /// card is disabled — the disabled-reason message takes precedence.
  note: string | null;
  disabled: boolean;
  disabledReason: string | null;
  selected: boolean;
  onSelect: () => void;
}) {
  // The badges row should only render when at least one pill is visible.
  // We mirror the original `Show when=...` condition by leaving the slot
  // undefined when nothing would show.
  const anyBadge = () =>
    (props.recommended && !props.disabled) ||
    (!!props.note && !props.disabled) ||
    (props.disabled && !!props.disabledReason);
  return (
    <RadioCard
      selected={props.selected}
      onSelect={props.onSelect}
      disabled={props.disabled}
      title={props.title}
      subtitle={props.subtitle}
      badges={
        anyBadge() ? (
          <>
            <Show when={props.recommended && !props.disabled}>
              <span class="rounded-full bg-[var(--color-accent)]/15 px-2 py-0.5 text-[10px] font-medium text-[var(--color-accent)]">
                Recommended
              </span>
            </Show>
            <Show when={props.note && !props.disabled}>
              <span class="inline-flex items-center gap-1 rounded-full bg-[var(--color-ink-faint)]/15 px-2 py-0.5 text-[10px] font-medium text-[var(--color-ink-mute)]">
                <svg
                  width="10"
                  height="10"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2.5"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  aria-hidden="true"
                >
                  <circle cx="12" cy="12" r="10" />
                  <line x1="12" y1="11" x2="12" y2="17" />
                  <line x1="12" y1="7.5" x2="12" y2="7.51" />
                </svg>
                {props.note}
              </span>
            </Show>
            <Show when={props.disabled && props.disabledReason}>
              <span class="rounded-full bg-[var(--color-ink-faint)]/15 px-2 py-0.5 text-[10px] font-medium text-[var(--color-ink-mute)]">
                {props.disabledReason}
              </span>
            </Show>
          </>
        ) : undefined
      }
    />
  );
}
