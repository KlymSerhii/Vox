import { Show } from "solid-js";
import { type AppleFmAvailability } from "@/lib/ipc";
import { isMac } from "@/lib/platform";
import { RaisedCard } from "../components/RaisedCard";

export function AppleFmEngineRow(props: {
  availability: AppleFmAvailability | null;
  isActive: boolean;
  onUse: () => void;
  onRefresh: () => void;
}) {
  const a = () => props.availability;
  const reachable = () => a()?.available ?? false;
  // Apple Intelligence is macOS-only. The parent already gates this row on
  // `appleFm()?.supported_host` (false on Windows), but guard here too so
  // the row can never render off-Mac regardless of caller.
  return (
    <Show when={isMac}>
    <RaisedCard>
      <div class="flex items-start justify-between gap-3">
        <div class="flex-1">
          <div class="flex items-center gap-2">
            <span class="text-[13px] font-medium text-[var(--color-ink)]">
              Apple Intelligence
            </span>
            <Show when={props.isActive}>
              <span class="rounded-full bg-[var(--color-accent)]/15 px-2 py-0.5 text-[10px] font-medium text-[var(--color-accent)]">
                Active
              </span>
            </Show>
          </div>
          <p class="mt-1 text-[11px] text-[var(--color-ink-mute)]">
            On-device, no download. Uses Apple's Foundation Models on
            macOS 26+ Apple Silicon Macs.
          </p>
          <Show when={!reachable() && a()}>
            <p class="mt-1 text-[11px] text-[var(--color-danger)]">
              Unavailable: {a()!.message}
            </p>
          </Show>
        </div>
        <div class="flex shrink-0 flex-col items-end gap-1">
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
              disabled={!reachable()}
              onClick={props.onUse}
              class="rounded-[8px] bg-[var(--color-accent)] px-3 py-1 text-[11px] font-medium text-[var(--color-on-accent)] hover:brightness-110 disabled:cursor-not-allowed disabled:opacity-40"
            >
              Use
            </button>
          </Show>
          <Show when={!reachable()}>
            <button
              type="button"
              onClick={props.onRefresh}
              class="rounded-[8px] bg-[var(--color-control)] px-3 py-1 text-[11px] font-medium text-[var(--color-ink)] hover:bg-[var(--color-control-hover)]"
            >
              Re-check
            </button>
          </Show>
        </div>
      </div>
    </RaisedCard>
    </Show>
  );
}
