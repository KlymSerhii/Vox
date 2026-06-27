import { Show } from "solid-js";
import { type ModelStatus } from "@/lib/ipc";
import { RaisedCard } from "../components/RaisedCard";
import { formatBytes, modelBlurb } from "../helpers";
import { useConfirm } from "../useConfirm";

export function ModelRow(props: {
  m: ModelStatus;
  isActive: boolean;
  progress?: { downloaded: number; total: number; eta: number | null };
  downloading: boolean;
  onUse: () => void;
  onDownload: () => void;
  onCancel: () => void;
  onDelete: () => void;
}) {
  const sizeMb = () => Math.round(props.m.descriptor.size_bytes / 1_000_000);
  const ramMb = () => props.m.descriptor.ram_peak_mb;
  const blurb = () => modelBlurb(props.m.descriptor.id);

  // Two-step confirm. `window.confirm` is a silent no-op in Tauri's WKWebView
  // so the delete never fired; `useConfirm` arms an inline confirm state that
  // auto-reverts after 5 s.
  const confirmDelete = useConfirm();

  return (
    <RaisedCard size="sm" class="p-4">
      <div class="flex items-start justify-between gap-3">
        <div class="flex-1">
          <div class="flex items-center gap-2">
            <span class="text-[13px] font-medium">{props.m.descriptor.name}</span>
            <Show when={props.isActive}>
              <span class="rounded-full bg-[var(--color-accent)]/20 px-2 py-0.5 text-[10px] text-[var(--color-accent)]">
                Active
              </span>
            </Show>
          </div>
          <Show when={blurb()}>
            <div class="mt-1 text-[12px] text-[var(--color-ink)]/85">
              {blurb()}
            </div>
          </Show>
          <div class="mt-0.5 text-[11px] text-[var(--color-ink-mute)]">
            ~{sizeMb()} MB on disk · ~{ramMb()} MB peak RAM · tier:{" "}
            {props.m.descriptor.tier}
          </div>
        </div>
        <div class="flex items-center gap-2">
          <Show when={props.m.installed && !props.isActive && !confirmDelete.armed()}>
            <button
              type="button"
              class="rounded-[6px] bg-[var(--color-accent)]/80 px-3 py-1.5 text-[12px] font-medium text-[var(--color-on-accent)] hover:bg-[var(--color-accent)]"
              onClick={props.onUse}
            >
              Use
            </button>
          </Show>
          <Show when={!props.m.installed && !props.downloading}>
            <button
              type="button"
              class="rounded-[6px] bg-[var(--color-accent)] px-3 py-1.5 text-[12px] font-medium text-[var(--color-on-accent)] hover:brightness-110"
              onClick={props.onDownload}
            >
              Download
            </button>
          </Show>
          <Show when={props.downloading}>
            <button
              type="button"
              class="rounded-[6px] bg-[var(--color-control)] px-3 py-1.5 text-[12px] hover:bg-[var(--color-control-hover)]"
              onClick={props.onCancel}
            >
              Cancel
            </button>
          </Show>
          <Show when={props.m.installed && !confirmDelete.armed()}>
            <button
              type="button"
              class="rounded-[6px] bg-[var(--color-control)] px-3 py-1.5 text-[12px] text-[var(--color-ink-mute)] hover:bg-[var(--color-danger)]/20 hover:text-[var(--color-danger)]"
              onClick={() => confirmDelete.armOrFire(props.onDelete)}
              title="Delete cached model"
            >
              Delete
            </button>
          </Show>
          <Show when={props.m.installed && confirmDelete.armed()}>
            <span class="text-[11px] text-[var(--color-ink-mute)]">
              Free ~{formatBytes(props.m.descriptor.size_bytes)}?
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
              title="Permanently delete the cached file"
            >
              Delete
            </button>
          </Show>
        </div>
      </div>
      <Show when={props.downloading && props.progress}>
        {(p) => (
          <div class="mt-3">
            <div class="flex justify-between text-[11px] text-[var(--color-ink-mute)]">
              <span>
                {formatBytes(p().downloaded)} / {formatBytes(p().total)}
              </span>
              <Show when={p().eta != null}>
                <span>ETA {p().eta}s</span>
              </Show>
            </div>
            <div class="mt-1 h-1.5 w-full overflow-hidden rounded-full bg-[var(--color-control)]">
              <div
                class="h-full rounded-full bg-[var(--color-accent)]"
                style={{
                  width: `${p().total > 0 ? Math.min(100, (p().downloaded / p().total) * 100) : 0}%`,
                }}
              />
            </div>
          </div>
        )}
      </Show>
    </RaisedCard>
  );
}
