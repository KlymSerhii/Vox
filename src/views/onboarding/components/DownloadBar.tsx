import { Show } from "solid-js";
import { type ProgressSample } from "../constants";
import { etaSeconds, formatEta } from "../helpers";

export function DownloadBar(props: {
  label: string;
  done: boolean;
  prog?: { downloaded: number; total: number };
  samples?: ProgressSample[];
}) {
  const pct = () => {
    if (props.done) return 100;
    if (!props.prog) return 0;
    if (props.prog.total <= 0) return 0;
    return Math.min(100, (props.prog.downloaded / props.prog.total) * 100);
  };
  const eta = () => {
    if (props.done) return null;
    return etaSeconds(props.prog, props.samples);
  };
  return (
    <div>
      <div class="mb-1 flex justify-between text-[11px] text-[var(--color-ink-mute)]">
        <span>{props.label}</span>
        <span>{props.done ? "Done" : `${pct().toFixed(0)}%`}</span>
      </div>
      <div class="h-1.5 w-full overflow-hidden rounded-full bg-[var(--color-control)]">
        <div
          class="h-full rounded-full bg-[var(--color-accent)] transition-[width]"
          style={{ width: `${pct()}%` }}
        />
      </div>
      <Show when={eta() !== null}>
        <div class="mt-1 text-[10px] text-[var(--color-ink-faint)]">
          {formatEta(eta()!)}
        </div>
      </Show>
    </div>
  );
}
