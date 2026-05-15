import { type JSX, Show } from "solid-js";
import { RadioCard } from "./RadioCard";

export function LanguageCard(props: {
  title: string;
  subtitle: string;
  recommended: boolean;
  engine: "parakeet-local" | "whisper-local";
  selected: boolean;
  onSelect: () => void;
  children?: JSX.Element;
}) {
  const engineLabel = () =>
    props.engine === "parakeet-local" ? "Parakeet" : "Whisper";
  return (
    <RadioCard
      selected={props.selected}
      onSelect={props.onSelect}
      title={props.title}
      subtitle={props.subtitle}
      badges={
        <>
          <Show when={props.recommended}>
            <span class="rounded-full bg-[var(--color-accent)]/15 px-2 py-0.5 text-[10px] font-medium text-[var(--color-accent)]">
              Recommended
            </span>
          </Show>
          <span class="rounded-full bg-[var(--color-ink-faint)]/15 px-2 py-0.5 text-[10px] font-medium text-[var(--color-ink-mute)]">
            {engineLabel()}
          </span>
        </>
      }
    >
      {props.children}
    </RadioCard>
  );
}
