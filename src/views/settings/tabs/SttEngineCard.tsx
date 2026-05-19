import { Show } from "solid-js";
import { RaisedCard } from "../components/RaisedCard";

// Wraps a group of STT model rows under a labeled card with the engine
// name, description, and (when applicable) a disabled-reason banner +
// a one-line caveat note. Mirrors the visual hierarchy of the Apple
// FM / Gemma cards on the LLM side so the two sections feel
// symmetrical.
export function SttEngineCard(props: {
  title: string;
  subtitle: string;
  recommendation: string;
  engineId: "parakeet-local" | "whisper-local";
  isActive: boolean;
  /** When false, the card is rendered disabled and the body (model
   *  rows) is hidden. */
  available: boolean;
  unavailableReason: string | null;
  /** Optional caveat shown beneath the engine description. */
  note: string | null;
  children: any;
}) {
  return (
    <RaisedCard disabled={!props.available}>
      <div class="flex items-start justify-between gap-3">
        <div class="flex-1">
          <div class="flex flex-wrap items-center gap-2">
            <span class="text-[13px] font-medium text-[var(--color-ink)]">
              {props.title}
            </span>
            <Show when={props.isActive && props.available}>
              <span class="rounded-full bg-[var(--color-accent)]/15 px-2 py-0.5 text-[10px] font-medium text-[var(--color-accent)]">
                Active
              </span>
            </Show>
            <Show when={!props.available}>
              <span class="rounded-full bg-[var(--color-ink-faint)]/15 px-2 py-0.5 text-[10px] font-medium text-[var(--color-ink-mute)]">
                Unavailable
              </span>
            </Show>
          </div>
          <p class="mt-1 text-[11px] text-[var(--color-ink-mute)]">
            {props.subtitle}
          </p>
          <p class="mt-0.5 text-[11px] text-[var(--color-ink-faint)]">
            {props.recommendation}
          </p>
          <Show when={!props.available && props.unavailableReason}>
            <p class="mt-1 text-[11px] text-[var(--color-danger)]">
              {props.unavailableReason}
            </p>
          </Show>
          <Show when={props.note && props.available}>
            <p class="mt-2 rounded-[6px] bg-[var(--color-ink-faint)]/10 px-2 py-1 text-[11px] text-[var(--color-ink-mute)]">
              Note: {props.note}
            </p>
          </Show>
        </div>
      </div>
      <Show when={props.available}>
        <div class="mt-3">{props.children}</div>
      </Show>
    </RaisedCard>
  );
}
