import { Show } from "solid-js";
import { type VoiceMode } from "@/lib/ipc";
import { RaisedCard } from "../components/RaisedCard";
import { useConfirm } from "../useConfirm";
import { ModeOverflowMenu } from "./ModeOverflowMenu";

export function ModeRow(props: {
  m: VoiceMode;
  isActive: boolean;
  onActivate: () => void;
  onEdit: () => void;
  onDuplicate: () => void;
  onDelete: () => void;
  onExport: () => void;
}) {
  const confirmDelete = useConfirm();
  return (
    <RaisedCard size="sm" class="px-5 py-4">
      <div class="flex items-start justify-between gap-4">
        <div class="flex min-w-0 flex-1 flex-col gap-1">
          <div class="flex flex-wrap items-center gap-x-2 gap-y-1">
            <span class="text-[16px]" aria-hidden="true">
              {props.m.icon}
            </span>
            <span class="text-[13px] font-medium">{props.m.name}</span>
            <Show when={props.m.built_in}>
              <span class="rounded-full bg-[var(--color-control)] px-2 py-0.5 text-[10px] text-[var(--color-ink-mute)]">
                Default
              </span>
            </Show>
            <Show when={!props.m.built_in}>
              <span class="rounded-full bg-[var(--color-accent)]/20 px-2 py-0.5 text-[10px] text-[var(--color-accent)]">
                Custom
              </span>
            </Show>
            <Show when={props.isActive}>
              <span class="rounded-full bg-[var(--color-success)]/20 px-2 py-0.5 text-[10px] text-[var(--color-success)]">
                Active
              </span>
            </Show>
          </div>
          <div class="text-[11px] leading-snug text-[var(--color-ink-mute)]">
            {props.m.description || "No description."}
          </div>
        </div>
        <div class="flex shrink-0 items-center gap-1.5">
          <Show when={!props.isActive && !confirmDelete.armed()}>
            <button
              type="button"
              class="rounded-[6px] bg-[var(--color-accent)]/80 px-3 py-1.5 text-[12px] font-medium text-[var(--color-on-accent)] hover:bg-[var(--color-accent)]"
              onClick={props.onActivate}
            >
              Use
            </button>
          </Show>
          <Show when={props.m.built_in && !confirmDelete.armed()}>
            <button
              type="button"
              class="rounded-[6px] bg-[var(--color-control)] px-3 py-1.5 text-[12px] hover:bg-[var(--color-control-hover)]"
              onClick={props.onDuplicate}
              title="Make a custom copy you can edit"
            >
              Duplicate
            </button>
          </Show>
          <Show when={!props.m.built_in && !confirmDelete.armed()}>
            <button
              type="button"
              class="rounded-[6px] bg-[var(--color-control)] px-3 py-1.5 text-[12px] hover:bg-[var(--color-control-hover)]"
              onClick={props.onEdit}
            >
              Edit
            </button>
            <ModeOverflowMenu
              onExport={props.onExport}
              onDuplicate={props.onDuplicate}
              onDelete={() => confirmDelete.armOrFire(props.onDelete)}
            />
          </Show>
          <Show when={confirmDelete.armed()}>
            <span class="text-[11px] text-[var(--color-ink-mute)]">
              Delete this mode?
            </span>
            <button
              type="button"
              class="rounded-[6px] bg-[var(--color-control)] px-3 py-1.5 text-[12px] hover:bg-[var(--color-control-hover)]"
              onClick={confirmDelete.disarm}
            >
              Keep
            </button>
            <button
              type="button"
              class="rounded-[6px] bg-[var(--color-danger)] px-3 py-1.5 text-[12px] font-medium text-white hover:brightness-110"
              onClick={() => confirmDelete.armOrFire(props.onDelete)}
            >
              Delete
            </button>
          </Show>
        </div>
      </div>
    </RaisedCard>
  );
}
