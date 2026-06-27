import { type ActiveModePayload } from "../types";

export function ModeHudView(props: { mode: ActiveModePayload }) {
  return (
    <div
      class="flex h-full w-full items-center justify-center gap-1.5 px-1 text-[12px] font-medium leading-none text-[var(--color-ink)]"
      aria-hidden="true"
    >
      <span class="shrink-0">{props.mode.icon}</span>
      <span class="truncate">{props.mode.name}</span>
    </div>
  );
}
