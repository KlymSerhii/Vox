import { createSignal, For, onCleanup, onMount, Show } from "solid-js";
import {
  closeTrayPopover,
  getPublicSettings,
  getRecentTranscripts,
  listModes,
  onSettingsChanged,
  openSettingsWindow,
  quitApp,
  setActiveMode,
  setDictationEnabled,
  type RecentTranscript,
  type Settings,
  type VoiceMode,
} from "../lib/ipc";

export default function TrayPopoverView() {
  const [loaded, setLoaded] = createSignal(false);
  const [s, setS] = createSignal<Settings | null>(null);
  const [recents, setRecents] = createSignal<RecentTranscript[]>([]);
  const [copyFlash, setCopyFlash] = createSignal<number | null>(null);
  const [modes, setModes] = createSignal<VoiceMode[]>([]);
  const [modeMenuOpen, setModeMenuOpen] = createSignal(false);

  let unlistenSettings: (() => void) | null = null;

  onMount(async () => {
    document.body.classList.add("translucent");
    try {
      const [settings, rec, list] = await Promise.all([
        getPublicSettings(),
        getRecentTranscripts(),
        listModes(),
      ]);
      setS(settings);
      setRecents(rec);
      setModes(list);
    } finally {
      setLoaded(true);
    }

    // Live-update when the user saves changes from the Settings window
    // — keeps the tray popover's hotkey labels and mode badge in sync
    // without having to close/reopen.
    unlistenSettings = await onSettingsChanged((next) => {
      setS(next);
    });

    document.addEventListener("keydown", (e) => {
      if (e.key === "Escape") closeTrayPopover();
    });
  });

  onCleanup(() => {
    unlistenSettings?.();
  });

  async function pickMode(id: string) {
    const cur = s();
    if (!cur) return;
    setModeMenuOpen(false);
    try {
      await setActiveMode(id);
      setS({ ...cur, active_mode_id: id });
    } catch (err) {
      console.error("setActiveMode failed:", err);
    }
  }

  function activeMode(): VoiceMode | undefined {
    const id = s()?.active_mode_id;
    if (!id) return undefined;
    return modes().find((m) => m.id === id) ?? modes()[0];
  }

  async function toggleDictation(v: boolean) {
    const cur = s();
    if (!cur) return;
    const next = { ...cur, dictation_enabled: v };
    setS(next);
    // Use the narrow command — the tray popover is not a secret-bearing
    // window and must not call set_settings (which carries API keys).
    await setDictationEnabled(v);
  }

  async function copyRecent(idx: number, text: string) {
    try {
      await navigator.clipboard.writeText(text);
      setCopyFlash(idx);
      setTimeout(() => setCopyFlash(null), 900);
    } catch (e) {
      console.error("copy failed:", e);
    }
  }

  function statusLine(): { label: string; tone: "ok" | "off" } {
    const cur = s();
    if (!cur) return { label: "Loading…", tone: "ok" };
    if (!cur.dictation_enabled) return { label: "Dictation off", tone: "off" };
    return { label: "Local · ready", tone: "ok" };
  }

  return (
    <div
      class="flex h-screen w-screen flex-col overflow-hidden rounded-[14px] text-[13px] text-[var(--color-ink)]"
      style={{
        background: "rgba(24, 24, 27, 0.88)",
        "backdrop-filter": "blur(28px) saturate(180%)",
        "-webkit-backdrop-filter": "blur(28px) saturate(180%)",
        "box-shadow":
          "0 12px 32px rgba(0,0,0,0.45), inset 0 0 0 1px rgba(255,255,255,0.06)",
      }}
    >
      <Show
        when={loaded() && s()}
        fallback={
          <div class="flex h-full items-center justify-center text-[var(--color-ink-mute)]">
            Loading…
          </div>
        }
      >
        {(st) => (
          <>
            <header class="flex items-center justify-between border-b border-[var(--color-border)] px-4 py-3">
              <div class="flex items-center gap-2">
                <StatusDot tone={statusLine().tone} />
                <span class="text-[13px] font-medium">
                  {statusLine().label}
                </span>
              </div>
              <button
                type="button"
                class="rounded p-1 text-[var(--color-ink-mute)] hover:bg-white/8 hover:text-[var(--color-ink)]"
                title="Open Settings"
                onClick={() => openSettingsWindow()}
              >
                <svg
                  width="14"
                  height="14"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="1.75"
                >
                  <circle cx="12" cy="12" r="3" />
                  <path d="M19.4 15a7.97 7.97 0 0 0 .1-3l2-1.5-2-3.4-2.3.8a8 8 0 0 0-2.6-1.5L14 3h-4l-.6 2.4a8 8 0 0 0-2.6 1.5l-2.3-.8-2 3.4L4.5 12a8 8 0 0 0 .1 3l-2 1.5 2 3.4 2.3-.8a8 8 0 0 0 2.6 1.5l.6 2.4h4l.6-2.4a8 8 0 0 0 2.6-1.5l2.3.8 2-3.4z" />
                </svg>
              </button>
            </header>

            <div class="border-b border-[var(--color-border)] px-4 py-3">
              <div class="flex items-center justify-between">
                <div class="flex flex-col">
                  <span class="text-[13px] font-medium">Dictation</span>
                  <span class="text-[11px] text-[var(--color-ink-mute)]">
                    Hold <span class="keycap">⌥</span> to speak
                  </span>
                </div>
                <Toggle
                  checked={st().dictation_enabled}
                  onChange={toggleDictation}
                />
              </div>
            </div>

            <div class="relative border-b border-[var(--color-border)] px-4 py-3">
              <div class="flex items-center justify-between">
                <div class="flex flex-col">
                  <span class="text-[13px] font-medium">Voice mode</span>
                  <span class="text-[11px] text-[var(--color-ink-mute)]">
                    Cycle with{" "}
                    <span class="keycap">
                      {hotkeyLabel(st().mode_cycle_hotkey || "Alt+M")}
                    </span>
                  </span>
                </div>
                <button
                  type="button"
                  class="inline-flex items-center gap-1.5 rounded-full bg-zinc-800/70 px-2 py-1 text-[12px] ring-1 ring-white/10 hover:bg-zinc-700/70"
                  onClick={() => setModeMenuOpen(!modeMenuOpen())}
                  aria-haspopup="listbox"
                  aria-expanded={modeMenuOpen()}
                >
                  <Show when={activeMode()} fallback={<span>—</span>}>
                    {(m) => (
                      <>
                        <span aria-hidden="true">{m().icon}</span>
                        <span class="max-w-[120px] truncate">{m().name}</span>
                      </>
                    )}
                  </Show>
                  <svg
                    width="10"
                    height="10"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
                    aria-hidden="true"
                  >
                    <polyline points="6 9 12 15 18 9" />
                  </svg>
                </button>
              </div>
              <Show when={modeMenuOpen()}>
                <div
                  class="absolute right-4 top-full z-10 mt-1 max-h-60 w-56 overflow-y-auto rounded-[10px] bg-[rgba(24,24,27,0.96)] py-1 shadow-lg ring-1 ring-white/10 scrollbar-thin"
                  role="listbox"
                >
                  <For each={modes()}>
                    {(m) => (
                      <button
                        type="button"
                        role="option"
                        aria-selected={m.id === st().active_mode_id}
                        class="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[12px] hover:bg-white/6"
                        classList={{
                          "text-[var(--color-accent)]":
                            m.id === st().active_mode_id,
                          "text-[var(--color-ink)]":
                            m.id !== st().active_mode_id,
                        }}
                        onClick={() => pickMode(m.id)}
                      >
                        <span aria-hidden="true">{m.icon}</span>
                        <span class="flex-1 truncate">{m.name}</span>
                        <Show when={m.id === st().active_mode_id}>
                          <span class="text-[10px]">●</span>
                        </Show>
                      </button>
                    )}
                  </For>
                </div>
              </Show>
            </div>

            <div class="flex-1 overflow-y-auto scrollbar-thin px-2 py-2">
              <div class="mb-1 px-2 text-[10px] font-semibold uppercase tracking-wider text-[var(--color-ink-faint)]">
                Recent (cleared on quit)
              </div>
              <Show
                when={recents().length > 0}
                fallback={
                  <div class="px-2 py-4 text-[12px] text-[var(--color-ink-mute)]">
                    Your last 5 transcripts will appear here.
                  </div>
                }
              >
                <For each={recents()}>
                  {(r, idx) => (
                    <div class="group flex items-center gap-2 rounded-[8px] px-2 py-2 hover:bg-white/4">
                      <span class="w-10 shrink-0 text-[11px] text-[var(--color-ink-faint)]">
                        {ago(r.ts)}
                      </span>
                      <span class="flex-1 truncate text-[12px] text-[var(--color-ink)]">
                        {r.text}
                      </span>
                      <button
                        type="button"
                        class="rounded p-1 text-[var(--color-ink-mute)] opacity-0 transition-opacity hover:bg-white/8 hover:text-[var(--color-ink)] group-hover:opacity-100"
                        onClick={() => copyRecent(idx(), r.text)}
                        title="Copy"
                      >
                        <Show
                          when={copyFlash() === idx()}
                          fallback={
                            <svg
                              width="14"
                              height="14"
                              viewBox="0 0 24 24"
                              fill="none"
                              stroke="currentColor"
                              stroke-width="1.75"
                              stroke-linecap="round"
                              stroke-linejoin="round"
                            >
                              <rect x="9" y="9" width="12" height="12" rx="2" />
                              <path d="M5 15V5a2 2 0 0 1 2-2h10" />
                            </svg>
                          }
                        >
                          <span class="text-[var(--color-success)] text-[11px]">
                            ✓
                          </span>
                        </Show>
                      </button>
                    </div>
                  )}
                </For>
              </Show>
            </div>

            <div class="border-t border-[var(--color-border)] px-4 py-2 text-[11px] text-[var(--color-ink-mute)]">
              <div class="flex items-center justify-between">
                <span>Hotkey</span>
                <span class="keycap">
                  {hotkeyLabel(st().hotkey)}
                </span>
              </div>
            </div>

            <footer class="flex border-t border-[var(--color-border)]">
              <button
                type="button"
                class="flex-1 py-3 text-[12px] text-[var(--color-ink)] hover:bg-white/4"
                onClick={() => openSettingsWindow()}
              >
                Settings…
              </button>
              <div class="w-px bg-[var(--color-border)]" />
              <button
                type="button"
                class="flex-1 py-3 text-[12px] text-[var(--color-ink-mute)] hover:bg-white/4 hover:text-[var(--color-danger)]"
                onClick={() => quitApp()}
              >
                Quit
              </button>
            </footer>
          </>
        )}
      </Show>
    </div>
  );
}

