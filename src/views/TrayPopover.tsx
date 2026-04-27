import { createSignal, For, onCleanup, onMount, Show } from "solid-js";
import {
  closeTrayPopover,
  copyText,
  getPublicSettings,
  getRecentTranscripts,
  listModes,
  onSettingsChanged,
  onTranscriptsChanged,
  openSettingsWindow,
  quitApp,
  setActiveMode,
  setAppearance,
  setHandsFreeEnabled,
  setPolishingEnabled,
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
  let unlistenTranscripts: (() => void) | null = null;

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

    // The popover webview is built once and reused on every tray click —
    // `onMount` only runs the first time. Subscribe to backend events so
    // the recents list and settings stay live across reopens.
    unlistenSettings = await onSettingsChanged((next) => {
      setS(next);
    });
    unlistenTranscripts = await onTranscriptsChanged((rec) => {
      setRecents(rec);
    });

    // Backstop: refetch on focus, in case the popover was hidden when
    // an event fired (e.g. earlier app launches that pre-date this code).
    window.addEventListener("focus", () => {
      void getRecentTranscripts().then(setRecents);
    });

    document.addEventListener("keydown", (e) => {
      if (e.key === "Escape") closeTrayPopover();
    });
  });

  onCleanup(() => {
    unlistenSettings?.();
    unlistenTranscripts?.();
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

  async function toggleHandsFree(v: boolean) {
    const cur = s();
    if (!cur) return;
    const next = { ...cur, hands_free_enabled: v };
    setS(next);
    await setHandsFreeEnabled(v);
  }

  async function togglePolishing(v: boolean) {
    const cur = s();
    if (!cur) return;
    const next = {
      ...cur,
      polishing_enabled: v,
      llm_engine: v ? "gemma-local" : "none",
    };
    setS(next);
    await setPolishingEnabled(v);
  }

  async function copyRecent(idx: number, text: string) {
    try {
      await copyText(text);
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
        background: "var(--color-surface-glass)",
        "backdrop-filter": "var(--glass-blur)",
        "-webkit-backdrop-filter": "var(--glass-blur)",
        border: "1px solid var(--glass-rim)",
        "box-shadow": "var(--glass-shadow-stack)",
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
                class="rounded p-1 text-[var(--color-ink-mute)] hover:bg-[var(--color-overlay)] hover:text-[var(--color-ink)]"
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
                  <span class="text-[13px] font-medium">
                    {st().hands_free_enabled ? "Press to talk" : "Hold to talk"}
                  </span>
                  <span class="text-[11px] text-[var(--color-ink-mute)]">
                    {st().hands_free_enabled
                      ? "Press the hotkey to start, press again to stop."
                      : "Hold the hotkey while speaking, release to stop."}
                  </span>
                </div>
                <Toggle
                  checked={st().hands_free_enabled}
                  onChange={toggleHandsFree}
                />
              </div>
            </div>

            <div class="border-b border-[var(--color-border)] px-4 py-3">
              <div class="flex items-center justify-between">
                <div class="flex flex-col">
                  <span class="text-[13px] font-medium">Polish transcripts</span>
                  <span class="text-[11px] text-[var(--color-ink-mute)]">
                    {st().polishing_enabled
                      ? "Gemma cleans up the transcript before copying."
                      : "Raw output is copied as-is."}
                  </span>
                </div>
                <Toggle
                  checked={st().polishing_enabled}
                  onChange={togglePolishing}
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
                  class="inline-flex items-center gap-1.5 rounded-full px-2 py-1 text-[12px] ring-1 ring-[var(--color-border-strong)] hover:brightness-110"
                  style={{ background: "var(--color-surface-raised)" }}
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
                  class="absolute right-4 top-full z-10 mt-1 max-h-60 w-56 overflow-y-auto rounded-[10px] py-1 shadow-lg ring-1 ring-[var(--color-border-strong)] scrollbar-thin"
                  style={{
                    background: "var(--color-surface-raised)",
                    "backdrop-filter": "blur(28px) saturate(180%)",
                    "-webkit-backdrop-filter": "blur(28px) saturate(180%)",
                  }}
                  role="listbox"
                >
                  <For each={modes()}>
                    {(m) => (
                      <button
                        type="button"
                        role="option"
                        aria-selected={m.id === st().active_mode_id}
                        class="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[12px] hover:bg-[var(--color-overlay)]"
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
                    <div class="group flex items-center gap-2 rounded-[8px] px-2 py-2 hover:bg-[var(--color-overlay)]">
                      <span class="w-10 shrink-0 text-[11px] text-[var(--color-ink-faint)]">
                        {ago(r.ts)}
                      </span>
                      <span class="flex-1 truncate text-[12px] text-[var(--color-ink)]">
                        {r.text}
                      </span>
                      <button
                        type="button"
                        class="rounded p-1 text-[var(--color-ink-mute)] hover:bg-[var(--color-overlay)] hover:text-[var(--color-ink)]"
                        onClick={() => copyRecent(idx(), r.text)}
                        title="Copy to clipboard (⌘V to paste)"
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

            <div class="border-t border-[var(--color-border)] px-4 py-3">
              <div class="flex items-center justify-between">
                <span class="text-[12px] text-[var(--color-ink-mute)]">
                  Appearance
                </span>
                <AppearanceToggle
                  value={st().appearance}
                  onChange={(v) => {
                    const cur = s();
                    if (cur) setS({ ...cur, appearance: v });
                    void setAppearance(v);
                  }}
                />
              </div>
            </div>
            <footer class="flex border-t border-[var(--color-border)]">
              <button
                type="button"
                class="flex-1 py-3 text-[12px] text-[var(--color-ink-mute)] hover:bg-[var(--color-overlay)] hover:text-[var(--color-danger)]"
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
        "bg-[var(--color-control)]": !props.checked,
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

function AppearanceToggle(props: {
  value: "light" | "dark";
  onChange: (v: "light" | "dark") => void;
}) {
  return (
    <div
      class="inline-flex items-center rounded-[12px] p-0.5"
      style={{ background: "var(--color-control)" }}
      role="radiogroup"
      aria-label="Appearance"
    >
      <AppearanceHalf
        active={props.value === "light"}
        label="Light"
        onClick={() => props.onChange("light")}
      >
        <SunGlyph />
      </AppearanceHalf>
      <AppearanceHalf
        active={props.value === "dark"}
        label="Dark"
        onClick={() => props.onChange("dark")}
      >
        <MoonGlyph />
      </AppearanceHalf>
    </div>
  );
}

function AppearanceHalf(props: {
  active: boolean;
  label: string;
  onClick: () => void;
  children: any;
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={props.active}
      aria-label={props.label}
      class="inline-flex h-6 w-8 items-center justify-center rounded-[10px] transition-colors"
      classList={{
        "hover:text-[var(--color-ink-mute)]": !props.active,
      }}
      style={{
        background: props.active ? "var(--color-surface-raised)" : "transparent",
        "box-shadow": props.active
          ? "var(--glass-shadow-stack-sm)"
          : "none",
        color: props.active
          ? "var(--color-ink)"
          : "var(--color-ink-faint)",
      }}
      onClick={props.onClick}
    >
      {props.children}
    </button>
  );
}

function SunGlyph() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="1.5"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <circle cx="12" cy="12" r="3.5" />
      <line x1="12" y1="2" x2="12" y2="4" />
      <line x1="12" y1="20" x2="12" y2="22" />
      <line x1="2" y1="12" x2="4" y2="12" />
      <line x1="20" y1="12" x2="22" y2="12" />
      <line x1="4.9" y1="4.9" x2="6.3" y2="6.3" />
      <line x1="17.7" y1="17.7" x2="19.1" y2="19.1" />
      <line x1="4.9" y1="19.1" x2="6.3" y2="17.7" />
      <line x1="17.7" y1="6.3" x2="19.1" y2="4.9" />
    </svg>
  );
}

function MoonGlyph() {
  return (
    <svg
      width="14"
      height="14"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="1.5"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <path d="M21 12.8A9 9 0 1 1 11.2 3a7 7 0 0 0 9.8 9.8z" />
    </svg>
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
