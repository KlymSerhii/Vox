import { Show } from "solid-js";
import { type ModelStatus } from "@/lib/ipc";
import { deviceWord } from "@/lib/platform";
import { RadioCard } from "./RadioCard";

/// Generic model card used for both Whisper and Gemma rows. Sizing is
/// derived from `ModelDescriptor.size_bytes`, so MB-class Whisper
/// models render their disk footprint in MB rather than "0.1 GB".
export function ModelCard(props: {
  model: ModelStatus;
  selected: boolean;
  recommended: boolean;
  swapWarn: boolean;
  onSelect: () => void;
}) {
  const d = () => props.model.descriptor;
  const displayName = () => d().name.replace(/\s*\([^)]*\)/g, "");
  const sizeLabel = () => {
    const bytes = d().size_bytes;
    if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB on disk`;
    return `${Math.round(bytes / 1e6)} MB on disk`;
  };
  const ramLabel = () => {
    const mb = d().ram_peak_mb;
    if (mb >= 1024) return `${(mb / 1024).toFixed(1)} GB peak RAM`;
    return `${mb} MB peak RAM`;
  };
  const anyBadge = () =>
    props.recommended || props.swapWarn || props.model.installed;
  return (
    <RadioCard
      selected={props.selected}
      onSelect={props.onSelect}
      liquidCard={false}
      title={displayName()}
      subtitle={`~${sizeLabel()} · ~${ramLabel()}`}
      badges={
        anyBadge() ? (
          <>
            <Show when={props.recommended}>
              <span class="rounded-full bg-[var(--color-accent)]/15 px-2 py-0.5 text-[10px] font-medium text-[var(--color-accent)]">
                Recommended
              </span>
            </Show>
            <Show when={props.model.installed}>
              <span class="rounded-full bg-[var(--color-success)]/15 px-2 py-0.5 text-[10px] font-medium text-[var(--color-success)]">
                Installed
              </span>
            </Show>
            <Show when={props.swapWarn}>
              <span class="rounded-full bg-[var(--color-danger)]/15 px-2 py-0.5 text-[10px] font-medium text-[var(--color-danger)]">
                May swap on this {deviceWord}
              </span>
            </Show>
          </>
        ) : undefined
      }
    />
  );
}