function StatusDot(props: { tone: "ok" | "off" }) {
  const color =
    props.tone === "ok" ? "var(--color-success)" : "var(--color-ink-faint)";
  return (
    <span
      class="inline-block h-2 w-2 rounded-full"
      style={{ background: color }}
    />
  );
}

function Toggle(props: { checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={props.checked}
      onClick={() => props.onChange(!props.checked)}
      class="relative inline-flex h-5 w-9 items-center rounded-full transition-colors"
      classList={{
        "bg-[var(--color-accent)]": props.checked,
        "bg-white/10": !props.checked,
      }}
    >
      <span
        class="inline-block h-4 w-4 transform rounded-full bg-white transition-transform"
        classList={{
          "translate-x-4": props.checked,
          "translate-x-0.5": !props.checked,
        }}
      />
    </button>
  );
}

function ago(tsSeconds: number): string {
  const now = Date.now() / 1000;
  const delta = Math.max(0, Math.floor(now - tsSeconds));
  if (delta < 60) return `${delta}s`;
  if (delta < 3600) return `${Math.floor(delta / 60)}m`;
  if (delta < 86_400) return `${Math.floor(delta / 3600)}h`;
  return `${Math.floor(delta / 86_400)}d`;
}

function hotkeyLabel(k: string): string {
  switch (k) {
    case "AltRight":
      return "⌥";
    case "AltLeft":
      return "⌥L";
    case "F13":
      return "F13";
    case "F14":
      return "F14";
    case "F15":
      return "F15";
    case "CommandOrControl+Shift+Space":
    case "Cmd+Shift+Space":
      return "⌘⇧Space";
    case "Alt+M":
      return "⌥M";
    case "Alt+Shift+M":
      return "⌥⇧M";
    case "CommandOrControl+Shift+M":
    case "Cmd+Shift+M":
      return "⌘⇧M";
    default:
      return k;
  }
}
