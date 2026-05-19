import { For } from "solid-js";
import { STEP_COUNT, type Step } from "../constants";

export function Progress(props: { step: Step }) {
  return (
    <div class="flex items-center gap-2" aria-hidden="true">
      <For each={Array.from({ length: STEP_COUNT }, (_, i) => i)}>
        {(i) => (
          <span
            class="h-1.5 w-1.5 rounded-full transition-colors"
            classList={{
              "bg-[var(--color-accent)]": props.step === i,
              "bg-[var(--color-border-strong)]": props.step !== i,
            }}
          />
        )}
      </For>
    </div>
  );
}
