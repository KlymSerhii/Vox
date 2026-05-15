import { Show } from "solid-js";
import { RaisedCard } from "../components/RaisedCard";

export function GemmaEngineRow(props: { isActive: boolean; onUse: () => void }) {
  return (
    <RaisedCard>
      <div class="flex items-start justify-between gap-3">
        <div class="flex-1">
          <div class="flex items-center gap-2">
            <span class="text-[13px] font-medium text-[var(--color-ink)]">
              Gemma 4 (open-source)
            </span>
            <Show when={props.isActive}>
              <span class="rounded-full bg-[var(--color-accent)]/15 px-2 py-0.5 text-[10px] font-medium text-[var(--color-accent)]">
                Active
              </span>
            </Show>
            <Show when={!props.isActive}>
              <span class="rounded-full bg-[var(--color-accent)]/15 px-2 py-0.5 text-[10px] font-medium text-[var(--color-accent)]">
                Recommended
              </span>
            </Show>
          </div>
          <p class="mt-1 text-[11px] text-[var(--color-ink-mute)]">
            Runs offline once downloaded. Pick a model size below.
          </p>
        </div>
        <div class="flex shrink-0">
          <Show
            when={!props.isActive}
            fallback={
              <span class="rounded-full bg-[var(--color-accent)]/15 px-2 py-0.5 text-[10px] font-medium text-[var(--color-accent)]">
                Active
              </span>
            }
          >
            <button
              type="button"
              onClick={props.onUse}
              class="rounded-[8px] bg-[var(--color-control)] px-3 py-1 text-[11px] font-medium text-[var(--color-ink)] hover:bg-[var(--color-control-hover)]"
            >
              Use
            </button>
          </Show>
        </div>
      </div>
    </RaisedCard>
  );
}
