import { createResource, For } from "solid-js";
import { getAppVersion } from "@/lib/ipc";
import { deviceWord } from "@/lib/platform";
import { VoxLogo } from "@/components/VoxLogo";
import { TABS, type Tab } from "../constants";
import { TabIcon } from "./TabIcon";

export function Sidebar(props: { tab: Tab; onTab: (t: Tab) => void }) {
  const [version] = createResource(() => getAppVersion());
  return (
    <aside
      class="flex h-full w-[208px] flex-col border-r border-[var(--color-border)] px-3 pt-5 pb-3"
      style={{
        background: "var(--color-sidebar)",
        "backdrop-filter": "var(--glass-blur)",
        "-webkit-backdrop-filter": "var(--glass-blur)",
        "box-shadow":
          "inset 0 1px 0 var(--glass-highlight), 1px 0 0 var(--glass-edge-bottom)",
      }}
    >
      <div class="mb-5 flex items-center gap-2 px-2">
        <VoxLogo size={28} />
        <div class="flex flex-col">
          <div class="text-[14px] font-semibold tracking-[-0.01em] text-[var(--color-ink)]">
            Vox
          </div>
          <div class="text-[10px] uppercase tracking-[0.08em] text-[var(--color-ink-faint)]">
            On-device dictation
          </div>
        </div>
      </div>
      <nav class="flex flex-col gap-0.5">
        <For each={TABS}>
          {(t) => (
            <button
              type="button"
              onClick={() => props.onTab(t.id)}
              class="flex items-center gap-2.5 rounded-[8px] px-2.5 py-1.5 text-left text-[13px] font-medium transition-all"
              classList={{
                "text-[var(--color-ink)]": props.tab === t.id,
                "text-[var(--color-ink-mute)] hover:bg-[var(--color-overlay)] hover:text-[var(--color-ink)]":
                  props.tab !== t.id,
              }}
              style={
                props.tab === t.id
                  ? {
                    background: "var(--color-surface-raised)",
                    "backdrop-filter": "var(--glass-blur)",
                    "-webkit-backdrop-filter": "var(--glass-blur)",
                    "box-shadow": "var(--glass-shadow-stack-sm)",
                  }
                  : {}
              }
            >
              <TabIcon name={t.icon} active={props.tab === t.id} />
              <span>{t.label}</span>
            </button>
          )}
        </For>
      </nav>
      <div class="mt-auto flex items-center justify-between px-2 text-[10px] text-[var(--color-ink-faint)]">
        <span>Vox {version() ?? "—"}</span>
        <span
          class="rounded-full bg-[var(--color-success)]/15 px-1.5 py-[1px] text-[9px] font-medium uppercase tracking-[0.08em] text-[var(--color-success)]"
          title={`Audio and transcripts never leave this ${deviceWord}.`}
        >
          Local
        </span>
      </div>
    </aside>
  );
}
